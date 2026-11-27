//! Zero-dependency `wasm32-unknown-unknown` `cdylib` ABI for Curie Clinic.
//!
//! Wraps the real Bevy `0.20.1` [`CurieClinicClientPlugin`] + [`EfUiCatalogPlugin`]
//! inside a pure-Rust runtime (`ClinicWasmRuntime`) that runs both on native hosts
//! (for deterministic unit/integration tests) and as exported `extern "C"` Wasm symbols
//! under strict browser CSP (`script-src 'self' 'wasm-unsafe-eval'`).
//!
//! Single-owner invariant: Rust/Bevy owns all AccessKit ARIA descriptors, locale/RTL
//! state, composer text, and `IdleRedrawGate` counters. JavaScript projects the emitted
//! JSON snapshot directly into `#ef-a11y-overlay` via `a11y-bridge.js` without holding a
//! second authoritative clinical store.

use std::sync::{Arc, Mutex};

use bevy::prelude::App;
use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, PhysicianAccount, PhysicianRole, SignInMode,
};
use ef_clinic_domain::chat::RetryBackoffMode;
use ef_clinic_domain::workspace::{
    CitationOrigin, CitedSourcePayload, ReadablePart, classify_record_status,
};
use ef_core::{
    ChatId, Locale as CoreLocale, PhysicianId, PracticeId, SourceKey, TabId, WorkspaceId,
};
use ef_harness::StubProvider;
use ef_ui_catalog::{
    A11yProjection, A11ySemanticKey, AriaRole, ComposerAction, ComposerSurface, FrameDecision,
    HorizontalEdge, IdleRedrawGate, Locale as UiLocaleKind, ModifierChord, NodeAvailability,
    NodeInteractivity, PoliteLiveQueue, ProjectedA11yNode, ShortcutInput, ShortcutKey,
    ShortcutRouter, TextDirection, UiLocale,
};
use serde::{Deserialize, Serialize};

use crate::plugin::{
    ClientFrameReport, ClinicAction, ClinicActionMessage, ClinicTransportResource,
    CurieClinicClientPlugin, SignInEntryMethod,
};
use crate::state::{
    ClientRuntimeConfig, ClinicSessionShell, SharedBrowserSessionHub, SharedLocaleStorage,
    SignInFormStatus,
};
use crate::transport::{LocalVerticalSliceTransport, VerticalSliceBackend};
use crate::wire::ClientBootConfig;

/// Current binary ABI contract version exported by `ef_clinic_wasm_version`.
pub const CLINIC_WASM_ABI_VERSION: u32 = 1;

/// Canonical runtime identifier exposed on `document.documentElement.dataset.efWasmRuntime`.
pub const BEVY_RUST_WASM_RUNTIME_ID: &str = "bevy-rust-wasm";

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

/// Typed command dispatched across the JSON Wasm boundary (`ef_clinic_wasm_dispatch_json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClinicWasmCommand {
    /// Returns the latest snapshot without mutating state.
    Snapshot,
    /// Switches the active UI locale (e.g., `"ar"` -> Arabic RTL mirroring) via `CurieClinicClientPlugin`.
    SetLocale { lang: String },
    /// Replaces the single-owner `ComposerSurface` draft text through `ClinicAction::ComposerEdit`.
    SetComposerText { text: String },
    /// Toggles the global `Cmd/Ctrl+Shift+L` privacy cover overlay.
    TogglePrivacyCover,
    /// Executes one idle Bevy `App::update()` tick so `IdleRedrawGate` verifies zero idle redraws.
    IdleTick,
}

/// Serializable descriptor for one AccessKit node consumed directly by `a11y-bridge.js`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WasmA11yNodeDescriptor {
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
pub struct WasmIdleRedrawSummary {
    pub frames_rendered: u64,
    pub idle_ticks_skipped: u64,
    pub last_decision: String,
}

