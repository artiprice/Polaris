//! Input Parser (descriptor: specs/descriptors/directives_input_parser.json).

use std::error::Error;

use polaris_kernel_directives::directive::{
    Arg, Budget, CiBackend, EnvKind, ListArg, NvidiaDocsArg, NvidiaMode, Pool, SandboxMode,
    SessionDirective, StorageLevel, Toggle,
};
use polaris_kernel_directives::{DirectiveError, ParseLimits, Segment, parse, parse_segments};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn directives(input: &str) -> Result<Vec<SessionDirective>, DirectiveError> {
    Ok(parse(input, &ParseLimits::default())?
        .directives
        .into_iter()
        .map(|d| d.value)
        .collect())
}

#[test]
fn bare_directive_means_on() -> TestResult {
    let parsed = parse("$websearch find docs", &ParseLimits::default())?;
    assert_eq!(parsed.directives.len(), 1);
    let first = parsed.directives.first().ok_or("no directive")?;
    assert_eq!(first.value, SessionDirective::WebSearch(Toggle::On));
    assert_eq!(first.span, 0..10);
    assert_eq!(parsed.prose, "find docs");
    assert!(parsed.command.is_none());
    Ok(())
}

#[test]
fn directive_with_off_argument() -> TestResult {
    assert_eq!(
        directives("$websearch(off)")?,
        vec![SessionDirective::WebSearch(Toggle::Off)]
    );
    assert_eq!(
        directives("$websearch( OFF )")?,
        vec![SessionDirective::WebSearch(Toggle::Off)]
    );
    assert_eq!(
        directives("$websearch(on)")?,
        vec![SessionDirective::WebSearch(Toggle::On)]
    );
    Ok(())
}

#[test]
fn directive_inside_code_span_is_ignored() -> TestResult {
    for input in [
        "use `$websearch` here",
        "```\n$rag\n```",
        "x ``$needle`` y",
        "```rust\nlet a = $b;\n",
    ] {
        let parsed = parse(input, &ParseLimits::default())?;
        assert!(
            parsed.directives.is_empty(),
            "{input:?} -> {:?}",
            parsed.directives
        );
        assert_eq!(parsed.prose, input.trim());
    }
    Ok(())
}

#[test]
fn unknown_directive_kept_as_custom() -> TestResult {
    let got = directives("$my-flag(a, k=v, f(x)) go")?;
    assert_eq!(
        got,
        vec![SessionDirective::Custom {
            name: "my-flag".into(),
            args: vec![
                Arg::Atom("a".into()),
                Arg::Kv {
                    key: "k".into(),
                    value: Box::new(Arg::Atom("v".into()))
                },
                Arg::Call {
                    name: "f".into(),
                    quoted: false,
                    args: vec![Arg::Atom("x".into())]
                },
            ],
        }]
    );
    assert_eq!(
        directives("$x-feature")?,
        vec![SessionDirective::Custom {
            name: "x-feature".into(),
            args: vec![]
        }]
    );
    Ok(())
}

#[test]
fn dollar_needs_a_boundary_and_a_lowercase_name() -> TestResult {
    for input in [
        "costs $5 and $100",
        "a$websearch",
        "echo $HOME",
        "$ alone",
        "100$",
    ] {
        let parsed = parse(input, &ParseLimits::default())?;
        assert!(
            parsed.directives.is_empty(),
            "{input:?} -> {:?}",
            parsed.directives
        );
        assert_eq!(parsed.prose, input.trim());
    }
    assert_eq!(
        directives("(see $rag)")?,
        vec![SessionDirective::Rag(ListArg::On)]
    );
    Ok(())
}

