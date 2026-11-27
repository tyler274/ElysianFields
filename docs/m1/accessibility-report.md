# M1 Accessibility Feasibility and Qualification Report

This report documents the M1 Bevy `0.20.1` + `accesskit 0.24.1` accessibility architecture, automated verification suite, narrow browser adapter cost analysis, and per-target qualification status for Elysian Fields. All measurements and prototypes operate exclusively on synthetic clinical placeholder data.

## 1. Architecture and Single-Owner Accessibility Model

The M1 accessibility prototype lives in [`crates/m1-accessibility`](../../crates/m1-accessibility/Cargo.toml) and exercises all four required clinical workspace surfaces inside a headless Bevy `0.20.1` `App` (`AccessibilityPrototypePlugin`):

1. **Navigation list (`ScreenSurface::Navigation`)**: `Role::Navigation` landmark with four keyboard-navigable workspace items (`Clinical Inbox`, `Streaming Transcript`, `Assessment Form`, `Lab Records`), supporting `ArrowUp` / `ArrowDown` / `Home` / `End`, `Enter` / `Space` selection, and synchronized AccessKit `selected` state.
2. **Streaming transcript with interactive citations (`ScreenSurface::Transcript`)**: `Role::Main` landmark, `Role::Heading`, interactive inline citation links (`[1]` and `[2]`, `Role::Link`) with descriptive accessible names, and a throttled `Role::Status` (`accesskit::Live::Polite`) region for streaming updates.
3. **Clinical assessment form (`ScreenSurface::ClinicalForm`)**: `Role::Form` landmark, labeled multiline text input (`Role::TextInput`), citation-attachment checkbox (`Role::CheckBox` with `accesskit::Toggled`), expandable clinical guidance disclosure (`Role::Button` with `expanded` state), assertive validation banner (`Role::Alert` with `accesskit::Live::Assertive` and `Invalid::True` on invalid submission), submit button, and an availability-gated sign-off trigger (`disabled` until a valid note is submitted).
4. **Modal confirmation dialog (`ScreenSurface::Modal`)**: `Role::Dialog` with `set_modal()`, heading, confirm/cancel controls, strict `Tab` / `Shift+Tab` focus trap, background node hiding while active, and `Escape` / Cancel / Confirm dismissal restoring keyboard focus to the exact prior `FocusTarget`.

### Single-Owner Invariant

Every semantic control is registered once in `SemanticEntityRegistry` (`SemanticControlId -> Entity`) and owns exactly one `bevy::a11y::AccessibilityNode` (`accesskit::Node`) component. Both native AccessKit consumers and the browser Wasm adapter consume the same `AccessibilityTreeSnapshot` extracted directly from Bevy ECS:

```text
ScreenState + FocusManager + AccessibilityPreferences + LiveAnnouncementQueue
    │
    ▼ (single owner: build_accesskit_node)
Bevy ECS Entities (Node + SemanticControlMarker + bevy::a11y::AccessibilityNode)
    │
    ▼ (direct ECS read)
AccessibilityTreeSnapshot  ──► verify_zero_duplicate_nodes (exact cardinality 1)
    ├──► Native Desktop OS AT (AT-SPI2 / NSAccessibility / UIA via AccessKit)
    ├──► Native Mobile Platform Proxy (Android NodeProvider / iOS UIAccessibility)
    └──► Browser Wasm NarrowSemanticBridge ──► BridgePatch ──► web/a11y-bridge.js
```

`AccessibilityTreeSnapshot::verify_zero_duplicate_nodes` enforces:
- Every `accesskit::NodeId` is unique across the tree.
- Every `SemanticControlId` appears with exact cardinality `1`.
- Every visible interactive control matches the active focus traversal order (`ordered_focus_targets`), carries a non-empty accessible name, and follows a strictly monotonic reading order.
- While `ModalState::Open` holds, exactly one visible modal `Role::Dialog` exists and all non-modal controls are hidden from assistive-technology traversal.

## 2. Core Accessibility Criteria Verification

All automated checks live in [`crates/m1-accessibility/src/lib.rs`](../../crates/m1-accessibility/src/lib.rs) and run against the live Bevy `0.20.1` plugin without GPU or display dependencies:

