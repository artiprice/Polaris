//! CUDA occupancy estimate over validated, provenance-carrying per-SM specifications.
use crate::MathError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Compute capability identifier.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct ComputeCapability {
    /// major version
    pub major: u8,
    /// minor version
    pub minor: u8,
}
/// Validated per-SM capacity and source row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuSpec {
    /// Compute capability key.
    pub capability: ComputeCapability,
    /// Max resident blocks per SM.
    pub max_blocks_per_sm: u32,
    /// Max resident threads per SM.
    pub max_threads_per_sm: u32,
    /// Max threads in one block.
    pub max_threads_per_block: u32,
    /// Max resident warps per SM.
    pub max_warps_per_sm: u32,
    /// 32-bit registers per SM.
    pub registers_per_sm: u64,
    /// Shared-memory bytes per SM.
    pub shared_bytes_per_sm: u64,
    /// Maximum shared-memory bytes per block.
    pub max_shared_bytes_per_block: u64,
    /// Source URL for this exact compute-capability row.
    pub source_url: String,
    /// Retrieval date as `YYYY-MM-DD`.
    pub retrieved: String,
}
#[derive(Debug, Deserialize)]
struct GpuSpecFile {
    version: u32,
    specs: Vec<GpuSpec>,
}
/// Parsed GPU specifications keyed by compute capability.
#[derive(Debug, Clone)]
pub struct GpuSpecTable {
    version: u32,
    rows: BTreeMap<ComputeCapability, GpuSpec>,
}
impl GpuSpecTable {
    /// Loads the bundled NVIDIA compute-capability table. Each row contains a source URL and
    /// retrieval date; the returned occupancy result remains a first-order estimate.
    pub fn nvidia_reference() -> Result<Self, MathError> {
        Self::from_toml(include_str!("gpu_specs.toml"))
    }

    /// Parses version-1 TOML and rejects duplicate, impossible, or unprovenanced rows.
    pub fn from_toml(source: &str) -> Result<Self, MathError> {
        if source.len() > 1_000_000 {
            return Err(MathError::TooLong);
        }
        let file: GpuSpecFile = toml::from_str(source).map_err(|_| MathError::Parse { at: 0 })?;
        if file.version != 1 {
            return Err(MathError::UnsupportedTableVersion(file.version));
        }
        if file.specs.is_empty() || file.specs.len() > 128 {
            return Err(MathError::TooLong);
        }
        let mut rows = BTreeMap::new();
        for row in file.specs {
            if !valid_source_url(&row.source_url) || !valid_iso_date(&row.retrieved) {
                return Err(MathError::MissingSource(format!(
                    "{}.{}",
                    row.capability.major, row.capability.minor
                )));
            }
            if row.max_blocks_per_sm == 0
                || row.max_threads_per_sm == 0
                || row.max_threads_per_block == 0
                || row.max_threads_per_block > row.max_threads_per_sm
                || row.max_threads_per_block > 1024
                || row.max_warps_per_sm == 0
                || row.max_warps_per_sm > row.max_threads_per_sm / 32
                || row.registers_per_sm == 0
                || row.shared_bytes_per_sm == 0
                || row.max_shared_bytes_per_block == 0
                || row.max_shared_bytes_per_block > row.shared_bytes_per_sm
            {
                return Err(MathError::InvalidLaunch(
                    "GPU row has zero or inconsistent resource limits",
                ));
            }
            let key = row.capability;
            if rows.insert(key, row).is_some() {
                return Err(MathError::InvalidLaunch("duplicate compute capability row"));
            }
        }
        Ok(Self {
            version: file.version,
            rows,
        })
    }
    /// Table schema version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
    /// Returns the provenance-bearing row for a capability.
    #[must_use]
    pub fn spec(&self, cc: ComputeCapability) -> Option<&GpuSpec> {
        self.rows.get(&cc)
    }
}

fn valid_source_url(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|host_and_path| {
        !host_and_path.is_empty() && !host_and_path.chars().any(char::is_whitespace)
    })
}

