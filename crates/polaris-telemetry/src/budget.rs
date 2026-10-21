//! `$budget` warnings. Warn-only: enforcement (refusing a request) is the kernel's job.

use polaris_sdk::ids::MicroUsd;
use serde::{Deserialize, Serialize};

use crate::error::TelemetryError;
use crate::sat::percent_floor;

/// How close a limit is. Ordered from mild to severe: `Ok < Warn < Exceeded`; two warnings
/// order by their percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BudgetStatus {
    /// Below the warn threshold, or no limit set.
    Ok,
    /// At or above the warn threshold but below the limit.
    Warn {
        /// `floor(100 * used / limit)`, always below 100 here.
        used_pct: u8,
    },
    /// At or above the limit (a limit of zero is exceeded by everything).
    Exceeded,
}

/// Status of both limits of a [`Budget`], checked independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetReport {
    /// Status of the money limit.
    pub usd: BudgetStatus,
    /// Status of the token limit.
    pub tokens: BudgetStatus,
}

impl BudgetReport {
    /// The more severe of the two statuses.
    #[must_use]
    pub fn worst(&self) -> BudgetStatus {
        self.usd.max(self.tokens)
    }
}

/// Optional money and token limits with a warn threshold (`$budget(usd/tokens)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    usd: Option<MicroUsd>,
    tokens: Option<u64>,
    warn_pct: u8,
}

impl Budget {
    /// Default warn threshold in percent of the limit.
    pub const DEFAULT_WARN_PCT: u8 = 80;

    /// A budget with the default warn threshold; `None` means unlimited.
    #[must_use]
    pub const fn new(usd: Option<MicroUsd>, tokens: Option<u64>) -> Self {
        Self {
            usd,
            tokens,
            warn_pct: Self::DEFAULT_WARN_PCT,
        }
    }

    /// The same budget with another warn threshold.
    ///
    /// # Errors
    /// [`TelemetryError::InvalidWarnPercent`] unless `1 <= warn_pct <= 100` (100 disables the
    /// warning: usage goes straight from `Ok` to `Exceeded`).
    pub fn with_warn_pct(self, warn_pct: u8) -> Result<Self, TelemetryError> {
        if warn_pct == 0 || warn_pct > 100 {
            return Err(TelemetryError::InvalidWarnPercent(warn_pct));
        }
        Ok(Self { warn_pct, ..self })
    }

    /// The money limit, if any.
    #[must_use]
    pub const fn usd_limit(&self) -> Option<MicroUsd> {
        self.usd
    }

    /// The token limit (prompt plus completion), if any.
    #[must_use]
    pub const fn token_limit(&self) -> Option<u64> {
        self.tokens
    }

    /// The warn threshold in percent.
    #[must_use]
    pub const fn warn_pct(&self) -> u8 {
        self.warn_pct
    }

    /// Evaluates the given usage against the limits.
    #[must_use]
    pub fn check(&self, usd_used: MicroUsd, tokens_used: u64) -> BudgetReport {
        BudgetReport {
            usd: self.status(usd_used.get(), self.usd.map(MicroUsd::get)),
            tokens: self.status(tokens_used, self.tokens),
        }
    }

    fn status(&self, used: u64, limit: Option<u64>) -> BudgetStatus {
        let Some(limit) = limit else {
            return BudgetStatus::Ok;
        };
        if used >= limit {
            return BudgetStatus::Exceeded;
        }
        let warn_at = u128::from(limit).saturating_mul(u128::from(self.warn_pct));
        if u128::from(used).saturating_mul(100) < warn_at {
            return BudgetStatus::Ok;
        }
        let pct = percent_floor(used, limit).unwrap_or(99);
        BudgetStatus::Warn {
            used_pct: u8::try_from(pct).unwrap_or(99),
        }
    }
}
