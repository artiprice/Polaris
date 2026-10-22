//! Ignore And Deny Rules (descriptor: specs/descriptors/sync_ignore_and_deny_rules.json).

use std::error::Error;

use polaris_storage_sync::{Exclusion, IgnoreRules, RelPath};

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
fn git_config_denied() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let rules = IgnoreRules::new(tmp.path(), None)?;
    assert_eq!(class(&rules, ".git/config", false)?, Exclusion::Denied);
    assert_eq!(
        class(&rules, "vendor/lib/.git/config", false)?,
        Exclusion::Denied
    );
    assert_eq!(class(&rules, ".git/HEAD", false)?, Exclusion::Included);
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
        ("work/.git", false),
        (".polaris", true),
        (".polaris/vcs-undo.json", false),
        ("work/.polaris/cas/file", false),
    ] {
        assert_eq!(rules.classify(&RelPath::new(path)?, is_dir), Exclusion::Denied, "{path}");
    }
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
