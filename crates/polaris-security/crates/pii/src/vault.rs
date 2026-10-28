//! Bounded DFA/entropy masking and session-local reversible vault.
mod snapshot;

use std::{
    collections::HashMap,
    fmt, str,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use polaris_sdk::{
    ids::{FrameId, SessionId},
    registry::{RedactStats, Redactor, SandboxHandle},
    system_one::{PiiEntitySpan, PiiSpanDetector},
};
use polaris_security_automata::{Category, DfaScanner, EntropyCfg, ScanBuf, high_entropy_spans};
use zeroize::Zeroizing;

use crate::PiiError;

const FAILURE_MARKER: &[u8] = b"[PII_REDACTION_FAILED]";
const TOKEN_PREFIX: &str = "<PII1:";
const MAX_ISSUED_TOKEN_BYTES: usize = TOKEN_PREFIX.len() + 32 + 1 + 20 + 1 + 5 + 1;
const HARD_MAX_INPUT_BYTES: usize = 16_777_216;
const HARD_MAX_OUTPUT_BYTES: usize = 33_554_432;
const HARD_MAX_MATCHES: usize = 65_536;
const HARD_MAX_ENTRIES: usize = 100_000;
const HARD_MAX_SECRET_BYTES: usize = 67_108_864;

/// PII classification for deterministic rule matches and entropy candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PiiKind {
    /// An email address matched by the default DFA.
    Email,
    /// An IPv4 address matched by the default DFA.
    Ip,
    /// A credential, key, or high-entropy candidate.
    ApiKey,
    /// Other personal or secret-looking data matched by the configured DFA.
    Other,
}

impl PiiKind {
    const fn marker_code(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Ip => "ip",
            Self::ApiKey => "key",
            Self::Other => "other",
        }
    }
}

/// Hard bounds for one mask operation and the session-local reversible map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PiiLimits {
    /// Maximum UTF-8 input bytes per call.
    pub max_input_bytes: usize,
    /// Maximum UTF-8 output bytes after mask or unmask expansion.
    pub max_output_bytes: usize,
    /// Maximum matches retained from each scanner pass.
    pub max_matches_per_input: usize,
    /// Maximum distinct values kept in the session vault.
    pub max_entries: usize,
    /// Maximum total UTF-8 secret bytes retained in the session vault.
    pub max_secret_bytes: usize,
}

impl Default for PiiLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 1_048_576,
            max_output_bytes: 2_097_152,
            max_matches_per_input: 4_096,
            max_entries: 10_000,
            max_secret_bytes: 4_194_304,
        }
    }
}

/// Honest coverage and counts for one masking operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizeReport {
    /// Number of matched occurrences replaced, including repeated occurrences of one value.
    pub items_masked: u32,
    /// Occurrence counts by deterministic category.
    pub counts: Vec<(PiiKind, u32)>,
    /// True only if a semantic PII NER model produced trusted character spans.
    pub ner_used: bool,
    /// True when NER is unavailable or any redactor failure required fail-closed replacement.
    pub degraded: bool,
}

/// Redacted text and actual scan counts. The report never contains matched values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Masked {
    /// Text with every deterministic match replaced by a session marker.
    pub text: String,
    /// Counts and coverage status for the operation.
    pub report: SanitizeReport,
}

/// Trusted composition-root policy for unmasking into one sandbox.
///
/// The method receives the vault's bound session and frame plus the requested sandbox. Returning
/// true is an authority decision and must only be implemented by the trusted execution boundary.
pub trait SandboxUnmaskAuthorizer: Send + Sync {
    /// Whether this exact vault binding may unmask for `sandbox`.
    fn permits(&self, session: SessionId, frame: FrameId, sandbox: &SandboxHandle) -> bool;
}

/// Authorizer that denies every unmask request.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenySandboxUnmask;

impl SandboxUnmaskAuthorizer for DenySandboxUnmask {
    fn permits(&self, _session: SessionId, _frame: FrameId, _sandbox: &SandboxHandle) -> bool {
        false
    }
}

