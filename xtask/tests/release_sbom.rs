//! Behavioral contracts for the release SPDX document (descriptor: specs/descriptors/release_sbom.json).

use serde_json::Value;
use std::error::Error;
use std::io;
use xtask::release_sbom::spdx_document;

const META: &str = r#"{
  "workspace_root": "/workspace/project",
  "workspace_members": ["path+file:///workspace/project#app@1.0.0", "registry+https://github.com/rust-lang/crates.io-index#dep@2.0.0"],
  "packages": [
    {"id":"path+file:///workspace/project#app@1.0.0","name":"app","version":"1.0.0","source":null,"license":"Apache-2.0","manifest_path":"/workspace/project/Cargo.toml"},
    {"id":"registry+https://github.com/rust-lang/crates.io-index#dep@2.0.0","name":"dep","version":"2.0.0","source":"registry+https://github.com/rust-lang/crates.io-index","license":null,"manifest_path":"/cargo/registry/src/dep/Cargo.toml"}
  ],
  "resolve": {"nodes": [
    {"id":"path+file:///workspace/project#app@1.0.0","deps":[{"pkg":"registry+https://github.com/rust-lang/crates.io-index#dep@2.0.0"}]},
    {"id":"registry+https://github.com/rust-lang/crates.io-index#dep@2.0.0","deps":[]}
  ]}
}"#;
const LOCK: &[u8] = b"[[package]]\nname = \"app\"\n";

fn document() -> Result<Value, Box<dyn Error>> {
    let metadata: Value = serde_json::from_str(META)?;
    let bytes = spdx_document(&metadata, LOCK, "2026-10-26T09:00:00Z")?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn expect_metadata_usage(metadata: &Value, expected: &str) -> Result<(), Box<dyn Error>> {
    let error = spdx_document(metadata, LOCK, "2026-10-26T09:00:00Z")
        .err()
        .ok_or_else(|| io::Error::other("malformed metadata was accepted"))?;
    assert!(
        error.to_string().contains(expected),
        "expected diagnostic containing {expected:?}, got {error}"
    );
    Ok(())
}

#[test]
fn spdx_document_contains_resolved_packages_and_dependency_relationships()
-> Result<(), Box<dyn Error>> {
    let doc = document()?;
    assert_eq!(
        doc.get("spdxVersion").and_then(Value::as_str),
        Some("SPDX-2.3")
    );
    assert_eq!(
        doc.get("packages").and_then(Value::as_array).map(Vec::len),
        Some(2)
    );
    assert!(
        doc.get("relationships")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter().any(|row| {
                    row.get("relationshipType").and_then(Value::as_str) == Some("DEPENDS_ON")
                })
            })
    );
    Ok(())
}

#[test]
fn workspace_packages_have_root_provenance_and_lock_hash() -> Result<(), Box<dyn Error>> {
    let doc = document()?;
    let expected_checksum = blake3::hash(LOCK).to_hex().to_string();
    assert!(
        doc.get("packages")
            .and_then(Value::as_array)
            .is_some_and(|rows| rows
                .iter()
                .any(|row| row.get("name").and_then(Value::as_str) == Some("app")))
    );
    assert!(
        doc.get("comment")
            .and_then(Value::as_str)
            .is_some_and(|comment| {
                comment.contains("workspace root") && comment.contains("Cargo.lock BLAKE3")
            })
    );
    assert!(
        doc.get("files")
            .and_then(Value::as_array)
            .is_some_and(|files| files.iter().any(|file| {
                file.get("fileName").and_then(Value::as_str) == Some("Cargo.lock")
                    && file
                        .get("checksums")
                        .and_then(Value::as_array)
                        .is_some_and(|checksums| {
                            checksums.iter().any(|checksum| {
                                checksum.get("algorithm").and_then(Value::as_str) == Some("BLAKE3")
                                    && checksum.get("checksumValue").and_then(Value::as_str)
                                        == Some(expected_checksum.as_str())
                            })
                        })
            }))
    );
    assert!(
        doc.get("documentDescribes")
            .and_then(Value::as_array)
            .is_some_and(|described| described
                .iter()
                .any(|item| item.as_str() == Some("SPDXRef-Cargo-Lock")))
    );
    Ok(())
}

