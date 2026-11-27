# Specification Package: Physician Session Shell and Sign-In Boundary

- **Package**: `specs/packages/example-session-shell`
- **Domain**: `CLINIC`
- **Product**: Curie Clinic (`curie-clinic`)
- **Requirements**: `EF-CLINIC-0001`
- **Acceptance Scenarios**: `SCN-CLINIC-0001`
- **Contracts**: `contracts/session-boundary.schema.json`

This package specifies the authentication boundary, session lifecycle, multi-tab synchronization, and recovery behavior for the Curie Clinic physician application shell. It defines purely observable behavioral, persistence, failure, and accessibility requirements independent of any rendering engine or host framework.

---

## Requirement `EF-CLINIC-0001`: Physician session shell and sign-in boundary

### 1. ID

`EF-CLINIC-0001`

### 2. Product and Platforms

- **Product**: Curie Clinic (`curie-clinic`)
- **Platforms**:
  - `browser-wasm-desktop`
  - `browser-wasm-mobile`
  - `desktop-linux`
  - `desktop-macos`
  - `desktop-windows`

### 3. Provenance

- **Status**: `observed-and-required`
- **Authority basis**:
  - Authority level 2 (Intent specifications: clinician identity verification, practice access isolation, and fail-closed session handling).
  - Authority level 3 (Compatibility contracts: subpath hosting under the configured application base path and cross-tab session revocation guarantees).
  - Authority level 4 (Observed behavior of the pinned reference physician sign-in and shell lifecycle).

### 4. Actors and Authority

- **Unauthenticated visitor**: Has zero access to protected clinical views, patient lists, messages, or practice directories. May view only the sign-in boundary and submit authentication credentials.
- **Authenticated clinician**: Verified member of at least one authorized medical practice. Holds an active, revocable session scoped to their identity and active practice context.
- **Session authority service**: Validates credentials, issues and revokes sessions, enforces expiration, and returns explicit typed standing responses (`authenticated`, `unauthenticated`, or `service-unavailable`).
- **Client shell**: Enforces the boundary locally before rendering any protected view, coordinates session standing across sibling windows or tabs, and clears protected local state immediately upon sign-out or revocation.

### 5. Preconditions

- The physician application is served from its configured application base path (supporting non-root subpath deployments).
- The client starts in one of three initial states:
  1. No active session (fresh profile or previously signed out).
  2. Active, valid session for a verified synthetic clinician account.
  3. Expired or server-revoked session still referenced by a prior window or tab.
- The session authority service is either reachable or fault-injected to simulate network outage / timeout / HTTP 5xx responses.
- No clinical records or patient identifiers are pre-rendered prior to session verification.

### 6. Actions

1. **Cold entry while unauthenticated**: Open the application root or navigate directly to a protected route (for example, a daily worklist or a specific patient chart view).
2. **Invalid sign-in attempt**: Submit malformed or rejected credentials at the sign-in boundary.
3. **Valid sign-in**: Submit valid synthetic clinician credentials and complete the sign-in flow.
4. **Concurrent multi-window or multi-tab session**: Open a second tab or window against the same client profile so both views display the authenticated physician shell.
5. **Explicit sign-out or remote revocation**: Trigger sign-out in one tab (or revoke the session at the session authority while a protected action runs in another tab).
6. **Reload and suspend/resume**: Reload the browser tab or suspend and resume the desktop application while authenticated, and again after session expiration.
7. **Fault recovery**: Disconnect or fail the session authority during initial session check and during sign-in submission, then restore service and activate the retry control.

### 7. Visible Result