fn valid_iso_date(date: &str) -> bool {
    let Some((year_text, remainder)) = date.split_once('-') else {
        return false;
    };
    let Some((month_text, day_text)) = remainder.split_once('-') else {
        return false;
    };
    if year_text.len() != 4
        || month_text.len() != 2
        || day_text.len() != 2
        || !year_text
            .chars()
            .chain(month_text.chars())
            .chain(day_text.chars())
            .all(|character| character.is_ascii_digit())
    {
        return false;
    }
    let Ok(year) = year_text.parse::<u32>() else {
        return false;
    };
    let Ok(month) = month_text.parse::<u32>() else {
        return false;
    };
    let Ok(day) = day_text.parse::<u32>() else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    day > 0 && day <= max_day
}
/// Block resources supplied by a kernel launch estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LaunchConfig {
    /// Threads per block.
    pub threads_per_block: u32,
    /// 32-bit registers per thread.
    pub regs_per_thread: u32,
    /// Shared-memory bytes allocated per block.
    pub shared_bytes_per_block: u32,
}
/// Resource that limited resident blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Limiter {
    Threads,
    Registers,
    SharedMemory,
    BlockSlots,
}
/// Conservative resource-bound occupancy estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Occupancy {
    /// Resident blocks per SM.
    pub blocks_per_sm: u32,
    /// Resident warps per SM.
    pub warps_per_sm: u32,
    /// Warps/max-warp capacity, in thousandths.
    pub fraction_milli: u16,
    /// Tightest calculated resource bound; ties use first listed below.
    pub limiter: Limiter,
}

/// Computes a first-order occupancy estimate. Register allocation granularity, compiler spills,
/// carveout choices, clusters, and architectural subpartitioning are not modeled.
pub fn occupancy(
    table: &GpuSpecTable,
    cc: ComputeCapability,
    launch: &LaunchConfig,
) -> Result<Occupancy, MathError> {
    let spec = table.spec(cc).ok_or(MathError::UnknownCapability {
        major: cc.major,
        minor: cc.minor,
    })?;
    if launch.threads_per_block == 0
        || launch.threads_per_block > spec.max_threads_per_block
        || launch.threads_per_block > 1024
    {
        return Err(MathError::InvalidLaunch(
            "threads per block outside supported range",
        ));
    }
    if launch.regs_per_thread == 0 || launch.regs_per_thread > 255 {
        return Err(MathError::InvalidLaunch(
            "registers per thread must be 1..=255",
        ));
    }
    let warps_per_block = launch
        .threads_per_block
        .checked_add(31)
        .ok_or(MathError::Overflow)?
        / 32;
    let register_use = u64::from(launch.regs_per_thread)
        .checked_mul(u64::from(launch.threads_per_block))
        .ok_or(MathError::Overflow)?;
    let limits = [
        (spec.max_blocks_per_sm, Limiter::BlockSlots),
        (
            spec.max_threads_per_sm / launch.threads_per_block,
            Limiter::Threads,
        ),
        (spec.max_warps_per_sm / warps_per_block, Limiter::Threads),
        (
            u32::try_from(spec.registers_per_sm / register_use).map_err(|_| MathError::Overflow)?,
            Limiter::Registers,
        ),
        (
            if launch.shared_bytes_per_block == 0 {
                spec.max_blocks_per_sm
            } else {
                if u64::from(launch.shared_bytes_per_block) > spec.max_shared_bytes_per_block {
                    return Err(MathError::InvalidLaunch(
                        "shared-memory allocation exceeds per-block limit",
                    ));
                }
                u32::try_from(spec.shared_bytes_per_sm / u64::from(launch.shared_bytes_per_block))
                    .map_err(|_| MathError::Overflow)?
            },
            Limiter::SharedMemory,
        ),
    ];
    let (blocks, limiter) = limits
        .into_iter()
        .min_by_key(|(count, _)| *count)
        .ok_or(MathError::InvalidLaunch("no occupancy limits"))?;
    if blocks == 0 {
        return Err(MathError::InvalidLaunch(
            "block cannot fit device resources",
        ));
    }
    let resident_warps = blocks
        .checked_mul(warps_per_block)
        .ok_or(MathError::Overflow)?
        .min(spec.max_warps_per_sm);
    let fraction = resident_warps
        .checked_mul(1000)
        .ok_or(MathError::Overflow)?
        / spec.max_warps_per_sm;
    Ok(Occupancy {
        blocks_per_sm: blocks,
        warps_per_sm: resident_warps,
        fraction_milli: u16::try_from(fraction).map_err(|_| MathError::Overflow)?,
        limiter,
    })
}