#[test]
fn windows_paths_are_atoms_not_kv() -> TestResult {
    let got = directives(r#"$custom("C:\Users\me\a b.txt", C:\dir, 'it\'s')"#)?;
    assert_eq!(
        got,
        vec![SessionDirective::Custom {
            name: "custom".into(),
            args: vec![
                Arg::Quoted(r"C:\Users\me\a b.txt".into()),
                Arg::Atom(r"C:\dir".into()),
                Arg::Quoted("it's".into()),
            ],
        }]
    );
    Ok(())
}

#[test]
fn unbalanced_paren_reports_offset() {
    assert_eq!(
        parse("$sandbox(openshell", &ParseLimits::default()),
        Err(DirectiveError::UnbalancedParen { at: 8 })
    );
    assert_eq!(
        parse("go $a(b(c)", &ParseLimits::default()),
        Err(DirectiveError::UnbalancedParen { at: 5 })
    );
    assert!(matches!(
        parse("$a(\"open", &ParseLimits::default()),
        Err(DirectiveError::UnterminatedQuote { at: 3 })
    ));
}

#[test]
fn limits_are_enforced() {
    let limits = ParseLimits {
        max_input_bytes: 16,
        max_depth: 3,
        max_args: 4,
    };
    assert_eq!(
        parse(&"x".repeat(17), &limits),
        Err(DirectiveError::TooLong)
    );
    let deep = "$a(".repeat(20_000);
    assert_eq!(
        parse(&deep, &ParseLimits::default()),
        Err(DirectiveError::TooDeep)
    );
    let small = ParseLimits {
        max_input_bytes: 1024,
        max_depth: 3,
        max_args: 4,
    };
    assert_eq!(
        parse("$a(b(c(d(e))))", &small),
        Err(DirectiveError::TooDeep)
    );
    assert_eq!(
        parse("$a(1, 2, 3, 4, 5)", &small),
        Err(DirectiveError::TooManyArgs)
    );
    let parens = "(".repeat(60_000);
    assert!(
        parse(&parens, &ParseLimits::default()).is_ok(),
        "bare parentheses are prose"
    );
}

#[test]
fn catalog_rows_parse_to_typed_directives() -> TestResult {
    let cases: Vec<(&str, SessionDirective)> = vec![
        ("$subagents", SessionDirective::Subagents(Toggle::On)),
        (
            "$links(https://a.dev/x?q=1, \"b c\")",
            SessionDirective::Links(ListArg::Items(vec![
                "https://a.dev/x?q=1".into(),
                "b c".into(),
            ])),
        ),
        ("$skills(off)", SessionDirective::Skills(ListArg::Off)),
        ("$mcp", SessionDirective::Mcp(ListArg::On)),
        (
            "$rag(docs)",
            SessionDirective::Rag(ListArg::Items(vec!["docs".into()])),
        ),
        ("$nvidia", SessionDirective::Nvidia(NvidiaMode::Auto)),
        ("$nvidia(list)", SessionDirective::Nvidia(NvidiaMode::List)),
        (
            "$nvidia-docs(sync)",
            SessionDirective::NvidiaDocs(NvidiaDocsArg::Sync),
        ),
        (
            "$sandbox(microvm)",
            SessionDirective::Sandbox(SandboxMode::Microvm),
        ),
        (
            "$nebius-sandbox(contree)",
            SessionDirective::Sandbox(SandboxMode::Contree),
        ),
        (
            "$nebius-branching(on, max_branches=4)",
            SessionDirective::NebiusBranching {
                on: true,
                max_branches: Some(4),
            },
        ),
        (
            "$nebius-datalab",
            SessionDirective::NebiusDatalab(Toggle::On),
        ),
        (
            "$nebius-speculative(off)",
            SessionDirective::NebiusSpeculative(Toggle::Off),
        ),
        (
            "$concurrency(web=15, inference=8, io=128)",
            SessionDirective::Concurrency(vec![
                (Pool::Web, 15),
                (Pool::Inference, 8),
                (Pool::Disk, 128),
            ]),
        ),
        (
            "$ratelimit(rpm=600, tpm=200000)",
            SessionDirective::RateLimit {
                rpm: Some(600),
                tpm: Some(200_000),
            },
        ),
        (
            "$ratelimit(600, 200000)",
            SessionDirective::RateLimit {
                rpm: Some(600),
                tpm: Some(200_000),
            },
        ),
        (
            "$budget(usd=5.25)",
            SessionDirective::Budget(Budget::Usd("5.25".into())),
        ),
        (
            "$budget(tokens=2000000)",
            SessionDirective::Budget(Budget::Tokens(2_000_000)),
        ),
        (
            "$storage-level(isolated)",
            SessionDirective::StorageLevel(StorageLevel::Isolated),
        ),
        ("$browser", SessionDirective::Browser(Toggle::On)),
        ("$needle(off)", SessionDirective::Needle(Toggle::Off)),
        ("$env(nix)", SessionDirective::Env(EnvKind::Nix)),
        ("$ci(dagger)", SessionDirective::Ci(CiBackend::Dagger)),
    ];
    for (input, want) in cases {
        assert_eq!(directives(input)?, vec![want], "{input}");
    }
    for bad in [
        "$sandbox(cloud)",
        "$budget(usd=five)",
        "$concurrency(gpu=2)",
        "$nvidia-docs",
        "$env(conda)",
    ] {
        assert!(
            matches!(directives(bad), Err(DirectiveError::BadArg { .. })),
            "{bad} must be BadArg"
        );
    }
    Ok(())
}

#[test]
fn crlf_multiline_input() -> TestResult {
    let parsed = parse(
        "first line\r\n$rag\r\nthird $needle line",
        &ParseLimits::default(),
    )?;
    let got: Vec<_> = parsed.directives.iter().map(|d| d.value.clone()).collect();
    assert_eq!(
        got,
        vec![
            SessionDirective::Rag(ListArg::On),
            SessionDirective::Needle(Toggle::On)
        ]
    );
    assert!(parsed.prose.starts_with("first line"), "{:?}", parsed.prose);
    assert!(parsed.prose.contains("third"), "{:?}", parsed.prose);
    Ok(())
}

#[test]
fn attachments_are_never_parsed() -> TestResult {
    let parsed = parse_segments(
        &[
            Segment::Typed("$rag summarise "),
            Segment::Attachment("$websearch @file.rs /quit"),
        ],
        &ParseLimits::default(),
    )?;
    let got: Vec<_> = parsed.directives.iter().map(|d| d.value.clone()).collect();
    assert_eq!(got, vec![SessionDirective::Rag(ListArg::On)]);
    assert!(parsed.mentions.is_empty());
    assert!(
        parsed.prose.ends_with("$websearch @file.rs /quit"),
        "{:?}",
        parsed.prose
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_parse_never_panics_and_spans_are_valid(input in "\\PC{0,200}") {
        if let Ok(parsed) = parse(&input, &ParseLimits::default()) {
            for span in parsed.directives.iter().map(|d| d.span.clone()).chain(parsed.mentions.iter().map(|m| m.span.clone())) {
                prop_assert!(span.start <= span.end && span.end <= input.len());
                prop_assert!(input.is_char_boundary(span.start) && input.is_char_boundary(span.end));
            }
        }
    }

    #[test]
    fn prop_text_without_markers_is_prose(input in "[a-zA-Z0-9 .,!?\n]{0,120}") {
        let parsed = parse(&input, &ParseLimits::default());
        prop_assert!(parsed.is_ok());
        if let Ok(parsed) = parsed {
            prop_assert_eq!(parsed.prose, input.trim());
        }
    }
}
