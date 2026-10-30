//! System One for interactive sessions. Every session kernel starts the local S1 worker that the
//! user's configuration names (`system_one.decide_model_dir` and `system_one.ort_dylib`, with
//! optional `system_one.worker` and `system_one.pii_model_dir`; the configuration loader takes
//! them only from the user's own layers, never from a repository). Change reviews then consult
//! the worker through the calibrated safety catalog `~/.polaris/system-one/safety-schemas.toml`:
//! a change the calibrated model is confident about is decided without asking, everything else
//! still goes to the person.
//!
//! [`calibrate`] writes that catalog: it measures the three mutation safety schemas with the same
//! worker on a labelled fixture file and keeps a schema calibrated only when its measured
//! precision reaches [`PRECISION_PPM`].

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use polaris_kernel::{Kernel, LocalKernelConfig, LocalS1Config, S1Normalization};
use polaris_sdk::ids::{Cid, FrameId, SessionId};
use polaris_sdk::registry::DecisionEngine;
use polaris_sdk::system_one::{Answer, DecisionInput, Question};
use polaris_system_one::schemas::calibration::{
    CalibrationDataset, CalibrationLabel, CalibrationPrediction, PrecisionRequirement,
};
use polaris_system_one::schemas::{CalibrationContext, SchemaSet};
use polaris_system_one::verifier::{
    CalibrationBindings, SafetyScanner, VerifierConfig, mutation_decision_input,
};

use crate::SessionHostError;

/// The mutation safety schemas a change review consults.
pub const SAFETY_SCHEMAS: [&str; 3] = [
    "safety.diff_tamper",
    "safety.secret_semantic",
    "watch.congruence",
];
/// The calibrated catalog, relative to the user's Polaris directory.
pub const CATALOG_FILE: &str = "system-one/safety-schemas.toml";
/// Measured precision a schema must reach to decide without asking (parts per million).
pub const PRECISION_PPM: u32 = 990_000;
/// Decisions a schema must accept at that precision.
const MINIMUM_ACCEPTED: u32 = 10;
/// A calibrated schema's decision deadline: one CPU inference of the fp32 model with headroom.
const DEADLINE_MS: u32 = 5000;
const MAX_CATALOG_BYTES: u64 = 1_048_576;
const MAX_FIXTURE_BYTES: u64 = 64 * 1024 * 1024;

/// The calibrated mutation safety catalog and the identities it binds.
#[derive(Debug, Clone)]
pub struct SafetyCatalog {
    /// Schemas with the calibrated thresholds.
    pub schemas: SchemaSet,
    /// Fixture and checkpoint identities of each calibrated schema.
    pub calibration: CalibrationBindings,
    /// Schemas the catalog calibrates.
    pub calibrated: Vec<String>,
}

#[derive(Debug, Clone)]
struct WorkerPaths {
    worker: PathBuf,
    decide: PathBuf,
    ort: PathBuf,
    pii: Option<PathBuf>,
}

#[derive(Debug)]
struct Setup {
    paths: Option<WorkerPaths>,
    catalog: Option<SafetyCatalog>,
    status: String,
}

static SETUP: OnceLock<Setup> = OnceLock::new();

/// Read the System One settings once per process (frontends call it at startup) and return a
/// one-line status.
pub fn configure(config: &polaris_config::PolarisConfig, global: &Path) -> String {
    SETUP.get_or_init(|| setup(config, global)).status.clone()
}

/// The status line of [`configure`], or why System One is off when it was never configured.
#[must_use]
pub fn status() -> String {
    SETUP.get().map_or_else(
        || String::from("System One is off: the frontend did not configure it"),
        |setup| setup.status.clone(),
    )
}

/// The calibrated safety catalog, when one was loaded.
#[must_use]
pub fn catalog() -> Option<&'static SafetyCatalog> {
    SETUP.get()?.catalog.as_ref()
}

/// Whether sessions start a System One worker.
#[must_use]
pub fn enabled() -> bool {
    SETUP.get().is_some_and(|setup| setup.paths.is_some())
}

