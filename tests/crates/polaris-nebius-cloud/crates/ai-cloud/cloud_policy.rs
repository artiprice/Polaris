use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use polaris_nebius_cloud_ai_cloud::{
    BudgetError, BudgetGuard, IngressPolicy, LifetimeError, LifetimeGuard, LifetimeStatus,
    OwnerMarker, ResourceId, ResourceLabels, SpecError, ValidatedVmSpec, select_owned_resources,
};

fn ip(value: Ipv4Addr) -> IpAddr {
    IpAddr::V4(value)
}

#[test]
fn ceil_estimate_rounds_any_partial_hour_up() {
    let guard = BudgetGuard::new(10);
    let reservation = guard.reserve(10, Duration::from_secs(1));
    assert!(reservation.is_ok(), "reservation should fit");
    let reservation = reservation.ok();
    let Some(reservation) = reservation else {
        return;
    };
    assert_eq!(
        reservation.reserved_micros(),
        1,
        "partial hour must round up"
    );
}

#[test]
fn concurrent_reservations_cannot_exceed_budget() {
    let guard = BudgetGuard::new(1);
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let worker_guard = guard.clone();
        let worker_barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            worker_barrier.wait();
            worker_guard.reserve(1, Duration::from_secs(1)).is_ok()
        }));
    }
    barrier.wait();
    let joined: Vec<_> = handles.into_iter().map(thread::JoinHandle::join).collect();
    let admitted = joined
        .iter()
        .filter(|result| matches!(result, Ok(true)))
        .count();
    let panicked = joined.iter().filter(|result| result.is_err()).count();
    assert_eq!(
        admitted, 1,
        "one outstanding reservation consumes the budget"
    );
    assert_eq!(panicked, 0, "budget worker threads must not panic");
}

#[test]
fn reservation_settlement_is_terminal_and_never_refunds_spend() {
    let guard = BudgetGuard::new(10);
    let reservation = guard.reserve(10, Duration::from_secs(1));
    assert!(reservation.is_ok(), "reservation should fit");
    let reservation = reservation.ok();
    let Some(reservation) = reservation else {
        return;
    };
    let settlement = reservation.settle(1);
    assert!(
        settlement.is_ok(),
        "settlement within reserved amount is accepted"
    );
    let snapshot = guard.snapshot();
    assert!(snapshot.is_ok(), "budget snapshot should be available");
    let snapshot = snapshot.ok();
    let Some(snapshot) = snapshot else {
        return;
    };
    assert_eq!(snapshot.spent_micros, 1, "settled spend is retained");
    assert_eq!(
        snapshot.outstanding_micros, 0,
        "settlement closes reservation"
    );
    assert!(
        guard.reserve(10, Duration::from_secs(1)).is_ok(),
        "unused budget remains available"
    );
}

#[test]
fn arithmetic_overflow_is_rejected() {
    let guard = BudgetGuard::new(u64::MAX);
    let result = guard.reserve(u64::MAX, Duration::from_secs(u64::MAX));
    assert!(
        matches!(result, Err(BudgetError::ArithmeticOverflow)),
        "overflow must be explicit"
    );
}

#[test]
fn zero_budget_denies_all() {
    let guard = BudgetGuard::new(0);
    assert!(
        matches!(
            guard.reserve(0, Duration::ZERO),
            Err(BudgetError::BudgetExceeded)
        ),
        "zero budget denies even a zero-cost reservation"
    );
}

#[test]
fn oversettlement_fails_closed() {
    let guard = BudgetGuard::new(1);
    let reservation = guard.reserve(1, Duration::from_secs(1));
    assert!(reservation.is_ok(), "reservation should fit");
    let reservation = reservation.ok();
    let Some(reservation) = reservation else {
        return;
    };
    assert!(
        matches!(
            reservation.settle(2),
            Err(BudgetError::SettlementExceedsReservation)
        ),
        "cost above reserved cap is rejected"
    );
    let snapshot = guard.snapshot();
    assert!(snapshot.is_ok(), "budget snapshot should be available");
    let snapshot = snapshot.ok();
    let Some(snapshot) = snapshot else {
        return;
    };
    assert_eq!(snapshot.spent_micros, 2, "actual overrun is recorded");
    assert_eq!(
        snapshot.outstanding_micros, 0,
        "overrun settlement closes reservation"
    );
    assert!(
        snapshot.admission_halted,
        "overrun permanently halts admission"
    );
    assert!(
        matches!(
            guard.reserve(1, Duration::from_secs(1)),
            Err(BudgetError::BudgetExceeded)
        ),
        "held reservation blocks further admission"
    );
}