#[test]
fn unknown_license_is_not_invented() -> Result<(), Box<dyn Error>> {
    let doc = document()?;
    assert!(
        doc.get("packages")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter().any(|row| {
                    row.get("name").and_then(Value::as_str) == Some("dep")
                        && row.get("licenseDeclared").and_then(Value::as_str) == Some("NOASSERTION")
                })
            })
    );
    Ok(())
}

#[test]
fn serialization_is_deterministic_for_same_inputs() -> Result<(), Box<dyn Error>> {
    let metadata: Value = serde_json::from_str(META)?;
    let first = spdx_document(&metadata, b"stable lock", "2026-10-26T09:00:00Z")?;
    let second = spdx_document(&metadata, b"stable lock", "2026-10-26T09:00:00Z")?;
    assert_eq!(first, second);
    Ok(())
}

#[test]
fn impossible_calendar_timestamp_is_rejected() -> Result<(), Box<dyn Error>> {
    let metadata: Value = serde_json::from_str(META)?;
    let result = spdx_document(&metadata, LOCK, "2026-99-99T99:99:99Z");
    assert!(
        result.is_err(),
        "impossible calendar and time fields must fail"
    );
    Ok(())
}

#[test]
fn non_leap_year_february_29_is_rejected() -> Result<(), Box<dyn Error>> {
    let metadata: Value = serde_json::from_str(META)?;
    let result = spdx_document(&metadata, LOCK, "2025-02-29T00:00:00Z");
    assert!(result.is_err(), "February 29 is invalid in a non-leap year");
    Ok(())
}

#[test]
fn leap_day_timestamp_is_accepted() -> Result<(), Box<dyn Error>> {
    let metadata: Value = serde_json::from_str(META)?;
    let bytes = spdx_document(&metadata, LOCK, "2024-02-29T23:59:59Z")?;
    let document: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        document
            .get("creationInfo")
            .and_then(|info| info.get("created"))
            .and_then(Value::as_str),
        Some("2024-02-29T23:59:59Z")
    );
    Ok(())
}

#[test]
fn duplicate_cargo_package_id_is_rejected() -> Result<(), Box<dyn Error>> {
    let mut metadata: Value = serde_json::from_str(META)?;
    let packages = metadata
        .get_mut("packages")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| io::Error::other("fixture packages array is absent"))?;
    let duplicate = packages
        .first()
        .cloned()
        .ok_or_else(|| io::Error::other("fixture package is absent"))?;
    packages.push(duplicate);
    expect_metadata_usage(&metadata, "duplicate Cargo package ID")
}

#[test]
fn unknown_resolve_node_id_is_rejected() -> Result<(), Box<dyn Error>> {
    let mut metadata: Value = serde_json::from_str(META)?;
    let nodes = metadata
        .get_mut("resolve")
        .and_then(|resolve| resolve.get_mut("nodes"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| io::Error::other("fixture resolve nodes array is absent"))?;
    nodes.push(serde_json::json!({ "id": "ghost-package-id", "deps": [] }));
    expect_metadata_usage(&metadata, "resolve node has unknown package ID")
}

#[test]
fn unknown_dependency_package_id_is_rejected() -> Result<(), Box<dyn Error>> {
    let mut metadata: Value = serde_json::from_str(META)?;
    let nodes = metadata
        .get_mut("resolve")
        .and_then(|resolve| resolve.get_mut("nodes"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| io::Error::other("fixture resolve nodes array is absent"))?;
    let node = nodes
        .iter_mut()
        .find(|node| {
            node.get("id").and_then(Value::as_str)
                == Some("path+file:///workspace/project#app@1.0.0")
        })
        .ok_or_else(|| io::Error::other("fixture root resolve node is absent"))?;
    let dependencies = node
        .get_mut("deps")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| io::Error::other("fixture dependency array is absent"))?;
    dependencies.push(serde_json::json!({ "pkg": "ghost-package-id" }));
    expect_metadata_usage(&metadata, "dependency edge has unknown package ID")
}
