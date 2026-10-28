//! Deterministic environment selection over the actual Contree image catalog.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{ImageId, ImageRecord};

/// Maximum number of catalog images or trusted annotations accepted by one resolution.
pub const MAX_RESOLVER_IMAGES: usize = 4_096;
const MAX_ATOM_BYTES: usize = 128;
const MAX_REPO_BYTES: usize = 256;
const MAX_GIT_OID_BYTES: usize = 64;

/// Validated repository and declared environment requirements from a trusted repository manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoManifest {
    repo: String,
    commit: String,
    languages: BTreeSet<String>,
    toolchains: BTreeMap<String, String>,
    runtime: Option<String>,
    architecture: Option<String>,
}

impl RepoManifest {
    /// Creates a manifest after validating its canonical repository, full Git object ID, and constraints.
    ///
    /// # Errors
    /// Returns [`ResolverError::InvalidManifest`] when any value is malformed or exceeds its bound.
    pub fn new(
        repo: impl Into<String>,
        commit: impl Into<String>,
        languages: BTreeSet<String>,
        toolchains: BTreeMap<String, String>,
        runtime: Option<String>,
        architecture: Option<String>,
    ) -> Result<Self, ResolverError> {
        let repo = canonical_repo(repo.into())?;
        let commit = canonical_oid(commit.into())?;
        validate_requirements(&languages, &toolchains, &runtime, &architecture)?;
        Ok(Self {
            repo,
            commit,
            languages,
            toolchains,
            runtime,
            architecture,
        })
    }

    /// Returns the canonical `owner/repository` identifier.
    #[must_use]
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// Returns the full lowercase Git object ID.
    #[must_use]
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// Returns the declared language requirements.
    #[must_use]
    pub fn languages(&self) -> &BTreeSet<String> {
        &self.languages
    }

    /// Returns exact toolchain name-to-version requirements.
    #[must_use]
    pub fn toolchains(&self) -> &BTreeMap<String, String> {
        &self.toolchains
    }

    /// Returns the requested runtime, when declared.
    #[must_use]
    pub fn runtime(&self) -> Option<&str> {
        self.runtime.as_deref()
    }

    /// Returns the requested architecture, when declared.
    #[must_use]
    pub fn architecture(&self) -> Option<&str> {
        self.architecture.as_deref()
    }
}

/// Host-trusted metadata explicitly joined to an actual image ID from `list_images`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedImageMetadata {
    image_id: ImageId,
    repo: Option<String>,
    commit: Option<String>,
    languages: BTreeSet<String>,
    toolchains: BTreeMap<String, String>,
    runtime: Option<String>,
    architecture: Option<String>,
}

impl TrustedImageMetadata {
    /// Creates bounded metadata for a listed image. Repository and commit must be supplied together.
    ///
    /// # Errors
    /// Returns [`ResolverError::InvalidMetadata`] for malformed or inconsistent annotations.
    pub fn new(
        image_id: ImageId,
        repo: Option<String>,
        commit: Option<String>,
        languages: BTreeSet<String>,
        toolchains: BTreeMap<String, String>,
        runtime: Option<String>,
        architecture: Option<String>,
    ) -> Result<Self, ResolverError> {
        let (repo, commit) = match (repo, commit) {
            (Some(repo), Some(commit)) => (
                Some(canonical_repo(repo).map_err(|_| ResolverError::InvalidMetadata)?),
                Some(canonical_oid(commit).map_err(|_| ResolverError::InvalidMetadata)?),
            ),
            (None, None) => (None, None),
            _ => return Err(ResolverError::InvalidMetadata),
        };
        validate_requirements(&languages, &toolchains, &runtime, &architecture)
            .map_err(|_| ResolverError::InvalidMetadata)?;
        validate_image_id(&image_id).map_err(|_| ResolverError::InvalidMetadata)?;
        Ok(Self {
            image_id,
            repo,
            commit,
            languages,
            toolchains,
            runtime,
            architecture,
        })
    }

