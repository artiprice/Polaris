//! Concrete durable checkpoint storage using the shared content-addressed blob store.
use crate::{Checkpoint, ResearchError, ResearchState};
use polaris_sdk::ids::Cid;
use polaris_storage_cas::{
    Cas, CasConfig, CasError,
    atomic::{RetryPolicy, write_atomic},
    content_id,
};
use std::fs;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

const POINTER_HEADER: &[u8] = b"PRSM\x01";
const MAX_POINTER_BYTES: u64 = 80;
const LEDGER_POINTER_HEADER: &[u8] = b"PRSL\x01";
const LEDGER_NODE_HEADER: &[u8] = b"PRLE\x01";
const MAX_LEDGER_ENTRIES: usize = 100_000;
const MAX_EVIDENCE_BYTES: u64 = 32 * 1024 * 1024;
const PROGRAM_HEADER: &[u8] = b"PRSP\x01";

/// Local single-program checkpoint store. Checkpoint blobs live in CAS; atomic pointers select
/// the current checkpoint and evidence-ledger head. Independent writers serialize per root.
#[derive(Debug)]
pub struct CheckpointStore {
    program: Cid,
    root: PathBuf,
    cas: Cas,
}

/// A reopened state and its monotone store generation.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredCheckpoint {
    generation: u64,
    state: ResearchState,
}

/// Kind of durable evidence linked into a research run's append-only ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LedgerKind {
    /// Content-addressed result returned by an authorized SDK tool.
    Tool = 1,
    /// Raw stdout/stderr and status of the immutable metric harness.
    MetricRun = 2,
    /// VCS keep or revert receipt.
    Mutation = 3,
    /// VCS recovery receipt after an unsuccessful or interrupted experiment.
    Recovery = 4,
}

impl LedgerKind {
    const fn wire_value(self) -> u8 {
        match self {
            Self::Tool => 1,
            Self::MetricRun => 2,
            Self::Mutation => 3,
            Self::Recovery => 4,
        }
    }
}

/// One verified link in the durable, append-only evidence ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LedgerEntry {
    /// Experiment ordinal associated with this evidence.
    pub experiment: u32,
    /// Evidence category.
    pub kind: LedgerKind,
    /// Content ID of the evidence blob or receipt.
    pub evidence: Cid,
}

impl StoredCheckpoint {
    /// Store generation, starting at one.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// State restored against the same immutable program. In-flight work is marked for recovery.
    #[must_use]
    pub const fn state(&self) -> &ResearchState {
        &self.state
    }

    /// Take ownership of the restored state.
    #[must_use]
    pub fn into_state(self) -> ResearchState {
        self.state
    }
}

impl CheckpointStore {
    /// Open a store root reserved for this immutable program, creating its directories if needed.
    ///
    /// Each program must use its own root directory. The program marker prevents accidental reuse
    /// of the same root for a different program; writer locks coordinate any handles using it.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the state root, program directory, or CAS store cannot be opened.
    pub fn open(root: &Path, program: Cid) -> Result<Self, StoreError> {
        fs::create_dir_all(root)?;
        let root = root.to_path_buf();
        let lock = writer_lock(&root)?;
        bind_program(&root, program)?;
        drop(lock);
        let cas = Cas::open(CasConfig::new(root.join("cas")))?;
        Ok(Self { program, root, cas })
    }