fn setup(config: &polaris_config::PolarisConfig, global: &Path) -> Setup {
    let s1 = &config.system_one;
    let (Some(decide), Some(ort)) = (s1.decide_model_dir.clone(), s1.ort_dylib.clone()) else {
        return Setup {
            paths: None,
            catalog: None,
            status: String::from(
                "System One is off: set system_one.decide_model_dir and system_one.ort_dylib in ~/.polaris/config.toml",
            ),
        };
    };
    let worker = s1.worker.clone().or_else(|| {
        std::env::current_exe().ok().map(|exe| {
            exe.with_file_name(format!("polaris-worker{}", std::env::consts::EXE_SUFFIX))
        })
    });
    let Some(worker) = worker.filter(|path| path.is_file()) else {
        return Setup {
            paths: None,
            catalog: None,
            status: String::from(
                "System One is off: polaris-worker was not found next to this program (or set system_one.worker)",
            ),
        };
    };
    if !decide.is_dir() || !ort.is_file() {
        return Setup {
            paths: None,
            catalog: None,
            status: String::from(
                "System One is off: system_one.decide_model_dir or system_one.ort_dylib does not exist",
            ),
        };
    }
    let paths = WorkerPaths {
        worker,
        decide,
        ort,
        pii: s1.pii_model_dir.clone().filter(|path| path.is_dir()),
    };
    let (catalog, status) = match load_catalog(global) {
        Ok(Some(catalog)) if !catalog.calibrated.is_empty() => {
            let status = format!(
                "System One: local worker, calibrated for {}",
                catalog.calibrated.join(", ")
            );
            (Some(catalog), status)
        }
        Ok(Some(catalog)) => (
            Some(catalog),
            String::from(
                "System One: local worker; no schema reached the calibration precision, so every change is reviewed",
            ),
        ),
        Ok(None) => (
            None,
            String::from(
                "System One: local worker, not calibrated yet (run `polaris s1-calibrate`); every change is reviewed",
            ),
        ),
        Err(reason) => (
            None,
            format!(
                "System One: local worker; the calibrated catalog was rejected ({reason}); every change is reviewed"
            ),
        ),
    };
    Setup {
        paths: Some(paths),
        catalog,
        status,
    }
}

/// A fresh worker profile for one session kernel rooted at `state_root`.
#[must_use]
pub fn local_s1(state_root: &Path) -> Option<LocalS1Config> {
    let paths = SETUP.get()?.paths.as_ref()?;
    worker_config(paths, state_root)
}

fn worker_config(paths: &WorkerPaths, state_root: &Path) -> Option<LocalS1Config> {
    let mut local = LocalS1Config::new(
        &paths.worker,
        &paths.decide,
        &paths.ort,
        endpoint(state_root)?,
    );
    local.normalization = S1Normalization::Gliner2Python21;
    local.pii_model_dir = paths.pii.clone();
    // The fp32 model takes seconds to load: keep it resident through a working session.
    local.idle_ttl = Duration::from_secs(1800);
    local.startup_timeout = Duration::from_secs(60);
    local.client_timeout = Duration::from_secs(30);
    Some(local)
}

/// A fresh per-kernel worker endpoint.
#[cfg(windows)]
fn endpoint(_state_root: &Path) -> Option<PathBuf> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(PathBuf::from(format!(
        r"\\.\pipe\polaris-s1-{}-{nonce}",
        std::process::id()
    )))
}

/// A fresh per-kernel worker endpoint in an owner-only run directory.
#[cfg(unix)]
fn endpoint(state_root: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(state_root).ok()?;
    let run = state_root.canonicalize().ok()?.join("run");
    std::fs::create_dir_all(&run).ok()?;
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).ok()?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .subsec_nanos();
    Some(run.join(format!("s1-{}-{nonce}.sock", std::process::id())))
}

/// The catalog at `global/CATALOG_FILE`: `None` when absent.
fn load_catalog(global: &Path) -> Result<Option<SafetyCatalog>, String> {
    let path = global.join(CATALOG_FILE);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(String::from("unreadable")),
    };
    if !metadata.is_file() || metadata.len() > MAX_CATALOG_BYTES {
        return Err(String::from("not a file of at most 1 MiB"));
    }
    let text = std::fs::read_to_string(&path).map_err(|_| String::from("not UTF-8"))?;
    catalog_from_toml(&text).map(Some)
}