/// Opaque sealed bytes supplied by an injected sealer. This crate provides no disk adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedBlob(Vec<u8>);

impl SealedBlob {
    /// Construct a blob returned by a real encryption/sealing backend.
    #[must_use]
    pub fn from_ciphertext(ciphertext: Vec<u8>) -> Self {
        Self(ciphertext)
    }

    /// Read the opaque ciphertext for transport or persistence.
    #[must_use]
    pub fn ciphertext(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SealedBlob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedBlob")
            .field("ciphertext_bytes", &self.0.len())
            .finish()
    }
}

/// Composition-root sealing port. No plaintext persistence is implemented by this crate.
pub trait Sealer: Send + Sync {
    /// Seal plaintext using a vetted, authenticated-encryption implementation.
    fn seal(&self, plaintext: &[u8]) -> Result<SealedBlob, PiiError>;

    /// Open an authenticated blob into zeroizing temporary memory.
    fn open(&self, blob: &SealedBlob) -> Result<Zeroizing<Vec<u8>>, PiiError>;
}

#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
    kind: PiiKind,
}

struct VaultEntry {
    secret: Zeroizing<String>,
    token: String,
}

#[derive(Default)]
struct VaultState {
    entries: Vec<VaultEntry>,
    token_to_entry: HashMap<String, usize>,
    next_token: u64,
    secret_bytes: usize,
}

/// Session/frame-bound, bounded PII vault. Secrets are held only in zeroizing RAM.
pub struct PiiVault {
    session: SessionId,
    frame: FrameId,
    session_tag: String,
    scanner: DfaScanner,
    entropy: EntropyCfg,
    limits: PiiLimits,
    state: Mutex<VaultState>,
    redactor_failures: AtomicU64,
    last_redactor_failed: AtomicBool,
}

impl fmt::Debug for PiiVault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PiiVault")
            .field("session", &self.session)
            .field("frame", &self.frame)
            .field("session_tag", &"[REDACTED]")
            .field("entry_count", &self.entry_count())
            .field("redactor_failures", &self.redactor_failures())
            .finish()
    }
}

impl PiiVault {
    /// Whether this vault belongs to the exact trusted host session/frame identity.
    #[must_use]
    pub fn belongs_to(&self, session: SessionId, frame: FrameId) -> bool {
        self.session == session && self.frame == frame
    }
    /// Current immutable entropy policy; no vaulted values are exposed.
    #[must_use]
    pub const fn entropy_policy(&self) -> EntropyCfg {
        self.entropy
    }
    /// Current immutable input, output and map capacity limits.
    #[must_use]
    pub const fn limits(&self) -> PiiLimits {
        self.limits
    }
    /// Build the default secret+PII DFA, bind it to a session/frame, and generate a random marker
    /// tag. Name and organization NER is unavailable because the SDK decision API has no span
    /// output; every mask report therefore marks coverage degraded while DFA and entropy run.
    ///
    /// # Errors
    /// Returns an error if scanner construction or secure random generation fails.
    pub fn new(
        session: SessionId,
        frame: FrameId,
        entropy: EntropyCfg,
        limits: PiiLimits,
    ) -> Result<Self, PiiError> {
        if limits.max_input_bytes == 0
            || limits.max_output_bytes == 0
            || limits.max_matches_per_input == 0
            || limits.max_entries == 0
            || limits.max_secret_bytes == 0
            || limits.max_input_bytes > HARD_MAX_INPUT_BYTES
            || limits.max_output_bytes > HARD_MAX_OUTPUT_BYTES
            || limits.max_matches_per_input > HARD_MAX_MATCHES
            || limits.max_entries > HARD_MAX_ENTRIES
            || limits.max_secret_bytes > HARD_MAX_SECRET_BYTES
        {
            return Err(PiiError::Capacity);
        }
        let scanner = DfaScanner::default_pack_for(&[Category::Secret, Category::Pii])?;
        let mut tag = [0_u8; 16];
        getrandom::fill(&mut tag).map_err(|_| PiiError::RandomUnavailable)?;
        Ok(Self {
            session,
            frame,
            session_tag: hex_tag(&tag),
            scanner,
            entropy,
            limits,
            state: Mutex::new(VaultState::default()),
            redactor_failures: AtomicU64::new(0),
            last_redactor_failed: AtomicBool::new(false),
        })
    }