    /// Returns the actual image ID this annotation describes.
    #[must_use]
    pub fn image_id(&self) -> &ImageId {
        &self.image_id
    }

    /// Returns the canonical repository annotation, when present.
    #[must_use]
    pub fn repo(&self) -> Option<&str> {
        self.repo.as_deref()
    }

    /// Returns the full commit annotation, when present.
    #[must_use]
    pub fn commit(&self) -> Option<&str> {
        self.commit.as_deref()
    }

    /// Returns the languages declared for this image.
    #[must_use]
    pub fn languages(&self) -> &BTreeSet<String> {
        &self.languages
    }

    /// Returns exact toolchain name-to-version annotations for this image.
    #[must_use]
    pub fn toolchains(&self) -> &BTreeMap<String, String> {
        &self.toolchains
    }

    /// Returns the annotated runtime, when present.
    #[must_use]
    pub fn runtime(&self) -> Option<&str> {
        self.runtime.as_deref()
    }

    /// Returns the annotated architecture, when present.
    #[must_use]
    pub fn architecture(&self) -> Option<&str> {
        self.architecture.as_deref()
    }
}

/// One actual catalog image paired with its trusted host annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageCandidate {
    record: ImageRecord,
    metadata: TrustedImageMetadata,
}

impl ImageCandidate {
    /// Returns the actual Contree image ID supplied by `list_images`.
    #[must_use]
    pub fn image_id(&self) -> &ImageId {
        &self.record.uuid
    }

    /// Returns the actual upstream `list_images` record, including its tag and provenance fields.
    #[must_use]
    pub fn record(&self) -> &ImageRecord {
        &self.record
    }

    /// Returns the trusted host metadata joined to this listed image.
    #[must_use]
    pub fn metadata(&self) -> &TrustedImageMetadata {
        &self.metadata
    }
}

/// Integer score in basis points, where 10,000 represents a score of 100 percent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoredImage {
    image_id: ImageId,
    score_basis_points: u16,
}

impl ScoredImage {
    /// Creates a score associated with an image ID.
    #[must_use]
    pub const fn new(image_id: ImageId, score_basis_points: u16) -> Self {
        Self {
            image_id,
            score_basis_points,
        }
    }

    /// Returns the image ID being scored.
    #[must_use]
    pub fn image_id(&self) -> &ImageId {
        &self.image_id
    }

    /// Returns the integer score in basis points.
    #[must_use]
    pub const fn score_basis_points(&self) -> u16 {
        self.score_basis_points
    }
}

/// Strategy supplied by the caller for a set of equally eligible images.
pub trait ImageRanker {
    /// Scores each candidate exactly once using a stable candidate order.
    ///
    /// # Errors
    /// Returns [`ResolverError::RankerFailure`] when ranking cannot be completed.
    fn rank(
        &self,
        manifest: &RepoManifest,
        candidates: &[ImageCandidate],
    ) -> Result<Vec<ScoredImage>, ResolverError>;
}

/// Why the resolver selected the returned image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchReason {
    /// Trusted metadata matches the repository and full commit, and all requested constraints.
    ExactRepoCommit,
    /// Exactly one candidate satisfies the declared compatibility requirements.
    CompatibleMetadata,
    /// The injected ranker uniquely preferred this eligible candidate.
    Ranked,
}

/// Selected actual image and the evidence category used for the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImage {
    image_id: ImageId,
    reason: MatchReason,
    score_basis_points: Option<u16>,
}

impl ResolvedImage {
    /// Returns the actual ID returned by Contree's image catalog.
    #[must_use]
    pub fn image_id(&self) -> &ImageId {
        &self.image_id
    }

    /// Returns the resolution path that selected this image.
    #[must_use]
    pub const fn reason(&self) -> MatchReason {
        self.reason
    }

