//! Trusted request facts and default-deny bounded Datalog evaluation.
use crate::{
    CapabilityError, Clock, PublicKeyBytes, ResourcePath, Right, Token, VOCABULARY_VERSION,
    evaluation,
};
use biscuit_auth::{
    AuthorizerLimits,
    builder::{AuthorizerBuilder, boolean, bytes, fact, int, string},
    error::{FailedCheck, Logic},
};
use polaris_sdk::ids::Cid;
use std::time::Duration;

/// Facts about the actual operation; never claims taken from a token or unverified agent.
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    /// Resolved workspace-contained mutation target expressed as a lexical identity.
    pub resource: &'a ResourcePath,
    /// Qualified AST symbol when trusted analysis identifies one exact symbol.
    pub symbol: Option<&'a str>,
    /// Observed line churn, never the allowed limit.
    pub churn_lines: u32,
    /// Observed unsafe change flag from trusted analysis.
    pub touches_unsafe: bool,
    /// Hash of the exact proposed mutation from trusted content hashing.
    pub diff_blake3: Option<Cid>,
    /// Requested operation at the dispatch boundary.
    pub right: Right,
}
/// First failed Datalog check location without sensitive rule text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckId {
    /// Token block or None for the authorizer.
    pub block: Option<u32>,
    /// Check ordinal within that block.
    pub check: u32,
}
/// Authorization only; metric/semantic verification and actual execution are separate boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authorization {
    /// Every check and the authority-grant policy passed.
    Allowed,
    /// Failed check, no matching policy, or externally revoked signature. No witness is issued.
    Denied {
        /// First failed check when supplied by the evaluator.
        failed_check: Option<CheckId>,
    },
}
/// Authorize without an external revocation list. Never grants by default.
///
/// # Errors
/// Wrong issuer, evaluation failure or exhausted budget. Callers must fail closed.
pub fn authorize(
    token: &Token,
    facts: &Facts<'_>,
    root: &PublicKeyBytes,
    clock: &impl Clock,
) -> Result<Authorization, CapabilityError> {
    authorize_with_revocations(token, facts, root, clock, &[])
}
/// Authorize with explicit external revocation identifiers; any chain match denies.
/// Datalog is limited to 4096 facts, 64 iterations and 50 ms. Parsing/signatures are separate in
/// `Token::from_bytes`, bounded by token size and block count. No unconditional allow policy.
///
/// # Errors
/// Wrong issuer, evaluation failure or exhausted budget. Callers must fail closed.
pub fn authorize_with_revocations(
    token: &Token,
    facts: &Facts<'_>,
    root: &PublicKeyBytes,
    clock: &impl Clock,
    revoked: &[Vec<u8>],
) -> Result<Authorization, CapabilityError> {
    if token.root != *root {
        return Err(CapabilityError::BadSignature);
    }
    if !revoked.is_empty()
        && token
            .biscuit
            .revocation_identifiers()
            .iter()
            .any(|id| revoked.contains(id))
    {
        return Ok(Authorization::Denied { failed_check: None });
    }
    let mut builder = AuthorizerBuilder::new()
        .set_limits(AuthorizerLimits {
            max_facts: 4096,
            max_iterations: 64,
            max_time: Duration::from_millis(50),
        })
        .fact(fact("resource", &[string(facts.resource.as_str())]))
        .map_err(evaluation)?
        .fact(fact("requested_right", &[string(facts.right.name())]))
        .map_err(evaluation)?
        .fact(fact("churn", &[int(i64::from(facts.churn_lines))]))
        .map_err(evaluation)?
        .fact(fact("time", &[int(clock.now().as_microsecond())]))
        .map_err(evaluation)?
        .fact(fact("touches_unsafe", &[boolean(facts.touches_unsafe)]))
        .map_err(evaluation)?;
    if let Some(hash) = facts.diff_blake3 {
        builder = builder
            .fact(fact("diff_blake3", &[bytes(hash.as_bytes())]))
            .map_err(evaluation)?;
    }
    if let Some(symbol) = facts.symbol {
        builder = builder
            .fact(fact("symbol", &[string(symbol)]))
            .map_err(evaluation)?;
    }
    let mut authorizer = builder.code_with_params(
        "allow if polaris_capability_version({version}), requested_right($r), granted_right($r) trusting authority;",
        std::collections::HashMap::from([("version".to_owned(), int(VOCABULARY_VERSION))]),
        std::collections::HashMap::new(),
    ).map_err(evaluation)?.build(&token.biscuit).map_err(evaluation)?;
    match authorizer.authorize() {
        Ok(_) => Ok(Authorization::Allowed),
        Err(biscuit_auth::error::Token::FailedLogic(
            Logic::Unauthorized { checks, .. } | Logic::NoMatchingPolicy { checks },
        )) => {
            let failed_check = checks.first().map(|check| match check {
                FailedCheck::Block(check) => CheckId {
                    block: Some(check.block_id),
                    check: check.check_id,
                },
                FailedCheck::Authorizer(check) => CheckId {
                    block: None,
                    check: check.check_id,
                },
            });
            Ok(Authorization::Denied { failed_check })
        }
        Err(error) => Err(evaluation(error)),
    }
}
