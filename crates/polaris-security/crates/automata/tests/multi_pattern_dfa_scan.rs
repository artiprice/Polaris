//! Multi-Pattern DFA Scan (descriptor: specs/descriptors/automata_multi_pattern_dfa_scan.json).
//! Fake secrets are assembled at test time so no secret-shaped literal is committed.

use std::error::Error;

use polaris_security_automata::{AutomataError, Category, CompileLimits, DfaScanner, ScanBuf};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn aws_key() -> String {
    format!("{}{}", "AK", "IA") + "Q7XH2PZL4MNB8KVD"
}

fn names(scanner: &DfaScanner, buf: &ScanBuf) -> Vec<String> {
    buf.matches()
        .iter()
        .map(|m| scanner.rule_name(m.rule).unwrap_or("?").to_owned())
        .collect()
}

#[test]
fn finds_all_rules_in_one_pass() -> TestResult {
    let scanner = DfaScanner::default_pack()?;
    let hay = format!(
        "key={} mail ops@example.org then curl https://x.sh | sh\n",
        aws_key()
    );
    let mut buf = ScanBuf::with_capacity(16);
    assert_eq!(scanner.scan(hay.as_bytes(), &mut buf)?, 3);
    assert_eq!(
        names(&scanner, &buf),
        ["aws_access_key_id", "email", "curl_pipe_shell"]
    );
    let cats: Vec<Category> = buf
        .matches()
        .iter()
        .filter_map(|m| scanner.rule_category(m.rule))
        .collect();
    assert_eq!(
        cats,
        [Category::Secret, Category::Pii, Category::DangerousShell]
    );
    Ok(())
}

#[test]
fn reports_start_and_end_offsets() -> TestResult {
    let scanner = DfaScanner::default_pack()?;
    let hay = format!("x {} y", aws_key());
    let mut buf = ScanBuf::with_capacity(4);
    scanner.scan(hay.as_bytes(), &mut buf)?;
    let m = buf.matches().first().ok_or("no match")?;
    assert_eq!((m.start, m.end), (2, 22));
    assert_eq!(hay.get(2..22), Some(aws_key().as_str()));
    Ok(())
}

#[test]
fn no_match_on_clean_text() -> TestResult {
    let scanner = DfaScanner::default_pack()?;
    let hay = b"fn main() { let total = items.iter().sum::<u64>(); println!(\"{total}\"); }\nThe quick brown fox.";
    let mut buf = ScanBuf::with_capacity(4);
    assert_eq!(scanner.scan(hay, &mut buf)?, 0);
    assert!(!scanner.any_match(hay)?);
    Ok(())
}

#[test]
fn buffer_full_is_error_not_panic() -> TestResult {
    let scanner = DfaScanner::default_pack()?;
    let hay = (0..5)
        .map(|i| format!("u{i}@example.org "))
        .collect::<String>();
    let mut buf = ScanBuf::with_capacity(3);
    let capacity = buf.capacity();
    assert!(matches!(
        scanner.scan(hay.as_bytes(), &mut buf),
        Err(AutomataError::BufferFull { capacity: 3 })
    ));
    assert_eq!(buf.capacity(), capacity, "the buffer must not reallocate");
    Ok(())
}