    /// Returns the ranker score when ambiguity required ranking.
    #[must_use]
    pub const fn score_basis_points(&self) -> Option<u16> {
        self.score_basis_points
    }
}

/// Typed failures from manifest/catalog validation or candidate selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ResolverError {
    /// Manifest repository, commit, or declared requirements are invalid.
    #[error("invalid repository manifest")]
    InvalidManifest,
    /// Trusted metadata is malformed, duplicated, or not joined to a listed image.
    #[error("invalid image metadata or catalog")]
    InvalidCatalog,
    /// Trusted metadata values are malformed or inconsistent.
    #[error("invalid trusted image metadata")]
    InvalidMetadata,
    /// Catalog or metadata count exceeded the fixed input bound.
    #[error("image catalog exceeds resolver input limit")]
    CatalogLimit,
    /// No image satisfies the identity or compatibility requirements.
    #[error("no image matches the repository or declared environment requirements")]
    NoMatchingImage,
    /// The injected ranker failed to produce scores.
    #[error("image ranker failed")]
    RankerFailure,
    /// Ranker output omitted, duplicated, invented, or invalidly scored an image.
    #[error("image ranker returned invalid scores")]
    InvalidRankerOutput,
    /// The top score was tied, so ranking did not resolve ambiguity.
    #[error("image ranker returned tied highest scores")]
    AmbiguousRankerOutput,
}

/// Resolves an environment using only actual catalog IDs and explicitly trusted annotations.
///
/// Exact repository/commit candidates are filtered by the manifest constraints. If none remain,
/// declared compatibility is considered. An injected ranker is consulted only when multiple
/// eligible candidates remain.
///
/// # Errors
/// Returns a typed error for malformed or oversized input, no compatible candidate, or invalid
/// ranker output.
pub fn resolve_environment<R: ImageRanker + ?Sized>(
    manifest: &RepoManifest,
    images: &[ImageRecord],
    metadata: &[TrustedImageMetadata],
    ranker: &R,
) -> Result<ResolvedImage, ResolverError> {
    if images.len() > MAX_RESOLVER_IMAGES || metadata.len() > MAX_RESOLVER_IMAGES {
        return Err(ResolverError::CatalogLimit);
    }
    validate_requirements(
        &manifest.languages,
        &manifest.toolchains,
        &manifest.runtime,
        &manifest.architecture,
    )
    .map_err(|_| ResolverError::InvalidManifest)?;
    if canonical_repo(manifest.repo.clone())? != manifest.repo
        || canonical_oid(manifest.commit.clone())? != manifest.commit
    {
        return Err(ResolverError::InvalidManifest);
    }

    let mut listed = BTreeMap::new();
    for record in images {
        validate_record(record)?;
        if listed
            .insert(record.uuid.as_str().to_owned(), record)
            .is_some()
        {
            return Err(ResolverError::InvalidCatalog);
        }
    }

    let mut annotations = BTreeMap::new();
    for annotation in metadata {
        validate_annotation(annotation)?;
        let id = annotation.image_id.as_str();
        if !listed.contains_key(id) || annotations.insert(id.to_owned(), annotation).is_some() {
            return Err(ResolverError::InvalidCatalog);
        }
    }

    let mut exact = Vec::new();
    for (id, annotation) in &annotations {
        if annotation.repo.as_deref() == Some(manifest.repo.as_str())
            && annotation.commit.as_deref() == Some(manifest.commit.as_str())
            && is_compatible(manifest, annotation)
        {
            exact.push(candidate_for(id, &listed, &annotations)?);
        }
    }
    if !exact.is_empty() {
        return choose_candidates(manifest, exact, MatchReason::ExactRepoCommit, ranker);
    }

    let has_requirements = !manifest.languages.is_empty()
        || !manifest.toolchains.is_empty()
        || manifest.runtime.is_some()
        || manifest.architecture.is_some();
    if !has_requirements {
        return Err(ResolverError::NoMatchingImage);
    }

    let mut compatible = Vec::new();
    for (id, annotation) in &annotations {
        if is_compatible(manifest, annotation) {
            compatible.push(candidate_for(id, &listed, &annotations)?);
        }
    }
    if compatible.is_empty() {
        return Err(ResolverError::NoMatchingImage);
    }
    choose_candidates(
        manifest,
        compatible,
        MatchReason::CompatibleMetadata,
        ranker,
    )
}

