# Shared snapshot refund-ledger implementation

## Change summary

`polaris-ratelimit` now keeps RPM and TPM logs in one versioned, immutable snapshot pool. Admission
appends both cells in one head CAS. Each active permit reserves an intent slot and generation at
admission, so settlement, undispatched cancellation, and dispatched-drop commit can publish work
without first finding a snapshot slot. Ready intents are replayed by the next admission or other
ledger mutation. A completed settlement reports a refund only after the generation-tagged outcome
is acknowledged; a pending settlement sets `settlement_deferred` and reports zero refunded tokens.

Settled runs compact as max-plus transforms. For an admission at time `t`, cost `c`, and period `p`,
`f(x) = max(x + p*c, t + p*c)`. Composition remains in this form and preserves later admission
floors. Expired debt is excluded from the reported refund by comparing both old and new tails at the
settlement time.

The 8 snapshot slots and fixed intent/log arrays are initialized with the limiter. A full slot pool
returns typed `Busy`; it does not wait or allocate. Snapshot readers pin through a checked reader
gate; reclamation requires the gate and zero readers. The head carries a checked, non-reused version
and slot index. All published snapshot fields and intent fields are atomic.

## Review findings incorporated

- RPM and TPM admission share one snapshot, so TPM denial publishes neither charge.
- The permit owns an intent slot through its reserved → ready → applied lifecycle. Deferred Drop
  intents are owned by the ledger. A synchronous reconcile keeps the slot in `APPLIED_OWNER` until
  it reads its result; deferred ownership transfers to the ledger before return.
- A mutation acknowledges the result referenced by its exact pinned source snapshot before clearing
  that result from a candidate. The result word is generation-tagged, so a stale publisher cannot
  overwrite a reused slot.
- Intent payload reads are followed by a generation/state recheck. Intent helpers process earlier
  monotonic settlement timestamps first.
- If a fixed replay log cannot fit a forced overrun charge, the candidate is rejected with `Busy`;
  it does not overwrite an unsettled leaf.
- `try_acquire` helps pending intents before utilization-based shedding, so Background requests
  cannot keep a refund hidden behind a stale pressure reading.
- Window metric counters saturate and set `accounting_saturated`; reconciliation publishes actual
  usage before updating that diagnostic window.
- The concurrent-admission integration fixture synchronizes 24 workers with one coordinator using a
  25-party barrier. An earlier draft used 24 parties and hung in the coordinator's second barrier
  generation; that run was stopped without a terminal test result.
- A dedicated `dhat` integration target now measures 512 warmed reserve/dispatch/reconcile cycles.
  Its zero-allocation assertion is pending execution.

## Verification recorded so far

- Descriptor JSON parsed with `python -m json.tool` after test-list updates.
- An earlier focused behavioral run reported the four independent out-of-order cases GREEN and the
  three protected contradictory fixtures RED (`11 passed; 3 failed`). The contradictions remain
  unchanged: two fixtures deny their second 9-token admission at t=3, and the third simultaneously
  requires a zero refund and capacity freed by that refund.
- A saturation unit test passed in an earlier checkpoint. More race, Background-shedding, window
  overflow, and concurrent admission tests were added afterward.
- `cargo check -p polaris-ratelimit -p polaris-search -p polaris-nebius-cloud-token-factory`
  passed before the latest window counter ordering changes. Its raw output ended with
  `Finished dev profile ... in 19.88s`.

## Open verification

The latest source edits have not yet completed a Cargo run. One integration invocation hung because
of the test fixture's mismatched barrier participant count; the exact test child was stopped after
process inspection, and the fixture is corrected. The expanded test suite, Clippy with the workspace
safety lint set, formal concurrency review, and release latency/allocation measurements remain open.
The existing window telemetry path still uses its pre-existing mutex; `try_acquire` does not enter
that path.