| Criterion | Owning Module | Automated Proof |
| --- | --- | --- |
| **Zero-duplicate-node tree & bridge bijection** | `src/tree.rs`, `src/bridge.rs` | `single_owner_accesskit_tree_and_bridge_have_zero_duplicate_nodes` + negative test `zero_duplicate_checker_catches_injected_duplicate_node` |
| **Full keyboard-only operation & modal focus restoration** | `src/focus.rs`, `src/plugin.rs` | `full_keyboard_only_workflow_form_submission_modal_trap_and_focus_restore` (Tab, Shift+Tab, ArrowUp/Down/Home/End, Enter/Space, text typing, validation error, valid submit, modal trap wraparound, Escape restore, Bevy `InputFocus` + visible focus ring check) |
| **200% zoom layout reachability** | `src/preferences.rs` | `zoom_200_percent_keeps_every_control_unclipped_and_non_overlapping` across desktop (`1280x720`), mobile (`390x844`), and popup (`360x520`) viewports + negative overflow/overlap test |
| **Reduced motion** | `src/preferences.rs` | `reduced_motion_disables_all_non_essential_animations` (`ReducedMotion::Reduce` sets all non-essential transition durations to `0 ms` and sets streaming cursor to `AnimationMode::Static`) |
| **WCAG 2.1 AA contrast** | `src/contrast.rs` | `wcag_21_aa_contrast_passes_for_both_elysian_themes_and_rejects_low_contrast` (11 semantic pairs each in Light and Dark themes: `>= 4.5:1` normal text, `>= 3.0:1` large text and UI/focus rings + negative low-contrast test) |
| **Throttled streaming live announcements** | `src/live_region.rs` | `throttled_live_queue_caps_40_tok_per_sec_streaming_announcements` (simulates the M1 40 tok/s, 15-second / 600-token workload; coalescing policy `750 ms..=1500 ms` caps output to `10..=20` polite announcements while assertive alerts bypass immediately without losing buffered prose) |
| **Zero-idle / coalesced DOM mutation discipline** | `src/bridge.rs`, `src/plugin.rs` | `bridge_diff_emits_zero_dom_patches_while_stream_tokens_coalesce` (0 DOM patches on idle frames and while stream tokens buffer below the coalescing window) |

## 3. Platform Reachability Analysis (`Bevy 0.20.1` + `accesskit 0.24.1`)

### Desktop Targets (`desktop-linux`, `desktop-macos`, `desktop-windows`)

Bevy `0.20.1` ships `bevy_a11y` integrated with `accesskit 0.24.1` and `accesskit_winit` across all three required desktop platforms:
- **Linux (`desktop-linux`)**: `accesskit_unix` publishes the AccessKit tree over AT-SPI2 D-Bus to Orca and Linux screen magnifiers on both Wayland and X11.
- **macOS (`desktop-macos`)**: `accesskit_macos` bridges `accesskit::TreeUpdate` directly to `NSAccessibility` for VoiceOver, Full Keyboard Access, and macOS Switch Control.
- **Windows (`desktop-windows`)**: `accesskit_windows` exposes native UI Automation (UIA) providers to NVDA, JAWS, and Windows Narrator.

Desktop targets require **zero** custom accessibility DOM or UI proxy layers; `AccessibilityNode` components in Bevy ECS map directly to OS assistive technology APIs.

### Mobile Targets (`android-native`, `ios-native`)

- **Android (`android-native`)**: AccessKit defines the tree model and `accesskit_android` provides an `AccessibilityNodeProvider` adapter for TalkBack, but Bevy `0.20.1`'s default winit mobile loop does not yet wire an Android AccessKit adapter automatically. Qualification requires a narrow platform adapter in the Android host shell that subscribes to `AccessibilityTreeSnapshot` updates and exposes `AccessibilityNodeInfo` nodes + `announceForAccessibility` / live-region events to TalkBack from the single Rust state owner.
- **iOS (`ios-native`)**: Bevy `0.20.1` + `accesskit 0.24.1` does not include a built-in iOS `UIAccessibility` winit adapter. Qualification requires a narrow UIKit `UIAccessibilityElement` container proxy over the Metal view driven strictly by `AccessibilityTreeSnapshot` + `UIAccessibilityPostNotification` for throttled polite and assertive announcements, keeping zero duplicate editable state.

### Browser Wasm Targets (`browser-wasm-desktop`, `browser-wasm-mobile`, `chrome-extension-popup`)

Canvas/WebGL2 surfaces on the web expose no semantic DOM structure to VoiceOver, TalkBack, NVDA, or Orca on their own. To preserve a single authoritative UI stack without duplicating styling or layout, Elysian Fields uses a **narrow semantic ARIA overlay** (`crates/m1-accessibility/web/a11y-bridge.js`) mounted under `/elysianfields`:

