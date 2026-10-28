# P3 rate limiter refund investigation

## Verified contract and conflict

- `@prime/PLAN.md` §7.5.4, lines 7771–7772 asks Scenario C to verify that TPM reconciliation “returns unused budget” and later requests proceed.
- `crates/polaris-ratelimit/src/permit.rs` says lower actual usage is recorded but does not refund because out-of-order GCRA rollback is unsafe. Dispatched permit drop also retains the debit.
- `crates/polaris-ratelimit/src/gcra.rs` stores aggregate theoretical arrival time in one `AtomicU64`; event identity and each request's current contribution are discarded.
- Root clarified PLAN §13 requires zero-allocation `try_acquire` and lock-free RPM and TPM fast paths. Therefore a mutex, allocation journal, or changed TPM token-bucket semantics is out of scope.
- The dispatched-drop fixture initially used reservation 8 followed by a cost-3 request. That was invalid: GCRA's `burst_tokens` is its theoretical-arrival tolerance, so combined same-tick weighted costs can exceed 10. The corrected case reserves the full 10-token tolerance and verifies that a further 1-token request is denied. This current behavior passes.

## Adversarial case

With a 1 second token period, reserve A=9 at t=0 (`TAT=9`). At t=3, reserve B=9 (`TAT=18`). Correctly removing A and replaying B gives `TAT=12`. Naively subtracting A's 9 seconds from aggregate TAT yields `TAT=9`, which would allow three subsequent 1-token reservations at t=3 while the correct tail permits one. The integration test records this counterexample and a real limiter workload that expects safe reconciliation behavior.

## Current verification

Commands:

- `cargo test -p polaris-ratelimit --test reservation_refund -- --nocapture`
- `cargo clippy -p polaris-ratelimit --test reservation_refund -- -D warnings`

Raw test result:

```text
running 7 tests
thread 'concurrent_refund_preserves_other_reservations' panicked: first permit returns only its four unused tokens
thread 'late_refund_never_erases_another_request_charge' panicked: late refund should admit from A's unused capacity
thread 'reconciliation_returns_unused_tokens' panicked: unused ten-token reservation must return immediately
thread 'undispatched_drop_refunds_reservation' panicked: undispatched permit drop releases its token reservation
test dispatched_drop_keeps_reservation_charged ... ok
test overrun_charges_actual_usage_before_return ... ok
test naive_scalar_refund_erases_fresh_request_debt ... ok

failures:
    concurrent_refund_preserves_other_reservations
    late_refund_never_erases_another_request_charge
    reconciliation_returns_unused_tokens
    undispatched_drop_refunds_reservation

test result: FAILED. 3 passed; 4 failed; 0 ignored
```

Raw clippy result:

```text
Checking polaris-ratelimit v0.1.0-alpha (C:\mDev\Polaris\crates\polaris-ratelimit)
Finished `dev` profile [unoptimized] target(s) in 0.81s
```

This is an intentional RED contract test result. No limiter source was changed because a safe bounded lock-free, zero-allocation design was not established during this investigation.