fn catalog_from_toml(text: &str) -> Result<SafetyCatalog, String> {
    let schemas = SchemaSet::from_toml(text).map_err(|error| error.to_string())?;
    let mut context = None;
    for id in SAFETY_SCHEMAS {
        let record = &schemas
            .latest(id)
            .ok_or_else(|| format!("{id} is missing"))?
            .calibration;
        if let (Some(fixture), Some(checkpoint)) =
            (&record.fixture_blake3, &record.checkpoint_blake3)
        {
            context = Some(CalibrationContext {
                fixture_blake3: fixture.clone(),
                checkpoint_blake3: checkpoint.clone(),
            });
            break;
        }
    }
    let mut calibration = CalibrationBindings::default();
    let mut calibrated = Vec::new();
    if let Some(context) = context {
        schemas
            .validate_calibration(&context)
            .map_err(|error| error.to_string())?;
        for id in SAFETY_SCHEMAS {
            if schemas
                .latest(id)
                .is_some_and(|schema| schema.is_calibrated_for(&context))
            {
                let _previous = calibration.insert(id.to_owned(), context.clone());
                calibrated.push(id.to_owned());
            }
        }
    }
    Ok(SafetyCatalog {
        schemas,
        calibration,
        calibrated,
    })
}

/// One labelled change of the fixture file.
struct Fixture {
    id: String,
    target: String,
    intent: String,
    before: String,
    after: String,
    labels: BTreeMap<String, bool>,
}

fn read_fixtures(bytes: &[u8]) -> Result<Vec<Fixture>, SessionHostError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| SessionHostError::Unavailable("the fixture file is not UTF-8"))?;
    let mut fixtures = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|_| {
            SessionHostError::Ui(format!("fixture line {} is not JSON", number + 1))
        })?;
        let field = |name: &str| {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    SessionHostError::Ui(format!("fixture line {} lacks {name}", number + 1))
                })
        };
        let labels = value
            .get("labels")
            .and_then(serde_json::Value::as_object)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|(id, truth)| truth.as_bool().map(|truth| (id.clone(), truth)))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        fixtures.push(Fixture {
            id: field("id")?,
            target: field("target")?,
            intent: field("intent")?,
            before: field("before")?,
            after: field("after")?,
            labels,
        });
    }
    Ok(fixtures)
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _hex = write!(out, "{byte:02x}");
            out
        })
}

/// Measure [`SAFETY_SCHEMAS`] on the labelled `fixtures` (JSON lines with `id`, `target`,
/// `intent`, `before`, `after` and `labels: {schema: safe}`) with the configured worker, and write
/// the calibrated catalog to `global/CATALOG_FILE`. `progress` receives a line per schema.
/// # Errors
/// System One is not configured, the fixture file is malformed, or the worker cannot run.
pub async fn calibrate(
    fixtures: &Path,
    workspace: &Path,
    state_dir: &Path,
    global: &Path,
    mut progress: impl FnMut(&str),
) -> Result<Vec<String>, SessionHostError> {
    let setup = SETUP.get().ok_or(SessionHostError::Unavailable(
        "System One is not configured",
    ))?;
    let paths = setup
        .paths
        .as_ref()
        .ok_or_else(|| SessionHostError::Ui(setup.status.clone()))?;
    let size = std::fs::metadata(fixtures)
        .map_err(|_| SessionHostError::Unavailable("the fixture file cannot be read"))?
        .len();
    if size == 0 || size > MAX_FIXTURE_BYTES {
        return Err(SessionHostError::Unavailable(
            "the fixture file is empty or larger than 64 MiB",
        ));
    }
    let bytes = std::fs::read(fixtures)
        .map_err(|_| SessionHostError::Unavailable("the fixture file cannot be read"))?;
    let fixture_digest = *blake3::hash(&bytes).as_bytes();
    let cases = read_fixtures(&bytes)?;

    let root = state_dir.join("s1-calibration");
    std::fs::create_dir_all(&root)
        .map_err(|_| SessionHostError::Unavailable("the calibration state directory"))?;
    let mut config =
        LocalKernelConfig::new(&root, workspace, SessionId::new_v7(), FrameId::new_v7());
    config.s1 = worker_config(paths, &root);
    let saved = state_dir.join("s1-measured");
    std::fs::create_dir_all(&saved)
        .map_err(|_| SessionHostError::Unavailable("the calibration state directory"))?;
    let kernel = Kernel::start(config).await?;
    let outcome = measure(&kernel, &cases, fixture_digest, &saved, &mut progress).await;
    let _report = kernel.shutdown().await;
    let (catalog, mut lines) = outcome?;

    // Never write a catalog the sessions would reject.
    let parsed = catalog_from_toml(&catalog).map_err(|reason| {
        SessionHostError::Ui(format!(
            "{}\ncalibrated catalog rejected: {reason}",
            lines.join("\n")
        ))
    })?;
    let path = global.join(CATALOG_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| SessionHostError::Unavailable("the System One directory"))?;
    }
    let temporary = path.with_extension("toml.tmp");
    std::fs::write(&temporary, catalog.as_bytes())
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|_| SessionHostError::Unavailable("the calibrated catalog cannot be written"))?;
    lines.push(format!(
        "Catalog written to {} ({} of {} schemas calibrated). Restart Polaris to use it.",
        path.display(),
        parsed.calibrated.len(),
        SAFETY_SCHEMAS.len()
    ));
    Ok(lines)
}