- **Fail-closed unauthenticated boundary**: When unauthenticated, the application presents a complete, readable sign-in view with an obvious primary heading, credential inputs, and submit control. It never displays a blank document, an infinite spinner without status, or any protected clinical data, patient names, or workspace navigation.
- **Safe return-destination preservation**: Navigating directly to a protected route while unauthenticated holds the intended in-app destination and navigates there automatically after successful sign-in. External or cross-origin redirect targets outside the configured application base path are rejected and fall back to the default physician landing view.
- **Invalid credential feedback**: Rejected credentials keep the user at the sign-in boundary with a clear, non-enumerating error message (never disclosing whether an email address or account identifier exists) and preserve safe user input focus for correction.
- **Authenticated physician shell**: Upon valid authentication, the client transitions into the physician shell showing the authenticated clinician's display identity, active practice name, primary navigation landmarks, and an explicit sign-out action.
- **Multi-tab synchronization**: Signing out or detecting session revocation in one tab updates all sibling tabs in the same profile promptly without requiring manual page refreshes. Every sibling tab clears protected clinical views and returns to the sign-in boundary with a calm, plain-language notice explaining that the session ended.
- **Explicit error states**: When the session authority is unreachable or returns an unexpected error, the shell displays a clear, recoverable error state with a working retry control rather than silently treating the user as signed out with an empty workspace or leaving the screen blank.

### 8. Persisted and External Effects

- **Session credential handling**: Session tokens or cookies reside strictly in platform-appropriate secure or HTTP-only storage. Provider secrets, backend keys, and long-lived credentials never ship in client bundles or plaintext client state.
- **Revocation and cache clearing**: Explicit sign-out notifies the session authority to invalidate the server-side session and clears all in-memory and local client caches containing protected clinical data before reporting sign-out complete.
- **Auditability**: Both successful authentications and failed sign-in attempts emit structured audit events at the session authority containing timestamp, outcome category, and request correlation metadata, without logging plaintext passwords or secrets.
- **Product isolation**: Physician authentication sessions never grant access to Curie Mobile patient accounts or patient-owned storage.

### 9. Failure and Restart Behavior

- **Unreachable session authority on startup**: If initial standing verification fails due to network timeout or service failure, the client stays on a non-clinical boundary screen reporting the outage and offering a retry trigger. A failed standing check never reads as `authenticated` and never renders a blank surface.
- **Mid-session expiration or revocation**: Any protected request rejected as unauthenticated transitions the shell cleanly to the sign-in boundary, preserves the safe in-app return destination, broadcasts the ended-session state to sibling tabs, and discards unsynchronized protected view state safely (without silently dropping in-progress work without notice).
- **Duplicate or racing transitions**: Clicking sign-in or sign-out repeatedly, or triggering sign-out concurrently in two tabs, is idempotent: the client neither deadlocks nor leaves one tab authenticated while another is signed out.
- **Restart and resume**: After a full page reload, browser restart, or desktop suspend/resume cycle, the client re-verifies session standing with the session authority before exposing protected views. Valid sessions restore the shell seamlessly; expired sessions transition cleanly to the sign-in boundary.

### 10. Accessibility and Input Behavior

- **Landmarks and semantics**: Both the sign-in boundary and the authenticated shell expose unambiguous semantic regions (main content, navigation, status/alert regions) and a single logical heading hierarchy.
- **Keyboard operation**: Every interactive control on the sign-in boundary and shell header is reachable and operable strictly by keyboard in logical reading order, with visible focus indicators and zero keyboard traps. Pressing Enter inside credential inputs submits the form; pressing Escape closes transient modals or notices and restores focus to the triggering control.
- **Screen-reader announcements**: Validation errors, service outage messages, session-ended notices, and authentication state transitions are exposed through assistive-technology status/alert semantics so screen readers announce them promptly.
- **Text input, IME, and localization**: Credential and search inputs support native OS text composition (IME), clipboard paste, password-manager autofill, and right-to-left or multi-script text without maintaining duplicate editable text copies or duplicate screen-reader nodes.

### 11. Executable Acceptance Scenario

Verified end-to-end by scenario `SCN-CLINIC-0001` in `specs/packages/example-session-shell/scenarios.toml`, conforming to the session standing state contract in `specs/packages/example-session-shell/contracts/session-boundary.schema.json`.
