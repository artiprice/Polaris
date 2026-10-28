# Bounded refund ledger design review

> Historical review from the initial scalar-ledger audit. Its stop decision is superseded by
> `03-implementation-review.md`, which records the shared snapshot and helpable-intent implementation
> and its remaining verification gaps.

## Status

VERIFIED: the max-plus transform shape is closed under composition. An admission at
time `t` with cost `c` and period `p` is `f(x) = max(x + p*c, t + p*c)`.
Composing transforms `f(x) = max(x + d1, a1)` and
`g(x) = max(x + d2, a2)` gives
`g(f(x)) = max(x + d1 + d2, a1 + d2, a2)`, which is the same shape. This can
compact contiguous settled history while preserving each later time floor.

UNVERIFIED: the proposed 8-slot snapshot publication/reclamation protocol has not
been implemented or stress-tested. No production code was changed in this review.

## Counterexample A — separate RPM and TPM snapshots can strand an RPM leaf

Assume fail-fast slot acquisition and a global reader count that prevents reclaiming
retired slots while any reader is pinned:

1. A reader pins an older published snapshot and is paused.
2. Writers publish snapshots using all seven non-head slots. The paused reader
   prevents retiring snapshots from being reclaimed.
3. `try_acquire` publishes an active RPM reservation.
4. TPM admission fails because the token burst is full.
5. Canceling the RPM reservation needs another snapshot, but all slots are occupied
   and reclamation cannot pass the paused reader. Cancellation returns `Busy`.

The caller returns a TPM denial with no permit, while the active RPM leaf remains
unsettled permanently. This is avoided by publishing both RPM and TPM changes in a
single shared quota snapshot: if TPM admission fails, publish neither change.

## Counterexample B — consuming a permit on settlement `Busy` strands TPM history

Suppose `reconcile(self, actual)` consumes the permit and its ledger publication
returns `Busy` because there is no reusable snapshot slot. The dispatched permit's
Drop path must conservatively retain the reservation, but the caller no longer has a
handle to retry settlement. The active leaf remains indefinitely and continues to
occupy bounded ledger capacity. Repeating this under contention can exhaust all
reservation slots even after provider usage is known.

A safe API needs either an owned retry handle returned with `Busy`, or a helpable
settlement intent that later operations can finish. A plain error after consuming
the permit is insufficient. The helpable-intent path and its publication ordering
are not implemented or verified here.

## Behavioral RED

`01-same-time-red.txt` records the current implementation's focused failure:
`same_time_non_tail_refund_preserves_newer_reservation` reports `left: 0`,
`right: 4`. The descriptor JSON was parsed successfully before this run. The existing
three contradictory fixtures remain unchanged.

## Decision

Do not replace the scalar implementation until the shared RPM/TPM transaction and
non-lossy settlement retry lifecycle are specified and verified. The transform
algebra alone does not establish safe publication, reclamation, or API behavior.
