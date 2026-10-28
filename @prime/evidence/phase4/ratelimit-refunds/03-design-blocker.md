# Weighted GCRA refund audit

## Status

VERIFIED: the existing GCRA is a scalar `AtomicU64` theoretical-arrival-time cell. Each reservation stores only `{ old_tat_ns, new_tat_ns, baseline_ns }`. `settle_if_unchanged` can replace a reservation only when the scalar still equals that reservation's `new_tat_ns`; a non-tail settlement retains all unused tokens. No production source was changed in this audit.

VERIFIED: `@prime/prompt.md` §2.2 requires lock-free/wait-free hot-path structures and forbids mutexes/blocking synchronization; `crates/polaris-ratelimit/src/lib.rs` documents `try_acquire` as nonblocking and no-allocation. A mutex or heap-backed event log would violate this contract.

## Behavioral evidence

Command: `cargo test -p polaris-ratelimit --test reservation_refund -- --nocapture`

Captured raw output: `01-current-suite.txt`.

Result: 7 passed, 7 failed. The original three failing contracts remain untouched. The additional failures are independent examples where tail-only settlement retains refundable capacity:

- `same_time_non_tail_refund_preserves_newer_reservation`: A reserves 6, B reserves 4 at t=0; A's actual use is 2. Expected refund is 4, leaving A actual 2 + B 4. Current result is 0 refunded.
- `staggered_non_tail_refund_preserves_later_admission_floor`: A reserves 6 at t=0, B reserves 4 at t=3. A uses 2. Replaying the requests yields TAT 7 (B's t=3 floor plus 4), versus current TAT 10; exactly 3 tokens are refundable and 1 of A's unused tokens is retained. Current result is 0 refunded.
- `out_of_order_same_time_refunds_recover_only_settled_capacity`: A/B/C reserve 6/2/2 at t=0. B settles to 0, then A settles to 2. The resulting charged total is A2 + C2 = 4, so a 6-token follow-up exactly fills the 10-token burst. Current implementation does not settle B while C is the tail.
- `earlier_refund_respects_multiple_later_time_floors`: A/B reserve 6/2 at t=0; C reserves 2 at t=3. If A uses 0, replay gives B2, then `max(2, 3)+C2 = 5`; thus 5 of A's 6 tokens are recoverable, 1 remains due to the t=3 floor. Current result is 0 refunded.

Focused behavioral RED command: `cargo test -p polaris-ratelimit --test reservation_refund same_time_non_tail_refund_preserves_newer_reservation -- --nocapture`

Captured raw output: `02-same-time-red.txt`. It fails at the refund count assertion (`left: 0`, `right: 4`).

## Existing contradictory fixtures (preserved)

- `late_refund_never_erases_another_request_charge` and `expired_older_reservation_cannot_refund_newer_live_debt` request two 9-token reservations with burst 10, advancing to t=3 before the second. At 60 TPM, the first TAT is 9; the second would produce 18 while weighted-burst capacity at t=3 is 13, so admission must be denied. Both fixtures terminate at “second reservation denied” before testing the named refund behavior.
- `concurrent_refund_preserves_other_reservations` requires the first settlement to report 0 refunded and 4 retained, then requires a new 4-token request to succeed immediately. With 60 TPM, burst 10, and both reservations at t=0, TAT remains 10 after the asserted zero refund; another cost 4 must be denied. The assertions cannot all hold simultaneously.

These assertions were not edited or weakened.

## Why no production workaround was applied

VERIFIED: one scalar TAT is insufficient to reconstruct the exact counterfactual tail for an older reservation once later reservations have been admitted. The later requests' floors depend on the prefix TAT at each admission. If reservations are settled out of order, earlier settlements also change those prefixes. Reconstructing the result requires ordered admission/settlement history and safe bounded reclamation while permits remain outstanding.

LIKELY: a bounded lock-free operation ledger could meet that need, but it would require a new helping/publication protocol that linearizes each admission with ledger publication, keeps records alive until all older/later settlement dependencies clear, and prevents slot-reuse ABA. The current API and scalar CAS do not supply those ordering/lifecycle guarantees. No bounded ledger algorithm was implemented or verified here. A partial “subtract unused tokens” CAS would be unsafe because it can cross later time floors; a mutex or unbounded log would violate the existing hot-path requirements.

## Descriptor

`specs/descriptors/ratelimit_out_of_order_refund.json` lists the original and new regression functions. JSON parsing succeeded with `python -m json.tool`; test-name/source matching was checked with `rg` and printed all seven listed functions.