    /// Acquire the exclusive lifetime lease for an active research loop.
    ///
    /// Store handles remain independently readable and may still serialize individual writes;
    /// this lease prevents two orchestration loops from advancing one logical run concurrently.
    ///
    /// # Errors
    /// Returns [`StoreError::RunAlreadyActive`] when another loop owns this run, or a filesystem
    /// error when the owner lock cannot be opened or acquired.
    pub(crate) fn acquire_run_owner(&self) -> Result<RunOwnerLease, StoreError> {
        let path = self.root.join("run-owner.lock");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(StoreError::RunLock);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreError::Io(error)),
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        if !file.metadata()?.is_file() {
            return Err(StoreError::RunLock);
        }
        match file.try_lock() {
            Ok(()) => Ok(RunOwnerLease { _file: file }),
            Err(TryLockError::WouldBlock) => Err(StoreError::RunAlreadyActive),
            Err(TryLockError::Error(error)) => Err(StoreError::Io(error)),
        }
    }

    /// Persist an immutable checkpoint and atomically select it as latest.
    ///
    /// The old checkpoint remains available in CAS if pointer publication fails.
    /// This trusted storage primitive serializes writes but does not acquire the research-loop
    /// lifetime owner lease. Embedders must not call it to mutate an active loop's state.
    ///
    /// # Errors
    /// Returns [`StoreError`] on encoding, storage, identity, or filesystem failure.
    pub fn save(&self, state: &ResearchState) -> Result<u64, StoreError> {
        if state.program() != self.program {
            return Err(StoreError::ProgramChanged);
        }
        let _lock = writer_lock(&self.root)?;
        let current = self.read_pointer()?;
        self.save_locked(state, current)
    }

    /// Publish the initial checkpoint only when no checkpoint has been selected yet.
    ///
    /// This check and publication use the same writer lock, so direct store writers cannot place
    /// a checkpoint between a caller's existence check and the fresh-run save.
    ///
    /// # Errors
    /// Returns [`StoreError::CheckpointExists`] when a checkpoint already exists, or a storage,
    /// identity, or encoding error.
    pub(crate) fn initialize(&self, state: &ResearchState) -> Result<u64, StoreError> {
        if state.program() != self.program {
            return Err(StoreError::ProgramChanged);
        }
        let _lock = writer_lock(&self.root)?;
        let current = self.read_pointer()?;
        if current.is_some() {
            return Err(StoreError::CheckpointExists);
        }
        self.save_locked(state, current)
    }

    fn save_locked(
        &self,
        state: &ResearchState,
        current: Option<(u64, Cid, Cid)>,
    ) -> Result<u64, StoreError> {
        if current.is_some_and(|(_, _, program)| program != self.program) {
            return Err(StoreError::ProgramChanged);
        }
        let generation = current.as_ref().map_or(Ok(1), |(generation, _, _)| {
            generation.checked_add(1).ok_or(StoreError::Generation)
        })?;
        let bytes = state.checkpoint().encode()?;
        let checkpoint_cid = self.cas.put(&bytes)?;
        let pointer = encode_pointer(generation, checkpoint_cid, self.program);
        write_atomic(
            &self.root.join("current"),
            &pointer,
            &RetryPolicy::default(),
        )?;
        Ok(generation)
    }

    /// Persist bounded raw experiment evidence and return its canonical content ID.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the evidence exceeds the CAS bound or cannot be stored.
    pub fn put_evidence(&self, bytes: &[u8]) -> Result<Cid, StoreError> {
        self.cas.put(bytes).map_err(StoreError::Cas)
    }

    /// Retrieve research evidence and verify its content-addressed identity.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the blob is missing, oversized, corrupt, or has a wrong CID.
    pub fn get_evidence(&self, cid: Cid) -> Result<Vec<u8>, StoreError> {
        let bytes = self.cas.get_bounded(&cid, MAX_EVIDENCE_BYTES)?;
        if content_id(&bytes) != cid {
            return Err(StoreError::Evidence);
        }
        Ok(bytes.to_vec())
    }

    /// Append a content-addressed evidence reference and return its ledger-node CID.
    ///
    /// The pointer is atomically replaced only after the complete immutable node is in CAS.
    /// Direct callers coordinate with the active loop owner; the per-write lock alone does not
    /// grant authority to inject evidence into another running orchestration instance.
    ///
    /// # Errors
    /// Returns [`StoreError`] for malformed pointers, CAS failures, or filesystem errors.
    pub fn append_ledger(
        &self,
        experiment: u32,
        kind: LedgerKind,
        evidence: Cid,
    ) -> Result<Cid, StoreError> {
        if experiment == 0 {
            return Err(StoreError::Ledger);
        }
        let _verified_payload = self.get_evidence(evidence)?;
        let _lock = writer_lock(&self.root)?;
        if self
            .read_pointer()?
            .is_some_and(|(_, _, program)| program != self.program)
        {
            return Err(StoreError::ProgramChanged);
        }
        let previous = self.read_ledger_head()?;
        let bytes = encode_ledger_node(
            self.program,
            previous,
            LedgerEntry {
                experiment,
                kind,
                evidence,
            },
        );
        let node = self.cas.put(&bytes)?;
        let pointer = encode_ledger_pointer(node);
        write_atomic(
            &self.root.join("evidence-current"),
            &pointer,
            &RetryPolicy::default(),
        )?;
        Ok(node)
    }

    /// Read and validate the full ledger from newest to oldest.
    ///
    /// # Errors
    /// Returns [`StoreError`] if any chain node is corrupt, missing, or belongs to another run.
    pub fn ledger(&self) -> Result<Vec<LedgerEntry>, StoreError> {
        let mut entries = Vec::new();
        let mut cursor = self.read_ledger_head()?;
        while let Some(cid) = cursor {
            if entries.len() >= MAX_LEDGER_ENTRIES {
                return Err(StoreError::Ledger);
            }
            let bytes = self.cas.get_bounded(&cid, 128)?;
            let (program, previous, entry) = decode_ledger_node(&bytes)?;
            if program != self.program {
                return Err(StoreError::ProgramChanged);
            }
            let _verified_payload = self.get_evidence(entry.evidence)?;
            entries.push(entry);
            cursor = previous;
        }
        Ok(entries)
    }

    /// Reopen the latest checkpoint and conservatively convert in-flight work to recovery.
    ///
    /// # Errors
    /// Returns [`StoreError`] for malformed pointer data, missing/corrupt CAS blobs, or a changed
    /// program identity.
    pub fn load(&self) -> Result<Option<StoredCheckpoint>, StoreError> {
        let Some((generation, cid, program)) = self.read_pointer()? else {
            return Ok(None);
        };
        if program != self.program {
            return Err(StoreError::ProgramChanged);
        }
        let bytes = self.cas.get_bounded(&cid, 32 * 1024 * 1024)?;
        let checkpoint = Checkpoint::decode(&bytes)?;
        let state = ResearchState::resume(self.program, checkpoint)?;
        Ok(Some(StoredCheckpoint { generation, state }))
    }

    fn read_pointer(&self) -> Result<Option<(u64, Cid, Cid)>, StoreError> {
        let path = self.root.join("current");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(StoreError::Pointer);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(StoreError::Io(error)),
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(StoreError::Io(error)),
        };
        if u64::try_from(bytes.len()).map_err(|_| StoreError::Pointer)? > MAX_POINTER_BYTES {
            return Err(StoreError::Pointer);
        }
        let expected = POINTER_HEADER
            .len()
            .checked_add(8)
            .and_then(|size| size.checked_add(64));
        if expected != Some(bytes.len()) || !bytes.starts_with(POINTER_HEADER) {
            return Err(StoreError::Pointer);
        }
        let generation_end = POINTER_HEADER
            .len()
            .checked_add(8)
            .ok_or(StoreError::Pointer)?;
        let generation_bytes = bytes
            .get(POINTER_HEADER.len()..generation_end)
            .ok_or(StoreError::Pointer)?;
        let generation_array: [u8; 8] = generation_bytes
            .try_into()
            .map_err(|_| StoreError::Pointer)?;
        let generation = u64::from_le_bytes(generation_array);
        let cid_end = generation_end.checked_add(32).ok_or(StoreError::Pointer)?;
        let cid_bytes = bytes
            .get(generation_end..cid_end)
            .ok_or(StoreError::Pointer)?;
        let cid_array: [u8; 32] = cid_bytes.try_into().map_err(|_| StoreError::Pointer)?;
        let program_bytes = bytes.get(cid_end..).ok_or(StoreError::Pointer)?;
        let program_array: [u8; 32] = program_bytes.try_into().map_err(|_| StoreError::Pointer)?;
        if generation == 0 {
            return Err(StoreError::Pointer);
        }
        Ok(Some((
            generation,
            Cid::from_bytes(cid_array),
            Cid::from_bytes(program_array),
        )))
    }

    fn read_ledger_head(&self) -> Result<Option<Cid>, StoreError> {
        let path = self.root.join("evidence-current");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(StoreError::Ledger);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(StoreError::Io(error)),
        }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(StoreError::Io(error)),
        };
        if bytes.len() != LEDGER_POINTER_HEADER.len().saturating_add(32)
            || !bytes.starts_with(LEDGER_POINTER_HEADER)
        {
            return Err(StoreError::Ledger);
        }
        let cid_bytes = bytes
            .get(LEDGER_POINTER_HEADER.len()..)
            .ok_or(StoreError::Ledger)?;
        let cid_array: [u8; 32] = cid_bytes.try_into().map_err(|_| StoreError::Ledger)?;
        Ok(Some(Cid::from_bytes(cid_array)))
    }
}

