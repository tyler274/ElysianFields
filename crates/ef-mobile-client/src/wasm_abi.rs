//! Zero-dependency `wasm32-unknown-unknown` `cdylib` ABI for Curie Mobile.
//!
//! Wraps the real Bevy `0.20.1` [`CurieMobileClientPlugin`] + [`EfUiCatalogPlugin`]
//! inside a pure-Rust runtime (`MobileWasmRuntime`) that runs both on native hosts
//! (for deterministic unit tests) and as exported `extern "C"` Wasm symbols under
//! strict browser CSP (`script-src 'self' 'wasm-unsafe-eval'`).
//!
//! Strictly product-isolated from Curie Clinic (`ef-clinic-*` is forbidden by `ef-boundaries`).

use bevy::prelude::App;
use ef_mobile_domain::ConnectivityState;
use ef_ui_catalog::{
    A11yProjection, A11ySemanticKey, AriaRole, ComposerAction, ComposerSurface, IdleRedrawGate,
    Locale as UiLocaleKind, NodeAvailability, NodeInteractivity, PoliteLiveQueue,
    ProjectedA11yNode, RedrawReason, TextDirection, UiLocale,
};
use serde::{Deserialize, Serialize};

use crate::plugin::{
    CurieMobileClientPlugin, MobileClientCommand, MobileClientCommandQueue, MobileClientState,
    MobileTab,
};

/// Current binary ABI contract version exported by `ef_mobile_wasm_version`.
pub const MOBILE_WASM_ABI_VERSION: u32 = 1;

/// Canonical runtime identifier exposed on `document.documentElement.dataset.efWasmRuntime`.
pub const BEVY_RUST_WASM_RUNTIME_ID: &str = "bevy-rust-wasm";

/// Canonical mobile mount subpath prefix under `/elysianfields/mobile`.
pub const MOBILE_MOUNT_SUBPATH: &str = "/elysianfields/mobile";

/// Packs a `(ptr, len)` pair into a single Wasm `u64` (`(ptr << 32) | len`).
#[must_use]
pub const fn pack_ptr_len(ptr: u32, len: u32) -> u64 {
    ((ptr as u64) << 32) | (len as u64)
}

/// Unpacks a `(ptr << 32) | len` Wasm `u64` return value.
#[must_use]
pub const fn unpack_ptr_len(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32)
}

/// Typed tab selector accepted across the JSON Wasm boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileWasmTab {
    Home,
    Records,
    HealthTrends,
    ForMyDoctor,
    Settings,
}

impl From<MobileWasmTab> for MobileTab {
    fn from(tab: MobileWasmTab) -> Self {
        match tab {
            MobileWasmTab::Home => Self::Home,
            MobileWasmTab::Records => Self::Records,
            MobileWasmTab::HealthTrends => Self::HealthTrends,
            MobileWasmTab::ForMyDoctor => Self::ForMyDoctor,
            MobileWasmTab::Settings => Self::Settings,
        }
    }
}

impl From<MobileTab> for MobileWasmTab {
    fn from(tab: MobileTab) -> Self {
        match tab {
            MobileTab::Home => Self::Home,
            MobileTab::Records => Self::Records,
            MobileTab::HealthTrends => Self::HealthTrends,
            MobileTab::ForMyDoctor => Self::ForMyDoctor,
            MobileTab::Settings => Self::Settings,
        }
    }
}

/// Typed command dispatched across the JSON Wasm boundary (`ef_mobile_wasm_dispatch_json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum MobileWasmCommand {
    /// Returns the current snapshot without mutating state.
    Snapshot,
    /// Switches the active UI locale (`"ar"` -> Arabic RTL, `"en"` -> English LTR, etc.).
    SetLocale { lang: String },
    /// Switches the active bottom-bar mobile tab.
    SwitchTab { tab: MobileWasmTab },
    /// Updates the single-owner `For my doctor` curated note inside `ComposerSurface`.
    EditDraft { text: String },
    /// Updates online/offline connectivity state.
    SetConnectivity { online: bool },
    /// Updates mobile soft-keyboard inset height in logical pixels.
    SetSoftKeyboardInset { bottom_inset_px: f32 },
    /// Executes one idle Bevy `App::update()` tick verifying zero idle redraws.
    IdleTick,
}

