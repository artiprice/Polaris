//! Typed resources and checks restricted to authority/current block plus authorizer origins.
use crate::{CapabilityError, Clock};
use biscuit_auth::builder::{BlockBuilder, Term, bytes, int, string};
use polaris_sdk::ids::Cid;
use std::{collections::HashMap, time::Duration};

/// Relative resource identity; `/` separators and ASCII case folding on Windows.
/// Non-ASCII characters retain exact identity. No filesystem/symlink resolution occurs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourcePath(String);
impl ResourcePath {
    /// Normalize separators and reject ambiguous, traversing, absolute or device paths.
    ///
    /// # Errors
    /// `InvalidScope` for invalid/over-4096-byte paths, percent encodings or control characters.
    pub fn new(path: &str) -> Result<Self, CapabilityError> {
        if path.is_empty()
            || path.len() > 4096
            || path.chars().any(char::is_control)
            || path.contains([':', '%', '?', '*', '"', '<', '>', '|'])
        {
            return Err(CapabilityError::InvalidScope);
        }
        let normalized = path.replace('\\', "/");
        for component in normalized.split('/') {
            if component.is_empty()
                || matches!(component, "." | "..")
                || component.ends_with([' ', '.'])
            {
                return Err(CapabilityError::InvalidScope);
            }
            let stem = component
                .split('.')
                .next()
                .unwrap_or_default()
                .trim_end_matches(' ')
                .to_ascii_uppercase();
            let device = matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            ) || ["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|tail| {
                    matches!(
                        tail,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            });
            if device {
                return Err(CapabilityError::InvalidScope);
            }
        }
        Ok(Self(if cfg!(windows) {
            normalized.to_ascii_lowercase()
        } else {
            normalized
        }))
    }
    /// Canonical lexical identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Workspace, path, or exact AST-symbol scope; symbol identity comes from trusted code analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceScope {
    /// Every lexical resource supplied at an authorization boundary.
    ///
    /// This deliberately broad scope is for trusted callers that explicitly need workspace-wide
    /// access. It still enforces rights, time, churn, unsafe-change, and exact-diff constraints.
    Workspace,
    /// Exactly one relative path.
    Exact(ResourcePath),
    /// A directory and component-bounded descendants.
    Prefix(ResourcePath),
    /// One qualified symbol within exactly one path.
    Symbol {
        /// Containing path.
        path: ResourcePath,
        /// Exact qualified symbol.
        symbol: String,
    },
}
/// Independently grantable operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Right {
    /// Read workspace content.
    Read,
    /// Submit a mutation to later metric/semantic verification.
    ProposeDiff,
    /// Execute through a separate sandbox/lease boundary.
    ExecuteTool,
    /// Delegate through the runtime frame boundary.
    SpawnSubagent,
    /// Cancel one exact runtime frame and its registered descendants.
    CancelFrame,
}
impl Right {
    /// Every supported right in stable vocabulary order.
    pub const ALL: [Self; 5] = [
        Self::Read,
        Self::ProposeDiff,
        Self::ExecuteTool,
        Self::SpawnSubagent,
        Self::CancelFrame,
    ];
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::ProposeDiff => "propose_diff",
            Self::ExecuteTool => "execute_tool",
            Self::SpawnSubagent => "spawn_subagent",
            Self::CancelFrame => "cancel_frame",
        }
    }
}
/// Fixed-size right set; unknown/reserved bits cannot be introduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rights([bool; 5]);
impl Rights {
    /// Construct an explicit set; duplicates have no effect.
    #[must_use]
    pub fn new(rights: &[Right]) -> Self {
        Self(Right::ALL.map(|right| rights.contains(&right)))
    }
    /// Whether a right belongs to the set.
    #[must_use]
    pub fn contains(self, right: Right) -> bool {
        Right::ALL
            .into_iter()
            .zip(self.0)
            .any(|(item, enabled)| enabled && item == right)
    }
}
/// One signed constraint set. Attenuation intersects it with all ancestor checks; wider requests
/// cannot erase restrictions. The parent bearer delegation key suffices to add constraints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// Resource permitted by this block.
    pub resource: ResourceScope,
    /// Inclusive actual churn limit.
    pub max_churn_lines: u32,
    /// Lifetime starting now, rounded down to whole microseconds.
    pub ttl: Duration,
    /// Whether this block permits unsafe changes; false ancestors remain effective.
    pub allow_unsafe: bool,
    /// Optional exact-patch binding, never removable by a child.
    pub diff_blake3: Option<Cid>,
    /// Permitted operations.
    pub rights: Rights,
}
pub(crate) fn expiry(issued: i64, lifetime_micros: u128) -> Result<i64, CapabilityError> {
    let lifetime = i64::try_from(lifetime_micros).map_err(|_| CapabilityError::Clock)?;
    if lifetime == 0 {
        return Err(CapabilityError::InvalidScope);
    }
    issued.checked_add(lifetime).ok_or(CapabilityError::Clock)
}
pub(crate) fn checks(scope: &Scope, clock: &impl Clock) -> Result<BlockBuilder, CapabilityError> {
    let issued = clock.now().as_microsecond();
    let expiry = expiry(issued, scope.ttl.as_micros())?;
    let mut params: HashMap<String, Term> = HashMap::from([
        ("churn".to_owned(), int(i64::from(scope.max_churn_lines))),
        ("issued".to_owned(), int(issued)),
        ("expiry".to_owned(), int(expiry)),
    ]);
    let mut code = match &scope.resource {
        ResourceScope::Workspace => "check if resource($resource) trusting authority;\n".to_owned(),
        ResourceScope::Exact(path) => {
            let _previous = params.insert("path".to_owned(), string(path.as_str()));
            "check if resource({path}) trusting authority;\n".to_owned()
        }
        ResourceScope::Prefix(path) => {
            let _previous = params.insert("path".to_owned(), string(path.as_str()));
            let _previous =
                params.insert("prefix".to_owned(), string(&format!("{}/", path.as_str())));
            "check if resource({path}) trusting authority or resource($p), $p.starts_with({prefix}) trusting authority;\n".to_owned()
        }
        ResourceScope::Symbol { path, symbol } => {
            if symbol.trim().is_empty()
                || symbol.len() > 1024
                || symbol.chars().any(char::is_control)
            {
                return Err(CapabilityError::InvalidScope);
            }
            let _previous = params.insert("path".to_owned(), string(path.as_str()));
            let _previous = params.insert("symbol".to_owned(), string(symbol));
            "check if resource({path}) trusting authority;\ncheck if symbol({symbol}) trusting authority;\n".to_owned()
        }
    };
    code.push_str("check if churn($c), $c <= {churn} trusting authority;\ncheck if time($t), $t >= {issued}, $t < {expiry} trusting authority;\n");
    if !scope.allow_unsafe {
        code.push_str("check if touches_unsafe(false) trusting authority;\n");
    }
    if let Some(hash) = scope.diff_blake3 {
        let _previous = params.insert("hash".to_owned(), bytes(hash.as_bytes()));
        code.push_str("check if diff_blake3({hash}) trusting authority;\n");
    }
    let choices = Right::ALL
        .into_iter()
        .filter(|right| scope.rights.contains(*right))
        .map(|right| format!("requested_right(\"{}\") trusting authority", right.name()))
        .collect::<Vec<_>>();
    if choices.is_empty() {
        return Err(CapabilityError::InvalidScope);
    }
    code.push_str(&format!("check if {};", choices.join(" or ")));
    BlockBuilder::new()
        .code_with_params(code, params, HashMap::new())
        .map_err(crate::evaluation)
}