/// Operating-system file lock held for exactly one `ResearchLoop` lifetime.
#[derive(Debug)]
pub(crate) struct RunOwnerLease {
    _file: File,
}

fn encode_ledger_pointer(head: Cid) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(LEDGER_POINTER_HEADER.len().saturating_add(32));
    bytes.extend_from_slice(LEDGER_POINTER_HEADER);
    bytes.extend_from_slice(head.as_bytes());
    bytes
}

fn writer_lock(root: &Path) -> Result<File, StoreError> {
    let path = root.join("writer.lock");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(StoreError::Lock);
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(StoreError::Io(error)),
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    if !file.metadata()?.is_file() {
        return Err(StoreError::Lock);
    }
    file.lock()?;
    Ok(file)
}

fn bind_program(root: &Path, program: Cid) -> Result<(), StoreError> {
    let path = root.join("program");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(StoreError::Pointer);
        }
        Ok(_) => {
            let bytes = fs::read(path)?;
            if bytes != encode_program(program) {
                return Err(StoreError::ProgramChanged);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let bytes = encode_program(program);
            write_atomic(&path, &bytes, &RetryPolicy::default())?;
        }
        Err(error) => return Err(StoreError::Io(error)),
    }
    Ok(())
}

fn encode_program(program: Cid) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(PROGRAM_HEADER.len().saturating_add(32));
    bytes.extend_from_slice(PROGRAM_HEADER);
    bytes.extend_from_slice(program.as_bytes());
    bytes
}