    /// Mask DFA and high-entropy matches in one bounded left-to-right replacement pass.
    ///
    /// Equal matched values reuse their session token. Capacity exhaustion is an error and does
    /// not evict prior secrets. Existing markers contain no DFA-sensitive patterns and are left
    /// unchanged, making repeated masking idempotent for the default marker format.
    pub fn mask(&self, text: &str) -> Result<Masked, PiiError> {
        if text.len() > self.limits.max_input_bytes {
            return Err(PiiError::InputLimit);
        }
        let spans = self.scan_spans(text.as_bytes())?;
        let mut state = self.state.lock().map_err(|_| PiiError::StateUnavailable)?;
        let masked = self.mask_locked(text, spans, &mut state)?;
        self.last_redactor_failed.store(false, Ordering::Relaxed);
        Ok(masked)
    }

    /// Runs bounded semantic PII detection before applying both NER and deterministic spans.
    ///
    /// NER receives the exact raw UTF-8 input, with no Decide normalization. Detector errors,
    /// timeouts, and malformed model ranges fall back to DFA and entropy masking; that result is
    /// marked degraded. A successful empty result is distinct and reports NER coverage as used.
    /// The detector's future is dropped on its configured timeout; cancellation by the caller
    /// cancels this operation and produces no masking result.
    ///
    /// # Errors
    /// Returns a vault capacity/scanner failure. NER availability failures are represented in
    /// `Masked.report` and do not bypass deterministic masking.
    pub async fn mask_with_ner<D: PiiSpanDetector + ?Sized>(
        &self,
        detector: &D,
        text: &str,
        timeout: std::time::Duration,
    ) -> Result<Masked, PiiError> {
        if text.len() > self.limits.max_input_bytes {
            return Err(PiiError::InputLimit);
        }
        if timeout.is_zero() || timeout > std::time::Duration::from_secs(120) {
            return self.mask(text);
        }
        let result = tokio::time::timeout(timeout, detector.detect_pii_spans(text)).await;
        match result {
            Ok(Ok(spans)) => match self.mask_with_ner_spans(text, &spans) {
                Ok(masked) => Ok(masked),
                Err(PiiError::InvalidNerSpans) => self.mask(text),
                Err(error) => Err(error),
            },
            Ok(Err(_)) | Err(_) => self.mask(text),
        }
    }

    /// Applies already detected semantic PII spans with the deterministic scanner in one vault
    /// transaction. Spans must be sorted, non-overlapping, and valid UTF-8 byte ranges.
    ///
    /// # Errors
    /// Returns [`PiiError::InvalidNerSpans`] before changing the vault for malformed spans.
    pub fn mask_with_ner_spans(
        &self,
        text: &str,
        ner_spans: &[PiiEntitySpan],
    ) -> Result<Masked, PiiError> {
        if text.len() > self.limits.max_input_bytes {
            return Err(PiiError::InputLimit);
        }
        if ner_spans.len() > self.limits.max_matches_per_input {
            return Err(PiiError::InvalidNerSpans);
        }
        let mut spans = self.scan_spans(text.as_bytes())?;
        let merged_len = spans
            .len()
            .checked_add(ner_spans.len())
            .ok_or(PiiError::CounterOverflow)?;
        if merged_len > self.limits.max_matches_per_input {
            return Err(PiiError::InvalidNerSpans);
        }
        spans
            .try_reserve(ner_spans.len())
            .map_err(|_| PiiError::Capacity)?;
        let mut previous_end = 0_usize;
        let mut validated_ner = Vec::<Span>::new();
        validated_ner
            .try_reserve(ner_spans.len())
            .map_err(|_| PiiError::Capacity)?;
        for candidate in ner_spans {
            let start =
                usize::try_from(candidate.start()).map_err(|_| PiiError::InvalidNerSpans)?;
            let end = usize::try_from(candidate.end()).map_err(|_| PiiError::InvalidNerSpans)?;
            if start < previous_end
                || end > text.len()
                || text.get(start..end).is_none()
                || !candidate.confidence().is_finite()
                || !(0.0..=1.0).contains(&candidate.confidence())
            {
                return Err(PiiError::InvalidNerSpans);
            }
            validated_ner.push(Span {
                start,
                end,
                kind: PiiKind::Other,
            });
            previous_end = end;
        }
        let mut state = self.state.lock().map_err(|_| PiiError::StateUnavailable)?;
        let issued_marker_ranges =
            collect_issued_marker_ranges(text, &state, self.limits.max_matches_per_input)?;
        for span in validated_ner {
            let overlaps_issued_marker = issued_marker_ranges
                .iter()
                .any(|marker| span.start < marker.1 && marker.0 < span.end);
            if !overlaps_issued_marker {
                spans.push(span);
            }
        }
        spans.sort_by_key(|span| (span.start, span.end));
        let spans = merge_spans(spans)?;
        let mut masked = self.mask_locked(text, spans, &mut state)?;
        masked.report.ner_used = true;
        masked.report.degraded = false;
        self.last_redactor_failed.store(false, Ordering::Relaxed);
        Ok(masked)
    }

