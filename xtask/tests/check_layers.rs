//! Tests for `xtask::layers::check_layers` (descriptor: specs/descriptors/xtask_check_layers.json).

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use xtask::layers::{LayerViolation, check_layers};
use xtask::table::CrateTable;

type TestResult = Result<(), Box<dyn Error>>;

/// sdk (L0) <- adapter-a (L2) ; facade (L3) -> child-low (L1), child-high (L3) ; other (L3)
fn table(extra_edge: Option<(&str, &str)>) -> Result<CrateTable, Box<dyn Error>> {
    let mut entries = vec![
        ("sdk", 0, None, vec![]),
        ("adapter-a", 2, None, vec!["sdk"]),
        ("facade", 3, None, vec!["child-low", "child-high"]),
        ("child-low", 1, Some("facade"), vec!["sdk"]),
        ("child-high", 3, Some("facade"), vec!["child-low"]),
        ("other", 3, None, vec!["sdk"]),
    ];
    if let Some((from, to)) = extra_edge {
        for entry in &mut entries {
            if entry.0 == from {
                entry.3.push(to);
            }
        }
    }
    let mut text = String::new();
    for (name, layer, parent, deps) in &entries {
        text.push_str(&format!("[[crate]]\nname = \"{name}\"\npath = \"crates/{name}\"\nlayer = {layer}\nkind = \"lib\"\n"));
        if let Some(p) = parent {
            text.push_str(&format!("parent = \"{p}\"\n"));
        }
        let quoted: Vec<String> = deps.iter().map(|d| format!("\"{d}\"")).collect();
        text.push_str(&format!("deps = [{}]\n\n", quoted.join(", ")));
    }
    Ok(CrateTable::from_toml_str(&text)?)
}

/// Actual edges equal to the table's declared edges.
fn actual_from(table: &CrateTable) -> BTreeMap<String, BTreeSet<String>> {
    table
        .crates
        .iter()
        .map(|c| (c.name.clone(), c.deps.iter().cloned().collect()))
        .collect()
}

#[test]
fn downward_cross_group_edge_is_ok() -> TestResult {
    let t = table(None)?;
    let violations = check_layers(&t, &actual_from(&t));
    assert!(
        violations.is_empty(),
        "expected no violations, got {violations:?}"
    );
    Ok(())
}

#[test]
fn same_layer_cross_group_edge_is_rejected() -> TestResult {
    let t = table(Some(("other", "child-high")))?;
    let violations = check_layers(&t, &actual_from(&t));
    assert!(
        violations.iter().any(|v| matches!(v, LayerViolation::NotDownward { from, to, .. } if from == "other" && to == "child-high")),
        "expected NotDownward(other -> child-high), got {violations:?}"
    );
    Ok(())
}

#[test]
fn facade_to_child_same_layer_is_ok() -> TestResult {
    let t = table(None)?;
    let violations = check_layers(&t, &actual_from(&t));
    assert!(
        !violations
            .iter()
            .any(|v| matches!(v, LayerViolation::NotDownward { from, .. } if from == "facade")),
        "facade -> child (same layer) must be allowed, got {violations:?}"
    );
    Ok(())
}

#[test]
fn child_to_parent_edge_is_rejected() -> TestResult {
    let t = table(Some(("child-low", "facade")))?;
    let violations = check_layers(&t, &actual_from(&t));
    assert!(
        violations.iter().any(|v| matches!(v, LayerViolation::ChildToParent { child, parent } if child == "child-low" && parent == "facade")),
        "expected ChildToParent(child-low -> facade), got {violations:?}"
    );
    Ok(())
}

#[test]
fn sibling_edge_upward_is_rejected() -> TestResult {
    let t = table(Some(("child-low", "child-high")))?;
    let violations = check_layers(&t, &actual_from(&t));
    assert!(
        violations.iter().any(|v| matches!(v, LayerViolation::NotDownward { from, to, .. } if from == "child-low" && to == "child-high")),
        "expected NotDownward(child-low -> child-high), got {violations:?}"
    );
    Ok(())
}

#[test]
fn manifest_edges_must_equal_table_edges() -> TestResult {
    let t = table(None)?;
    let mut actual = actual_from(&t);
    if let Some(edges) = actual.get_mut("other") {
        edges.insert("adapter-a".to_owned());
    }
    let violations = check_layers(&t, &actual);
    assert!(
        violations.iter().any(|v| matches!(v, LayerViolation::EdgeMismatch { package, extra, .. } if package == "other" && extra.contains("adapter-a"))),
        "expected EdgeMismatch(other, extra adapter-a), got {violations:?}"
    );
    Ok(())
}
