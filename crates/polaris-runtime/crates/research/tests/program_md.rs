use polaris_runtime_research::ProgramMd;

#[test]
fn valid_program_parses() -> Result<(), Box<dyn std::error::Error>> {
    let source = "---\ngoal: minimise validation loss\nwall_ms: 5000\ntokens: 1000\nmicro_usd: 10000\nmax_experiments: 20\neditable: src/**\nreadonly: program.md,metrics/**\n---\n```metric\nname: validation_loss\ndirection: min\nformat: named\nharness: metrics/run.py\ncommand: python metrics/run.py\n```\n";
    let program = ProgramMd::parse(source)?;
    assert_eq!(program.goal(), "minimise validation loss");
    assert_eq!(program.metric().name(), "validation_loss");
    assert_eq!(program.budget().experiments(), 20);
    Ok(())
}

#[test]
fn missing_metric_is_rejected() {
    let source = "---\ngoal: minimise validation loss\nwall_ms: 5000\ntokens: 1000\nmicro_usd: 10000\nmax_experiments: 20\neditable: src/**\nreadonly: program.md,metrics/**\n---\n";
    assert!(ProgramMd::parse(source).is_err());
}

#[test]
fn overlapping_editable_and_readonly_rejected() {
    let source = "---\ngoal: minimise validation loss\nwall_ms: 5000\ntokens: 1000\nmicro_usd: 10000\nmax_experiments: 20\neditable: src/**,metrics/run.py\nreadonly: program.md,metrics/**\n---\n```metric\nname: validation_loss\ndirection: min\nformat: named\nharness: metrics/run.py\ncommand: python metrics/run.py\n```\n";
    assert!(ProgramMd::parse(source).is_err());
}

#[test]
fn non_positive_budget_rejected() {
    let source = "---\ngoal: minimise validation loss\nwall_ms: 0\ntokens: 1000\nmicro_usd: 10000\nmax_experiments: 20\neditable: src/**\nreadonly: program.md,metrics/**\n---\n```metric\nname: validation_loss\ndirection: min\nformat: named\nharness: metrics/run.py\ncommand: python metrics/run.py\n```\n";
    assert!(ProgramMd::parse(source).is_err());
}

#[test]
fn program_digest_binds_exact_source() -> Result<(), Box<dyn std::error::Error>> {
    let base = "---\ngoal: minimise validation loss\nwall_ms: 5000\ntokens: 1000\nmicro_usd: 10000\nmax_experiments: 20\neditable: src/**\nreadonly: program.md,metrics/**\n---\n```metric\nname: validation_loss\ndirection: min\nformat: named\nharness: metrics/run.py\ncommand: python metrics/run.py\n```\n";
    let changed = base.replace("minimise validation loss", "minimise training loss");
    let first = ProgramMd::parse(base)?;
    let second = ProgramMd::parse(&changed)?;
    assert_ne!(first.cid(), second.cid());
    Ok(())
}