/// Complete JSON snapshot returned from `ef_clinic_wasm_init` and `ef_clinic_wasm_dispatch_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClinicWasmSnapshot {
    pub ok: bool,
    pub error: Option<String>,
    pub runtime: String,
    pub product: String,
    pub abi_version: u32,
    pub mount_subpath: String,
    pub locale: String,
    pub direction: String,
    pub sidebar_edge: String,
    pub citation_panel_edge: String,
    pub privacy_cover_active: bool,
    pub composer_text: String,
    pub a11y_tree_generation: u64,
    pub a11y_node_count: usize,
    pub duplicate_a11y_nodes: usize,
    pub idle_redraw: WasmIdleRedrawSummary,
    pub a11y_nodes: Vec<WasmA11yNodeDescriptor>,
    pub polite_live: Option<String>,
}

/// Pure-Rust runtime hosting the real Bevy `App` + [`CurieClinicClientPlugin`].
pub struct ClinicWasmRuntime {
    app: App,
}

impl ClinicWasmRuntime {
    /// Boots a real Bevy `0.20.1` `App` with [`CurieClinicClientPlugin`] and a synthetic
    /// local vertical-slice backend so the full AccessKit workspace tree is populated.
    #[must_use]
    pub fn new() -> Self {
        let mut app = App::new();

        let (transport, demo_email, demo_ws_id, demo_chat_id) = build_demo_vertical_slice();
        let boot_config = ClientBootConfig::from_location("/elysianfields/")
            .unwrap_or_else(|_| unreachable!("canonical /elysianfields/ path always parses"));
        let tab_id =
            TabId::new("wasm-tab-1").unwrap_or_else(|_| unreachable!("static tab id is valid"));
        let session_hub = SharedBrowserSessionHub::new(None);
        let locale_storage = SharedLocaleStorage::default();

        app.insert_resource(ClinicTransportResource::new(transport))
            .add_plugins(CurieClinicClientPlugin::new(
                tab_id,
                boot_config,
                session_hub,
                locale_storage,
            ));

        // Boot tab -> request token -> verify session -> open synthetic workspace.
        app.world_mut()
            .write_message(ClinicActionMessage(ClinicAction::BootTab));
        app.update();

        app.world_mut()
            .write_message(ClinicActionMessage(ClinicAction::SubmitSignInEmail {
                email: demo_email.clone(),
            }));
        app.update();

        let token = match &app.world().resource::<ClinicSessionShell>().sign_in_form {
            SignInFormStatus::LinkRequested {
                proof_token: Some(tok),
                ..
            } => tok.clone(),
            _ => String::new(),
        };

        if !token.is_empty() {
            app.world_mut()
                .write_message(ClinicActionMessage(ClinicAction::VerifySignInToken {
                    email: demo_email,
                    token,
                    entry: SignInEntryMethod::DirectForm,
                }));
            app.update();

            app.world_mut().write_message(ClinicActionMessage(
                ClinicAction::OpenPatientWorkspace {
                    workspace_id: demo_ws_id,
                    chat_id: Some(demo_chat_id),
                },
            ));
            app.update();
        }

        // Flush deferred Bevy Commands into EfUiCatalogPlugin's A11yProjection.
        app.update();

        Self { app }
    }

