//! Deterministic SPDX 2.3 JSON generation from locked Cargo metadata.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::error::XtaskError;

/// Build deterministic SPDX JSON from full `cargo metadata` output and lockfile bytes.
///
/// The document records Cargo's declared license expression or `NOASSERTION`; it does not
/// independently validate licensing or make vulnerability claims.
///
/// # Errors
/// Returns an error if required Cargo metadata is absent or JSON serialization fails.
pub fn spdx_document(
    metadata: &Value,
    lock_bytes: &[u8],
    created: &str,
) -> Result<Vec<u8>, XtaskError> {
    validate_created(created)?;
    let workspace_root = required_str(metadata, "workspace_root")?;
    let members = string_set(metadata, "workspace_members")?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::Usage("cargo metadata JSON has no `packages` array".to_owned())
        })?;
    let nodes = metadata
        .get("resolve")
        .and_then(|value| value.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::Usage("cargo metadata JSON has no `resolve.nodes` array".to_owned())
        })?;

    let mut packages_by_id = BTreeMap::new();
    for package in packages {
        let id = required_str(package, "id")?;
        if packages_by_id.contains_key(id) {
            return Err(XtaskError::Usage(format!(
                "cargo metadata contains duplicate Cargo package ID: {id}"
            )));
        }
        packages_by_id.insert(id.to_owned(), package);
    }
    let mut resolve_ids = BTreeSet::new();
    for node in nodes {
        let id = required_str(node, "id")?;
        if !packages_by_id.contains_key(id) {
            return Err(XtaskError::Usage(format!(
                "cargo metadata resolve node has unknown package ID: {id}"
            )));
        }
        if !resolve_ids.insert(id.to_owned()) {
            return Err(XtaskError::Usage(format!(
                "cargo metadata contains duplicate resolve node ID: {id}"
            )));
        }
    }
    for id in packages_by_id.keys() {
        if !resolve_ids.contains(id) {
            return Err(XtaskError::Usage(format!(
                "cargo metadata package has no resolve node: {id}"
            )));
        }
    }
    let references: BTreeMap<String, String> = packages_by_id
        .keys()
        .enumerate()
        .map(|(index, id)| (id.clone(), format!("SPDXRef-Package-{index}")))
        .collect();

    let spdx_packages = packages_by_id
        .iter()
        .map(|(id, package)| {
            let name = required_str(package, "name")?;
            let version = required_str(package, "version")?;
            let source = package.get("source").and_then(Value::as_str);
            let manifest_path = required_str(package, "manifest_path")?;
            let declared_license = package
                .get("license")
                .and_then(Value::as_str)
                .map_or_else(|| "NOASSERTION".to_owned(), str::to_owned);
            let download_location =
                source.map_or_else(|| "NOASSERTION".to_owned(), source_download_location);
            let source_info = source_description(source, manifest_path, workspace_root);
            let spdx_id = references.get(id).ok_or_else(|| {
                XtaskError::Usage(format!("missing SPDX reference for Cargo package {id}"))
            })?;
            Ok(json!({
                "name": name,
                "SPDXID": spdx_id,
                "versionInfo": version,
                "downloadLocation": download_location,
                "filesAnalyzed": false,
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": declared_license,
                "copyrightText": "NOASSERTION",
                "sourceInfo": source_info
            }))
        })
        .collect::<Result<Vec<Value>, XtaskError>>()?;

    let mut relationships = BTreeSet::new();
    for member in &members {
        let reference = references.get(member).ok_or_else(|| {
            XtaskError::Usage(format!(
                "workspace member is missing from Cargo packages: {member}"
            ))
        })?;
        relationships.insert((
            "SPDXRef-DOCUMENT".to_owned(),
            "DESCRIBES".to_owned(),
            reference.clone(),
        ));
    }
    let mut edges = BTreeSet::new();
    for node in nodes {
        let from = required_str(node, "id")?;
        if !packages_by_id.contains_key(from) {
            return Err(XtaskError::Usage(format!(
                "cargo metadata resolve node has unknown package ID: {from}"
            )));
        }
        let dependencies = node.get("deps").and_then(Value::as_array).ok_or_else(|| {
            XtaskError::Usage("cargo metadata node has no `deps` array".to_owned())
        })?;
        for dependency in dependencies {
            let to = required_str(dependency, "pkg")?;
            if !packages_by_id.contains_key(to) {
                return Err(XtaskError::Usage(format!(
                    "cargo metadata dependency edge has unknown package ID: {to}"
                )));
            }
            edges.insert((from.to_owned(), to.to_owned()));
        }
    }
    for (from, to) in edges {
        let from_ref = references.get(&from).ok_or_else(|| {
            XtaskError::Usage(format!("missing SPDX reference for Cargo package {from}"))
        })?;
        let to_ref = references.get(&to).ok_or_else(|| {
            XtaskError::Usage(format!("missing SPDX reference for Cargo package {to}"))
        })?;
        relationships.insert((from_ref.clone(), "DEPENDS_ON".to_owned(), to_ref.clone()));
    }
    let lock_hash = blake3::hash(lock_bytes).to_hex().to_string();
    let metadata_bytes = serde_json::to_vec(metadata)?;
    let mut namespace_hasher = blake3::Hasher::new();
    namespace_hasher.update(&metadata_bytes);
    namespace_hasher.update(lock_bytes);
    namespace_hasher.update(created.as_bytes());
    let namespace_hash = namespace_hasher.finalize().to_hex().to_string();
    let mut document_describes = members
        .iter()
        .map(|member| {
            references.get(member).cloned().ok_or_else(|| {
                XtaskError::Usage(format!(
                    "workspace member is missing from Cargo packages: {member}"
                ))
            })
        })
        .collect::<Result<Vec<String>, XtaskError>>()?;
    document_describes.push("SPDXRef-Cargo-Lock".to_owned());
    relationships.insert((
        "SPDXRef-DOCUMENT".to_owned(),
        "DESCRIBES".to_owned(),
        "SPDXRef-Cargo-Lock".to_owned(),
    ));
    let spdx_relationships = relationships
        .into_iter()
        .map(|(from, relationship_type, to)| {
            json!({
                "spdxElementId": from,
                "relationshipType": relationship_type,
                "relatedSpdxElement": to
            })
        })
        .collect::<Vec<Value>>();
    let doc = json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": "Polaris Cargo workspace release SBOM",
        "documentNamespace": format!("https://spdx.org/spdxdocs/polaris-{namespace_hash}"),
        "comment": format!("Cargo workspace root: {workspace_root}; Cargo.lock BLAKE3: {lock_hash}. Generated from cargo metadata --locked --offline. Scope: full resolved workspace graph, including build, development and platform dependencies; not a target-binary inventory. This inventory is not a license audit or vulnerability assessment."),
        "creationInfo": { "creators": ["Tool: cargo-xtask-release-sbom"], "created": created },
        "documentDescribes": document_describes,
        "packages": spdx_packages,
        "files": [{
            "fileName": "Cargo.lock",
            "SPDXID": "SPDXRef-Cargo-Lock",
            "checksums": [{ "algorithm": "BLAKE3", "checksumValue": lock_hash }]
        }],
        "relationships": spdx_relationships
    });
    let mut bytes = serde_json::to_vec_pretty(&doc)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Run locked offline Cargo metadata and write the generated document atomically.
