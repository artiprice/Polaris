//! Explicit measured calibration artifacts. Never mutates builtin thresholds or promotes policy.
use polaris_sdk::ids::Cid;
use polaris_system_one_schemas::calibration::{
    CalibrationDataset, CalibrationLabel, CalibrationPrediction, CalibrationReport,
    MAX_CALIBRATION_SAMPLES, PrecisionRequirement,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
const MAX_JSONL_BYTES: u64 = 32 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 4096;
const MAX_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024 * 1024;
/// Inputs to a deterministic, create-new calibration artifact run.
#[derive(Debug, Clone)]
pub struct CalibrationRunConfig {
    /// Labelled JSONL. IDs and positive truth; provenance belongs with this immutable input.
    pub fixtures: PathBuf,
    /// Measured JSONL scores with exact matching IDs and integer positive probabilities.
    pub predictions: PathBuf,
    /// Actual checkpoint file to hash, never a model-supplied claimed digest.
    pub checkpoint: PathBuf,
    /// New TOML artifact; existing output is always refused.
    pub output: PathBuf,
    /// Exact measured schema identity.
    pub schema: String,
    /// Exact measured schema version.
    pub version: u32,
    /// Measured precision requirement in integer parts per million.
    pub minimum_precision_ppm: u32,
    /// Minimum number of positive predictions at selected threshold.
    pub minimum_positive: u32,
}

/// Parse the strict calibration command options and execute a measured candidate run.
/// # Errors
/// Unknown, duplicate or missing options, malformed bounds, or the underlying run failure.
pub fn run_cli(args: impl Iterator<Item = String>) -> Result<(), CalibrationRunError> {
    use std::collections::BTreeMap;
    let mut values = BTreeMap::new();
    let mut args = args;
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--fixtures"
                | "--predictions"
                | "--checkpoint"
                | "--output"
                | "--schema"
                | "--version"
                | "--minimum-precision-ppm"
                | "--minimum-positive"
        ) {
            return Err(CalibrationRunError::Input);
        }
        let value = args.next().ok_or(CalibrationRunError::Input)?;
        if value.len() > 4096 || values.insert(flag, value).is_some() {
            return Err(CalibrationRunError::Input);
        }
    }
    let get = |key: &str| values.get(key).cloned().ok_or(CalibrationRunError::Input);
    let number = |key: &str| -> Result<u32, CalibrationRunError> {
        get(key)?.parse().map_err(|_| CalibrationRunError::Input)
    };
    create_binary_calibration(&CalibrationRunConfig {
        fixtures: PathBuf::from(get("--fixtures")?),
        predictions: PathBuf::from(get("--predictions")?),
        checkpoint: PathBuf::from(get("--checkpoint")?),
        output: PathBuf::from(get("--output")?),
        schema: get("--schema")?,
        version: number("--version")?,
        minimum_precision_ppm: number("--minimum-precision-ppm")?,
        minimum_positive: number("--minimum-positive")?,
    })
}
/// Failure in calibration input, measurement or durable artifact creation.
#[derive(Debug, thiserror::Error)]
pub enum CalibrationRunError {
    /// Bounded file I/O failed. Input bytes and payloads are not included.
    #[error("calibration file operation failed")]
    Io(#[from] std::io::Error),
    /// JSONL, shape, provenance source or size violated the contract.
    #[error("invalid calibration source format or bound")]
    Input,
    /// No threshold met the required measured support and precision.
    #[error("no threshold meets measured precision and support")]
    UnmetPrecision,
    /// Exact joined measurement failed.
    #[error(transparent)]
    Measurement(#[from] polaris_system_one_schemas::calibration::CalibrationError),
    /// Measured report could not be serialized.
    #[error("calibration artifact encoding failed")]
    Encoding,
}
#[derive(Serialize)]
struct Artifact {
    format_version: u32,
    status: &'static str,
    score_encoding: &'static str,
    fixtures_blake3: String,
    predictions_blake3: String,
    checkpoint_blake3: String,
    minimum_precision_ppm: u32,
    minimum_positive: u32,
    report: CalibrationReport,
}
/// Join every case, choose a supported threshold and write an auditable measured candidate.
///
/// Candidate status is intentional: label independence/licensing, representativeness, holdout
/// leakage and original-response quantization are review obligations, not inferred from filenames.
/// Input hashing binds the exact bytes consumed. Output is written and synced through create-new;
/// write/sync failure removes only this run's newly created partial artifact when possible.
/// # Errors
/// Malformed/incomplete/oversized input, unmet precision, changed checkpoint metadata, or I/O failure.
pub fn create_binary_calibration(config: &CalibrationRunConfig) -> Result<(), CalibrationRunError> {
    let fixtures = read_jsonl_bytes(&config.fixtures)?;
    let predictions = read_jsonl_bytes(&config.predictions)?;
    let labels: Vec<CalibrationLabel> = parse_jsonl(&fixtures)?;
    let outputs: Vec<CalibrationPrediction> = parse_jsonl(&predictions)?;
    let fixture = Cid::from_bytes(*blake3::hash(&fixtures).as_bytes());
    let checkpoint = hash_checkpoint(&config.checkpoint)?;
    let data = CalibrationDataset::join(
        &config.schema,
        config.version,
        fixture,
        checkpoint,
        &labels,
        &outputs,
    )?;
    let report = data
        .select_threshold(PrecisionRequirement {
            minimum_ppm: config.minimum_precision_ppm,
            minimum_predicted_positive: config.minimum_positive,
        })?
        .ok_or(CalibrationRunError::UnmetPrecision)?;
    let artifact = Artifact {
        format_version: 1,
        status: "measured_candidate",
        score_encoding: "positive_class_probability_parts_per_million",
        fixtures_blake3: fixture.to_hex(),
        predictions_blake3: blake3::hash(&predictions).to_hex().to_string(),
        checkpoint_blake3: checkpoint.to_hex(),
        minimum_precision_ppm: config.minimum_precision_ppm,
        minimum_positive: config.minimum_positive,
        report,
    };
    let encoded = toml::to_string_pretty(&artifact).map_err(|_| CalibrationRunError::Encoding)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&config.output)?;
    let written = file
        .write_all(encoded.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(error) = written {
        drop(file);
        if let Err(cleanup) = std::fs::remove_file(&config.output) {
            return Err(CalibrationRunError::Io(cleanup));
        }
        return Err(CalibrationRunError::Io(error));
    }
    Ok(())
}
fn read_jsonl_bytes(path: &Path) -> Result<Vec<u8>, CalibrationRunError> {
    let mut file = File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_JSONL_BYTES {
        return Err(CalibrationRunError::Input);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_JSONL_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_err(|_| CalibrationRunError::Input)? > MAX_JSONL_BYTES {
        return Err(CalibrationRunError::Input);
    }
    Ok(bytes)
}
fn parse_jsonl<T: DeserializeOwned>(bytes: &[u8]) -> Result<Vec<T>, CalibrationRunError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CalibrationRunError::Input)?;
    let mut rows = Vec::new();
    for line in text.lines() {
        if rows.len() >= MAX_CALIBRATION_SAMPLES || line.is_empty() || line.len() > MAX_LINE_BYTES {
            return Err(CalibrationRunError::Input);
        }
        rows.push(serde_json::from_str(line).map_err(|_| CalibrationRunError::Input)?);
    }
    if rows.is_empty() {
        return Err(CalibrationRunError::Input);
    }
    Ok(rows)
}
fn hash_checkpoint(path: &Path) -> Result<Cid, CalibrationRunError> {
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() == 0 || before.len() > MAX_CHECKPOINT_BYTES {
        return Err(CalibrationRunError::Input);
    }
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 65536];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(count).map_err(|_| CalibrationRunError::Input)?)
            .ok_or(CalibrationRunError::Input)?;
        if total > MAX_CHECKPOINT_BYTES {
            return Err(CalibrationRunError::Input);
        }
        hasher.update(buffer.get(..count).ok_or(CalibrationRunError::Input)?);
    }
    let after = file.metadata()?;
    if total != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(CalibrationRunError::Input);
    }
    Ok(Cid::from_bytes(*hasher.finalize().as_bytes()))
}