fn choose_candidates<R: ImageRanker + ?Sized>(
    manifest: &RepoManifest,
    mut candidates: Vec<ImageCandidate>,
    unique_reason: MatchReason,
    ranker: &R,
) -> Result<ResolvedImage, ResolverError> {
    candidates.sort_by(|left, right| left.image_id().as_str().cmp(right.image_id().as_str()));
    if candidates.len() == 1 {
        let candidate = candidates.first().ok_or(ResolverError::InvalidCatalog)?;
        return Ok(ResolvedImage {
            image_id: candidate.image_id().clone(),
            reason: unique_reason,
            score_basis_points: None,
        });
    }

    let scores = ranker.rank(manifest, &candidates)?;
    if scores.len() != candidates.len() {
        return Err(ResolverError::InvalidRankerOutput);
    }
    let mut by_id = BTreeMap::new();
    for score in scores {
        validate_image_id(&score.image_id).map_err(|_| ResolverError::InvalidRankerOutput)?;
        if score.score_basis_points > 10_000
            || !candidates
                .iter()
                .any(|candidate| candidate.image_id() == &score.image_id)
            || by_id
                .insert(score.image_id.as_str().to_owned(), score.score_basis_points)
                .is_some()
        {
            return Err(ResolverError::InvalidRankerOutput);
        }
    }

    let mut best: Option<(&str, u16)> = None;
    let mut tied = false;
    for candidate in &candidates {
        let id = candidate.image_id().as_str();
        let score = by_id
            .get(id)
            .copied()
            .ok_or(ResolverError::InvalidRankerOutput)?;
        match best {
            None => {
                best = Some((id, score));
                tied = false;
            }
            Some((_, best_score)) if score > best_score => {
                best = Some((id, score));
                tied = false;
            }
            Some((_, best_score)) if score == best_score => tied = true,
            Some((_, _)) => {}
        }
    }
    if tied {
        return Err(ResolverError::AmbiguousRankerOutput);
    }
    let (id, score) = best.ok_or(ResolverError::InvalidRankerOutput)?;
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.image_id().as_str() == id)
        .ok_or(ResolverError::InvalidRankerOutput)?;
    Ok(ResolvedImage {
        image_id: candidate.image_id().clone(),
        reason: MatchReason::Ranked,
        score_basis_points: Some(score),
    })
}

fn candidate_for<'a>(
    id: &str,
    listed: &BTreeMap<String, &'a ImageRecord>,
    annotations: &BTreeMap<String, &'a TrustedImageMetadata>,
) -> Result<ImageCandidate, ResolverError> {
    let record = listed.get(id).ok_or(ResolverError::InvalidCatalog)?;
    let metadata = annotations.get(id).ok_or(ResolverError::InvalidCatalog)?;
    Ok(ImageCandidate {
        record: (*record).clone(),
        metadata: (*metadata).clone(),
    })
}

fn is_compatible(manifest: &RepoManifest, metadata: &TrustedImageMetadata) -> bool {
    manifest.languages.is_subset(&metadata.languages)
        && manifest
            .toolchains
            .iter()
            .all(|(name, version)| metadata.toolchains.get(name) == Some(version))
        && option_matches(&manifest.runtime, &metadata.runtime)
        && option_matches(&manifest.architecture, &metadata.architecture)
}

fn option_matches(requested: &Option<String>, actual: &Option<String>) -> bool {
    match requested {
        Some(value) => actual.as_ref() == Some(value),
        None => true,
    }
}

