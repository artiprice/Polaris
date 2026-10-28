//! Bounded command auditing through category-specific DFA metadata.

use crate::audit::push_finding;
use crate::error::RulesError;
use crate::model::Finding;
use crate::pack::RuleEngine;
use polaris_code_intel::ByteSpan;
use polaris_security_automata::{Category, ScanBuf};
use std::path::Path;

const MAX_COMMAND_BYTES: usize = 64 * 1024;
const MAX_COMMAND_FINDINGS: usize = 256;

impl RuleEngine {
    /// Scans one bounded command string for dangerous shell operations and secret-shaped values.
    ///
    /// DFA rule categories are preserved independently. Findings retain only the matched span's
    /// BLAKE3 content id; raw command or secret text is never stored in the result.
    ///
    /// # Errors
    /// Returns a typed error when the command exceeds its size limit or DFA scanning fails.
    pub fn audit_command(&self, command: &str) -> Result<Vec<Finding>, RulesError> {
        if command.len() > MAX_COMMAND_BYTES {
            return Err(RulesError::Bound(format!(
                "command exceeds {MAX_COMMAND_BYTES} bytes"
            )));
        }
        let Some(scanner) = &self.scanner else {
            return Ok(Vec::new());
        };
        let mut buffer = ScanBuf::with_capacity(MAX_COMMAND_FINDINGS);
        scanner
            .scan(command.as_bytes(), &mut buffer)
            .map_err(|error| RulesError::Dfa(error.to_string()))?;
        let virtual_path = Path::new("<command>");
        let mut findings = Vec::with_capacity(buffer.len());
        for matched in buffer.matches() {
            let category = match scanner.rule_category(matched.rule) {
                Some(Category::Secret) => crate::model::FindingCategory::Secret,
                Some(Category::DangerousShell) => crate::model::FindingCategory::DangerousShell,
                Some(Category::Pii) | None => continue,
            };
            let name = scanner.rule_name(matched.rule).ok_or_else(|| {
                RulesError::Dfa(String::from("scanner returned an unknown rule index"))
            })?;
            let rule = self
                .dfa_rules
                .iter()
                .find(|rule| rule.id().as_str() == name)
                .ok_or_else(|| RulesError::Dfa(String::from("scanner rule metadata missing")))?;
            let start = usize::try_from(matched.start)
                .map_err(|_| RulesError::Bound(String::from("match offset overflow")))?;
            let end = usize::try_from(matched.end)
                .map_err(|_| RulesError::Bound(String::from("match offset overflow")))?;
            push_finding(
                &mut findings,
                rule,
                virtual_path,
                ByteSpan::new(start, end),
                command.as_bytes(),
                category,
            )?;
        }
        Ok(findings)
    }
}
