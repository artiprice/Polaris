//! Strict parser and reference-cap checks for Linux `/proc/meminfo` evidence.

const SIX_GIB_KIB: u64 = 6 * 1024 * 1024;
const REFERENCE_TOLERANCE_KIB: u64 = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryInfo {
    pub total_kib: u64,
    pub available_kib: u64,
    pub swap_total_kib: u64,
    pub swap_free_kib: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MemInfoError {
    #[error("meminfo line is malformed")]
    MalformedLine,
    #[error("meminfo field {0} is duplicated")]
    Duplicate(&'static str),
    #[error("meminfo field {0} is missing")]
    Missing(&'static str),
    #[error("meminfo field {0} must contain a nonnegative integer and kB unit")]
    InvalidValue(&'static str),
    #[error("meminfo available bytes exceed total memory")]
    AvailableExceedsTotal,
    #[error("meminfo free swap exceeds total swap")]
    FreeSwapExceedsTotal,
}

impl MemoryInfo {
    /// Parse the required `/proc/meminfo` fields with exact units and no duplicate keys.
    pub fn parse(input: &str) -> Result<Self, MemInfoError> {
        let mut total_kib = None;
        let mut available_kib = None;
        let mut swap_total_kib = None;
        let mut swap_free_kib = None;
        for line in input.lines() {
            let (key, raw) = line.split_once(':').ok_or(MemInfoError::MalformedLine)?;
            let slot = match key {
                "MemTotal" => &mut total_kib,
                "MemAvailable" => &mut available_kib,
                "SwapTotal" => &mut swap_total_kib,
                "SwapFree" => &mut swap_free_kib,
                _ => continue,
            };
            if slot.is_some() {
                return Err(MemInfoError::Duplicate(match key {
                    "MemTotal" => "MemTotal",
                    "MemAvailable" => "MemAvailable",
                    "SwapTotal" => "SwapTotal",
                    "SwapFree" => "SwapFree",
                    _ => return Err(MemInfoError::MalformedLine),
                }));
            }
            let mut values = raw.split_whitespace();
            let number = values
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or(MemInfoError::InvalidValue(field_name(key)))?;
            if values.next() != Some("kB") || values.next().is_some() {
                return Err(MemInfoError::InvalidValue(field_name(key)));
            }
            *slot = Some(number);
        }
        let total_kib = total_kib.ok_or(MemInfoError::Missing("MemTotal"))?;
        let available_kib = available_kib.ok_or(MemInfoError::Missing("MemAvailable"))?;
        let swap_total_kib = swap_total_kib.ok_or(MemInfoError::Missing("SwapTotal"))?;
        let swap_free_kib = swap_free_kib.ok_or(MemInfoError::Missing("SwapFree"))?;
        if available_kib > total_kib {
            return Err(MemInfoError::AvailableExceedsTotal);
        }
        if swap_free_kib > swap_total_kib {
            return Err(MemInfoError::FreeSwapExceedsTotal);
        }
        Ok(Self {
            total_kib,
            available_kib,
            swap_total_kib,
            swap_free_kib,
        })
    }

    #[must_use]
    pub const fn used_kib(self) -> u64 {
        self.total_kib - self.available_kib
    }

    #[must_use]
    pub const fn swap_used_kib(self) -> u64 {
        self.swap_total_kib - self.swap_free_kib
    }

    /// The live reference target is six GiB, allowing at most 512 MiB for host kernel reserve.
    #[must_use]
    pub const fn within_six_gib_reference(self) -> bool {
        self.total_kib <= SIX_GIB_KIB && self.total_kib >= SIX_GIB_KIB - REFERENCE_TOLERANCE_KIB
    }
}

fn field_name(key: &str) -> &'static str {
    match key {
        "MemTotal" => "MemTotal",
        "MemAvailable" => "MemAvailable",
        "SwapTotal" => "SwapTotal",
        "SwapFree" => "SwapFree",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "MemTotal: 6000000 kB\nMemAvailable: 3500000 kB\nSwapTotal: 1000000 kB\nSwapFree: 900000 kB\nHugePages_Total: 0\n";

    #[test]
    fn parses_required_kib_fields_and_computes_usage() -> Result<(), MemInfoError> {
        let info = MemoryInfo::parse(VALID)?;
        assert_eq!(info.total_kib, 6_000_000);
        assert_eq!(info.used_kib(), 2_500_000);
        assert_eq!(info.swap_used_kib(), 100_000);
        assert!(info.within_six_gib_reference());
        Ok(())
    }

    #[test]
    fn rejects_missing_duplicate_bad_unit_and_malformed_fields() {
        assert_eq!(
            MemoryInfo::parse("MemTotal: 6000000 kB\n").err(),
            Some(MemInfoError::Missing("MemAvailable"))
        );
        assert!(matches!(
            MemoryInfo::parse(&format!("{VALID}MemTotal: 6000000 kB\n")),
            Err(MemInfoError::Duplicate("MemTotal"))
        ));
        assert!(matches!(
            MemoryInfo::parse(&VALID.replace("6000000 kB", "6000000 MB")),
            Err(MemInfoError::InvalidValue("MemTotal"))
        ));
        assert!(matches!(
            MemoryInfo::parse(&VALID.replace("3500000 kB", "not-a-number kB")),
            Err(MemInfoError::InvalidValue("MemAvailable"))
        ));
        assert!(matches!(
            MemoryInfo::parse(&format!("{VALID}broken line\n")),
            Err(MemInfoError::MalformedLine)
        ));
        assert!(matches!(
            MemoryInfo::parse(&VALID.replace("6000000 kB", "6000000 kB extra")),
            Err(MemInfoError::InvalidValue("MemTotal"))
        ));
    }

    #[test]
    fn rejects_impossible_available_and_swap_counters() {
        assert!(matches!(
            MemoryInfo::parse(&VALID.replace("3500000 kB", "7000000 kB")),
            Err(MemInfoError::AvailableExceedsTotal)
        ));
        assert!(matches!(
            MemoryInfo::parse(&VALID.replace("900000 kB", "1100000 kB")),
            Err(MemInfoError::FreeSwapExceedsTotal)
        ));
    }

    #[test]
    fn reference_cap_accepts_six_gib_and_kernel_reserve_but_rejects_eight_gib() {
        let mut info = MemoryInfo::parse(VALID).expect("valid meminfo");
        info.total_kib = SIX_GIB_KIB;
        assert!(info.within_six_gib_reference());
        info.total_kib = SIX_GIB_KIB - REFERENCE_TOLERANCE_KIB;
        assert!(info.within_six_gib_reference());
        info.total_kib = SIX_GIB_KIB - REFERENCE_TOLERANCE_KIB - 1;
        assert!(!info.within_six_gib_reference());
        info.total_kib = 8 * 1024 * 1024;
        assert!(!info.within_six_gib_reference());
    }
}