    /// Dispatches one JSON-encoded [`ClinicWasmCommand`] and returns the serialized JSON snapshot.
    #[must_use]
    pub fn dispatch_json(&mut self, raw_json: &str) -> String {
        let outcome = match serde_json::from_str::<ClinicWasmCommand>(raw_json) {
            Ok(cmd) => self.apply_command(cmd),
            Err(err) => Err(format!("invalid clinic wasm command JSON: {err}")),
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

    /// Captures the current [`ClinicWasmSnapshot`] directly from Bevy ECS resources.
    #[must_use]
    pub fn snapshot(&self, error: Option<String>) -> ClinicWasmSnapshot {
        let world = self.app.world();
        let locale = world.resource::<UiLocale>().current;
        let runtime_cfg = world.resource::<ClientRuntimeConfig>();
        let router = world.resource::<ShortcutRouter>();
        let composer = world.resource::<ComposerSurface>();
        let projection = world.resource::<A11yProjection>();
        let gate = world.resource::<IdleRedrawGate>();
        let report = world.resource::<ClientFrameReport>();
        let live = world.resource::<PoliteLiveQueue>();

        let duplicate_a11y_nodes = match projection.verify_zero_duplicate_nodes() {
            Ok(audit) => audit.duplicated_nodes,
            Err(errs) => errs.len(),
        };

        let a11y_nodes: Vec<WasmA11yNodeDescriptor> = projection
            .nodes
            .iter()
            .map(projected_node_to_descriptor)
            .collect();

        let last_decision = match &report.last_decision {
            Some(FrameDecision::Render { .. }) => "render",
            Some(FrameDecision::SkipIdle) => "skip_idle",
            None => "none",
        };

        ClinicWasmSnapshot {
            ok: error.is_none(),
            error,
            runtime: BEVY_RUST_WASM_RUNTIME_ID.to_owned(),
            product: "curie-clinic".to_owned(),
            abi_version: CLINIC_WASM_ABI_VERSION,
            mount_subpath: projection.mount_subpath.clone(),
            locale: locale.bcp47_tag().to_owned(),
            direction: direction_str(locale.direction()).to_owned(),
            sidebar_edge: edge_str(runtime_cfg.sidebar_edge).to_owned(),
            citation_panel_edge: edge_str(runtime_cfg.citation_panel_edge).to_owned(),
            privacy_cover_active: router.is_privacy_cover_active(),
            composer_text: composer.display_text(),
            a11y_tree_generation: report.a11y_tree_generation,
            a11y_node_count: a11y_nodes.len(),
            duplicate_a11y_nodes,
            idle_redraw: WasmIdleRedrawSummary {
                frames_rendered: gate.frames_rendered(),
                idle_ticks_skipped: gate.idle_ticks_skipped(),
                last_decision: last_decision.to_owned(),
            },
            a11y_nodes,
            polite_live: live
                .latest_text(ef_ui_catalog::LivePoliteness::Polite)
                .map(str::to_owned),
        }
    }

    fn apply_command(&mut self, cmd: ClinicWasmCommand) -> Result<(), String> {
        match cmd {
            ClinicWasmCommand::Snapshot => Ok(()),
            ClinicWasmCommand::SetLocale { lang } => {
                let parsed = UiLocaleKind::parse_bcp47(&lang)
                    .ok_or_else(|| format!("unsupported BCP-47 locale '{lang}'"))?;
                self.app
                    .world_mut()
                    .write_message(ClinicActionMessage(ClinicAction::SwitchLocale(parsed)));
                self.app.update();
                // Second tick projects newly spawned localized AccessKit nodes into A11yProjection.
                self.app.update();
                Ok(())
            }
            ClinicWasmCommand::SetComposerText { text } => {
                self.app.world_mut().write_message(ClinicActionMessage(
                    ClinicAction::ComposerEdit(ComposerAction::SelectAll),
                ));
                self.app.world_mut().write_message(ClinicActionMessage(
                    ClinicAction::ComposerEdit(ComposerAction::Backspace),
                ));
                if !text.is_empty() {
                    self.app.world_mut().write_message(ClinicActionMessage(
                        ClinicAction::ComposerEdit(ComposerAction::InsertText(text)),
                    ));
                }
                self.app.update();
                self.app.update();
                Ok(())
            }
            ClinicWasmCommand::TogglePrivacyCover => {
                self.app.world_mut().write_message(ClinicActionMessage(
                    ClinicAction::DispatchShortcut(ShortcutInput::new(
                        ShortcutKey::Char('l'),
                        ModifierChord::PrimaryShift,
                    )),
                ));
                self.app.update();
                self.app.update();
                Ok(())
            }
            ClinicWasmCommand::IdleTick => {
                self.app.update();
                Ok(())
            }
        }
    }
}

impl Default for ClinicWasmRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn build_demo_vertical_slice() -> (LocalVerticalSliceTransport, String, WorkspaceId, ChatId) {
    let practice_id =
        PracticeId::new("prac-wasm-demo").unwrap_or_else(|_| unreachable!("static id"));
    let physician_id =
        PhysicianId::new("phys-wasm-demo").unwrap_or_else(|_| unreachable!("static id"));
    let ws_primary = WorkspaceId::new("ws-demo-1").unwrap_or_else(|_| unreachable!("static id"));
    let ws_secondary = WorkspaceId::new("ws-demo-2").unwrap_or_else(|_| unreachable!("static id"));
    let chat_id = ChatId::new("chat-demo-1").unwrap_or_else(|_| unreachable!("static id"));
    let source_key = SourceKey::new("src-note-1").unwrap_or_else(|_| unreachable!("static id"));
    let demo_email = "clinician@practice.example".to_owned();

    let stub = StubProvider::clinical_two_round_fixture(source_key.clone());
    let mut backend = VerticalSliceBackend::new(
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        stub,
    );

    backend.auth.upsert_account(PhysicianAccount {
        physician_id: physician_id.clone(),
        practice_id: practice_id.clone(),
        email: EmailAddress::parse(&demo_email).unwrap_or_else(|_| unreachable!("static email")),
        display_name: "Dr. Curie Synthetic".to_owned(),
        role: PhysicianRole::Standard,
        standing: AccountStanding::Active,
        sessions_ended_before: None,
    });

    backend.evidence.register_workspace(
        ws_primary.clone(),
        practice_id.clone(),
        "Avery, Jordan (Synthetic)",
        vec![physician_id.clone()],
    );
    backend.evidence.register_workspace(
        ws_secondary,
        practice_id,
        "Rivera, Sam (Synthetic)",
        vec![physician_id],
    );
    backend
        .default_chats
        .insert(ws_primary.clone(), chat_id.clone());
    backend.synopses.insert(
        ws_primary.clone(),
        "Synthetic glycemic and renal follow-up workspace.".to_owned(),
    );
    backend.evidence.insert_source(CitedSourcePayload {
        source_key,
        workspace_id: ws_primary.clone(),
        number: 1,
        source_label: "Progress Note [1]".to_owned(),
        title: "Synthetic Outpatient Progress Note".to_owned(),
        record_type: "progress_note".to_owned(),
        recorded_at: "2026-10-15T09:00:00Z".to_owned(),
        record_status: "final".to_owned(),
        status_class: classify_record_status("final"),
        origin: CitationOrigin::SourceRecord,
        original_language: CoreLocale::En,
        quote: "HbA1c 6.8%, eGFR 74 mL/min/1.73m2.".to_owned(),
        body: "Assessment: Stable glycemic control on current regimen.".to_owned(),
        readable_parts: vec![ReadablePart {
            label: "Assessment".to_owned(),
            text: "Stable glycemic control on current regimen.".to_owned(),
            unreadable_notice: None,
        }],
        fields: Vec::new(),
        raw_content: None,
        external_url: None,
    });

    let transport = LocalVerticalSliceTransport::new(Arc::new(Mutex::new(backend)));
    (transport, demo_email, ws_primary, chat_id)
}

fn projected_node_to_descriptor(node: &ProjectedA11yNode) -> WasmA11yNodeDescriptor {
    WasmA11yNodeDescriptor {
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
        A11ySemanticKey::NavigationLandmark => "nav-patient-roster".to_owned(),
        A11ySemanticKey::MainLandmark => "main-clinical-workspace".to_owned(),
        A11ySemanticKey::SearchLandmark => "search-landmark".to_owned(),
        A11ySemanticKey::ComposerFormLandmark => "form-composer".to_owned(),
        A11ySemanticKey::CitationPanelRegion => "region-citation-panel".to_owned(),
        A11ySemanticKey::PoliteLiveRegion => "region-polite-live".to_owned(),
        A11ySemanticKey::AssertiveAlertRegion => "region-assertive-alert".to_owned(),
        A11ySemanticKey::ComposerInput => "textbox-composer".to_owned(),
        A11ySemanticKey::SendButton => "button-send-prompt".to_owned(),
        A11ySemanticKey::StopButton => "button-stop-work".to_owned(),
        A11ySemanticKey::ResumeButton => "button-resume-work".to_owned(),
        A11ySemanticKey::TryAgainButton => "button-try-again".to_owned(),
        A11ySemanticKey::MobileMenuButton => "button-mobile-menu".to_owned(),
        A11ySemanticKey::PrivacyCoverDialog => "dialog-privacy-cover".to_owned(),
        A11ySemanticKey::PrivacyCoverDismissButton => "button-modal-dismiss-privacy".to_owned(),
        A11ySemanticKey::PatientFinderDialog => "dialog-patient-finder".to_owned(),
        A11ySemanticKey::PatientFinderInput => "input-patient-finder".to_owned(),
        A11ySemanticKey::ShortcutsHelpDialog => "dialog-shortcuts-help".to_owned(),
        A11ySemanticKey::ListRow(idx) => format!("btn-patient-{idx}"),
        A11ySemanticKey::TableRow(idx) => format!("row-lab-{idx}"),
        A11ySemanticKey::CitationChip(idx) => format!("button-citation-{idx}"),
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

const fn edge_str(edge: HorizontalEdge) -> &'static str {
    match edge {
        HorizontalEdge::Left => "left",
        HorizontalEdge::Right => "right",
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_exports {
    use std::alloc::{Layout, alloc, dealloc};
    use std::cell::RefCell;

    use super::{CLINIC_WASM_ABI_VERSION, ClinicWasmRuntime, pack_ptr_len};

    thread_local! {
        static CLINIC_RUNTIME: RefCell<Option<ClinicWasmRuntime>> = const { RefCell::new(None) };
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

    /// Returns the Curie Clinic Wasm ABI version.
    #[unsafe(no_mangle)]
    pub extern "C" fn ef_clinic_wasm_version() -> u32 {
        CLINIC_WASM_ABI_VERSION
    }

    /// Initializes the singleton Bevy `CurieClinicClientPlugin` runtime and returns a packed
    /// `(ptr << 32) | len` UTF-8 JSON snapshot. Caller releases the buffer via [`ef_dealloc`].
    #[unsafe(no_mangle)]
    pub extern "C" fn ef_clinic_wasm_init() -> u64 {
        CLINIC_RUNTIME.with(|slot| {
            let mut guard = slot.borrow_mut();
            let runtime = guard.get_or_insert_with(ClinicWasmRuntime::new);
            let json = runtime.dispatch_json(r#"{"cmd":"snapshot"}"#);
            alloc_bytes(json.as_bytes())
        })
    }

    /// Dispatches a UTF-8 JSON command into the Bevy `CurieClinicClientPlugin` runtime and
    /// returns a packed `(ptr << 32) | len` UTF-8 JSON snapshot.
    #[unsafe(no_mangle)]
    pub extern "C" fn ef_clinic_wasm_dispatch_json(ptr: *const u8, len: usize) -> u64 {
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

        CLINIC_RUNTIME.with(|slot| {
            let mut guard = slot.borrow_mut();
            let runtime = guard.get_or_insert_with(ClinicWasmRuntime::new);
            let json = runtime.dispatch_json(input_str);
            alloc_bytes(json.as_bytes())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_and_unpack_ptr_len_roundtrips_extremes() {
        for (ptr, len) in [
            (0, 0),
            (16, 4096),
            (0x1234_5678, 0x00AB_CDEF),
            (u32::MAX, u32::MAX),
        ] {
            let packed = pack_ptr_len(ptr, len);
            assert_eq!(unpack_ptr_len(packed), (ptr, len));
        }
    }

    #[test]
    fn clinic_wasm_runtime_emits_real_bevy_a11y_nodes_switches_arabic_rtl_and_skips_idle() {
        let mut runtime = ClinicWasmRuntime::new();
        let initial_json = runtime.dispatch_json(r#"{"cmd":"snapshot"}"#);
        let initial: ClinicWasmSnapshot =
            serde_json::from_str(&initial_json).expect("initial snapshot JSON must parse");

        assert!(initial.ok);
        assert_eq!(initial.runtime, BEVY_RUST_WASM_RUNTIME_ID);
        assert_eq!(initial.product, "curie-clinic");
        assert_eq!(initial.abi_version, CLINIC_WASM_ABI_VERSION);
        assert_eq!(initial.mount_subpath, "/elysianfields");
        assert_eq!(initial.locale, "en");
        assert_eq!(initial.direction, "ltr");
        assert_eq!(initial.sidebar_edge, "left");
        assert_eq!(initial.citation_panel_edge, "right");
        assert_eq!(initial.duplicate_a11y_nodes, 0);
        assert!(
            initial.a11y_node_count >= 5,
            "expected >= 5 AccessKit nodes, got {}",
            initial.a11y_node_count
        );
        assert!(initial.a11y_nodes.iter().any(|n| n.role == "navigation"));
        assert!(initial.a11y_nodes.iter().any(|n| n.role == "main"));
        assert!(initial.a11y_nodes.iter().any(|n| n.role == "textbox"));
        assert!(initial.a11y_nodes.iter().any(|n| n.role == "button"));

        // Switch locale to Arabic RTL and verify mirrored edges + localized AccessKit nodes.
        let rtl_json = runtime.dispatch_json(r#"{"cmd":"set_locale","lang":"ar"}"#);
        let rtl: ClinicWasmSnapshot =
            serde_json::from_str(&rtl_json).expect("RTL snapshot JSON must parse");
        assert!(rtl.ok);
        assert_eq!(rtl.locale, "ar");
        assert_eq!(rtl.direction, "rtl");
        assert_eq!(rtl.sidebar_edge, "right");
        assert_eq!(rtl.citation_panel_edge, "left");
        assert_eq!(rtl.duplicate_a11y_nodes, 0);
        assert!(rtl.a11y_node_count >= 5);

        // Composer edit updates single-owner Rust ComposerSurface.
        let edit_json = runtime
            .dispatch_json(r#"{"cmd":"set_composer_text","text":"Review recent eGFR trend"}"#);
        let edited: ClinicWasmSnapshot =
            serde_json::from_str(&edit_json).expect("edited snapshot JSON must parse");
        assert!(edited.ok);
        assert_eq!(edited.composer_text, "Review recent eGFR trend");

        // Idle ticks must NOT increment frames_rendered while incrementing idle_ticks_skipped.
        let before_idle_frames = edited.idle_redraw.frames_rendered;
        let before_idle_skips = edited.idle_redraw.idle_ticks_skipped;
        let idle_json = runtime.dispatch_json(r#"{"cmd":"idle_tick"}"#);
        let idle: ClinicWasmSnapshot =
            serde_json::from_str(&idle_json).expect("idle snapshot JSON must parse");
        assert!(idle.ok);
        assert_eq!(idle.idle_redraw.frames_rendered, before_idle_frames);
        assert_eq!(idle.idle_redraw.idle_ticks_skipped, before_idle_skips + 1);
        assert_eq!(idle.idle_redraw.last_decision, "skip_idle");

        // Privacy cover toggles cleanly and hides clinical controls behind modal dialog.
        let cover_json = runtime.dispatch_json(r#"{"cmd":"toggle_privacy_cover"}"#);
        let cover: ClinicWasmSnapshot =
            serde_json::from_str(&cover_json).expect("cover snapshot JSON must parse");
        assert!(cover.ok);
        assert!(cover.privacy_cover_active);
        assert!(
            cover
                .a11y_nodes
                .iter()
                .any(|n| n.role == "dialog" && n.modal)
        );

        // Failure looks like failure: invalid JSON or unknown locale reports `ok: false` + cause.
        let bad_locale: ClinicWasmSnapshot = serde_json::from_str(
            &runtime.dispatch_json(r#"{"cmd":"set_locale","lang":"xx-invalid"}"#),
        )
        .expect("error snapshot must parse");
        assert!(!bad_locale.ok);
        assert!(
            bad_locale
                .error
                .as_deref()
                .is_some_and(|e| e.contains("xx-invalid"))
        );

        let bad_json: ClinicWasmSnapshot =
            serde_json::from_str(&runtime.dispatch_json("{not json"))
                .expect("parse error snapshot must parse");
        assert!(!bad_json.ok);
        assert!(bad_json.error.is_some());
    }
}