/// What the worker answered for one schema: `(case id, label, P(safe) in ppm)`.
type Measured = Vec<(String, bool, u32)>;
/// Decisions in flight at once; the worker batches concurrent requests.
const CONCURRENT_DECISIONS: usize = 4;

/// Ask the worker `questions` for every case labelled for `id`, a few at a time; a case that
/// fails is retried once on its own. Returns the answers and the first failure, if any.
async fn measure_schema(
    engine: &impl DecisionEngine,
    questions: &[Question],
    cases: &[Fixture],
    id: &str,
    scanner: &SafetyScanner,
    limits: &VerifierConfig,
) -> (Measured, Option<String>) {
    use futures::StreamExt as _;
    let mut first_error = None;
    let mut inputs = Vec::new();
    for case in cases {
        let Some(truth) = case.labels.get(id) else {
            continue;
        };
        match mutation_decision_input(
            scanner,
            limits,
            &case.target,
            &case.intent,
            &case.before,
            &case.after,
        ) {
            Ok(input) => inputs.push((case.id.as_str(), *truth, input)),
            Err(error) => {
                first_error.get_or_insert_with(|| format!("input rejected: {error}"));
            }
        }
    }
    let answers: Vec<_> =
        futures::stream::iter(inputs.iter().map(|(case, truth, input)| async move {
            (*case, *truth, input, ask(engine, questions, input).await)
        }))
        .buffer_unordered(CONCURRENT_DECISIONS)
        .collect()
        .await;
    let mut rows = Vec::with_capacity(answers.len());
    for (case, truth, input, answer) in answers {
        let answer = match answer {
            Ok(ppm) => Ok(ppm),
            Err(_) => ask(engine, questions, input).await,
        };
        match answer {
            Ok(ppm) => rows.push((case.to_owned(), truth, ppm)),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    (rows, first_error)
}

/// The worker's P(yes) for the first question, in parts per million.
async fn ask(
    engine: &impl DecisionEngine,
    questions: &[Question],
    input: &DecisionInput,
) -> Result<u32, String> {
    let decision = engine
        .decide(questions, input)
        .await
        .map_err(|error| format!("worker: {error}"))?;
    let yes = decision
        .answers
        .first()
        .and_then(|answer| match answer.answer {
            Answer::Noul { yes } => Some(yes.get()),
            _ => None,
        })
        .ok_or_else(|| String::from("no yes/no answer"))?;
    #[allow(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a probability in 0..=1 scaled to parts per million fits u32"
    )]
    let ppm = (f64::from(yes) * 1_000_000.0)
        .round()
        .clamp(0.0, 1_000_000.0) as u32;
    Ok(ppm)
}

