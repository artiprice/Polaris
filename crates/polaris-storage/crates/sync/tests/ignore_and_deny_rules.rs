//! Ignore And Deny Rules (descriptor: specs/descriptors/sync_ignore_and_deny_rules.json).

use std::error::Error;
use std::io::Write;

use polaris_storage_sync::{Exclusion, IgnoreRules, MAX_POLARISIGNORE_BYTES, RelPath, SyncError};

type TestResult = Result<(), Box<dyn Error>>;

fn class(rules: &IgnoreRules, path: &str, is_dir: bool) -> Result<Exclusion, Box<dyn Error>> {
    Ok(rules.classify(&RelPath::new(path)?, is_dir))
}

#[test]
fn polarisignore_excludes_matches() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let rules = IgnoreRules::new(tmp.path(), Some("target/\n*.log\n# comment\n"))?;
    assert_eq!(class(&rules, "build.log", false)?, Exclusion::Ignored);
    assert_eq!(class(&rules, "src/main.rs", false)?, Exclusion::Included);
    assert_eq!(class(&rules, "target", true)?, Exclusion::Ignored);
    Ok(())
}

#[test]
fn deny_list_defaults_present() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let rules = IgnoreRules::new(tmp.path(), None)?;
    for denied in [
        ".env",
        ".env.local",
        "deploy/server.pem",
        "tls.key",
        ".git/config",
        "home/.ssh/known_hosts",
        ".aws/credentials",
        ".aws/config",
        "id_rsa",
        "cert.pfx",
    ] {
        assert_eq!(class(&rules, denied, false)?, Exclusion::Denied, "{denied}");
    }
    Ok(())
}

#[test]
fn negation_cannot_reinclude_denied() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let rules = IgnoreRules::new(tmp.path(), Some("*\n!.env\n!*.pem\n"))?;
    assert_eq!(class(&rules, ".env", false)?, Exclusion::Denied);
    assert_eq!(class(&rules, "a.pem", false)?, Exclusion::Denied);
    Ok(())
}

#[test]
fn git_metadata_denied() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let rules = IgnoreRules::new(tmp.path(), None)?;
    assert_eq!(class(&rules, ".git/config", false)?, Exclusion::Denied);
    assert_eq!(
        class(&rules, "vendor/lib/.git/config", false)?,
        Exclusion::Denied
    );
    assert_eq!(class(&rules, ".git/HEAD", false)?, Exclusion::Denied);
    Ok(())
}

#[test]
fn git_and_polaris_metadata_are_denied_even_with_negation() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let rules = IgnoreRules::new(
        tmp.path(),
        Some("!.git/**\n!.polaris/**\n!.git\n!.polaris\n"),
    )?;
    for (path, is_dir) in [
        (".git", true),
        (".git/config", false),
        (".GIT/config", false),
        ("work/.git", false),
        (".polaris", true),
        (".POLARIS/vcs-undo.json", false),
        (".polaris/vcs-undo.json", false),
        ("work/.polaris/cas/file", false),
    ] {
        assert_eq!(
            rules.classify(&RelPath::new(path)?, is_dir),
            Exclusion::Denied,
            "{path}"
        );
    }
    Ok(())
}

#[test]
fn aws_credential_tree_is_non_overridable_and_case_insensitive() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let rules = IgnoreRules::new(tmp.path(), Some("!.aws\n!.aws/**\n!**/.aws/**\n"))?;
    for (path, is_dir) in [
        (".aws", true),
        (".aws/credentials", false),
        (".aws/config", false),
        ("home/.aws", true),
        ("home/.aws/credentials", false),
        ("HOME/.AWS/CONFIG", false),
    ] {
        assert_eq!(
            rules.classify(&RelPath::new(path)?, is_dir),
            Exclusion::Denied,
            "{path}"
        );
    }
    Ok(())
}

#[test]
fn workspace_ignore_file_is_bounded_before_pattern_parsing() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let path = tmp.path().join(".polarisignore");
    let mut file = std::fs::File::create(path)?;
    file.write_all(&vec![b'a'; usize::try_from(MAX_POLARISIGNORE_BYTES + 1)?])?;
    let result = IgnoreRules::from_workspace(tmp.path());
    assert!(matches!(
        result,
        Err(SyncError::Limit {
            what: "polarisignore bytes",
            ..
        })
    ));
    Ok(())
}

#[test]
fn ignored_directory_prunes_children() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let rules = IgnoreRules::new(tmp.path(), Some("node_modules/\n"))?;
    assert_eq!(
        class(&rules, "web/node_modules/left-pad/index.js", false)?,
        Exclusion::Ignored
    );
    Ok(())
}