- Consumes differential `BridgePatch` updates computed directly from `AccessibilityTreeSnapshot` by `NarrowSemanticBridge`.
- Projects **only** the visible active landmarks, interactive controls, modal scope (`aria-modal="true"` + `inert` background), focus target, and polite/assertive live regions.
- Keys every projected DOM element strictly by `data-ef-node-id` matching `accesskit::NodeId`, making duplicate accessibility nodes structurally impossible (`verify_bijective_projection` enforces `duplicated_nodes == 0`).
- Applies a fixed visually-hidden (`1x1px`, `pointer-events: none`) container style only; mirrors **zero** application CSS layout, colors, or typography, and holds **zero** authoritative text or form state (DOM `click` / `focus` / `input` events forward straight back into Rust).

For `chrome-extension-popup`, the same narrow bridge complies with MV3 CSP (`script-src 'self' 'wasm-unsafe-eval'; object-src 'self'`) with zero inline scripts or `eval`. If popup cold-start (`<= 250 ms`) or Brotli size (`<= 1.0 MiB`) triggers Decision D2, the popup falls back cleanly to a standalone semantic HTML document with native browser accessibility.

### Non-Canvas Targets (`hosted-services`, `static-public-web`)

- `hosted-services`: Headless Rust services; no client UI or accessibility surface (`N/A`).
- `static-public-web`: Indexable semantic HTML/CSS with native accessibility and zero canvas rendering.

## 4. Narrow Browser Adapter Cost Analysis

Per `specs/m1-feasibility-budgets.md` §4 (Decision D2 and the Browser Wasm Program Stop Rule), a browser accessibility adapter is acceptable only if it stays narrow and does not become a second UI stack.

| Cost Dimension | Measured / Enforced Value | Proof That Adapter Is Narrow |
| --- | --- | --- |
| **Adapter JS footprint** | 253 lines (7.5 KiB uncompressed, `< 2.5 KiB` Brotli-11) in [`crates/m1-accessibility/web/a11y-bridge.js`](../../crates/m1-accessibility/web/a11y-bridge.js) | `< 0.1%` of the `2.5–3.5 MiB` Browser Wasm compressed budget; zero external JS dependencies |
| **Mirrored CSS / visual layout rules** | `0` (`BridgeBudgetReport::mirrored_css_rules == 0`) | Visually-hidden semantic overlay only; WebGL2/Bevy remains the sole visual renderer |
| **Duplicate state / accessibility nodes** | `0` (`BridgeBudgetReport::duplicated_nodes == 0`) | Bijective 1:1 `accesskit::NodeId` <-> `data-ef-node-id` projection verified on every frame |
| **Projected DOM node ceiling** | `O(active visible controls & landmarks)` (17 visible nodes non-modal; 4 visible nodes while modal is open) | Virtualized long transcripts and 5,000-row lab tables never project off-screen rows into DOM |
| **Idle & streaming DOM mutation rate** | `0` mutations/frame when idle; `0` mutations during buffered stream tokens; `<= 1.33` live-region text writes/sec at 40 tok/s | Differential `BridgePatch` + `750–1500 ms` `LiveAnnouncementQueue` prevents main-thread layout thrash and AT flooding |
| **Subpath compatibility** | Enforced `/elysianfields` prefix (`ELYSIAN_SUBPATH_PREFIX`) | Verified in both Rust (`verify_bijective_projection`) and JS constructor guard |
| **Maintenance ownership** | 1 Rust projection module (`src/bridge.rs`) + 1 small ES module (`web/a11y-bridge.js`) | Adding a new Bevy control updates `build_accesskit_node` once; the bridge automatically projects standard `accesskit::Role` values |

**Program Stop Rule Assessment**: Browser Wasm accessibility is achievable through this narrow, zero-duplication semantic bridge driven directly from Bevy's single AccessKit tree. The Browser Wasm Program Stop Rule is **not triggered**; the narrow adapter is ready for Gatekeeper approval alongside physical-device screen-reader recordings.

## 5. Per-Target M1 Qualification Matrix

Following `specs/m1-platforms.md` §2 ("Honest coverage reporting"), automated headless and `wasm32-unknown-unknown` verification is separated explicitly from physical-hardware assistive-technology runs. Targets lacking attached physical hardware or live OS screen-reader sessions in this Linux CI environment are recorded as `blocked-no-hardware`, never `pass`.