///
/// # Errors
/// Returns an error when Cargo fails, inputs cannot be read, or output cannot be committed.
pub fn create_release_sbom(
    root: &Path,
    output: &Path,
    force: bool,
    created: &str,
) -> Result<usize, XtaskError> {
    let lock_path = root.join("Cargo.lock");
    let lock_before_metadata = fs::read(&lock_path)?;
    let metadata_output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked", "--offline"])
        .current_dir(root)
        .output()?;
    if !metadata_output.status.success() {
        return Err(XtaskError::Cargo {
            status: metadata_output.status.to_string(),
            stderr: String::from_utf8_lossy(&metadata_output.stderr).into_owned(),
        });
    }
    let metadata: Value = serde_json::from_slice(&metadata_output.stdout)?;
    let lock_after_metadata = fs::read(&lock_path)?;
    ensure_lock_unchanged(&lock_before_metadata, &lock_after_metadata)?;
    let document = spdx_document(&metadata, &lock_after_metadata, created)?;
    write_atomic(output, &document, force)?;
    Ok(document.len())
}

fn ensure_lock_unchanged(before: &[u8], after: &[u8]) -> Result<(), XtaskError> {
    if before == after {
        Ok(())
    } else {
        Err(XtaskError::Usage(
            "Cargo.lock changed while generating the release SBOM; retry with a stable lockfile"
                .to_owned(),
        ))
    }
}

