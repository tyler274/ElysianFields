# 1. Physician Sign-In, Session Lifecycle & Multi-Tab Consistency

## 1.1 Actors & Preconditions

- **Actor:** An invited, active physician belonging to one practice (`practice_id`, `physician_id`, normalized lower-case `email`, `display_name`, `admin` boolean).
- **Invite-only invariant:** Unknown or deactivated email addresses never reveal account existence on sign-in initiation and never send outbound mail.
- **Client environment:** One or more concurrent browser tabs (or native app windows) sharing origin-scoped session storage and cross-tab change notifications.

## 1.2 Authentication Endpoints & Wire Semantics

All authentication responses carrying session credentials or refusals MUST set `Cache-Control: no-store`.

### Initiate Sign-In (`POST /auth/sign-in`)

- **Request:** `{ "email": "<address>" }` (body cap `≤ 64 KiB`, email trimmed and ASCII lower-cased).
- **Hosted behavior:**
  - Active account: mints a single-use sign-in token (TTL **24 hours** = `86,400 s`; colleague invitation TTL **14 days** = `1,209,600 s`), delivers sign-in link via outbound mail transport, and returns `200 OK` with `{ "status": "unavailable", "token": null, "message": "If that address has an account, a sign-in link is on its way." }`.
  - Unknown or deactivated account: sends **no** email and returns the **identical** `200 OK` response body (anti-enumeration).
  - Mail delivery failure: returns `502 Bad Gateway` with `{ "detail": "Sign-in email could not be delivered right now.", "code": "mail_delivery_failed" }`.
- **Local proof / deterministic test behavior:** When no outbound mail transport is configured (proof mode only, never hosted), returns `200 OK` with `{ "status": "token_issued", "token": "<one-time-token>", "message": "Sign-in token issued." }`.

### Verify Sign-In Link / Token (`POST /auth/verify`)

- **Request:** `{ "email": "<address>", "token": "<one-time-token>" }`.
- **Atomicity:** Token consumption and session minting happen atomically; replaying a previously verified token is rejected.
- **Outcomes:**
  - Valid active account & unused non-expired token: returns `200 OK` (`Cache-Control: no-store`) with `{ "session": "<bearer-token>", "physician_id": "...", "practice_id": "..." }`.
  - Deactivated account (`active == false`): returns `403 Forbidden` (`Cache-Control: no-store`) with `{ "detail": "Access removed.", "reason": "access_removed", "code": "access_removed" }`.
  - Invalid, expired, or already-used token: returns `401 Unauthorized` (`Cache-Control: no-store`) with `{ "detail": "Sign in to read the practice.", "reason": "session_ended" }`.
  - Blank email or token: returns `401 Unauthorized`.

### Session Identity Check (`GET /auth/whoami`)

- **Authenticated active session:** Returns `200 OK` with `{ "physician_id", "practice_id", "email", "display_name", "admin", "practice_accounts": [...] }` and records physician activity for sliding idle renewal.
- **Refusal codes (`401 Unauthorized`, `Cache-Control: no-store`):**
  - Missing/empty bearer token -> `"reason": "no_session"`
  - Unknown, signed-out, or expired token (or token created before `account.sessions_ended_before`) -> `"reason": "session_ended"`
  - Account deactivated or practice access removed -> `"reason": "access_removed"`

### Sign Out (`POST /auth/sign-out`)

- **Behavior:** Ends the presented session token on the server, broadcasts authority revocation (`resync` / `physician_authority` notification) so open SSE streams for that session close across all backend instances immediately, and returns `204 No Content`.
- **Fallback sign-out rule:** If shared browser storage is already empty when a tab invokes sign-out, the tab MUST still present its own held session token to the server so the session is terminated server-side.

## 1.3 Session Lifetime & Sliding Renewal Rules