| Ledger Platform Key | Assistive Tech / Bridge Path | Automated Headless & Build Proofs | Real-Device Screen-Reader Verification | Current M1 Accessibility Status |
| --- | --- | --- | --- | --- |
| `browser-wasm-desktop` | Narrow ARIA semantic overlay (`web/a11y-bridge.js`) from AccessKit snapshot; Chromium/Firefox/Safari + Orca/VoiceOver/NVDA | **Verified**: `wasm32-unknown-unknown` Clippy `-D warnings`, bijective projection, zero-duplicate-node check, differential patch rate cap, WCAG AA contrast, 200% zoom reflow (`1280x720`), reduced motion | **Pending hardware/AT session**: Live Chromium/Firefox/Safari VoiceOver, NVDA, and Orca walkthrough recordings | `blocked-no-hardware` (narrow adapter ready for `pass-with-approved-adapter` once AT recordings land) |
| `browser-wasm-mobile` | Narrow ARIA semantic overlay (`web/a11y-bridge.js`) + polite live region; iOS Safari VoiceOver & Android Chrome TalkBack | **Verified**: `wasm32-unknown-unknown` Clippy `-D warnings`, 200% zoom reflow (`390x844`), throttled 40 tok/s live-region cap, zero-duplicate-node check | **Pending hardware**: Physical iPhone (iOS 17/18 Safari VoiceOver) and midrange Android (Android 14 Chrome TalkBack) recordings | `blocked-no-hardware` (narrow adapter ready for `pass-with-approved-adapter` once device runs land) |
| `desktop-linux` | Native AccessKit -> `accesskit_unix` -> AT-SPI2 -> Orca | **Verified**: Native host `x86_64-unknown-linux-gnu` build, Clippy `-D warnings`, headless Bevy `0.20.1` `AccessibilityNode` tree & keyboard-only workflow tests | **Pending interactive desktop AT session**: Live Wayland/X11 window + Orca AT-SPI2 audio/tree inspection | `blocked-no-hardware` (headless Linux runner has no interactive Wayland/Orca audio session) |
| `desktop-macos` | Native AccessKit -> `accesskit_macos` -> `NSAccessibility` -> VoiceOver | **Verified (shared core)**: Portable Rust AccessKit tree, keyboard focus, contrast, zoom, reduced-motion, and live-region unit proofs | **Pending hardware**: Physical Apple Silicon MacBook Air (macOS 14/15) VoiceOver walkthrough & build timing | `blocked-no-hardware` |
| `desktop-windows` | Native AccessKit -> `accesskit_windows` -> UIA -> NVDA / Narrator | **Verified (shared core)**: Portable Rust AccessKit tree, keyboard focus, contrast, zoom, reduced-motion, and live-region unit proofs | **Pending hardware**: Physical Windows 11 23H2 laptop NVDA + Narrator walkthrough & build timing | `blocked-no-hardware` |
| `android-native` | Narrow Android `AccessibilityNodeProvider` bridge fed by `AccessibilityTreeSnapshot` -> TalkBack | **Verified (shared core)**: Portable AccessKit snapshot, 200% zoom (`390x844`), throttled live-region queue | **Pending hardware & shell wiring**: Physical Android 14 ARM64 device TalkBack verification of narrow `AccessibilityNodeProvider` adapter | `blocked-no-hardware` (requires narrow platform adapter approval) |
| `ios-native` | Narrow UIKit `UIAccessibilityElement` container proxy fed by `AccessibilityTreeSnapshot` -> VoiceOver | **Verified (shared core)**: Portable AccessKit snapshot, 200% zoom (`390x844`), throttled live-region queue | **Pending hardware & shell wiring**: Physical iPhone (iOS 17/18) VoiceOver verification of narrow UIKit accessibility proxy | `blocked-no-hardware` (requires narrow platform adapter approval) |
| `chrome-extension-popup` | Narrow ARIA overlay under MV3 CSP (`360x520`), or Decision D2 semantic HTML fallback | **Verified**: `wasm32-unknown-unknown` Clippy `-D warnings`, 200% zoom reflow (`360x520`), strict CSP-clean ES module (zero `eval`/inline scripts) | **Pending interactive browser session**: Chrome 120+ MV3 toolbar popup screen-reader + cold-start/size gate | `blocked-no-hardware` (narrow adapter or Decision D2 HTML fallback ready) |
| `hosted-services` | None (headless Rust services) | `N/A` | `N/A` | `N/A` |
| `static-public-web` | Native indexable semantic HTML/CSS (zero canvas) | `N/A` (not Bevy canvas) | Verified via standard HTML/ARIA semantics on static pages | `pass` (architecture boundary: semantic HTML only) |