fn write_atomic(path: &Path, content: &[u8], force: bool) -> Result<(), XtaskError> {
    let parent = match path.parent() {
        Some(parent) => parent,
        None => Path::new("."),
    };
    let file_name = path
        .file_name()
        .ok_or_else(|| XtaskError::Usage("SBOM output must name a file".to_owned()))?;
    let mut temporary = PathBuf::new();
    let mut temporary_file = None;
    for attempt in 0_u32..100 {
        let mut candidate_name = file_name.to_os_string();
        candidate_name.push(format!(".tmp-{}-{attempt}", std::process::id()));
        let candidate = parent.join(candidate_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = candidate;
                temporary_file = Some(file);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut file = temporary_file.ok_or_else(|| {
        XtaskError::Usage("could not allocate a unique temporary SBOM file".to_owned())
    })?;
    file.write_all(content)?;
    file.sync_all()?;
    drop(file);

    let result = if force {
        fs::rename(&temporary, path)
    } else {
        fs::hard_link(&temporary, path)
    };
    if let Err(error) = result {
        let _cleanup = fs::remove_file(&temporary);
        if !force && error.kind() == std::io::ErrorKind::AlreadyExists {
            return Err(XtaskError::Usage(format!(
                "SBOM output already exists: {}; pass --force to replace it",
                path.display()
            )));
        }
        return Err(error.into());
    }
    if !force {
        fs::remove_file(temporary)?;
    }
    Ok(())
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, XtaskError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| XtaskError::Usage(format!("cargo metadata item has no string `{field}`")))
}

fn validate_created(created: &str) -> Result<(), XtaskError> {
    let mut chars = created.chars();
    let year = parse_timestamp_component(&mut chars, 4)?;
    consume_timestamp_char(&mut chars, '-')?;
    let month = parse_timestamp_component(&mut chars, 2)?;
    consume_timestamp_char(&mut chars, '-')?;
    let day = parse_timestamp_component(&mut chars, 2)?;
    consume_timestamp_char(&mut chars, 'T')?;
    let hour = parse_timestamp_component(&mut chars, 2)?;
    consume_timestamp_char(&mut chars, ':')?;
    let minute = parse_timestamp_component(&mut chars, 2)?;
    consume_timestamp_char(&mut chars, ':')?;
    let second = parse_timestamp_component(&mut chars, 2)?;
    consume_timestamp_char(&mut chars, 'Z')?;
    if chars.next().is_some() || year == 0 || hour > 23 || minute > 59 || second > 59 {
        return Err(invalid_created());
    }
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        _ => return Err(invalid_created()),
    };
    if day == 0 || day > days_in_month {
        return Err(invalid_created());
    }
    Ok(())
}

fn parse_timestamp_component(
    chars: &mut std::str::Chars<'_>,
    width: usize,
) -> Result<u32, XtaskError> {
    let mut value = 0_u32;
    for _ in 0..width {
        let character = chars.next().ok_or_else(invalid_created)?;
        if !character.is_ascii_digit() {
            return Err(invalid_created());
        }
        let digit = character.to_digit(10).ok_or_else(invalid_created)?;
        value = value
            .checked_mul(10)
            .and_then(|number| number.checked_add(digit))
            .ok_or_else(invalid_created)?;
    }
    Ok(value)
}