- **Idle timeout (`IDLE_LIMIT`):** `12 hours` (`43,200 s`) from mint or last recorded activity.
- **Absolute lifetime cap (`ABSOLUTE_CAP`):** `7 days` (`604,800 s`) from `created_at`. Persisted deadlines exceeding `created_at + 7d` are clamped to `created_at + 7d`.
- **Activity renewal quantum (`ACTIVITY_QUANTUM`):** `15 minutes` (`900 s`). Valid requests within 15 minutes of the existing idle deadline do not issue redundant DB writes (`Covered`), except when clamping right onto the 7-day cap; once `now + 12h >= idle_deadline + 15m`, the idle deadline advances (`Extended`), never past `absolute_end`.
- **Fixed-end sessions:** A session minted with an explicit hard `ends_at` (e.g., bounded demo/rehearsal session) never extends past `min(ends_at, created_at + 7d)` regardless of activity.
- **Strict inequality:** A session is valid strictly while `now < idle_deadline`; at `now == idle_deadline` it is `Ended` and cannot renew.
- **Deactivation persistence:** Account deactivation (`active = false`) and `sessions_ended_before` fences persist across migrations and server restarts; every request checks live account standing.

## 1.4 Client View State Machine & Multi-Tab Sync

### Tab View States

| View State | Protected Content Shown | Session Held | Meaning |
| --- | --- | --- | --- |
| `Opening` | None | None (checking stored token) | Initial boot verifying session while preserving any deep-link `?patient=` target (`wants`). |
| `SignInPage` | None | None | Unauthenticated sign-in screen (`opens = Nothing`). |
| `SignInLink` | None | Prior held (until verified) | Verifying an emailed sign-in link; always resets `wants` to `Today`. |
| `Workspace` | `Today` or `LinkedPatient` | Active live session | Authenticated clinical workspace. |
| `SessionEndedPrompt` | None | None | Replaces workspace immediately on expiry/revocation; clears all patient content (`opens = Nothing`, `holds = None`). |

### Cross-Tab Notification Channel

- Browser tabs announce session storage changes over an origin-scoped `BroadcastChannel` (filtering out self-originated tab IDs and ignoring malformed payloads) with a `window` `storage`-event fallback plus `focus` / `visibilitychange` reconciliation.
- Native desktop/mobile single-process multi-window apps use an equivalent in-process event bus obeying the exact same state-transition rules.

## 1.5 Formal Behavioral Invariants (Derived from Verified Sign-In Protocol)

Every M2 implementation MUST satisfy all **11 invariants**:

1. **`OnceOut`:** Once signed out (browser storage cleared), a tab catching up to peer news never remains in or re-enters `Workspace` without a fresh live session.
2. **`Expiry`:** When a tab's held session expires on the server and no newer live session has replaced it in browser storage, an ended-session refusal clears all patient content and transitions the tab out of `Workspace` into `SessionEndedPrompt` (if previously in `Workspace`) or `SignInPage`.
3. **`KeepsWork`:** A late ended-session HTTP response from an older request never kicks a tab back to sign-in when browser storage already holds a newer live session established by another tab; the tab adopts the newer live session cleanly.
4. **`SignedIn`:** Every tab displaying `Workspace` holds a concrete live session token, and no tab remains stuck on `SignInPage` or `SessionEndedPrompt` once a live session exists in browser storage.
5. **`TodayFirst`:** Signing in via an emailed sign-in link or re-signing in from `SessionEndedPrompt` always lands on `Today` (never restoring stale patient context from a prior session or user).
6. **`Lands`:** A tab opened directly with a deep link (`?patient=<workspace_id>`) lands on that linked patient once signed in—even if another tab completes sign-in while the deep-linked tab is still `Opening` or on `SignInPage`.
7. **`ActsAsShown`:** Every authenticated workspace mutation or clinical read executes using the tab's held session only while it matches shared browser storage and is live on the server.
8. **`PromptIsEmpty`:** Whenever `SessionEndedPrompt` is displayed, zero patient content is rendered or retained in memory (`opens = Nothing`) and `holds = None`.
9. **`ServerRefusesSignedOut`:** Signing out terminates both the stored session and the tab's held session on the server (including fallback sign-out when storage was already cleared by another tab).
10. **`CatchesUp`:** Under normal delivery of peer notifications and server answers, every open tab eventually converges: signed-out tabs leave `Workspace`, expired tabs clear patient content, and sign-in tabs adopt fresh live sessions.
11. **`NoDeadlock`:** No reachable combination of concurrent tabs, link verifications, expiries, and sign-outs stalls a tab in `Opening` or `SignInLink` without a valid continuation.