/// Serializable descriptor for one AccessKit node consumed directly by `a11y-bridge.js`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileWasmA11yNodeDescriptor {
    pub node_id: String,
    pub control_key: String,
    pub role: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub disabled: bool,
    pub modal: bool,
    pub tab_order: String,
}

/// Serializable summary of `IdleRedrawGate` counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileWasmIdleRedrawSummary {
    pub frames_rendered: u64,
    pub idle_ticks_skipped: u64,
    pub frames_drawn_state: u64,
}

/// Complete JSON snapshot returned from `ef_mobile_wasm_init` and `ef_mobile_wasm_dispatch_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileWasmSnapshot {
    pub ok: bool,
    pub error: Option<String>,
    pub runtime: String,
    pub product: String,
    pub abi_version: u32,
    pub mount_subpath: String,
    pub locale: String,
    pub direction: String,
    pub active_tab: MobileWasmTab,
    pub connectivity: String,
    pub draft_text: String,
    pub owed_sign_outs: usize,
    pub a11y_tree_generation: u64,
    pub a11y_node_count: usize,
    pub duplicate_a11y_nodes: usize,
    pub idle_redraw: MobileWasmIdleRedrawSummary,
    pub a11y_nodes: Vec<MobileWasmA11yNodeDescriptor>,
    pub polite_live: Option<String>,
}

/// Pure-Rust runtime hosting the real Bevy `App` + [`CurieMobileClientPlugin`].
pub struct MobileWasmRuntime {
    app: App,
}

impl MobileWasmRuntime {
    /// Boots a real Bevy `0.20.1` `App` with [`CurieMobileClientPlugin`] and populates
    /// the single-owner AccessKit accessibility projection.
    #[must_use]
    pub fn new() -> Self {
        let mut app = App::new();
        app.add_plugins(CurieMobileClientPlugin);

        // First update spawns ManagedMobileA11yNode entities; second projects them into A11yProjection.
        app.update();
        app.update();

        Self { app }
    }

    /// Dispatches one JSON-encoded [`MobileWasmCommand`] and returns the serialized JSON snapshot.
    #[must_use]
    pub fn dispatch_json(&mut self, raw_json: &str) -> String {
        let outcome = match serde_json::from_str::<MobileWasmCommand>(raw_json) {
            Ok(cmd) => self.apply_command(cmd),
            Err(err) => Err(format!("invalid mobile wasm command JSON: {err}")),
        };
        let snapshot = match outcome {
            Ok(()) => self.snapshot(None),
            Err(err) => self.snapshot(Some(err)),
        };
        serde_json::to_string(&snapshot).unwrap_or_else(|err| {
            format!(
                r#"{{"ok":false,"error":"snapshot serialization error: {err}","runtime":"{BEVY_RUST_WASM_RUNTIME_ID}"}}"#
            )
        })
    }

