// Unit tests of crates/polaris-code-intel/src/metrics.rs (`arithmetic_regressions`); compiled into that module via #[path].
use super::{Grid, GridError, SigTree, zhang_shasha};

#[test]
fn grid_rejects_invalid_coordinates_and_addresses() {
    let result = Grid::new(2, 3);
    assert!(result.is_ok());
    let mut grid = match result {
        Ok(grid) => grid,
        Err(_) => return,
    };
    assert_eq!(grid.get(1, 2), Ok(0));
    assert_eq!(grid.set(1, 2, 7), Ok(()));
    assert_eq!(grid.get(1, 2), Ok(7));
    assert_eq!(grid.get(2, 0), Err(GridError::OutOfBounds));
    assert_eq!(grid.get(0, 3), Err(GridError::OutOfBounds));
    assert_eq!(
        grid.get(usize::MAX, usize::MAX),
        Err(GridError::OutOfBounds)
    );
    assert_eq!(grid.set(2, 0, 9), Err(GridError::OutOfBounds));
    assert!(matches!(
        Grid::new(usize::MAX, 2),
        Err(GridError::DimensionOverflow)
    ));
    assert!(matches!(
        Grid::new(usize::MAX, 1),
        Err(GridError::AllocationFailed)
    ));
}

#[test]
fn single_node_tree_distance_matches_independent_edit_cost() {
    let left = SigTree {
        labels: vec![11],
        parents: vec![None],
    };
    let same = SigTree {
        labels: vec![11],
        parents: vec![None],
    };
    let relabeled = SigTree {
        labels: vec![19],
        parents: vec![None],
    };
    let empty = SigTree {
        labels: Vec::new(),
        parents: Vec::new(),
    };
    assert_eq!(zhang_shasha(&left, &same), Ok(0));
    assert_eq!(zhang_shasha(&left, &relabeled), Ok(1));
    assert_eq!(zhang_shasha(&left, &empty), Ok(1));
    assert_eq!(zhang_shasha(&empty, &left), Ok(1));
}

proptest::proptest! {
    #[test]
    fn unary_tree_distance_matches_independent_levenshtein(
        left in proptest::collection::vec(0_u64..4, 0..6),
        right in proptest::collection::vec(0_u64..4, 0..6),
    ) {
        let left_tree = SigTree { labels: left.clone(), parents: chain_parents(left.len()) };
        let right_tree = SigTree { labels: right.clone(), parents: chain_parents(right.len()) };
        let mut previous: Vec<usize> = (0..=right.len()).collect();
        for (left_index, left_label) in left.iter().enumerate() {
            let mut current = vec![left_index.saturating_add(1)];
            for (right_index, right_label) in right.iter().enumerate() {
                let deletion = previous.get(right_index.saturating_add(1)).copied().unwrap_or(usize::MAX).saturating_add(1);
                let insertion = current.last().copied().unwrap_or(usize::MAX).saturating_add(1);
                let relabel = previous.get(right_index).copied().unwrap_or(usize::MAX).saturating_add(usize::from(left_label != right_label));
                current.push(deletion.min(insertion).min(relabel));
            }
            previous = current;
        }
        let expected = previous.last().copied().unwrap_or(0);
        proptest::prop_assert_eq!(zhang_shasha(&left_tree, &right_tree), Ok(expected));
        proptest::prop_assert_eq!(zhang_shasha(&right_tree, &left_tree), Ok(expected));
    }
}

fn chain_parents(length: usize) -> Vec<Option<usize>> {
    (0..length).map(|index| index.checked_sub(1)).collect()
}
