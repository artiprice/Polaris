use polaris_nebius_cloud_sandboxes::{
    ImageId, ImageRanker, ImageRecord, MatchReason, RepoManifest, ResolvedImage, ResolverError,
    ScoredImage, TrustedImageMetadata, resolve_environment,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

type TestResult = Result<(), Box<dyn Error>>;
type AmbiguousFixture = (RepoManifest, Vec<ImageRecord>, Vec<TrustedImageMetadata>);

fn check(condition: bool, message: &'static str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn make_manifest(
    repo: &str,
    commit: &str,
    languages: &[&str],
    toolchains: &[(&str, &str)],
    runtime: Option<&str>,
    architecture: Option<&str>,
) -> Result<RepoManifest, Box<dyn Error>> {
    let language_set = languages.iter().map(|value| (*value).to_owned()).collect();
    let toolchain_map = toolchains
        .iter()
        .map(|(name, version)| ((*name).to_owned(), (*version).to_owned()))
        .collect();
    Ok(RepoManifest::new(
        repo,
        commit,
        language_set,
        toolchain_map,
        runtime.map(str::to_owned),
        architecture.map(str::to_owned),
    )?)
}

fn image(id: &str) -> Result<ImageRecord, Box<dyn Error>> {
    Ok(ImageRecord {
        uuid: ImageId::new(id.to_owned())?,
        tag: None,
        created_at: "2026-10-27T00:00:00Z".to_owned(),
        operation_uuid: None,
    })
}

fn metadata(
    id: &str,
    repo: Option<&str>,
    commit: Option<&str>,
    languages: &[&str],
    toolchains: &[(&str, &str)],
    runtime: Option<&str>,
    architecture: Option<&str>,
) -> Result<TrustedImageMetadata, Box<dyn Error>> {
    let language_set = languages.iter().map(|value| (*value).to_owned()).collect();
    let toolchain_map = toolchains
        .iter()
        .map(|(name, version)| ((*name).to_owned(), (*version).to_owned()))
        .collect();
    Ok(TrustedImageMetadata::new(
        ImageId::new(id.to_owned())?,
        repo.map(str::to_owned),
        commit.map(str::to_owned),
        language_set,
        toolchain_map,
        runtime.map(str::to_owned),
        architecture.map(str::to_owned),
    )?)
}

#[derive(Debug)]
struct StaticRanker {
    scores: Vec<ScoredImage>,
    called: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl StaticRanker {
    fn new(scores: Vec<ScoredImage>) -> Self {
        Self {
            scores,
            called: Arc::new(AtomicBool::new(false)),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ImageRanker for StaticRanker {
    fn rank(
        &self,
        _manifest: &RepoManifest,
        candidates: &[polaris_nebius_cloud_sandboxes::ImageCandidate],
    ) -> Result<Vec<ScoredImage>, ResolverError> {
        self.called.store(true, Ordering::Release);
        let ids: Vec<String> = candidates
            .iter()
            .map(|candidate| candidate.image_id().as_str().to_owned())
            .collect();
        let mut seen = self.seen.lock().map_err(|_| ResolverError::RankerFailure)?;
        *seen = ids;
        Ok(self.scores.clone())
    }
}

#[derive(Debug, Default)]
struct MetadataRanker {
    inspected: AtomicBool,
}

impl ImageRanker for MetadataRanker {
    fn rank(
        &self,
        manifest: &RepoManifest,
        candidates: &[polaris_nebius_cloud_sandboxes::ImageCandidate],
    ) -> Result<Vec<ScoredImage>, ResolverError> {
        let manifest_is_inspectable = manifest.repo() == "example/project"
            && manifest.commit() == "0123456789abcdef0123456789abcdef01234567"
            && manifest.languages().contains("rust")
            && manifest.toolchains().get("rust").map(String::as_str) == Some("1.82.0")
            && manifest.runtime() == Some("rust")
            && manifest.architecture() == Some("x86_64");
        let candidate_metadata_is_inspectable = candidates.iter().all(|candidate| {
            let annotation = candidate.metadata();
            annotation.image_id() == candidate.image_id()
                && annotation.repo() == Some("example/project")
                && annotation.commit() == Some("0123456789abcdef0123456789abcdef01234567")
                && annotation.languages().contains("rust")
                && annotation.toolchains().get("rust").map(String::as_str) == Some("1.82.0")
                && annotation.runtime() == Some("rust")
                && annotation.architecture() == Some("x86_64")
                && candidate.record().uuid == *candidate.image_id()
                && candidate.record().created_at == "2026-10-27T00:00:00Z"
                && candidate.record().operation_uuid.as_deref() == Some("operation")
        });
        self.inspected.store(
            manifest_is_inspectable && candidate_metadata_is_inspectable,
            Ordering::Release,
        );
        Ok(candidates
            .iter()
            .map(|candidate| {
                let preferred = candidate.record().tag.as_deref() == Some("preferred");
                let score = if preferred { 9_000 } else { 5_000 };
                ScoredImage::new(candidate.image_id().clone(), score)
            })
            .collect())
    }
}

fn ranker_called(ranker: &StaticRanker) -> bool {
    ranker.called.load(Ordering::Acquire)
}

fn ranked(id: &str, score: u16) -> Result<ScoredImage, Box<dyn Error>> {
    Ok(ScoredImage::new(ImageId::new(id.to_owned())?, score))
}

fn resolved_id(result: &ResolvedImage) -> &str {
    result.image_id().as_str()
}

#[test]
fn exact_commit_match_wins() -> TestResult {
    let manifest = make_manifest(
        "fastino/gliner",
        "0123456789abcdef0123456789abcdef01234567",
        &["python"],
        &[("python", "3.11.8")],
        Some("python-3.11"),
        Some("x86_64"),
    )?;
    let images = vec![image("exact-image")?, image("other-image")?];
    let metadata = vec![
        metadata(
            "exact-image",
            Some("fastino/gliner"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            &["python"],
            &[("python", "3.11.8")],
            Some("python-3.11"),
            Some("x86_64"),
        )?,
        metadata(
            "other-image",
            Some("fastino/other"),
            Some("1123456789abcdef0123456789abcdef01234567"),
            &["python"],
            &[("python", "3.11.8")],
            Some("python-3.11"),
            Some("x86_64"),
        )?,
    ];
    let ranker = StaticRanker::new(Vec::new());
    let result = resolve_environment(&manifest, &images, &metadata, &ranker)?;
    check(
        resolved_id(&result) == "exact-image",
        "exact repo and commit should win",
    )?;
    check(
        result.reason() == MatchReason::ExactRepoCommit,
        "exact selection must carry its match reason",
    )?;
    check(
        !ranker_called(&ranker),
        "unique exact match must bypass ranker",
    )
}

#[test]
fn exact_commit_incompatible_runtime_is_not_selected() -> TestResult {
    let manifest = make_manifest(
        "fastino/gliner",
        "0123456789abcdef0123456789abcdef01234567",
        &["python"],
        &[("python", "3.11.8")],
        Some("python-3.11"),
        Some("x86_64"),
    )?;
    let images = vec![image("wrong-runtime")?, image("compatible")?];
    let metadata = vec![
        metadata(
            "wrong-runtime",
            Some("fastino/gliner"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            &["python"],
            &[("python", "3.11.8")],
            Some("python-3.10"),
            Some("x86_64"),
        )?,
        metadata(
            "compatible",
            Some("fastino/other"),
            Some("1123456789abcdef0123456789abcdef01234567"),
            &["python"],
            &[("python", "3.11.8")],
            Some("python-3.11"),
            Some("x86_64"),
        )?,
    ];
    let ranker = StaticRanker::new(Vec::new());
    let result = resolve_environment(&manifest, &images, &metadata, &ranker)?;
    check(
        resolved_id(&result) == "compatible",
        "incompatible exact image must not win",
    )?;
    check(
        result.reason() == MatchReason::CompatibleMetadata,
        "fallback must report compatible-metadata selection",
    )?;
    Ok(())
}

#[test]
fn exact_commit_incompatible_toolchain_is_not_selected() -> TestResult {
    let manifest = make_manifest(
        "example/project",
        "0123456789abcdef0123456789abcdef01234567",
        &["python"],
        &[("python", "3.11.8")],
        None,
        None,
    )?;
    let images = vec![image("wrong-toolchain")?, image("compatible")?];
    let metadata = vec![
        metadata(
            "wrong-toolchain",
            Some("example/project"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            &["python"],
            &[("python", "3.10.14")],
            None,
            None,
        )?,
        metadata(
            "compatible",
            None,
            None,
            &["python"],
            &[("python", "3.11.8")],
            None,
            None,
        )?,
    ];
    let ranker = StaticRanker::new(Vec::new());
    let result = resolve_environment(&manifest, &images, &metadata, &ranker)?;
    check(
        resolved_id(&result) == "compatible",
        "exact repository and commit must not bypass a toolchain constraint",
    )?;
    check(
        result.reason() == MatchReason::CompatibleMetadata,
        "compatible fallback must report its evidence",
    )
}

#[test]
fn compatible_metadata_selects_unique_candidate() -> TestResult {
    let manifest = make_manifest(
        "example/project",
        "0123456789abcdef0123456789abcdef01234567",
        &["rust"],
        &[("rust", "1.82.0")],
        Some("rust"),
        Some("x86_64"),
    )?;
    let images = vec![image("candidate")?, image("unrelated")?];
    let metadata = vec![
        metadata(
            "candidate",
            None,
            None,
            &["rust", "toml"],
            &[("rust", "1.82.0")],
            Some("rust"),
            Some("x86_64"),
        )?,
        metadata("unrelated", None, None, &["python"], &[], None, None)?,
    ];
    let ranker = StaticRanker::new(Vec::new());
    let result = resolve_environment(&manifest, &images, &metadata, &ranker)?;
    check(
        resolved_id(&result) == "candidate",
        "compatible image should be selected",
    )?;
    check(
        result.reason() == MatchReason::CompatibleMetadata,
        "selection must report compatible metadata",
    )?;
    check(
        !ranker_called(&ranker),
        "unique compatible image must bypass ranker",
    )
}

#[test]
fn ambiguous_candidates_use_ranker() -> TestResult {
    let manifest = make_manifest(
        "fastino/gliner",
        "0123456789abcdef0123456789abcdef01234567",
        &["python"],
        &[("python", "3.11.8")],
        None,
        None,
    )?;
    let images = vec![image("z-image")?, image("a-image")?];
    let metadata = vec![
        metadata(
            "z-image",
            Some("fastino/gliner"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            &["python"],
            &[("python", "3.11.8")],
            None,
            None,
        )?,
        metadata(
            "a-image",
            Some("fastino/gliner"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            &["python"],
            &[("python", "3.11.8")],
            None,
            None,
        )?,
    ];
    let ranker = StaticRanker::new(vec![ranked("a-image", 7000)?, ranked("z-image", 6000)?]);
    let result = resolve_environment(&manifest, &images, &metadata, &ranker)?;
    check(
        resolved_id(&result) == "a-image",
        "highest ranker score should win",
    )?;
    check(
        result.reason() == MatchReason::Ranked,
        "ranked result reason must be explicit",
    )?;
    check(
        ranker_called(&ranker),
        "ambiguous exact candidates must use injected ranker",
    )?;
    let seen = ranker
        .seen
        .lock()
        .map_err(|_| "ranker observation lock poisoned")?;
    check(
        seen.iter().map(String::as_str).eq(["a-image", "z-image"]),
        "ranker input must have stable ID order",
    )
}

#[test]
fn injected_ranker_can_inspect_manifest_metadata_and_actual_image_record() -> TestResult {
    let manifest = make_manifest(
        "example/project",
        "0123456789abcdef0123456789abcdef01234567",
        &["rust"],
        &[("rust", "1.82.0")],
        Some("rust"),
        Some("x86_64"),
    )?;
    let mut preferred = image("preferred-image")?;
    preferred.tag = Some("preferred".to_owned());
    preferred.operation_uuid = Some("operation".to_owned());
    let mut ordinary = image("ordinary-image")?;
    ordinary.tag = Some("ordinary".to_owned());
    ordinary.operation_uuid = Some("operation".to_owned());
    let images = vec![preferred, ordinary];
    let metadata = vec![
        metadata(
            "preferred-image",
            Some("example/project"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            &["rust"],
            &[("rust", "1.82.0")],
            Some("rust"),
            Some("x86_64"),
        )?,
        metadata(
            "ordinary-image",
            Some("example/project"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            &["rust"],
            &[("rust", "1.82.0")],
            Some("rust"),
            Some("x86_64"),
        )?,
    ];
    let ranker = MetadataRanker::default();
    let result = resolve_environment(&manifest, &images, &metadata, &ranker)?;
    check(
        ranker.inspected.load(Ordering::Acquire),
        "ranker must be able to inspect all declared manifest and candidate fields through getters",
    )?;
    check(
        resolved_id(&result) == "preferred-image",
        "ranker should use the actual upstream tag to distinguish candidates",
    )
}

#[test]
fn tied_highest_scores_remain_ambiguous() -> TestResult {
    let (manifest, images, metadata) = ambiguous_fixture()?;
    let ranker = StaticRanker::new(vec![ranked("a-image", 7000)?, ranked("z-image", 7000)?]);
    check(
        resolve_environment(&manifest, &images, &metadata, &ranker)
            == Err(ResolverError::AmbiguousRankerOutput),
        "ranker ties must not be resolved by input order",
    )
}

#[test]
fn manifest_requires_canonical_repo_and_full_git_oid() -> TestResult {
    let invalid_repo = RepoManifest::new(
        "https://example/project",
        "0123456789abcdef0123456789abcdef01234567",
        BTreeSet::new(),
        BTreeMap::new(),
        None,
        None,
    );
    check(
        invalid_repo == Err(ResolverError::InvalidManifest),
        "repository URLs must not be accepted as canonical owner/repo identifiers",
    )?;
    let abbreviated_oid = RepoManifest::new(
        "example/project",
        "0123456789abcdef",
        BTreeSet::new(),
        BTreeMap::new(),
        None,
        None,
    );
    check(
        abbreviated_oid == Err(ResolverError::InvalidManifest),
        "abbreviated Git object IDs must not identify an image",
    )
}

#[test]
fn ranker_omitted_scores_are_rejected() -> TestResult {
    let (manifest, images, metadata) = ambiguous_fixture()?;
    let ranker = StaticRanker::new(vec![ranked("a-image", 8000)?]);
    let result = resolve_environment(&manifest, &images, &metadata, &ranker);
    check(
        result == Err(ResolverError::InvalidRankerOutput),
        "ranker must score every candidate",
    )
}

#[test]
fn ranker_duplicate_ids_are_rejected() -> TestResult {
    let (manifest, images, metadata) = ambiguous_fixture()?;
    let ranker = StaticRanker::new(vec![ranked("a-image", 8000)?, ranked("a-image", 7000)?]);
    let result = resolve_environment(&manifest, &images, &metadata, &ranker);
    check(
        result == Err(ResolverError::InvalidRankerOutput),
        "duplicate ranker IDs must fail closed",
    )
}

#[test]
fn ranker_invented_id_is_rejected() -> TestResult {
    let (manifest, images, metadata) = ambiguous_fixture()?;
    let ranker = StaticRanker::new(vec![ranked("a-image", 8000)?, ranked("invented", 7000)?]);
    let result = resolve_environment(&manifest, &images, &metadata, &ranker);
    check(
        result == Err(ResolverError::InvalidRankerOutput),
        "ranker cannot invent a Contree image ID",
    )
}

#[test]
fn ranker_invalid_score_is_rejected() -> TestResult {
    let (manifest, images, metadata) = ambiguous_fixture()?;
    let ranker = StaticRanker::new(vec![ranked("a-image", 10_001)?, ranked("z-image", 7000)?]);
    let result = resolve_environment(&manifest, &images, &metadata, &ranker);
    check(
        result == Err(ResolverError::InvalidRankerOutput),
        "basis-point scores above one hundred percent must fail",
    )
}

#[test]
fn duplicate_catalog_or_annotation_ids_are_rejected() -> TestResult {
    let (manifest, images, metadata) = ambiguous_fixture()?;
    let ranker = StaticRanker::new(Vec::new());
    let duplicate_images = vec![images[0].clone(), images[0].clone()];
    check(
        resolve_environment(&manifest, &duplicate_images, &metadata, &ranker)
            == Err(ResolverError::InvalidCatalog),
        "duplicate IDs from list_images must fail closed",
    )?;
    let duplicate_metadata = vec![metadata[0].clone(), metadata[0].clone()];
    check(
        resolve_environment(&manifest, &images, &duplicate_metadata, &ranker)
            == Err(ResolverError::InvalidCatalog),
        "duplicate host annotations must fail closed",
    )
}

#[test]
fn annotation_for_unlisted_image_is_rejected() -> TestResult {
    let manifest = make_manifest(
        "example/project",
        "0123456789abcdef0123456789abcdef01234567",
        &["python"],
        &[("python", "3.11.8")],
        None,
        None,
    )?;
    let images = vec![image("actual")?];
    let metadata = vec![metadata(
        "invented",
        Some("example/project"),
        Some("0123456789abcdef0123456789abcdef01234567"),
        &["python"],
        &[("python", "3.11.8")],
        None,
        None,
    )?];
    let ranker = StaticRanker::new(Vec::new());
    check(
        resolve_environment(&manifest, &images, &metadata, &ranker)
            == Err(ResolverError::InvalidCatalog),
        "annotations cannot add image IDs absent from list_images",
    )
}

#[test]
fn no_match_and_unconstrained_fallback_are_rejected() -> TestResult {
    let manifest = make_manifest(
        "example/project",
        "0123456789abcdef0123456789abcdef01234567",
        &[],
        &[],
        None,
        None,
    )?;
    let images = vec![image("candidate")?];
    let metadata = vec![metadata(
        "candidate",
        None,
        None,
        &["python"],
        &[],
        None,
        None,
    )?];
    let ranker = StaticRanker::new(vec![ranked("candidate", 9000)?]);
    check(
        resolve_environment(&manifest, &images, &metadata, &ranker)
            == Err(ResolverError::NoMatchingImage),
        "an unconstrained manifest cannot choose an arbitrary annotated image",
    )?;
    check(
        !ranker_called(&ranker),
        "ranker cannot create a match without candidates",
    )?;

    let missing = make_manifest(
        "example/project",
        "1123456789abcdef0123456789abcdef01234567",
        &["python"],
        &[("python", "3.12.0")],
        None,
        None,
    )?;
    check(
        resolve_environment(&missing, &images, &metadata, &ranker)
            == Err(ResolverError::NoMatchingImage),
        "incompatible metadata must produce a typed no-match error",
    )
}

#[test]
fn resolver_is_deterministic_for_permuted_inputs() -> TestResult {
    let manifest = make_manifest(
        "example/project",
        "0123456789abcdef0123456789abcdef01234567",
        &["rust"],
        &[("rust", "1.82.0")],
        Some("rust"),
        Some("x86_64"),
    )?;
    let first_images = vec![image("z-compatible")?, image("a-compatible")?];
    let first_metadata = vec![
        metadata(
            "z-compatible",
            None,
            None,
            &["rust"],
            &[("rust", "1.82.0")],
            Some("rust"),
            Some("x86_64"),
        )?,
        metadata(
            "a-compatible",
            None,
            None,
            &["rust"],
            &[("rust", "1.82.0")],
            Some("rust"),
            Some("x86_64"),
        )?,
    ];
    let ranker_a = StaticRanker::new(vec![
        ranked("a-compatible", 8000)?,
        ranked("z-compatible", 7000)?,
    ]);
    let first = resolve_environment(&manifest, &first_images, &first_metadata, &ranker_a)?;
    let second_images = vec![first_images[1].clone(), first_images[0].clone()];
    let second_metadata = vec![first_metadata[1].clone(), first_metadata[0].clone()];
    let ranker_b = StaticRanker::new(vec![
        ranked("a-compatible", 8000)?,
        ranked("z-compatible", 7000)?,
    ]);
    let second = resolve_environment(&manifest, &second_images, &second_metadata, &ranker_b)?;
    check(
        first == second,
        "input permutations must preserve the ranked result",
    )
}

#[test]
fn oversized_catalog_is_rejected() -> TestResult {
    let (manifest, images, metadata) = ambiguous_fixture()?;
    let mut oversized = Vec::new();
    for _ in 0..=polaris_nebius_cloud_sandboxes::MAX_RESOLVER_IMAGES {
        oversized.push(images[0].clone());
    }
    let ranker = StaticRanker::new(Vec::new());
    check(
        resolve_environment(&manifest, &oversized, &metadata, &ranker)
            == Err(ResolverError::CatalogLimit),
        "catalog cap must be checked before duplicate ID processing",
    )
}

fn ambiguous_fixture() -> Result<AmbiguousFixture, Box<dyn Error>> {
    let manifest = make_manifest(
        "fastino/gliner",
        "0123456789abcdef0123456789abcdef01234567",
        &["python"],
        &[("python", "3.11.8")],
        None,
        None,
    )?;
    let images = vec![image("a-image")?, image("z-image")?];
    let metadata = vec![
        metadata(
            "a-image",
            Some("fastino/gliner"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            &["python"],
            &[("python", "3.11.8")],
            None,
            None,
        )?,
        metadata(
            "z-image",
            Some("fastino/gliner"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            &["python"],
            &[("python", "3.11.8")],
            None,
            None,
        )?,
    ];
    Ok((manifest, images, metadata))
}
