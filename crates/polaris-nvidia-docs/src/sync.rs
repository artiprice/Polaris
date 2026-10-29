//! Initial and incremental download of the NVIDIA documentation sets (P5-X-3) into a local
//! offline seed: `seed.jsonl` (the [`DocIndex::read_seed`] format) and `state.json`, the
//! provenance of every page (listing, topic, validators, chunk count, retrieval time).
//!
//! Each run reads the `llms.txt` listings again, then fetches the listed pages conditionally. A
//! page that did not change (HTTP 304) or could not be fetched keeps its earlier chunks; a page
//! no longer listed is dropped unless its listing failed. Both files are replaced atomically. The
//! seed is a local artifact under the user's Polaris directory, never committed (documentation
//! licensing, PLAN R14).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::NvidiaError;
use crate::docs::{DocChunk, DocIndex, parse_llms_txt};
use crate::fetch::{DOC_HOSTS, DocFetcher, Validators};
use crate::profile::DocTopic;

/// Seed file name inside the documentation directory.
pub const SEED_FILE: &str = "seed.jsonl";
/// Provenance file name inside the documentation directory.
pub const STATE_FILE: &str = "state.json";
const STATE_VERSION: u16 = 1;

/// One `llms.txt` listing. With a topic, its pages belong to that topic unless a link names a
/// more specific one; without, only links that name a topic are synchronised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocSource {
    /// Default topic of the listed pages.
    pub topic: Option<DocTopic>,
    /// HTTPS URL of the listing.
    pub listing: &'static str,
}

/// The listings synchronised by default: the `llms.txt` files PLAN F20 verified.
pub const SOURCES: &[DocSource] = &[
    DocSource {
        topic: Some(DocTopic::Cuda),
        listing: "https://docs.nvidia.com/cuda/llms.txt",
    },
    DocSource {
        topic: Some(DocTopic::OpenShell),
        listing: "https://docs.nvidia.com/openshell/llms.txt",
    },
    DocSource {
        topic: Some(DocTopic::NemoClaw),
        listing: "https://docs.nvidia.com/nemoclaw/llms.txt",
    },
    DocSource {
        topic: None,
        listing: "https://docs.nvidia.com/llms.txt",
    },
    DocSource {
        topic: None,
        listing: "https://developer.nvidia.com/llms.txt",
    },
];

/// Bounds of one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncLimits {
    /// Pages fetched in one run, over all listings.
    pub max_pages: usize,
    /// Pages taken from one listing.
    pub max_pages_per_listing: usize,
    /// Largest chunk, in characters.
    pub max_chars: usize,
    /// Pause between two requests to one host.
    pub spacing: Duration,
}

impl Default for SyncLimits {
    fn default() -> Self {
        Self {
            max_pages: 600,
            max_pages_per_listing: 200,
            max_chars: 2400,
            spacing: Duration::from_millis(500),
        }
    }
}

/// Provenance of one synchronised page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageState {
    /// Topic the page's chunks carry.
    pub topic: String,
    /// Listing the page came from.
    pub listing: String,
    /// Validators of the last successful fetch.
    pub validators: Validators,
    /// Chunks the page contributed.
    pub chunks: usize,
    /// When the content was last downloaded, Unix seconds.
    pub retrieved_unix: u64,
}

/// `state.json`: every page of the seed by URL.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// File format version.
    pub version: u16,
    /// Pages by URL.
    pub pages: BTreeMap<String, PageState>,
}

/// What one run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Listings read.
    pub listings: usize,
    /// Listings that could not be read, with the reason.
    pub listing_failures: Vec<(String, String)>,
    /// Pages considered this run.
    pub pages: usize,
    /// Pages downloaded with new content.
    pub fetched: usize,
    /// Pages unchanged since the last run.
    pub unchanged: usize,
    /// Pages that could not be fetched (earlier chunks kept), with the reason.
    pub failed: Vec<(String, String)>,
    /// Pages dropped because their listing no longer names them.
    pub dropped: usize,
    /// Chunks in the seed after the run.
    pub chunks: usize,
    /// Chunks by topic.
    pub per_topic: BTreeMap<String, usize>,
    /// A page bound stopped the run before every listed page.
    pub truncated: bool,
    /// Where the seed was written.
    pub seed: PathBuf,
}

