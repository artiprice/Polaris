//! Category-isolated scanners prevent a benign overlapping rule from hiding a secret rule.
use crate::{Finding, SourceLocation, VerifierError};
use polaris_code_intel::ByteSpan;
use polaris_security_automata::{
    AutomataError, Category, DfaScanner, EntropyCfg, RuleId, ScanBuf, high_entropy_spans,
    shannon_entropy_milli,
};

/// Trusted, bounded signature scanning boundary.
pub trait Scanner: Send + Sync {
    /// Scan exactly one category; errors must not be discarded.
    fn scan(
        &self,
        category: Category,
        bytes: &[u8],
        out: &mut ScanBuf,
    ) -> Result<(), AutomataError>;
}

/// Separate compiled packs for credentials, personal data and command hazards.
#[derive(Debug, Clone)]
pub struct SafetyScanner {
    secrets: DfaScanner,
    pii: DfaScanner,
    commands: DfaScanner,
}

impl SafetyScanner {
    /// Compile the repository's actual default rule pack, without claiming exhaustive coverage.
    ///
    /// # Errors
    /// Returns the automata compiler error when a pack cannot be compiled.
    pub fn new() -> Result<Self, AutomataError> {
        Ok(Self {
            secrets: DfaScanner::default_pack_for(&[Category::Secret])?,
            pii: DfaScanner::default_pack_for(&[Category::Pii])?,
            commands: DfaScanner::default_pack_for(&[Category::DangerousShell])?,
        })
    }
}

impl Scanner for SafetyScanner {
    fn scan(
        &self,
        category: Category,
        bytes: &[u8],
        out: &mut ScanBuf,
    ) -> Result<(), AutomataError> {
        let scanner = match category {
            Category::Secret => &self.secrets,
            Category::Pii => &self.pii,
            Category::DangerousShell => &self.commands,
        };
        let _count = scanner.scan(bytes, out)?;
        Ok(())
    }
}

pub(crate) fn secret_findings(
    scanner: &impl Scanner,
    source: &[u8],
    capacity: usize,
    entropy: EntropyCfg,
) -> Result<Vec<Finding>, VerifierError> {
    let mut buffer = ScanBuf::with_capacity(capacity);
    scanner.scan(Category::Secret, source, &mut buffer)?;
    let mut findings = Vec::with_capacity(buffer.len());
    for found in buffer.matches() {
        findings.push(Finding::SecretLeak {
            rule: found.rule,
            loc: SourceLocation {
                span: checked_span(found.start, found.end)?,
                in_old: false,
            },
            entropy_milli: None,
        });
    }
    let _count = high_entropy_spans(source, &entropy, &mut buffer)?;
    for found in buffer.matches() {
        let span = checked_span(found.start, found.end)?;
        let bytes = span.slice(source).ok_or(VerifierError::InvalidInput {
            reason: "scanner span outside source",
        })?;
        findings.push(Finding::SecretLeak {
            rule: RuleId::ENTROPY,
            loc: SourceLocation {
                span,
                in_old: false,
            },
            entropy_milli: Some(shannon_entropy_milli(bytes)),
        });
    }
    Ok(findings)
}

fn checked_span(start: u32, end: u32) -> Result<ByteSpan, VerifierError> {
    let start = usize::try_from(start).map_err(|_| VerifierError::InvalidInput {
        reason: "unsupported scanner offset",
    })?;
    let end = usize::try_from(end).map_err(|_| VerifierError::InvalidInput {
        reason: "unsupported scanner offset",
    })?;
    if end < start {
        return Err(VerifierError::InvalidInput {
            reason: "reversed scanner span",
        });
    }
    Ok(ByteSpan::new(start, end))
}

/// Mask signature and entropy candidates before inference, including overlapping ranges.
pub(crate) fn mask(
    scanner: &impl Scanner,
    input: &[u8],
    capacity: usize,
    entropy: EntropyCfg,
) -> Result<String, VerifierError> {
    let mut ranges = Vec::new();
    let mut buffer = ScanBuf::with_capacity(capacity);
    for category in [Category::Secret, Category::Pii] {
        scanner.scan(category, input, &mut buffer)?;
        for found in buffer.matches() {
            ranges.push(checked_span(found.start, found.end)?);
        }
    }
    let _count = high_entropy_spans(input, &entropy, &mut buffer)?;
    for found in buffer.matches() {
        ranges.push(checked_span(found.start, found.end)?);
    }
    ranges.sort_unstable();
    let mut output = Vec::with_capacity(input.len());
    let mut position = 0usize;
    for span in ranges {
        if span.end > input.len() {
            return Err(VerifierError::InvalidInput {
                reason: "scanner span outside input",
            });
        }
        if span.end <= position {
            continue;
        }
        if span.start > position {
            let prefix = input
                .get(position..span.start)
                .ok_or(VerifierError::InvalidInput {
                    reason: "invalid scanner prefix",
                })?;
            output.extend_from_slice(prefix);
        }
        output.extend_from_slice(b"[REDACTED]");
        position = span.end;
    }
    let suffix = input.get(position..).ok_or(VerifierError::InvalidInput {
        reason: "invalid scanner suffix",
    })?;
    output.extend_from_slice(suffix);
    String::from_utf8(output).map_err(|_| VerifierError::InvalidInput {
        reason: "inference source must be UTF-8",
    })
}
