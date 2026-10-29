//! Mention Parser (descriptor: specs/descriptors/directives_mention_parser.json).

use std::error::Error;

use polaris_kernel_directives::mention::{Mention, PathArg};
use polaris_kernel_directives::{ParseLimits, parse};

type TestResult = Result<(), Box<dyn Error>>;

fn mentions(input: &str) -> Result<Vec<Mention>, Box<dyn Error>> {
    Ok(parse(input, &ParseLimits::default())?
        .mentions
        .into_iter()
        .map(|m| m.value)
        .collect())
}

#[test]
fn path_mention() -> TestResult {
    let parsed = parse("look at @src/lib.rs please", &ParseLimits::default())?;
    let first = parsed.mentions.first().ok_or("no mention")?;
    assert_eq!(first.value, Mention::File(PathArg::from("src/lib.rs")));
    assert_eq!(first.span, 8..19);
    assert_eq!(
        parsed.prose, "look at @src/lib.rs please",
        "mentions stay in prose"
    );
    assert_eq!(
        mentions("@\"dir with space/a.rs\"")?,
        vec![Mention::File(PathArg::from("dir with space/a.rs"))]
    );
    Ok(())
}

#[test]
fn typed_mentions() -> TestResult {
    assert_eq!(
        mentions("@docx(report.docx)")?,
        vec![Mention::Docx(PathArg::from("report.docx"))]
    );
    assert_eq!(
        mentions("@pdf(\"a b.pdf\")")?,
        vec![Mention::Pdf(PathArg::from("a b.pdf"))]
    );
    assert_eq!(
        mentions("@xlsx( data.xlsx )")?,
        vec![Mention::Xlsx(Some(PathArg::from("data.xlsx")))]
    );
    assert_eq!(
        mentions("@img(shot.png)")?,
        vec![Mention::Img(Some(PathArg::from("shot.png")))]
    );
    assert_eq!(
        mentions("@scans and @img and @xlsx")?,
        vec![
            Mention::Scans(None),
            Mention::Img(None),
            Mention::Xlsx(None)
        ]
    );
    Ok(())
}

#[test]
fn mention_stops_at_closing_bracket_and_trailing_punctuation() -> TestResult {
    assert_eq!(
        mentions("(@src/a.rs)")?,
        vec![Mention::File(PathArg::from("src/a.rs"))]
    );
    assert_eq!(
        mentions("[@b.rs]")?,
        vec![Mention::File(PathArg::from("b.rs"))]
    );
    assert_eq!(
        mentions("see @lib.rs.")?,
        vec![Mention::File(PathArg::from("lib.rs"))]
    );
    assert_eq!(
        mentions("@a.rs, @b.rs; done")?,
        vec![
            Mention::File(PathArg::from("a.rs")),
            Mention::File(PathArg::from("b.rs"))
        ]
    );
    Ok(())
}

#[test]
fn email_is_not_a_mention() -> TestResult {
    for input in [
        "mail dev@company.com now",
        "foo@bar",
        "@",
        "@ space",
        "x @.",
    ] {
        assert!(mentions(input)?.is_empty(), "{input:?}");
    }
    Ok(())
}

#[test]
fn windows_path_mention() -> TestResult {
    assert_eq!(
        mentions(r"open @C:\proj\a.rs")?,
        vec![Mention::File(PathArg::from(r"C:\proj\a.rs"))]
    );
    assert_eq!(
        mentions(r#"@pdf("C:\Users\me\a b.pdf")"#)?,
        vec![Mention::Pdf(PathArg::from(r"C:\Users\me\a b.pdf"))]
    );
    Ok(())
}

#[test]
fn mention_in_code_span_is_ignored() -> TestResult {
    assert!(mentions("call `@decorator` here")?.is_empty());
    assert!(mentions("```\n@src/x.rs\n```")?.is_empty());
    Ok(())
}