#[test]
fn unrepresentable_overrun_halts_admission() {
    let guard = BudgetGuard::new(u64::MAX);
    let first = guard.reserve(1, Duration::from_secs(3_600));
    assert!(first.is_ok(), "initial reservation should fit");
    let first = first.ok();
    let Some(first) = first else {
        return;
    };
    assert!(first.settle(1).is_ok(), "initial spend should settle");

    let near_limit = u64::MAX.checked_sub(1);
    assert!(
        near_limit.is_some(),
        "test reservation rate should be representable"
    );
    let Some(near_limit) = near_limit else {
        return;
    };
    let second = guard.reserve(near_limit, Duration::from_secs(3_600));
    assert!(second.is_ok(), "remaining budget should fit exactly");
    let second = second.ok();
    let Some(second) = second else {
        return;
    };
    assert!(
        matches!(
            second.settle(u64::MAX),
            Err(BudgetError::ArithmeticOverflow)
        ),
        "overflowing actual spend is reported"
    );
    let snapshot = guard.snapshot();
    assert!(snapshot.is_ok(), "budget snapshot should be available");
    let snapshot = snapshot.ok();
    let Some(snapshot) = snapshot else {
        return;
    };
    assert_eq!(
        snapshot.spent_micros, 1,
        "unrepresentable spend does not corrupt accounting"
    );
    assert!(
        snapshot.admission_halted,
        "overflow latches admission closed"
    );
    assert!(
        matches!(
            guard.reserve(1, Duration::ZERO),
            Err(BudgetError::BudgetExceeded)
        ),
        "overflow permanently blocks later admission"
    );
}

#[test]
fn overrun_records_actual_cost_and_halts_admission() {
    let guard = BudgetGuard::new(100);
    let reservation = guard.reserve(10, Duration::from_secs(3_600));
    assert!(reservation.is_ok(), "ten micro-USD reservation should fit");
    let reservation = reservation.ok();
    let Some(reservation) = reservation else {
        return;
    };
    assert!(
        matches!(
            reservation.settle(100),
            Err(BudgetError::SettlementExceedsReservation)
        ),
        "provider cost above reservation is reported"
    );
    let snapshot = guard.snapshot();
    assert!(snapshot.is_ok(), "budget snapshot should be available");
    let snapshot = snapshot.ok();
    let Some(snapshot) = snapshot else {
        return;
    };
    assert_eq!(
        snapshot.spent_micros, 100,
        "actual terminal charge is recorded"
    );
    assert_eq!(
        snapshot.outstanding_micros, 0,
        "settled reservation is closed"
    );
    assert!(
        snapshot.admission_halted,
        "overrun permanently halts admission"
    );
    assert!(
        matches!(
            guard.reserve(1, Duration::from_secs(3_600)),
            Err(BudgetError::BudgetExceeded)
        ),
        "actual spend exhausts budget and denies later admission"
    );
}

#[test]
fn overrun_above_budget_records_actual_and_halts_admission() {
    let guard = BudgetGuard::new(100);
    let reservation = guard.reserve(10, Duration::from_secs(3_600));
    assert!(reservation.is_ok(), "ten micro-USD reservation should fit");
    let reservation = reservation.ok();
    let Some(reservation) = reservation else {
        return;
    };
    assert!(
        matches!(
            reservation.settle(101),
            Err(BudgetError::SettlementExceedsReservation)
        ),
        "provider overspend is reported"
    );
    let snapshot = guard.snapshot();
    assert!(snapshot.is_ok(), "budget snapshot should be available");
    let snapshot = snapshot.ok();
    let Some(snapshot) = snapshot else {
        return;
    };
    assert_eq!(
        snapshot.spent_micros, 101,
        "actual overspend is recorded accurately"
    );
    assert_eq!(
        snapshot.outstanding_micros, 0,
        "settled reservation is closed"
    );
    assert!(
        snapshot.admission_halted,
        "overrun permanently halts admission"
    );
    assert!(
        matches!(
            guard.reserve(1, Duration::from_secs(3_600)),
            Err(BudgetError::BudgetExceeded)
        ),
        "overrun permanently denies later admission"
    );
}

#[test]
fn rejects_zero_disk_or_lifetime() {
    let ingress = IngressPolicy::new(vec![ip(Ipv4Addr::LOCALHOST)], true);
    assert!(ingress.is_ok(), "valid ingress should build");
    let ingress = ingress.ok();
    let Some(ingress) = ingress else {
        return;
    };
    assert!(
        matches!(
            ValidatedVmSpec::new(0, Duration::from_secs(60), Duration::from_secs(30), ingress),
            Err(SpecError::ZeroDisk)
        ),
        "zero disk must be rejected"
    );

    let second_ingress = IngressPolicy::new(vec![ip(Ipv4Addr::LOCALHOST)], true);
    assert!(second_ingress.is_ok(), "valid ingress should build");
    let second_ingress = second_ingress.ok();
    let Some(second_ingress) = second_ingress else {
        return;
    };
    assert!(
        matches!(
            ValidatedVmSpec::new(32, Duration::ZERO, Duration::ZERO, second_ingress),
            Err(SpecError::ZeroTtl)
        ),
        "zero hard TTL must be rejected"
    );
}