/// Run every fixture through the worker and build the calibrated catalog text; answers are
/// saved in `saved`.
async fn measure(
    kernel: &polaris_kernel::KernelHandle,
    cases: &[Fixture],
    fixture_digest: [u8; 32],
    saved: &Path,
    progress: &mut impl FnMut(&str),
) -> Result<(String, Vec<String>), SessionHostError> {
    let engine = kernel
        .s1_host()
        .ok_or(SessionHostError::Unavailable(
            "the System One worker did not start",
        ))?
        .engine();
    let scanner = SafetyScanner::new()
        .map_err(|_| SessionHostError::Unavailable("the safety scanner did not compile"))?;
    let limits = VerifierConfig::default();
    let builtin = SchemaSet::load_builtin()
        .map_err(|_| SessionHostError::Unavailable("the builtin schemas are invalid"))?;
    let first = cases.first().ok_or(SessionHostError::Unavailable(
        "the fixture file has no cases",
    ))?;
    // The first decision loads the model; its identity is the checkpoint the catalog pins.
    let warm_schema = builtin
        .latest(SAFETY_SCHEMAS[0])
        .ok_or(SessionHostError::Unavailable("a safety schema is missing"))?;
    let warm_input = mutation_decision_input(
        &scanner,
        &limits,
        &first.target,
        &first.intent,
        &first.before,
        &first.after,
    )
    .map_err(|_| SessionHostError::Unavailable("fixture input rejected by the scanner"))?;
    engine
        .decide(&warm_schema.questions, &warm_input)
        .await
        .map_err(|error| SessionHostError::Ui(format!("the worker did not answer: {error}")))?;
    let checkpoint = engine
        .loaded_model_digest()
        .await
        .map_err(|error| SessionHostError::Ui(format!("model identity: {error}")))?
        .ok_or(SessionHostError::Unavailable(
            "the worker reports no loaded model",
        ))?;

    let mut catalog = SchemaSet::builtin_toml().to_owned();
    let mut lines = vec![format!(
        "Measured with the local worker (model identity {}).",
        hex(&checkpoint).get(..16).unwrap_or_default()
    )];
    let date = jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    // What the worker answered is saved per fixture file and model, so a later run (after a
    // catalog fix, say) does not ask again.
    let saved = saved.join(format!(
        "measured-{}-{}.json",
        hex(&fixture_digest).get(..16).unwrap_or_default(),
        hex(&checkpoint).get(..16).unwrap_or_default()
    ));
    let mut measured: BTreeMap<String, Measured> = std::fs::read(&saved)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    for id in SAFETY_SCHEMAS {
        let schema = builtin
            .latest(id)
            .ok_or(SessionHostError::Unavailable("a safety schema is missing"))?;
        let rows = if let Some(rows) = measured.get(id) {
            progress(&format!(
                "{id}: {} cases from the saved measurement",
                rows.len()
            ));
            rows.clone()
        } else {
            let (rows, first_error) =
                measure_schema(&*engine, &schema.questions, cases, id, &scanner, &limits).await;
            progress(&format!("{id}: {} cases measured", rows.len()));
            if let Some(error) = first_error {
                lines.push(format!("{id}: some cases not measured (first: {error})."));
            }
            if !rows.is_empty() {
                let _previous = measured.insert(id.to_owned(), rows.clone());
                if let Ok(bytes) = serde_json::to_vec(&measured) {
                    let _saved = std::fs::write(&saved, bytes);
                }
            }
            rows
        };
        let failed = cases
            .iter()
            .filter(|case| case.labels.contains_key(id))
            .count()
            .saturating_sub(rows.len());
        if rows.is_empty() {
            lines.push(format!("{id}: no measured cases; left uncalibrated."));
            continue;
        }
        let labels: Vec<CalibrationLabel> = rows
            .iter()
            .map(|(case, truth, _)| CalibrationLabel {
                id: case.clone(),
                positive: *truth,
            })
            .collect();
        let predictions: Vec<CalibrationPrediction> = rows
            .iter()
            .map(|(case, _, positive_ppm)| CalibrationPrediction {
                id: case.clone(),
                positive_ppm: *positive_ppm,
            })
            .collect();
        let dataset = CalibrationDataset::join(
            id,
            schema.version,
            Cid::from_bytes(fixture_digest),
            Cid::from_bytes(checkpoint),
            &labels,
            &predictions,
        )
        .map_err(|error| SessionHostError::Ui(format!("{id}: {error}")))?;
        let selected = dataset
            .select_threshold(PrecisionRequirement {
                minimum_ppm: PRECISION_PPM,
                minimum_predicted_positive: MINIMUM_ACCEPTED,
            })
            .map_err(|error| SessionHostError::Ui(format!("{id}: {error}")))?;
        let Some(report) = selected else {
            lines.push(format!(
                "{id}: {} cases ({failed} not measured); precision {}% was not reached at any threshold; left uncalibrated (asks the person).",
                labels.len(),
                PRECISION_PPM / 10_000
            ));
            continue;
        };
        let ratio = |value: Option<polaris_system_one::schemas::calibration::ExactRatio>| {
            value.map(|ratio| {
                #[allow(
                    clippy::as_conversions,
                    clippy::cast_precision_loss,
                    reason = "a report ratio of sample counts"
                )]
                let value = ratio.numerator as f64 / ratio.denominator.max(1) as f64;
                value
            })
        };
        let precision = ratio(report.precision).unwrap_or(0.0);
        let recall = ratio(report.recall).unwrap_or(0.0);
        let ece = ratio(Some(report.ece)).unwrap_or(0.0);
        let accept = f64::from(report.threshold_ppm) / 1_000_000.0;
        lines.push(format!(
            "{id}: {} cases ({failed} not measured); accepts at P ≥ {accept:.3} with precision {:.1}% and recall {:.1}%.",
            labels.len(),
            precision * 100.0,
            recall * 100.0
        ));
        catalog = calibrated_block(
            &catalog,
            id,
            accept,
            &format!(
                "calibration={{status=\"calibrated\",fixture_set=\"polaris-synthetic-v1\",fixture_blake3=\"{}\",n={},precision_target={:.2},observed_precision={precision:.4},observed_recall={recall:.4},ece={ece:.4},checkpoint_blake3=\"{}\",date=\"{date}\"}}",
                hex(&fixture_digest),
                labels.len(),
                f64::from(PRECISION_PPM) / 1_000_000.0,
                hex(&checkpoint)
            ),
        )?;
    }
    Ok((catalog, lines))
}