    /// Restore only exact markers issued by this vault, and only after explicit sandbox authority.
    ///
    /// The result is zeroizing and must be kept ephemeral by its caller. This operation does not
    /// authorize model, log, or network output.
    pub fn unmask_for_sandbox<A: SandboxUnmaskAuthorizer + ?Sized>(
        &self,
        text: &str,
        sandbox: &SandboxHandle,
        authorizer: &A,
    ) -> Result<Zeroizing<String>, PiiError> {
        if text.len() > self.limits.max_input_bytes {
            return Err(PiiError::InputLimit);
        }
        if !authorizer.permits(self.session, self.frame, sandbox) {
            return Err(PiiError::BoundaryDenied);
        }
        let state = self.state.lock().map_err(|_| PiiError::StateUnavailable)?;
        unmask_issued_tokens(text, &state, self.limits.max_output_bytes)
    }

    /// Number of reversible values currently retained, without exposing their contents.
    pub fn entry_count(&self) -> Result<usize, PiiError> {
        self.state
            .lock()
            .map(|state| state.entries.len())
            .map_err(|_| PiiError::StateUnavailable)
    }

    /// Number of times the infallible SDK redactor had to fail closed.
    #[must_use]
    pub fn redactor_failures(&self) -> u64 {
        self.redactor_failures.load(Ordering::Relaxed)
    }

    /// Whether the most recent SDK redactor call used its opaque whole-input failure marker.
    #[must_use]
    pub fn last_redactor_failed(&self) -> bool {
        self.last_redactor_failed.load(Ordering::Relaxed)
    }

    fn scan_spans(&self, input: &[u8]) -> Result<Vec<Span>, PiiError> {
        let mut rule_matches = ScanBuf::with_capacity(self.limits.max_matches_per_input);
        self.scanner.scan(input, &mut rule_matches)?;
        let mut entropy_matches = ScanBuf::with_capacity(self.limits.max_matches_per_input);
        high_entropy_spans(input, &self.entropy, &mut entropy_matches)?;
        let total = rule_matches
            .len()
            .checked_add(entropy_matches.len())
            .ok_or(PiiError::CounterOverflow)?;
        if total > self.limits.max_matches_per_input {
            return Err(PiiError::Capacity);
        }
        let mut spans = Vec::new();
        spans.try_reserve(total).map_err(|_| PiiError::Capacity)?;
        for matched in rule_matches
            .matches()
            .iter()
            .chain(entropy_matches.matches())
        {
            let start = usize::try_from(matched.start).map_err(|_| PiiError::CounterOverflow)?;
            let end = usize::try_from(matched.end).map_err(|_| PiiError::CounterOverflow)?;
            let kind = if matched.rule == polaris_security_automata::RuleId::ENTROPY {
                PiiKind::ApiKey
            } else {
                match self.scanner.rule_category(matched.rule) {
                    Some(Category::Secret) => PiiKind::ApiKey,
                    Some(Category::Pii) => match self.scanner.rule_name(matched.rule) {
                        Some("email") => PiiKind::Email,
                        Some("ipv4") => PiiKind::Ip,
                        Some(_) | None => PiiKind::Other,
                    },
                    Some(Category::DangerousShell) | None => PiiKind::Other,
                }
            };
            spans.push(Span { start, end, kind });
        }
        spans.sort_by_key(|span| (span.start, span.end));
        merge_spans(spans)
    }