/// The documentation directory under the user's Polaris directory (`~/.polaris`).
#[must_use]
pub fn default_dir(global: &Path) -> PathBuf {
    global.join("docs").join("nvidia")
}

/// The seed in `dir`, `None` when no run has written one yet.
/// # Errors
/// An unreadable or malformed seed.
pub fn load_seed(dir: &Path) -> Result<Option<DocIndex>, NvidiaError> {
    let path = dir.join(SEED_FILE);
    match std::fs::File::open(&path) {
        Ok(file) => DocIndex::read_seed(BufReader::new(file)).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(NvidiaError::Seed),
    }
}

/// The provenance in `dir` (empty before the first run).
/// # Errors
/// An unreadable or malformed state file.
pub fn load_state(dir: &Path) -> Result<SyncState, NvidiaError> {
    match std::fs::read(dir.join(STATE_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| NvidiaError::Seed),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SyncState {
            version: STATE_VERSION,
            pages: BTreeMap::new(),
        }),
        Err(_) => Err(NvidiaError::Seed),
    }
}

/// The topic a listed link belongs to: the most specific one its URL or title names, else the
/// listing's default.
#[must_use]
pub fn classify(url: &str, title: &str, default: Option<DocTopic>) -> Option<DocTopic> {
    let text = format!("{url} {title}").to_ascii_lowercase();
    const SPECIFIC: &[(&[&str], DocTopic)] = &[
        (
            &["tensorrt-llm", "tensorrt_llm", "trt-llm"],
            DocTopic::TensorRtLlm,
        ),
        (&["triton"], DocTopic::Triton),
        (&["nccl"], DocTopic::Nccl),
        (&["ptx"], DocTopic::Ptx),
        (&["nemotron"], DocTopic::Nemotron),
        (&["openshell"], DocTopic::OpenShell),
        (&["nemoclaw"], DocTopic::NemoClaw),
        (&["cuda"], DocTopic::Cuda),
    ];
    SPECIFIC
        .iter()
        .find(|(words, _)| words.iter().any(|word| text.contains(word)))
        .map(|(_, topic)| *topic)
        .or(default)
}