    /// Captures the current [`MobileWasmSnapshot`] directly from Bevy ECS resources.
    #[must_use]
    pub fn snapshot(&self, error: Option<String>) -> MobileWasmSnapshot {
        let world = self.app.world();
        let locale = world.resource::<UiLocale>().current;
        let state = world.resource::<MobileClientState>();
        let composer = world.resource::<ComposerSurface>();
        let projection = world.resource::<A11yProjection>();
        let gate = world.resource::<IdleRedrawGate>();
        let live = world.resource::<PoliteLiveQueue>();

        let duplicate_a11y_nodes = match projection.verify_zero_duplicate_nodes() {
            Ok(audit) => audit.duplicated_nodes,
            Err(errs) => errs.len(),
        };

        let a11y_nodes: Vec<MobileWasmA11yNodeDescriptor> = projection
            .nodes
            .iter()
            .map(projected_node_to_descriptor)
            .collect();

        let connectivity = match state.auth_device.connectivity() {
            ConnectivityState::Online => "online",
            ConnectivityState::Offline => "offline",
        };

        MobileWasmSnapshot {
            ok: error.is_none(),
            error,
            runtime: BEVY_RUST_WASM_RUNTIME_ID.to_owned(),
            product: "curie-mobile".to_owned(),
            abi_version: MOBILE_WASM_ABI_VERSION,
            mount_subpath: MOBILE_MOUNT_SUBPATH.to_owned(),
            locale: locale.bcp47_tag().to_owned(),
            direction: direction_str(locale.direction()).to_owned(),
            active_tab: MobileWasmTab::from(state.active_tab),
            connectivity: connectivity.to_owned(),
            draft_text: composer.display_text(),
            owed_sign_outs: state.auth_device.owed_sign_outs().len(),
            a11y_tree_generation: state.a11y_tree_generation,
            a11y_node_count: a11y_nodes.len(),
            duplicate_a11y_nodes,
            idle_redraw: MobileWasmIdleRedrawSummary {
                frames_rendered: gate.frames_rendered(),
                idle_ticks_skipped: gate.idle_ticks_skipped(),
                frames_drawn_state: state.frames_drawn,
            },
            a11y_nodes,
            polite_live: live
                .latest_text(ef_ui_catalog::LivePoliteness::Polite)
                .map(str::to_owned),
        }
    }

    fn apply_command(&mut self, cmd: MobileWasmCommand) -> Result<(), String> {
        match cmd {
            MobileWasmCommand::Snapshot => Ok(()),
            MobileWasmCommand::SetLocale { lang } => {
                let parsed = UiLocaleKind::parse_bcp47(&lang)
                    .ok_or_else(|| format!("unsupported BCP-47 locale '{lang}'"))?;
                self.app
                    .world_mut()
                    .resource_mut::<MobileClientCommandQueue>()
                    .push(MobileClientCommand::SwitchLocale(parsed));
                self.app.update();
                self.app.update();
                Ok(())
            }
            MobileWasmCommand::SwitchTab { tab } => {
                self.app
                    .world_mut()
                    .resource_mut::<MobileClientCommandQueue>()
                    .push(MobileClientCommand::SwitchTab(MobileTab::from(tab)));
                self.app.update();
                self.app.update();
                Ok(())
            }
            MobileWasmCommand::EditDraft { text } => {
                {
                    let mut composer = self.app.world_mut().resource_mut::<ComposerSurface>();
                    let _ = composer.apply(ComposerAction::SelectAll);
                    let _ = composer.apply(ComposerAction::Backspace);
                    if !text.is_empty() {
                        let _ = composer.apply(ComposerAction::InsertText(text));
                    }
                }
                {
                    let mut state = self.app.world_mut().resource_mut::<MobileClientState>();
                    state.a11y_tree_generation = state.a11y_tree_generation.saturating_add(1);
                }
                self.app
                    .world_mut()
                    .resource_mut::<IdleRedrawGate>()
                    .request_redraw(RedrawReason::UserInput);
                self.app.update();
                self.app.update();
                Ok(())
            }
            MobileWasmCommand::SetConnectivity { online } => {
                let conn = if online {
                    ConnectivityState::Online
                } else {
                    ConnectivityState::Offline
                };
                self.app
                    .world_mut()
                    .resource_mut::<MobileClientCommandQueue>()
                    .push(MobileClientCommand::SetConnectivity(conn));
                self.app.update();
                self.app.update();
                Ok(())
            }
            MobileWasmCommand::SetSoftKeyboardInset { bottom_inset_px } => {
                self.app
                    .world_mut()
                    .resource_mut::<MobileClientCommandQueue>()
                    .push(MobileClientCommand::SetSoftKeyboardInsetPx(bottom_inset_px));
                self.app.update();
                Ok(())
            }
            MobileWasmCommand::IdleTick => {
                self.app.update();
                Ok(())
            }
        }
    }
}