    fn mask_locked(
        &self,
        text: &str,
        spans: Vec<Span>,
        state: &mut VaultState,
    ) -> Result<Masked, PiiError> {
        let mut output = String::new();
        output
            .try_reserve(text.len())
            .map_err(|_| PiiError::Capacity)?;
        let mut pending = Vec::<VaultEntry>::new();
        let mut counts = Vec::<(PiiKind, u32)>::new();
        let mut items_masked = 0_u32;
        let mut cursor = 0_usize;

        for span in spans {
            if span.start < cursor {
                continue;
            }
            let value = text.get(span.start..span.end).ok_or(PiiError::InputLimit)?;
            append_bounded(
                &mut output,
                text.get(cursor..span.start).ok_or(PiiError::InputLimit)?,
                self.limits.max_output_bytes,
            )?;
            let existing_token = state
                .entries
                .iter()
                .find(|entry| entry.secret.as_str() == value)
                .map(|entry| entry.token.clone())
                .or_else(|| {
                    pending
                        .iter()
                        .find(|entry| entry.secret.as_str() == value)
                        .map(|entry| entry.token.clone())
                });
            let token = match existing_token {
                Some(token) => token,
                None => {
                    let next_id = state
                        .next_token
                        .checked_add(
                            u64::try_from(pending.len()).map_err(|_| PiiError::CounterOverflow)?,
                        )
                        .ok_or(PiiError::CounterOverflow)?;
                    let token = self.unique_marker(text, span.kind, next_id, state, &pending)?;
                    let mut secret = Zeroizing::new(String::new());
                    secret
                        .try_reserve(value.len())
                        .map_err(|_| PiiError::Capacity)?;
                    secret.push_str(value);
                    pending.try_reserve(1).map_err(|_| PiiError::Capacity)?;
                    pending.push(VaultEntry {
                        secret,
                        token: token.clone(),
                    });
                    token
                }
            };
            append_bounded(&mut output, &token, self.limits.max_output_bytes)?;
            items_masked = items_masked
                .checked_add(1)
                .ok_or(PiiError::CounterOverflow)?;
            increment_count(&mut counts, span.kind)?;
            cursor = span.end;
        }
        append_bounded(
            &mut output,
            text.get(cursor..).ok_or(PiiError::InputLimit)?,
            self.limits.max_output_bytes,
        )?;

        let added_secret_bytes = pending.iter().try_fold(0_usize, |sum, entry| {
            sum.checked_add(entry.secret.len())
                .ok_or(PiiError::CounterOverflow)
        })?;
        let new_count = state
            .entries
            .len()
            .checked_add(pending.len())
            .ok_or(PiiError::CounterOverflow)?;
        let new_secret_bytes = state
            .secret_bytes
            .checked_add(added_secret_bytes)
            .ok_or(PiiError::CounterOverflow)?;
        let pending_count = u64::try_from(pending.len()).map_err(|_| PiiError::CounterOverflow)?;
        let _final_token_counter = state
            .next_token
            .checked_add(pending_count)
            .ok_or(PiiError::CounterOverflow)?;
        if new_count > self.limits.max_entries || new_secret_bytes > self.limits.max_secret_bytes {
            return Err(PiiError::Capacity);
        }
        state
            .entries
            .try_reserve(pending.len())
            .map_err(|_| PiiError::Capacity)?;
        state
            .token_to_entry
            .try_reserve(pending.len())
            .map_err(|_| PiiError::Capacity)?;
        for entry in pending {
            let index = state.entries.len();
            state.next_token = state
                .next_token
                .checked_add(1)
                .ok_or(PiiError::CounterOverflow)?;
            state.secret_bytes = state
                .secret_bytes
                .checked_add(entry.secret.len())
                .ok_or(PiiError::CounterOverflow)?;
            state.token_to_entry.insert(entry.token.clone(), index);
            state.entries.push(entry);
        }
        Ok(Masked {
            text: output,
            report: SanitizeReport {
                items_masked,
                counts,
                ner_used: false,
                degraded: true,
            },
        })
    }