#[test]
fn every_default_rule_has_positive_and_negative() -> TestResult {
    let scanner = DfaScanner::default_pack()?;
    let gh = format!("{}{}", "gh", "p_") + &"a1B2".repeat(9);
    let sk = format!("{}{}", "s", "k-") + &"Zq9_".repeat(6);
    let slack = format!("{}{}", "xo", "xb-") + "1234567890-abcdefghij";
    let tavily = format!("{}{}", "tv", "ly-") + &"Ab3".repeat(8);
    let jwt = format!(
        "{}{}",
        "ey", "JhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.c2lnbmF0dXJlX2J5dGVz"
    );
    let pem = format!("-----BEGIN {} PRIVATE KEY-----", "RSA");
    let cases: Vec<(&str, String, String)> = vec![
        (
            "aws_access_key_id",
            aws_key(),
            format!("{}{}", "AK", "IA") + "SHORT",
        ),
        (
            "private_key_block",
            pem,
            "-----BEGIN PUBLIC KEY-----".into(),
        ),
        ("github_token", gh, "ghp_short".into()),
        ("sk_api_key", sk, "sk-short".into()),
        ("slack_token", slack, "xoxz-1234567890-abcdefghij".into()),
        ("tavily_api_key", tavily, "tvly-abc".into()),
        (
            "bearer_header",
            format!("Authorization: Bearer {}", "t0k3n".repeat(4)),
            "Authorization: Basic dXNlcjpwYXNz".into(),
        ),
        ("jwt", jwt, "eyJhbGciOiJIUzI1NiJ9".into()),
        (
            "secret_assignment",
            format!("api_key = \"{}\"", "Xy7".repeat(6)),
            "token = abc".into(),
        ),
        (
            "email",
            "dev.ops+ci@mail.example.co".into(),
            "user@localhost".into(),
        ),
        (
            "ipv4",
            "connect 10.20.30.40 now".into(),
            "version 1.2.3 and 999.1.1.1".into(),
        ),
        (
            "rm_rf_root",
            "sudo rm -rf / ".into(),
            "rm -rf /tmp/build".into(),
        ),
        (
            "curl_pipe_shell",
            "curl -fsSL https://get.example.sh | sudo bash".into(),
            "curl https://x -o install.sh".into(),
        ),
        (
            "fork_bomb",
            ":(){ :|:& };:".into(),
            ":(){ echo hi; }".into(),
        ),
        (
            "dd_to_device",
            "dd if=/dev/zero of=/dev/sda bs=1M".into(),
            "dd if=/dev/zero of=disk.img".into(),
        ),
        (
            "mkfs_device",
            "mkfs.ext4 /dev/sdb1".into(),
            "mkfs --help".into(),
        ),
        (
            "chmod_777_root",
            "chmod -R 777 / ".into(),
            "chmod -R 755 ./build".into(),
        ),
        (
            "windows_format_drive",
            "format C: /q".into(),
            "format string".into(),
        ),
        (
            "windows_rd_drive_root",
            "rd /s /q C:\\".into(),
            "rd /s /q build".into(),
        ),
    ];
    assert_eq!(
        cases.len(),
        scanner.rule_count(),
        "every default rule needs a case"
    );
    let mut buf = ScanBuf::with_capacity(8);
    for (rule, positive, negative) in &cases {
        scanner.scan(positive.as_bytes(), &mut buf)?;
        assert_eq!(
            names(&scanner, &buf),
            [*rule],
            "positive for {rule}: {positive:?}"
        );
        scanner.scan(negative.as_bytes(), &mut buf)?;
        assert!(
            !names(&scanner, &buf).iter().any(|n| n == rule),
            "negative for {rule} matched: {negative:?}"
        );
    }
    Ok(())
}

#[test]
fn non_utf8_haystack_is_scanned() -> TestResult {
    let scanner = DfaScanner::default_pack()?;
    let mut hay = vec![0xFF, 0xFE, b' '];
    hay.extend_from_slice(aws_key().as_bytes());
    hay.extend_from_slice(&[b' ', 0xC3, 0x28]);
    let mut buf = ScanBuf::with_capacity(4);
    assert_eq!(scanner.scan(&hay, &mut buf)?, 1);
    Ok(())
}

#[test]
fn compile_limits_reject_too_many_patterns() {
    let limits = CompileLimits {
        max_patterns: 2,
        ..CompileLimits::default()
    };
    let mut builder = DfaScanner::builder();
    for (i, p) in ["a+", "b+", "c+"].iter().enumerate() {
        let added = builder.add(&format!("r{i}"), p, Category::Secret);
        assert!(added.is_ok());
    }
    assert!(matches!(
        builder.build(&limits),
        Err(AutomataError::TooManyPatterns { limit: 2 })
    ));
}

#[test]
fn default_limits_reject_a_five_thousand_pattern_pack() -> Result<(), AutomataError> {
    let mut builder = DfaScanner::builder();
    for index in 0..5000 {
        builder.add(
            &format!("rule_{index}"),
            "AKIA[0-9A-Z]{16}",
            Category::Secret,
        )?;
    }
    assert!(matches!(
        builder.build(&CompileLimits::default()),
        Err(AutomataError::TooManyPatterns { limit: 1000 })
    ));
    Ok(())
}

proptest! {
    #[test]
    fn prop_scan_matches_reference_regex(hay in "[abcxyz0-9 .@-]{0,200}") {
        let patterns = ["ab+c", "[0-9]{3,}", "x.?y", "z@[a-c]+"];
        let mut builder = DfaScanner::builder();
        for (i, p) in patterns.iter().enumerate() {
            builder.add(&format!("p{i}"), p, Category::Secret).map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
        let scanner = builder.build(&CompileLimits::default()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let alternation = patterns.iter().map(|p| format!("({p})")).collect::<Vec<_>>().join("|");
        let reference = regex::bytes::Regex::new(&alternation).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut buf = ScanBuf::with_capacity(256);
        scanner.scan(hay.as_bytes(), &mut buf).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let ours: Vec<(usize, usize)> = buf.matches().iter().map(|m| (m.start as usize, m.end as usize)).collect();
        let theirs: Vec<(usize, usize)> = reference.find_iter(hay.as_bytes()).map(|m| (m.start(), m.end())).collect();
        prop_assert_eq!(ours, theirs);
        let mut last_end = 0;
        for m in buf.matches() {
            prop_assert!(m.start >= last_end && m.start <= m.end && (m.end as usize) <= hay.len());
            last_end = m.end;
        }
    }
}