fn encode_ledger_node(program: Cid, previous: Option<Cid>, entry: LedgerEntry) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(LEDGER_NODE_HEADER.len().saturating_add(101));
    bytes.extend_from_slice(LEDGER_NODE_HEADER);
    bytes.extend_from_slice(program.as_bytes());
    match previous {
        Some(cid) => {
            bytes.push(1);
            bytes.extend_from_slice(cid.as_bytes());
        }
        None => bytes.push(0),
    }
    bytes.extend_from_slice(&entry.experiment.to_le_bytes());
    bytes.push(entry.kind.wire_value());
    bytes.extend_from_slice(entry.evidence.as_bytes());
    bytes
}

fn decode_ledger_node(bytes: &[u8]) -> Result<(Cid, Option<Cid>, LedgerEntry), StoreError> {
    let minimum = LEDGER_NODE_HEADER
        .len()
        .checked_add(32)
        .and_then(|size| size.checked_add(1 + 4 + 1 + 32))
        .ok_or(StoreError::Ledger)?;
    if bytes.len() < minimum || !bytes.starts_with(LEDGER_NODE_HEADER) {
        return Err(StoreError::Ledger);
    }
    let program_end = LEDGER_NODE_HEADER
        .len()
        .checked_add(32)
        .ok_or(StoreError::Ledger)?;
    let program_bytes = bytes
        .get(LEDGER_NODE_HEADER.len()..program_end)
        .ok_or(StoreError::Ledger)?;
    let program_array: [u8; 32] = program_bytes.try_into().map_err(|_| StoreError::Ledger)?;
    let program = Cid::from_bytes(program_array);
    let flag = *bytes.get(program_end).ok_or(StoreError::Ledger)?;
    let previous_start = program_end.checked_add(1).ok_or(StoreError::Ledger)?;
    let (previous, mut cursor) = match flag {
        0 => (None, previous_start),
        1 => {
            let end = previous_start.checked_add(32).ok_or(StoreError::Ledger)?;
            let data = bytes.get(previous_start..end).ok_or(StoreError::Ledger)?;
            let array: [u8; 32] = data.try_into().map_err(|_| StoreError::Ledger)?;
            (Some(Cid::from_bytes(array)), end)
        }
        _ => return Err(StoreError::Ledger),
    };
    let experiment_end = cursor.checked_add(4).ok_or(StoreError::Ledger)?;
    let ordinal_bytes = bytes
        .get(cursor..experiment_end)
        .ok_or(StoreError::Ledger)?;
    let ordinal_array: [u8; 4] = ordinal_bytes.try_into().map_err(|_| StoreError::Ledger)?;
    let experiment = u32::from_le_bytes(ordinal_array);
    cursor = experiment_end;
    let kind_byte = *bytes.get(cursor).ok_or(StoreError::Ledger)?;
    cursor = cursor.checked_add(1).ok_or(StoreError::Ledger)?;
    let kind = match kind_byte {
        1 => LedgerKind::Tool,
        2 => LedgerKind::MetricRun,
        3 => LedgerKind::Mutation,
        4 => LedgerKind::Recovery,
        _ => return Err(StoreError::Ledger),
    };
    let evidence_end = cursor.checked_add(32).ok_or(StoreError::Ledger)?;
    let evidence_bytes = bytes.get(cursor..evidence_end).ok_or(StoreError::Ledger)?;
    if evidence_end != bytes.len() || experiment == 0 {
        return Err(StoreError::Ledger);
    }
    let evidence_array: [u8; 32] = evidence_bytes.try_into().map_err(|_| StoreError::Ledger)?;
    Ok((
        program,
        previous,
        LedgerEntry {
            experiment,
            kind,
            evidence: Cid::from_bytes(evidence_array),
        },
    ))
}