    fn unique_marker(
        &self,
        input: &str,
        kind: PiiKind,
        mut id: u64,
        state: &VaultState,
        pending: &[VaultEntry],
    ) -> Result<String, PiiError> {
        loop {
            let marker = format!("<PII1:{}:{}:{}>", self.session_tag, id, kind.marker_code());
            let exists = state.token_to_entry.contains_key(&marker)
                || pending.iter().any(|entry| entry.token == marker);
            if !input.contains(&marker) && !exists {
                return Ok(marker);
            }
            id = id.checked_add(1).ok_or(PiiError::CounterOverflow)?;
        }
    }
}

impl Redactor for PiiVault {
    /// The sdk redactor cannot return an error. On any failure it drops the whole input and emits
    /// an opaque failure marker; it never copies potentially sensitive bytes through.
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        let bytes_in = input.len();
        let result = str::from_utf8(input)
            .map_err(|_| PiiError::InvalidUtf8)
            .and_then(|text| self.mask(text));
        match result {
            Ok(masked) => {
                out.extend_from_slice(masked.text.as_bytes());
                self.last_redactor_failed.store(false, Ordering::Relaxed);
                RedactStats {
                    bytes_in,
                    replacements: masked.report.items_masked,
                }
            }
            Err(_) => {
                out.extend_from_slice(FAILURE_MARKER);
                self.last_redactor_failed.store(true, Ordering::Relaxed);
                let _previous = self.redactor_failures.try_update(
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                    |count| count.checked_add(1),
                );
                RedactStats {
                    bytes_in,
                    replacements: 0,
                }
            }
        }
    }
}

fn merge_spans(mut spans: Vec<Span>) -> Result<Vec<Span>, PiiError> {
    let mut merged = Vec::<Span>::new();
    merged
        .try_reserve(spans.len())
        .map_err(|_| PiiError::Capacity)?;
    for span in spans.drain(..) {
        if let Some(previous) = merged.last_mut()
            && span.start < previous.end
        {
            previous.end = previous.end.max(span.end);
            if span.kind == PiiKind::ApiKey {
                previous.kind = PiiKind::ApiKey;
            }
            continue;
        }
        merged.push(span);
    }
    Ok(merged)
}

fn increment_count(counts: &mut Vec<(PiiKind, u32)>, kind: PiiKind) -> Result<(), PiiError> {
    if let Some((_, count)) = counts.iter_mut().find(|(present, _)| *present == kind) {
        *count = count.checked_add(1).ok_or(PiiError::CounterOverflow)?;
    } else {
        counts.try_reserve(1).map_err(|_| PiiError::Capacity)?;
        counts.push((kind, 1));
    }
    Ok(())
}

fn append_bounded(output: &mut String, value: &str, maximum: usize) -> Result<(), PiiError> {
    let new_len = output
        .len()
        .checked_add(value.len())
        .ok_or(PiiError::CounterOverflow)?;
    if new_len > maximum {
        return Err(PiiError::Capacity);
    }
    output
        .try_reserve(value.len())
        .map_err(|_| PiiError::Capacity)?;
    output.push_str(value);
    Ok(())
}