impl Default for MobileWasmRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn projected_node_to_descriptor(node: &ProjectedA11yNode) -> MobileWasmA11yNodeDescriptor {
    MobileWasmA11yNodeDescriptor {
        node_id: node.id.0.to_string(),
        control_key: semantic_key_slug(&node.key),
        role: aria_role_str(node.role).to_owned(),
        name: node.label.clone(),
        value: node.value.clone(),
        disabled: matches!(node.availability, NodeAvailability::Disabled),
        modal: matches!(node.modal_scope, ef_ui_catalog::ModalScopeKind::ModalDialog),
        tab_order: match node.interactivity {
            NodeInteractivity::Interactive => "Sequential".to_owned(),
            NodeInteractivity::Structural => "None".to_owned(),
        },
    }
}

fn semantic_key_slug(key: &A11ySemanticKey) -> String {
    match key {
        A11ySemanticKey::NavigationLandmark => "nav-mobile-sections".to_owned(),
        A11ySemanticKey::CitationPanelRegion => "region-crisis-support".to_owned(),
        A11ySemanticKey::MainLandmark => "main-mobile-workspace".to_owned(),
        A11ySemanticKey::ListRow(0) => "button-connected-care-share".to_owned(),
        A11ySemanticKey::ComposerInput => "textbox-doctor-note".to_owned(),
        A11ySemanticKey::SendButton => "button-offline-signout".to_owned(),
        A11ySemanticKey::SearchLandmark => "search-landmark".to_owned(),
        A11ySemanticKey::ComposerFormLandmark => "form-doctor-note".to_owned(),
        A11ySemanticKey::PoliteLiveRegion => "region-polite-live".to_owned(),
        A11ySemanticKey::AssertiveAlertRegion => "region-assertive-alert".to_owned(),
        A11ySemanticKey::StopButton => "button-stop".to_owned(),
        A11ySemanticKey::ResumeButton => "button-resume".to_owned(),
        A11ySemanticKey::TryAgainButton => "button-retry".to_owned(),
        A11ySemanticKey::MobileMenuButton => "button-mobile-menu".to_owned(),
        A11ySemanticKey::PrivacyCoverDialog => "dialog-privacy-cover".to_owned(),
        A11ySemanticKey::PrivacyCoverDismissButton => "button-dismiss-privacy".to_owned(),
        A11ySemanticKey::PatientFinderDialog => "dialog-finder".to_owned(),
        A11ySemanticKey::PatientFinderInput => "input-finder".to_owned(),
        A11ySemanticKey::ShortcutsHelpDialog => "dialog-shortcuts".to_owned(),
        A11ySemanticKey::ListRow(idx) => format!("button-mobile-row-{idx}"),
        A11ySemanticKey::TableRow(idx) => format!("row-mobile-{idx}"),
        A11ySemanticKey::CitationChip(idx) => format!("button-mobile-chip-{idx}"),
    }
}

const fn aria_role_str(role: AriaRole) -> &'static str {
    match role {
        AriaRole::Navigation => "navigation",
        AriaRole::Main => "main",
        AriaRole::Form => "form",
        AriaRole::Region => "region",
        AriaRole::Heading => "heading",
        AriaRole::List => "list",
        AriaRole::ListItem => "listitem",
        AriaRole::Link => "link",
        AriaRole::Button => "button",
        AriaRole::Textbox => "textbox",
        AriaRole::Searchbox => "searchbox",
        AriaRole::Checkbox => "checkbox",
        AriaRole::Dialog => "dialog",
        AriaRole::Alert => "alert",
        AriaRole::Status => "status",
        AriaRole::Table => "table",
        AriaRole::Row => "row",
    }
}