/// `toml` with the block of schema `id` given the calibrated threshold, the calibrated deadline
/// and `calibration` (the whole `calibration=…` line).
fn calibrated_block(
    toml: &str,
    id: &str,
    accept: f64,
    calibration: &str,
) -> Result<String, SessionHostError> {
    let marker = format!("id=\"{id}\"\n");
    let start = toml.find(&marker).ok_or(SessionHostError::Unavailable(
        "a safety schema block is missing",
    ))?;
    let end = toml[start..]
        .find("\n\n")
        .map_or(toml.len(), |offset| start + offset);
    let accept = accept.clamp(0.5, 1.0);
    let high = accept.min(0.9);
    let low = high.min(0.1);
    let reject = low.min(0.01);
    let block: Vec<String> = toml[start..end]
        .lines()
        .map(|line| {
            if line.starts_with("thresholds=") {
                format!(
                    "thresholds={{accept={accept:.6},reject={reject:.6},abstain_band=[{low:.6},{high:.6}]}}"
                )
            } else if line.starts_with("calibration=") {
                calibration.to_owned()
            } else if let Some(rest) = line.strip_prefix("fail=") {
                rest.find("deadline_ms=").map_or_else(
                    || line.to_owned(),
                    |at| {
                        let after = &rest[at + "deadline_ms=".len()..];
                        let digits = after.chars().take_while(char::is_ascii_digit).count();
                        format!(
                            "fail={}deadline_ms={DEADLINE_MS}{}",
                            &rest[..at],
                            &after[digits..]
                        )
                    },
                )
            } else {
                line.to_owned()
            }
        })
        .collect();
    Ok(format!(
        "{}{}{}",
        &toml[..start],
        block.join("\n"),
        &toml[end..]
    ))
}