fn collect_issued_marker_ranges(
    text: &str,
    state: &VaultState,
    maximum_occurrences: usize,
) -> Result<Vec<(usize, usize)>, PiiError> {
    let mut ranges = Vec::<(usize, usize)>::new();
    let mut cursor = 0_usize;
    let mut occurrences = 0_usize;
    while let Some(relative_start) = text
        .get(cursor..)
        .ok_or(PiiError::InputLimit)?
        .find(TOKEN_PREFIX)
    {
        let start = cursor
            .checked_add(relative_start)
            .ok_or(PiiError::CounterOverflow)?;
        occurrences = occurrences
            .checked_add(1)
            .ok_or(PiiError::CounterOverflow)?;
        if occurrences > maximum_occurrences {
            return Err(PiiError::Capacity);
        }
        let marker_tail = text.get(start..).ok_or(PiiError::InputLimit)?;
        let marker_end = marker_tail
            .char_indices()
            .take_while(|(offset, _)| *offset < MAX_ISSUED_TOKEN_BYTES)
            .find(|(_, character)| *character == '>')
            .map(|(offset, character)| {
                offset
                    .checked_add(character.len_utf8())
                    .and_then(|length| start.checked_add(length))
                    .ok_or(PiiError::CounterOverflow)
            })
            .transpose()?;
        if let Some(end) = marker_end {
            let candidate = text.get(start..end).ok_or(PiiError::InputLimit)?;
            if state.token_to_entry.contains_key(candidate) {
                ranges.try_reserve(1).map_err(|_| PiiError::Capacity)?;
                ranges.push((start, end));
            }
        }
        cursor = start
            .checked_add(TOKEN_PREFIX.len())
            .ok_or(PiiError::CounterOverflow)?;
    }
    Ok(ranges)
}

fn unmask_issued_tokens(
    text: &str,
    state: &VaultState,
    maximum: usize,
) -> Result<Zeroizing<String>, PiiError> {
    let mut output = Zeroizing::new(String::new());
    output
        .try_reserve(text.len())
        .map_err(|_| PiiError::Capacity)?;
    let mut cursor = 0_usize;
    while let Some(relative_start) = text
        .get(cursor..)
        .ok_or(PiiError::InputLimit)?
        .find(TOKEN_PREFIX)
    {
        let start = cursor
            .checked_add(relative_start)
            .ok_or(PiiError::CounterOverflow)?;
        let token_tail = text.get(start..).ok_or(PiiError::InputLimit)?;
        let Some(relative_end) = token_tail.find('>') else {
            append_bounded(
                &mut output,
                text.get(cursor..).ok_or(PiiError::InputLimit)?,
                maximum,
            )?;
            return Ok(output);
        };
        let end = start
            .checked_add(relative_end)
            .and_then(|value| value.checked_add(1))
            .ok_or(PiiError::CounterOverflow)?;
        let token = text.get(start..end).ok_or(PiiError::InputLimit)?;
        let Some(entry_index) = state.token_to_entry.get(token).copied() else {
            let copy_end = start.checked_add(1).ok_or(PiiError::CounterOverflow)?;
            append_bounded(
                &mut output,
                text.get(cursor..copy_end).ok_or(PiiError::InputLimit)?,
                maximum,
            )?;
            cursor = copy_end;
            continue;
        };
        let entry = state
            .entries
            .get(entry_index)
            .ok_or(PiiError::StateUnavailable)?;
        append_bounded(
            &mut output,
            text.get(cursor..start).ok_or(PiiError::InputLimit)?,
            maximum,
        )?;
        append_bounded(&mut output, entry.secret.as_str(), maximum)?;
        cursor = end;
    }
    append_bounded(
        &mut output,
        text.get(cursor..).ok_or(PiiError::InputLimit)?,
        maximum,
    )?;
    Ok(output)
}

fn hex_tag(tag: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(32);
    for byte in tag {
        let high = usize::from(*byte >> 4);
        let low = usize::from(*byte & 0x0f);
        if let (Some(high), Some(low)) = (HEX.get(high), HEX.get(low)) {
            output.push(char::from(*high));
            output.push(char::from(*low));
        }
    }
    output
}
