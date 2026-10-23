use polaris_runtime_research::MetricParser;
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn scalar_requires_one_finite_number() -> TestResult {
    let parser = MetricParser::scalar(64)?;
    assert_eq!(parser.parse(b" \r\n-1.25e2\r\n")?.get(), -125.0);
    for bytes in [
        b"NaN".as_slice(),
        b"inf",
        b"-inf",
        b"1\n2",
        b"",
        b"1 extra",
        b"\xff",
        b"1\0",
    ] {
        assert!(parser.parse(bytes).is_err(), "invalid scalar accepted");
    }
    assert!(parser.parse(&[b'1'; 65]).is_err());
    assert!(MetricParser::scalar(0).is_err());
    assert!(MetricParser::scalar(65_537).is_err());
    Ok(())
}

#[test]
fn named_metric_tolerates_crlf_but_not_duplicates() -> TestResult {
    let parser = MetricParser::named("validation_loss", 65_536)?;
    assert_eq!(
        parser
            .parse(b"starting experiment\r\nvalidation_loss= 0.125\r\ncomplete\r\n")?
            .get(),
        0.125
    );
    assert!(
        parser
            .parse(b"validation_loss=0.125\nvalidation_loss=0.125\n")
            .is_err()
    );
    assert!(
        parser
            .parse(b"validation_loss=bad\nvalidation_loss=0.125\n")
            .is_err()
    );
    assert!(parser.parse(b"xvalidation_loss=0.125\n").is_err());
    assert!(parser.parse(b"validation_loss=NaN\n").is_err());
    assert!(parser.parse(b"validation_loss=0.125 extra\n").is_err());
    assert!(MetricParser::named("loss=", 64).is_err());
    assert!(MetricParser::named("", 64).is_err());
    Ok(())
}