#[test]
fn accepts_only_exact_operator_ip_allowlist_with_mtls() {
    let operator = ip(Ipv4Addr::new(192, 0, 2, 44));
    let ingress = IngressPolicy::new(vec![operator], true);
    assert!(ingress.is_ok(), "valid ingress should build");
    let ingress = ingress.ok();
    let Some(ingress) = ingress else {
        return;
    };
    assert!(ingress.allows(operator), "listed address must be allowed");
    assert!(
        !ingress.allows(ip(Ipv4Addr::new(192, 0, 2, 45))),
        "neighbor must be denied"
    );
    assert!(ingress.mtls_required(), "mTLS must remain required");
}

#[test]
fn rejects_unspecified_multicast_and_broadcast_operator_ips() {
    let addresses = [
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V4(Ipv4Addr::BROADCAST),
        IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1)),
        IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        IpAddr::V6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1)),
    ];
    for address in addresses {
        assert!(
            IngressPolicy::new(vec![address], true).is_err(),
            "unspecified, multicast, or broadcast IP must be denied"
        );
    }
}

#[test]
fn canonicalizes_ipv4_mapped_ipv6_and_deduplicates() {
    let v4 = Ipv4Addr::new(192, 0, 2, 9);
    let mapped = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xc000, 0x0209);
    let policy = IngressPolicy::new(vec![IpAddr::V6(mapped), IpAddr::V4(v4)], true);
    assert!(policy.is_ok(), "mapped IPv6 address should canonicalize");
    let policy = policy.ok();
    let Some(policy) = policy else {
        return;
    };
    assert_eq!(
        policy.operator_ips().len(),
        1,
        "canonical duplicates collapse to one entry"
    );
    assert!(
        policy.allows(IpAddr::V4(v4)),
        "canonical IPv4 form is allowed"
    );
    assert!(
        policy.allows(IpAddr::V6(mapped)),
        "mapped IPv6 input resolves to the same policy address"
    );
}

#[test]
fn rejects_allowlists_larger_than_the_finite_limit() {
    let maximum_addresses = vec![IpAddr::V4(Ipv4Addr::LOCALHOST); 256];
    assert!(
        IngressPolicy::new(maximum_addresses, true).is_ok(),
        "allowlist at the fixed maximum is accepted"
    );
    let addresses = vec![IpAddr::V4(Ipv4Addr::LOCALHOST); 257];
    assert!(
        IngressPolicy::new(addresses, true).is_err(),
        "allowlists above the fixed maximum must be rejected"
    );
}

#[test]
fn rejects_empty_allowlist_and_missing_mtls() {
    assert!(
        matches!(
            IngressPolicy::new(Vec::new(), true),
            Err(SpecError::EmptyIngressAllowlist)
        ),
        "empty ingress allowlist must be rejected"
    );
    assert!(
        matches!(
            IngressPolicy::new(vec![ip(Ipv4Addr::LOCALHOST)], false),
            Err(SpecError::MtlsRequired)
        ),
        "mTLS must be mandatory"
    );
}

#[test]
fn idle_timeout_must_not_exceed_hard_ttl() {
    let ingress = IngressPolicy::new(vec![ip(Ipv4Addr::LOCALHOST)], true);
    assert!(ingress.is_ok(), "valid ingress should build");
    let ingress = ingress.ok();
    let Some(ingress) = ingress else {
        return;
    };
    let result = ValidatedVmSpec::new(
        32,
        Duration::from_secs(30),
        Duration::from_secs(31),
        ingress,
    );
    assert!(
        matches!(result, Err(SpecError::IdleExceedsTtl)),
        "idle limit cannot exceed hard TTL"
    );
}

#[test]
fn hard_ttl_expires_even_after_recent_activity() {
    let started = Instant::now();
    let guard = LifetimeGuard::new(started, Duration::from_secs(20), Duration::from_secs(10));
    assert!(guard.is_ok(), "valid lifetime should build");
    let guard = guard.ok();
    let Some(mut guard) = guard else {
        return;
    };
    assert!(
        guard
            .record_activity(started + Duration::from_secs(9))
            .is_ok(),
        "recent activity is accepted"
    );
    assert!(
        matches!(
            guard.evaluate(started + Duration::from_secs(20)),
            Ok(LifetimeStatus::ExpiredByHardTtl)
        ),
        "hard TTL wins over activity"
    );
}