fn consume_timestamp_char(
    chars: &mut std::str::Chars<'_>,
    expected: char,
) -> Result<(), XtaskError> {
    if chars.next() == Some(expected) {
        Ok(())
    } else {
        Err(invalid_created())
    }
}

fn invalid_created() -> XtaskError {
    XtaskError::Usage(
        "--created must be a valid UTC timestamp in YYYY-MM-DDTHH:MM:SSZ form".to_owned(),
    )
}

fn string_set(value: &Value, field: &str) -> Result<BTreeSet<String>, XtaskError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| XtaskError::Usage(format!("cargo metadata JSON has no `{field}` array")))?
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                XtaskError::Usage(format!("cargo metadata `{field}` contains a non-string"))
            })
        })
        .collect()
}

fn source_download_location(source: &str) -> String {
    if let Some(registry) = source.strip_prefix("registry+") {
        if registry.contains("crates.io-index") {
            return "https://crates.io".to_owned();
        }
        return registry.to_owned();
    }
    match source.strip_prefix("git+") {
        Some(url) => url.to_owned(),
        None => source.to_owned(),
    }
}

fn source_description(source: Option<&str>, manifest_path: &str, workspace_root: &str) -> String {
    let source_text = source.map_or_else(
        || "path dependency (Cargo metadata source is null)".to_owned(),
        str::to_owned,
    );
    let manifest = Path::new(manifest_path);
    let root = Path::new(workspace_root);
    let path_text = manifest.strip_prefix(root).map_or_else(
        |_| manifest_path.to_owned(),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    );
    let vendor_note = if path_text.starts_with("vendor/") || path_text.contains("/vendor/") {
        "; controlled vendor source"
    } else {
        ""
    };
    if vendor_note.is_empty() && source.is_some() {
        format!("Cargo source: {source_text}")
    } else {
        format!("Cargo source: {source_text}; manifest: {path_text}{vendor_note}")
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    use super::write_atomic;

    #[test]
    fn atomic_output_preserves_existing_file_without_force() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("release.spdx.json");
        fs::write(&output, b"user data")?;
        let result = write_atomic(&output, b"new SBOM", false);
        assert!(result.is_err(), "existing output must be protected");
        assert_eq!(fs::read(&output)?, b"user data");
        Ok(())
    }

    #[test]
    fn atomic_output_replaces_existing_file_only_when_forced() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("release.spdx.json");
        fs::write(&output, b"user data")?;
        write_atomic(&output, b"new SBOM", true)?;
        assert_eq!(fs::read(&output)?, b"new SBOM");
        Ok(())
    }

    #[test]
    fn failed_atomic_replacement_preserves_existing_path() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("release.spdx.json");
        fs::create_dir(&output)?;
        let sentinel = output.join("preserve.txt");
        fs::write(&sentinel, b"existing destination")?;
        let result = write_atomic(&output, b"replacement", true);
        assert!(
            result.is_err(),
            "a directory cannot be replaced with a file"
        );
        assert!(
            output.is_dir(),
            "failed replacement must preserve destination"
        );
        assert_eq!(fs::read(sentinel)?, b"existing destination");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn failed_atomic_replacement_preserves_locked_existing_file() -> Result<(), Box<dyn Error>> {
        use std::os::windows::fs::OpenOptionsExt;

        let directory = tempfile::tempdir()?;
        let output = directory.path().join("release.spdx.json");
        fs::write(&output, b"existing artifact")?;
        let locked_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&output)?;
        let result = write_atomic(&output, b"replacement", true);
        assert!(
            result.is_err(),
            "a destination locked against rename must fail"
        );
        drop(locked_file);
        assert_eq!(fs::read(output)?, b"existing artifact");
        Ok(())
    }

    #[test]
    fn cargo_lock_change_during_metadata_is_rejected() -> Result<(), Box<dyn Error>> {
        use super::ensure_lock_unchanged;

        let result = ensure_lock_unchanged(b"lock before", b"lock after");
        assert!(
            result.is_err(),
            "changed lock bytes must invalidate the result"
        );
        ensure_lock_unchanged(b"same lock", b"same lock")?;
        Ok(())
    }
}