fn validate_record(record: &ImageRecord) -> Result<(), ResolverError> {
    validate_image_id(&record.uuid)?;
    validate_bounded_text(&record.created_at, MAX_ATOM_BYTES)?;
    if let Some(tag) = &record.tag {
        validate_bounded_text(tag, MAX_ATOM_BYTES)?;
    }
    if let Some(operation_id) = &record.operation_uuid {
        validate_bounded_text(operation_id, MAX_ATOM_BYTES)?;
    }
    Ok(())
}

fn validate_annotation(annotation: &TrustedImageMetadata) -> Result<(), ResolverError> {
    validate_image_id(&annotation.image_id)?;
    match (&annotation.repo, &annotation.commit) {
        (Some(repo), Some(commit)) => {
            if canonical_repo(repo.clone()).map_err(|_| ResolverError::InvalidCatalog)? != *repo
                || canonical_oid(commit.clone()).map_err(|_| ResolverError::InvalidCatalog)?
                    != *commit
            {
                return Err(ResolverError::InvalidCatalog);
            }
        }
        (None, None) => {}
        _ => return Err(ResolverError::InvalidCatalog),
    }
    validate_requirements(
        &annotation.languages,
        &annotation.toolchains,
        &annotation.runtime,
        &annotation.architecture,
    )
    .map_err(|_| ResolverError::InvalidCatalog)
}

fn validate_image_id(image_id: &ImageId) -> Result<(), ResolverError> {
    ImageId::new(image_id.as_str().to_owned()).map_err(|_| ResolverError::InvalidCatalog)?;
    Ok(())
}

fn validate_requirements(
    languages: &BTreeSet<String>,
    toolchains: &BTreeMap<String, String>,
    runtime: &Option<String>,
    architecture: &Option<String>,
) -> Result<(), ResolverError> {
    if languages.len() > 32 || toolchains.len() > 32 {
        return Err(ResolverError::InvalidManifest);
    }
    for language in languages {
        validate_atom(language)?;
    }
    for (name, version) in toolchains {
        validate_atom(name)?;
        validate_atom(version)?;
    }
    if let Some(runtime) = runtime {
        validate_atom(runtime)?;
    }
    if let Some(architecture) = architecture {
        validate_atom(architecture)?;
    }
    Ok(())
}

fn validate_atom(value: &str) -> Result<(), ResolverError> {
    if value.is_empty()
        || value.len() > MAX_ATOM_BYTES
        || value.chars().any(|character| {
            character.is_control() || character.is_whitespace() || !character.is_ascii()
        })
    {
        return Err(ResolverError::InvalidManifest);
    }
    Ok(())
}

fn validate_bounded_text(value: &str, max_bytes: usize) -> Result<(), ResolverError> {
    if value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(ResolverError::InvalidCatalog);
    }
    Ok(())
}

fn canonical_repo(repo: String) -> Result<String, ResolverError> {
    if repo.is_empty() || repo.len() > MAX_REPO_BYTES || repo.contains([':', '@', '?', '#', '%']) {
        return Err(ResolverError::InvalidManifest);
    }
    let mut parts = repo.split('/');
    let owner = parts.next().ok_or(ResolverError::InvalidManifest)?;
    let name = parts.next().ok_or(ResolverError::InvalidManifest)?;
    if parts.next().is_some() || !valid_repo_component(owner) || !valid_repo_component(name) {
        return Err(ResolverError::InvalidManifest);
    }
    Ok(repo)
}

fn valid_repo_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= MAX_ATOM_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

fn canonical_oid(mut oid: String) -> Result<String, ResolverError> {
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ResolverError::InvalidManifest);
    }
    oid.make_ascii_lowercase();
    if oid.len() > MAX_GIT_OID_BYTES {
        return Err(ResolverError::InvalidManifest);
    }
    Ok(oid)
}