fn encode_pointer(generation: u64, cid: Cid, program: Cid) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(POINTER_HEADER.len().saturating_add(72));
    bytes.extend_from_slice(POINTER_HEADER);
    bytes.extend_from_slice(&generation.to_le_bytes());
    bytes.extend_from_slice(cid.as_bytes());
    bytes.extend_from_slice(program.as_bytes());
    bytes
}

/// Durable store, filesystem, or checkpoint validation failure.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Filesystem operation failed.
    #[error("research checkpoint filesystem error: {0}")]
    Io(#[from] std::io::Error),
    /// Shared content-addressed store failed.
    #[error("research checkpoint CAS error: {0}")]
    Cas(#[from] CasError),
    /// Binary checkpoint failed validation or encoding.
    #[error("research checkpoint error: {0}")]
    Checkpoint(#[from] ResearchError),
    /// Pointer file is malformed or unsupported.
    #[error("invalid research checkpoint pointer")]
    Pointer,
    /// Store generation exhausted its integer range.
    #[error("research checkpoint generation overflow")]
    Generation,
    /// The checkpoint belongs to another immutable program.
    #[error("research checkpoint belongs to a different program")]
    ProgramChanged,
    /// An initial checkpoint cannot replace an existing checkpoint.
    #[error("research checkpoint already exists")]
    CheckpointExists,
    /// Evidence ledger pointer or chain node is malformed or exceeds its bound.
    #[error("invalid research evidence ledger")]
    Ledger,
    /// Writer lock path is not a regular file.
    #[error("invalid research checkpoint writer lock")]
    Lock,
    /// Run-owner lock path is not a regular file.
    #[error("invalid research run-owner lock")]
    RunLock,
    /// Another research loop currently owns this run.
    #[error("research run is already active")]
    RunAlreadyActive,
    /// Evidence blob is missing, oversized, or has a mismatched content ID.
    #[error("research evidence blob is missing or invalid")]
    Evidence,
}
