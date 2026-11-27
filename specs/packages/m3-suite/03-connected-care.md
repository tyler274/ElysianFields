# 3. Connected Care Bridge & Product Isolation (M3 Increment 3)

Connected care is the sole authorized bridge between a patient's standalone Curie Mobile account and a medical practice in Curie Clinic.

## 3.1 Single Authoritative State Machine

One isolated bridge service/store owns the canonical connection state for each `(practice_id, practice_patient_id)` pair:

```text
Not invited ──(practice invites)──> Invited ──(DOB verified + patient taps Share)──> Connected
     ^                                 │                                                │
     │                                 ├──(practice cancels)──> Not invited / Ended     │
     │                                 └──(5 wrong DOBs)──> Invited (Locked)            │
     │                                                                                  │
     └────────────────────(patient leaves / practice disconnects / account deleted)─────┴──> Ended
                                                                                               │
                                                             (practice re-invites)─────────────┘
```

- **Zero drift:** Both Curie Clinic and Curie Mobile read connection standing (`Not invited`, `Invited`, `Connected`, `Ended`) from this single owner (`SameConnectionState`).
- **Zero cross-product code or storage sharing:** The bridge service itself holds zero clinical chart dependencies from either product. Curie Clinic and Curie Mobile maintain separate databases, user accounts, service identities, secret scopes, and cache brokers (Curie Mobile never connects to physician Redis/Valkey). Compile-time workspace boundary tests enforce this isolation permanently.

## 3.2 Invitation, DOB Verification & Share Consent (11 Formal Invariants)

1. **`AtMostOneOpen`:** A patient-practice pair has at most one open invitation at any time; creating or resending never spawns a second open invitation.
2. **`ConnectsOnlyOnOpenShare`:** A connection transitions to `Connected` solely when the signed-in patient explicitly taps **Share** on an invitation that the practice holds open at that exact moment.
3. **`OnlyShareGrantsConsent`:** Sending an invitation, resending it, opening the `/invite` link or short code, and passing date-of-birth verification grant **zero** sharing consent and transfer nothing.
4. **`ResendNeverSplitsOrUnlocks`:** Resending applies only to an open, **unlocked** invitation; it never creates a second invitation, never closes the open one, never unlocks a locked invitation, and keeps previously emailed links and short codes valid.
5. **`CancelledNeverConnects`:** Cancelling an open invitation (whether unlocked or locked) immediately invalidates every previously mailed link and short code so none can connect, while allowing the practice to issue a brand-new invitation later.
6. **`PreviewOnlyAfterDob` / `DobVerifiedOnlyAfterMatch`:** Before entering a date of birth matching the practice's invited patient record, the patient sees **no** practice name, physician name, or sharing preview.
7. **`LockedAfterFiveWrongDob`:** Exactly **5** wrong date-of-birth guesses lock the open invitation permanently against further attempts. Only cancelling the locked invitation and issuing a fresh invitation starts over.
8. **`FailedMailLeavesOpen`:** If outbound invitation email delivery fails, the invitation stays open so the practice can retry/resend or cancel without losing state.
9. **`ShareConnectsSignedInAccount`:** Tapping **Share** binds the currently signed-in Curie Mobile account (never creating a duplicate patient account). Leaving the preview screen without tapping **Share** keeps the invitation open and shares nothing.
10. **`PracticesStayIndependent`:** Inviting, connecting, locking, cancelling, or ending a connection with Practice A never alters the patient's state with Practice B.
11. **`SameConnectionState`:** Curie Clinic and Curie Mobile always agree on the current connection status for any patient-practice pair.

## 3.3 Data Flow, Disconnection, Rejoining & Account Deletion

- **Explicit transfers only:** While `Connected`, only confirmed immutable sends from Curie Mobile's **For my doctor** queue cross into the patient's Curie Clinic chart (attributed to the patient and timestamped when sent). Historical `To send` items never transfer automatically upon connecting.
- **Ending a connection:** Either the patient (from Curie Mobile) or an authorized practice member (from Curie Clinic) can end an active connection (`Connected -> Ended`). Ending stops all future transfers immediately, halts any `Sending`/`Uncertain` mobile sends without deleting earlier `Sent` history, and leaves the patient's standalone Curie Mobile account intact.
- **Rejoining:** A practice may issue a fresh invitation after a connection reaches `Ended`; the patient goes through the full DOB-verification + explicit **Share** consent flow again.
- **Account deletion coordination:** Deleting a Curie Mobile account (including across independent privacy journal replay after a DB restore) idempotently terminates every associated connected-care link and scrubs any staged transfer records in the bridge store.