const fn direction_str(direction: TextDirection) -> &'static str {
    match direction {
        TextDirection::Ltr => "ltr",
        TextDirection::Rtl => "rtl",
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_exports {
    use std::alloc::{Layout, alloc, dealloc};
    use std::cell::RefCell;

    use super::{MOBILE_WASM_ABI_VERSION, MobileWasmRuntime, pack_ptr_len};

    thread_local! {
        static MOBILE_RUNTIME: RefCell<Option<MobileWasmRuntime>> = const { RefCell::new(None) };
    }

    fn alloc_bytes(bytes: &[u8]) -> u64 {
        let len = bytes.len();
        let ptr = ef_alloc(len);
        if ptr.is_null() {
            return 0;
        }
        if len > 0 {
            // SAFETY: `ef_alloc(len)` returned a valid writable buffer of at least `len` bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len);
            }
        }
        pack_ptr_len(ptr as u32, len as u32)
    }

    /// Allocates a linear-memory byte buffer owned by the host until released via [`ef_dealloc`].
    #[unsafe(no_mangle)]
    pub extern "C" fn ef_alloc(len: usize) -> *mut u8 {
        let alloc_len = len.max(1);
        let Ok(layout) = Layout::from_size_align(alloc_len, 1) else {
            return std::ptr::null_mut();
        };
        // SAFETY: `alloc_len >= 1` and alignment `1` is always a valid power of two.
        unsafe { alloc(layout) }
    }

    /// Frees a linear-memory byte buffer previously allocated by [`ef_alloc`] or returned by a packed ABI call.
    #[unsafe(no_mangle)]
    pub extern "C" fn ef_dealloc(ptr: *mut u8, len: usize) {
        if ptr.is_null() {
            return;
        }
        let alloc_len = len.max(1);
        if let Ok(layout) = Layout::from_size_align(alloc_len, 1) {
            // SAFETY: `ptr` and `alloc_len` match the allocation performed in `ef_alloc`.
            unsafe { dealloc(ptr, layout) };
        }
    }

    /// Returns the Curie Mobile Wasm ABI version.
    #[unsafe(no_mangle)]
    pub extern "C" fn ef_mobile_wasm_version() -> u32 {
        MOBILE_WASM_ABI_VERSION
    }

    /// Initializes the singleton Bevy `CurieMobileClientPlugin` runtime and returns a packed
    /// `(ptr << 32) | len` UTF-8 JSON snapshot. Caller releases the buffer via [`ef_dealloc`].
    #[unsafe(no_mangle)]
    pub extern "C" fn ef_mobile_wasm_init() -> u64 {
        MOBILE_RUNTIME.with(|slot| {
            let mut guard = slot.borrow_mut();
            let runtime = guard.get_or_insert_with(MobileWasmRuntime::new);
            let json = runtime.dispatch_json(r#"{"cmd":"snapshot"}"#);
            alloc_bytes(json.as_bytes())
        })
    }

    /// Dispatches a UTF-8 JSON command into the Bevy `CurieMobileClientPlugin` runtime and
    /// returns a packed `(ptr << 32) | len` UTF-8 JSON snapshot.
    #[unsafe(no_mangle)]
    pub extern "C" fn ef_mobile_wasm_dispatch_json(ptr: *const u8, len: usize) -> u64 {
        let input_bytes = if ptr.is_null() || len == 0 {
            &[][..]
        } else {
            // SAFETY: Host passes a valid linear-memory slice `(ptr, len)` allocated via `ef_alloc`.
            unsafe { std::slice::from_raw_parts(ptr, len) }
        };
        let input_str = match std::str::from_utf8(input_bytes) {
            Ok(s) => s,
            Err(err) => {
                let err_json = format!(
                    r#"{{"ok":false,"error":"invalid UTF-8 command payload: {err}","runtime":"bevy-rust-wasm"}}"#
                );
                return alloc_bytes(err_json.as_bytes());
            }
        };

        MOBILE_RUNTIME.with(|slot| {
            let mut guard = slot.borrow_mut();
            let runtime = guard.get_or_insert_with(MobileWasmRuntime::new);
            let json = runtime.dispatch_json(input_str);
            alloc_bytes(json.as_bytes())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_wasm_runtime_emits_a11y_nodes_switches_arabic_rtl_and_skips_idle() {
        let mut runtime = MobileWasmRuntime::new();
        let initial_json = runtime.dispatch_json(r#"{"cmd":"snapshot"}"#);
        let initial: MobileWasmSnapshot =
            serde_json::from_str(&initial_json).expect("initial mobile snapshot must parse");

        assert!(initial.ok);
        assert_eq!(initial.runtime, BEVY_RUST_WASM_RUNTIME_ID);
        assert_eq!(initial.product, "curie-mobile");
        assert_eq!(initial.abi_version, MOBILE_WASM_ABI_VERSION);
        assert_eq!(initial.mount_subpath, "/elysianfields/mobile");
        assert_eq!(initial.locale, "en");
        assert_eq!(initial.direction, "ltr");
        assert_eq!(initial.duplicate_a11y_nodes, 0);
        assert_eq!(initial.a11y_node_count, 6);
        assert!(initial.a11y_nodes.iter().any(|n| n.role == "navigation"));
        assert!(initial.a11y_nodes.iter().any(|n| n.role == "main"));
        assert!(initial.a11y_nodes.iter().any(|n| n.role == "textbox"));
        assert!(
            initial
                .a11y_nodes
                .iter()
                .any(|n| n.role == "region" && n.name.contains("988") && n.name.contains("SAMHSA"))
        );

        // Switch locale to Arabic RTL and verify localized crisis region + RTL direction.
        let rtl: MobileWasmSnapshot =
            serde_json::from_str(&runtime.dispatch_json(r#"{"cmd":"set_locale","lang":"ar"}"#))
                .expect("RTL mobile snapshot must parse");
        assert!(rtl.ok);
        assert_eq!(rtl.locale, "ar");
        assert_eq!(rtl.direction, "rtl");
        assert_eq!(rtl.duplicate_a11y_nodes, 0);
        assert_eq!(rtl.a11y_node_count, 6);
        assert!(
            rtl.a11y_nodes
                .iter()
                .any(|n| n.role == "region" && n.name.contains("988") && n.name.contains("SAMHSA"))
        );

        // Update curated doctor note & switch tab.
        let edited: MobileWasmSnapshot = serde_json::from_str(
            &runtime.dispatch_json(r#"{"cmd":"edit_draft","text":"BP 118/76 this morning"}"#),
        )
        .expect("edited mobile snapshot must parse");
        assert!(edited.ok);
        assert_eq!(edited.draft_text, "BP 118/76 this morning");

        let tab_sw: MobileWasmSnapshot = serde_json::from_str(
            &runtime.dispatch_json(r#"{"cmd":"switch_tab","tab":"for_my_doctor"}"#),
        )
        .expect("tab switch snapshot must parse");
        assert!(tab_sw.ok);
        assert_eq!(tab_sw.active_tab, MobileWasmTab::ForMyDoctor);

        // Idle tick skips rendering without bumping frames_rendered.
        let before_frames = tab_sw.idle_redraw.frames_rendered;
        let before_skips = tab_sw.idle_redraw.idle_ticks_skipped;
        let idle: MobileWasmSnapshot =
            serde_json::from_str(&runtime.dispatch_json(r#"{"cmd":"idle_tick"}"#))
                .expect("idle snapshot must parse");
        assert!(idle.ok);
        assert_eq!(idle.idle_redraw.frames_rendered, before_frames);
        assert_eq!(idle.idle_redraw.idle_ticks_skipped, before_skips + 1);

        // Invalid locale returns `ok: false` with cause.
        let bad: MobileWasmSnapshot =
            serde_json::from_str(&runtime.dispatch_json(r#"{"cmd":"set_locale","lang":"zz-bad"}"#))
                .expect("error snapshot must parse");
        assert!(!bad.ok);
        assert!(bad.error.as_deref().is_some_and(|e| e.contains("zz-bad")));
    }
}
