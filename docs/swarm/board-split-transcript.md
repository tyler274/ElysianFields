# Split Two-Board Isolation Proof Transcript

Recorded live execution of `scripts/board_split.py` verifying independent observer and builder boards and strict cross-wall rejection.

## Run Metadata

| Field | Value |
| --- | --- |
| Verdict | **PASS** |
| Timestamp (UTC) | `2026-11-27T13:25:55+00:00` -> `2026-11-27T13:25:55+00:00` |
| Total Checks | `6` |

## Verification Summary

| # | Check | Expected | Verdict | Summary |
| --- | --- | --- | --- | --- |
| 1 | `observer_board_roundtrip` | PASS (observer worker posts to #observe and observer coordinator receives it) | **PASS** | Observer board initialized with ['#observe', '#spec', '#verify', '#integrate']; posted seq=1 by su-observer and read by coord-observer |
| 2 | `builder_board_roundtrip` | PASS (builder worker posts to #slice-m1 and builder coordinator receives it) | **PASS** | Builder board initialized with ['#slice-m1', '#questions']; posted seq=1 by villv-m1 and read by coord-builder |
| 3 | `builder_read_observer_blocked` | FAIL with WallViolationError | **PASS** | Blocked with WallViolationError: Cross-wall read from denied: builder-board member 'villv-m1' (Vill-V · build) attempted to read from observer board 'observer-main'. |
| 4 | `two_coordinator_rule_enforced` | FAIL with WallViolationError (observer coordinator cannot post builder briefs) | **PASS** | Blocked with WallViolationError: Cross-wall post to denied: observer-board member 'coord-observer' (coord-observer) attempted to post to builder board 'builder-realm'. |
| 5 | `dual_board_membership_blocked` | FAIL with WallViolationError (no agent joins both boards) | **PASS** | Blocked with WallViolationError: Dual-board membership denied: agent 'villv-m1' is already registered on builder board 'builder-realm'. |
| 6 | `filesystem_permissions_audit` | PASS (0700 board directories, 0600 secrets/metadata/channel logs) | **PASS** | Verified 0o700 directories and 0o600 files on both observer and builder boards |

## Check Details

### Check 1: `observer_board_roundtrip` (PASS)

- **Expected**: PASS (observer worker posts to #observe and observer coordinator receives it)
- **Observed**: Observer board initialized with ['#observe', '#spec', '#verify', '#integrate']; posted seq=1 by su-observer and read by coord-observer

```json
{
  "board_id": "observer-main",
  "channels": [
    "#observe",
    "#spec",
    "#verify",
    "#integrate"
  ],
  "posted_message": {
    "seq": 1,
    "timestamp_utc": "2026-11-27T13:25:55+00:00",
    "channel": "#observe",
    "agent_id": "su-observer",
    "role": "Su \u00b7 observe",
    "body": "[done] Reference inventory captured for M0 review; ready for Gatekeeper sanitization."
  },
  "received_count": 1
}
```

### Check 2: `builder_board_roundtrip` (PASS)

- **Expected**: PASS (builder worker posts to #slice-m1 and builder coordinator receives it)
- **Observed**: Builder board initialized with ['#slice-m1', '#questions']; posted seq=1 by villv-m1 and read by coord-builder

```json
{
  "board_id": "builder-realm",
  "channels": [
    "#slice-m1",
    "#questions"
  ],
  "posted_message": {
    "seq": 1,
    "timestamp_utc": "2026-11-27T13:25:55+00:00",
    "channel": "#slice-m1",
    "agent_id": "villv-m1",
    "role": "Vill-V \u00b7 build",
    "body": "[done] Prototype built against specs/packages/m1-feasibility; clippy and tests green."
  },
  "received_count": 1
}
```

### Check 3: `builder_read_observer_blocked` (PASS)

- **Expected**: FAIL with WallViolationError
- **Observed**: Blocked with WallViolationError: Cross-wall read from denied: builder-board member 'villv-m1' (Vill-V · build) attempted to read from observer board 'observer-main'.

```json
{
  "error": "Cross-wall read from denied: builder-board member 'villv-m1' (Vill-V \u00b7 build) attempted to read from observer board 'observer-main'."
}
```

### Check 4: `two_coordinator_rule_enforced` (PASS)

- **Expected**: FAIL with WallViolationError (observer coordinator cannot post builder briefs)
- **Observed**: Blocked with WallViolationError: Cross-wall post to denied: observer-board member 'coord-observer' (coord-observer) attempted to post to builder board 'builder-realm'.

```json
{
  "error": "Cross-wall post to denied: observer-board member 'coord-observer' (coord-observer) attempted to post to builder board 'builder-realm'."
}
```

### Check 5: `dual_board_membership_blocked` (PASS)

- **Expected**: FAIL with WallViolationError (no agent joins both boards)
- **Observed**: Blocked with WallViolationError: Dual-board membership denied: agent 'villv-m1' is already registered on builder board 'builder-realm'.

```json
{
  "error": "Dual-board membership denied: agent 'villv-m1' is already registered on builder board 'builder-realm'."
}
```

### Check 6: `filesystem_permissions_audit` (PASS)

- **Expected**: PASS (0700 board directories, 0600 secrets/metadata/channel logs)
- **Observed**: Verified 0o700 directories and 0o600 files on both observer and builder boards

```json
{
  "observer_board": {
    "board_id": "observer-main",
    "side": "observer",
    "ok": true,
    "modes": {
      ".": "0o700",
      "channels": "0o700",
      ".board-secret": "0o600",
      "board.json": "0o600",
      "members.json": "0o600",
      "channels/observe.jsonl": "0o600",
      "channels/spec.jsonl": "0o600",
      "channels/verify.jsonl": "0o600",
      "channels/integrate.jsonl": "0o600"
    },
    "violations": []
  },
  "builder_board": {
    "board_id": "builder-realm",
    "side": "builder",
    "ok": true,
    "modes": {
      ".": "0o700",
      "channels": "0o700",
      ".board-secret": "0o600",
      "board.json": "0o600",
      "members.json": "0o600",
      "channels/slice-m1.jsonl": "0o600",
      "channels/questions.jsonl": "0o600"
    },
    "violations": []
  }
}
```