/// Synchronise `sources` into `dir` with `fetcher` (create it with [`DocFetcher::new`] and the
/// limits' spacing). `progress` receives one line per listing and every 25 pages.
/// # Errors
/// The directory cannot be created, the earlier seed or state is malformed, or the new files
/// cannot be written. Unreachable listings and pages are reported, not errors.
pub async fn sync(
    dir: &Path,
    sources: &[DocSource],
    fetcher: &mut DocFetcher,
    limits: SyncLimits,
    mut progress: impl FnMut(&str),
) -> Result<SyncReport, NvidiaError> {
    std::fs::create_dir_all(dir).map_err(|_| NvidiaError::Seed)?;
    let mut state = load_state(dir)?;
    let mut chunks = previous_chunks(dir)?;
    // Only pages whose chunks are still here may be answered with 304.
    fetcher.remember(
        state
            .pages
            .iter()
            .filter(|(url, _)| chunks.contains_key(*url))
            .map(|(url, page)| (url.clone(), page.validators.clone()))
            .collect(),
    );

    let mut report = SyncReport {
        seed: dir.join(SEED_FILE),
        ..SyncReport::default()
    };
    let mut planned: Vec<(String, DocTopic, &'static str)> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut failed_listings = BTreeSet::new();
    for source in sources {
        progress(&format!("reading {}", source.listing));
        let listing = match fetcher.fetch(source.listing).await {
            Ok(crate::fetch::FetchOutcome::Updated { body, .. }) => body,
            Ok(crate::fetch::FetchOutcome::NotModified) => {
                // Listings are never sent validators; treat a 304 as unreadable.
                report
                    .listing_failures
                    .push((source.listing.to_owned(), "not modified".to_owned()));
                failed_listings.insert(source.listing);
                continue;
            }
            Err(error) => {
                report
                    .listing_failures
                    .push((source.listing.to_owned(), error.to_string()));
                failed_listings.insert(source.listing);
                continue;
            }
        };
        report.listings = report.listings.saturating_add(1);
        let Ok(base) = url::Url::parse(source.listing) else {
            continue;
        };
        let mut taken = 0_usize;
        for link in parse_llms_txt(&listing).links {
            let Some(url) = resolve(&base, &link.url) else {
                continue;
            };
            let Some(topic) = classify(&url, &link.title, source.topic) else {
                continue;
            };
            if !seen.insert(url.clone()) {
                continue;
            }
            if taken >= limits.max_pages_per_listing || planned.len() >= limits.max_pages {
                report.truncated = true;
                break;
            }
            taken = taken.saturating_add(1);
            planned.push((url, topic, source.listing));
        }
    }

    report.pages = planned.len();
    let now = unix_now();
    for (done, (url, topic, listing)) in planned.iter().enumerate() {
        if done % 25 == 0 {
            progress(&format!("{done}/{} pages", planned.len()));
        }
        match fetcher.ingest(url, *topic, limits.max_chars).await {
            Ok(Some(fresh)) => {
                report.fetched = report.fetched.saturating_add(1);
                state.pages.insert(
                    url.clone(),
                    PageState {
                        topic: topic.as_str().to_owned(),
                        listing: (*listing).to_owned(),
                        validators: fetcher.validators().get(url).cloned().unwrap_or_default(),
                        chunks: fresh.len(),
                        retrieved_unix: now,
                    },
                );
                chunks.insert(url.clone(), fresh);
            }
            Ok(None) => {
                report.unchanged = report.unchanged.saturating_add(1);
                if let Some(page) = state.pages.get_mut(url) {
                    (*listing).clone_into(&mut page.listing);
                }
            }
            Err(error) => report.failed.push((url.clone(), error.to_string())),
        }
    }

    // Pages of a listing that was read but no longer names them are dropped.
    let before = state.pages.len();
    state
        .pages
        .retain(|url, page| seen.contains(url) || failed_listings.contains(page.listing.as_str()));
    report.dropped = before.saturating_sub(state.pages.len());
    chunks.retain(|url, _| state.pages.contains_key(url));
    state.version = STATE_VERSION;

    for page_chunks in chunks.values() {
        for chunk in page_chunks {
            *report.per_topic.entry(chunk.topic.clone()).or_insert(0) += 1;
            report.chunks = report.chunks.saturating_add(1);
        }
    }
    write_atomically(&dir.join(SEED_FILE), |writer| {
        for chunk in chunks.values().flatten() {
            serde_json::to_writer(&mut *writer, chunk).map_err(|_| NvidiaError::Seed)?;
            writer.write_all(b"\n").map_err(|_| NvidiaError::Seed)?;
        }
        Ok(())
    })?;
    write_atomically(&dir.join(STATE_FILE), |writer| {
        serde_json::to_writer_pretty(&mut *writer, &state).map_err(|_| NvidiaError::Seed)
    })?;
    Ok(report)
}

/// Chunks of the earlier seed, grouped by source URL.
fn previous_chunks(dir: &Path) -> Result<BTreeMap<String, Vec<DocChunk>>, NvidiaError> {
    let mut grouped: BTreeMap<String, Vec<DocChunk>> = BTreeMap::new();
    let file = match std::fs::File::open(dir.join(SEED_FILE)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(grouped),
        Err(_) => return Err(NvidiaError::Seed),
    };
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|_| NvidiaError::Seed)?;
        if line.trim().is_empty() {
            continue;
        }
        let chunk: DocChunk = serde_json::from_str(&line).map_err(|_| NvidiaError::Seed)?;
        grouped.entry(chunk.source.clone()).or_default().push(chunk);
    }
    Ok(grouped)
}

/// `link` resolved against its listing, when it lands on an NVIDIA documentation host over
/// HTTPS; the fragment is dropped.
fn resolve(base: &url::Url, link: &str) -> Option<String> {
    let mut url = base.join(link).ok()?;
    url.set_fragment(None);
    let host = url.host_str()?;
    (url.scheme() == "https" && DOC_HOSTS.contains(&host)).then(|| url.to_string())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Write `path` through a temporary sibling and rename it into place.
fn write_atomically(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<std::fs::File>) -> Result<(), NvidiaError>,
) -> Result<(), NvidiaError> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| {
        let file = std::fs::File::create(&temporary).map_err(|_| NvidiaError::Seed)?;
        let mut writer = BufWriter::new(file);
        write(&mut writer)?;
        let file = writer.into_inner().map_err(|_| NvidiaError::Seed)?;
        file.sync_all().map_err(|_| NvidiaError::Seed)?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|_| NvidiaError::Seed)
    })();
    if result.is_err() {
        let _removed = std::fs::remove_file(&temporary);
    }
    result
}
