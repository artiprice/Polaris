#![no_main]
use libfuzzer_sys::fuzz_target;
use polaris_runtime_leases::{
    LeaseConfig, LeaseGuard, LeaseKey, LeaseManager, LeaseRequest, LeaseScope, SymbolPath,
};
use polaris_sdk::ids::FrameId;

struct Held {
    guard: LeaseGuard,
    key: LeaseKey,
    scope: LeaseScope,
}
fn model_overlap(left: &LeaseScope, right: &LeaseScope) -> bool {
    match (left, right) {
        (LeaseScope::EntireFile, _) | (_, LeaseScope::EntireFile) => true,
        (LeaseScope::AstNode(left), LeaseScope::AstNode(right)) => {
            let common = left
                .components()
                .iter()
                .zip(right.components())
                .take_while(|(a, b)| a == b)
                .count();
            common == left.len().min(right.len())
        }
    }
}
fuzz_target!(|data: &[u8]| {
    let manager = LeaseManager::new(LeaseConfig {
        max_active_leases: 32,
        ..Default::default()
    });
    let mut slots: Vec<Option<Held>> = (0..32).map(|_| None).collect();
    for chunk in data.chunks_exact(3).take(256) {
        let [code, path, scope] = chunk else {
            continue;
        };
        let slot = usize::from(*code) % 32;
        if code % 3 == 0 {
            if let Some(entry) = slots.get_mut(slot) {
                let _released = entry.take();
            }
            continue;
        }
        let Ok(key) = LeaseKey::from_resolved_relative(&format!("source-{}.rs", path % 8)) else {
            continue;
        };
        let selected = if scope % 3 == 0 {
            LeaseScope::EntireFile
        } else {
            let parts = if scope % 3 == 1 {
                vec!["root"]
            } else {
                vec!["root", if scope % 2 == 0 { "a" } else { "b" }]
            };
            let Ok(symbol) = SymbolPath::new(parts) else {
                continue;
            };
            LeaseScope::AstNode(symbol)
        };
        let conflicts = slots
            .iter()
            .flatten()
            .any(|held| held.key == key && model_overlap(&held.scope, &selected));
        if slots.get(slot).is_some_and(Option::is_none) {
            match manager.try_acquire(LeaseRequest {
                agent: FrameId::from_u128(u128::from(*code).saturating_add(1)),
                key: key.clone(),
                scope: selected.clone(),
                base: None,
            }) {
                Ok(guard) => {
                    assert!(!conflicts, "two conflicting leases were granted");
                    if let Some(entry) = slots.get_mut(slot) {
                        *entry = Some(Held {
                            guard,
                            key: key.clone(),
                            scope: selected.clone(),
                        });
                    }
                }
                Err(_) => assert!(
                    conflicts,
                    "nonconflicting request was rejected below capacity"
                ),
            }
        }
        if let Some((_, id)) = manager.holder_of(&key, &selected) {
            assert!(
                slots.iter().flatten().any(|held| held.guard.id() == id
                    && held.key == key
                    && model_overlap(&held.scope, &selected)),
                "lookup returned an unowned or nonconflicting lease"
            );
        } else {
            assert!(
                !slots
                    .iter()
                    .flatten()
                    .any(|held| held.key == key && model_overlap(&held.scope, &selected)),
                "lookup missed conflicting holder"
            );
        }
    }
    drop(slots);
    assert_eq!(manager.active_count(), 0, "guards must release every lease");
});
