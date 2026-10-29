//! Token Classification (descriptor: specs/descriptors/uimodel_token_classification.json).

use polaris_ui_model::classify::{ClassifiedSpan, TokenClass, classify};
use proptest::prelude::*;

fn span(start: usize, end: usize, class: TokenClass) -> ClassifiedSpan {
    ClassifiedSpan { start, end, class }
}

#[test]
fn known_complete_command_is_command() {
    assert_eq!(classify("/model"), [span(0, 6, TokenClass::Command)]);
    assert_eq!(classify("/model gpt-x"), [span(0, 6, TokenClass::Command)]);
    assert_eq!(classify("/exit"), [span(0, 5, TokenClass::Command)]);
    assert_eq!(
        classify("/model $websearch @a"),
        [
            span(0, 6, TokenClass::Command),
            span(7, 17, TokenClass::Directive),
            span(18, 20, TokenClass::Mention),
        ]
    );
}

#[test]
fn incomplete_or_unknown_command_is_neutral() {
    assert_eq!(classify("/mod"), [span(0, 4, TokenClass::CommandUnknown)]);
    assert_eq!(
        classify("/nonsense arg"),
        [span(0, 9, TokenClass::CommandUnknown)]
    );
    assert!(
        classify("/").is_empty(),
        "a bare slash has nothing to colour"
    );
}

#[test]
fn known_directive_is_directive() {
    assert_eq!(
        classify("use $websearch and $custom"),
        [
            span(4, 14, TokenClass::Directive),
            span(19, 26, TokenClass::DirectiveUnknown),
        ]
    );
    assert_eq!(
        classify("$subagent-models(m(low-high))"),
        [span(0, 16, TokenClass::Directive)]
    );
    assert_eq!(classify("x($mcp)"), [span(2, 6, TokenClass::Directive)]);
}

#[test]
fn mention_spans_including_quoted() {
    assert_eq!(
        classify("see @src/main.rs."),
        [span(4, 16, TokenClass::Mention)]
    );
    assert_eq!(
        classify("@\"my file.txt\" ok"),
        [span(0, 14, TokenClass::Mention)]
    );
    assert_eq!(classify("@\"my fi"), [span(0, 7, TokenClass::Mention)]);
    assert_eq!(classify("(@a/b)"), [span(1, 5, TokenClass::Mention)]);
    assert_eq!(
        classify("@docx(src/a.docx) x"),
        [span(0, 17, TokenClass::Mention)]
    );
    assert!(classify("@").is_empty());
}

#[test]
fn inert_text_has_no_spans() {
    for input in [
        "http://x/y",
        "a$b",
        "dev@x.com",
        "$5",
        "$HOME",
        "see /model",
        "price $5 today",
        "`$web` `/model` `@a`",
        "```\n$websearch @a\n```",
        "plain prose, nothing special",
        "",
    ] {
        assert!(
            classify(input).is_empty(),
            "{input:?} -> {:?}",
            classify(input)
        );
    }
}

proptest! {
    #[test]
    fn prop_spans_are_in_bounds_ordered_non_overlapping(
        input in prop_oneof![
            any::<String>(),
            "[a-z$@/ ()\\[\\]`\"\u{e9}\u{301}\u{1F600}.,-]{0,32}",
        ],
    ) {
        let mut last_end = 0usize;
        for s in classify(&input) {
            prop_assert!(s.start < s.end);
            prop_assert!(s.start >= last_end);
            prop_assert!(s.end <= input.len());
            prop_assert!(input.is_char_boundary(s.start));
            prop_assert!(input.is_char_boundary(s.end));
            last_end = s.end;
        }
    }
}
