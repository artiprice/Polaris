//! Bounded proof of the production scope predicate, not of async FIFO scheduling or filesystem I/O.
use crate::{LeaseKey, LeaseScope, SymbolPath, scopes_conflict};

fn scope(depth: u8, first: bool, second: bool) -> LeaseScope {
    if depth == 0 {
        return LeaseScope::EntireFile;
    }
    let first = if first { "a" } else { "b" };
    let second = if second { "a" } else { "b" };
    let symbol = if depth == 1 {
        SymbolPath::new([first].as_slice())
    } else {
        SymbolPath::new([first, second].as_slice())
    };
    match symbol {
        Ok(symbol) => LeaseScope::AstNode(symbol),
        Err(_) => {
            assert!(false, "fixed proof symbols must validate");
            LeaseScope::EntireFile
        }
    }
}

/// For all two-component paths over a two-symbol alphabet, whole-file and AST scope conflicts
/// equal the independent component-prefix formula and never conflict across different file keys.
#[kani::proof]
#[kani::unwind(16)]
fn scope_conflict_matches_two_component_prefix_model() {
    let left_depth: u8 = kani::any();
    let right_depth: u8 = kani::any();
    kani::assume(left_depth <= 2 && right_depth <= 2);
    let left_first: bool = kani::any();
    let right_first: bool = kani::any();
    let left_second: bool = kani::any();
    let right_second: bool = kani::any();
    let same_key: bool = kani::any();
    let left_key = match LeaseKey::from_resolved_relative("a") {
        Ok(key) => key,
        Err(_) => {
            assert!(false, "fixed proof key must validate");
            return;
        }
    };
    let right_key = match LeaseKey::from_resolved_relative(if same_key { "a" } else { "b" }) {
        Ok(key) => key,
        Err(_) => {
            assert!(false, "fixed proof key must validate");
            return;
        }
    };
    let left = scope(left_depth, left_first, left_second);
    let right = scope(right_depth, right_first, right_second);
    let expected = same_key
        && (left_depth == 0
            || right_depth == 0
            || (left_first == right_first
                && (left_depth == 1 || right_depth == 1 || left_second == right_second)));
    assert!(scopes_conflict(&left_key, &left, &right_key, &right) == expected);
    assert!(scopes_conflict(&right_key, &right, &left_key, &left) == expected);
    assert!(scopes_conflict(&left_key, &left, &left_key, &left));
}