#[test]
fn idle_timeout_expires_without_activity() {
    let started = Instant::now();
    let guard = LifetimeGuard::new(started, Duration::from_secs(60), Duration::from_secs(10));
    assert!(guard.is_ok(), "valid lifetime should build");
    let guard = guard.ok();
    let Some(mut guard) = guard else {
        return;
    };
    assert!(
        matches!(
            guard.evaluate(started + Duration::from_secs(10)),
            Ok(LifetimeStatus::ExpiredByIdleTimeout)
        ),
        "idle timeout expires at its boundary"
    );
}

#[test]
fn observed_idle_expiry_is_terminal_after_activity() {
    let started = Instant::now();
    let guard = LifetimeGuard::new(started, Duration::from_secs(60), Duration::from_secs(10));
    assert!(guard.is_ok(), "valid lifetime should build");
    let guard = guard.ok();
    let Some(mut guard) = guard else {
        return;
    };
    let idle_deadline = started + Duration::from_secs(10);
    assert!(
        matches!(
            guard.evaluate(idle_deadline),
            Ok(LifetimeStatus::ExpiredByIdleTimeout)
        ),
        "idle deadline is terminal when observed"
    );
    assert!(
        guard
            .record_activity(started + Duration::from_secs(11))
            .is_ok(),
        "late activity is observed"
    );
    assert!(
        matches!(
            guard.evaluate(started + Duration::from_secs(12)),
            Ok(LifetimeStatus::ExpiredByIdleTimeout)
        ),
        "late activity cannot revive an expired VM"
    );
}

#[test]
fn activity_resets_only_idle_deadline() {
    let started = Instant::now();
    let guard = LifetimeGuard::new(started, Duration::from_secs(60), Duration::from_secs(10));
    assert!(guard.is_ok(), "valid lifetime should build");
    let guard = guard.ok();
    let Some(mut guard) = guard else {
        return;
    };
    assert!(
        guard
            .record_activity(started + Duration::from_secs(9))
            .is_ok(),
        "activity is accepted"
    );
    assert!(
        matches!(
            guard.evaluate(started + Duration::from_secs(18)),
            Ok(LifetimeStatus::Running)
        ),
        "activity extends idle lifetime"
    );
    assert!(
        matches!(
            guard.evaluate(started + Duration::from_secs(60)),
            Ok(LifetimeStatus::ExpiredByHardTtl)
        ),
        "activity does not move hard TTL"
    );
}

#[test]
fn rejects_non_monotonic_observation() {
    let started = Instant::now();
    let guard = LifetimeGuard::new(started, Duration::from_secs(60), Duration::from_secs(10));
    assert!(guard.is_ok(), "valid lifetime should build");
    let guard = guard.ok();
    let Some(mut guard) = guard else {
        return;
    };
    assert!(
        guard
            .record_activity(started + Duration::from_secs(5))
            .is_ok(),
        "later observation is accepted"
    );
    assert!(
        matches!(
            guard.evaluate(started + Duration::from_secs(4)),
            Err(LifetimeError::NonMonotonicTime)
        ),
        "clock reversal is rejected"
    );
    assert!(
        matches!(
            guard.evaluate(started + Duration::from_millis(4_500)),
            Err(LifetimeError::NonMonotonicTime)
        ),
        "rejected observation cannot move the last observed time backward"
    );
}

#[test]
fn selects_only_exact_owner_marker() {
    let marker = OwnerMarker::new("polaris.owner".to_owned(), "ai-cloud".to_owned());
    assert!(marker.is_ok(), "valid owner marker should build");
    let marker = marker.ok();
    let Some(marker) = marker else {
        return;
    };
    let mut labels = HashMap::new();
    labels.insert("polaris.owner".to_owned(), "ai-cloud".to_owned());
    let resources = vec![ResourceLabels::new("vm-1".to_owned(), labels)];
    let selected = select_owned_resources(resources, &marker);
    assert_eq!(selected.len(), 1, "exact ownership marker is selected");
    assert_eq!(
        selected.first().map(ResourceId::as_str),
        Some("vm-1"),
        "resource id is preserved"
    );
}

#[test]
fn never_selects_untagged_or_similarly_tagged_resources() {
    let marker = OwnerMarker::new("polaris.owner".to_owned(), "ai-cloud".to_owned());
    assert!(marker.is_ok(), "valid owner marker should build");
    let marker = marker.ok();
    let Some(marker) = marker else {
        return;
    };
    let untagged = ResourceLabels::new("vm-untagged".to_owned(), HashMap::new());
    let mut similar_labels = HashMap::new();
    similar_labels.insert("polaris.owner".to_owned(), "ai-cloud-extra".to_owned());
    let similar = ResourceLabels::new("vm-similar".to_owned(), similar_labels);
    let selected = select_owned_resources(vec![untagged, similar], &marker);
    assert!(
        selected.is_empty(),
        "unowned resources must never be selected"
    );
}
