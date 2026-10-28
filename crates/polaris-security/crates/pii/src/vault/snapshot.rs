//! Bounded plaintext encoding inside an explicitly injected authenticated encryption envelope.
use super::{PiiError, PiiLimits, PiiVault, SealedBlob, Sealer, VaultEntry, VaultState};
use polaris_sdk::ids::{FrameId, SessionId};
use polaris_security_automata::EntropyCfg;
use std::collections::HashSet;
use std::sync::Mutex;
use zeroize::Zeroizing;
const MAGIC: &[u8] = b"POLARIS-PII-SNAPSHOT-1\0";
/// Maximum encoded plaintext snapshot. Oversized vaults require explicit smaller lifecycle bounds.
const MAX_SNAPSHOT_BYTES: usize = 1024 * 1024;

impl PiiVault {
    /// Seal a consistent copy of the exact marker map for the bound session/frame.
    ///
    /// Plaintext encoding exists only in a zeroizing buffer. The host must retain or persist its
    /// sealer key separately; this API never writes a key, ciphertext or plaintext to disk.
    /// # Errors
    /// Bounded map/encoding failure or the injected authenticated encryption error.
    pub fn export_sealed<S: Sealer + ?Sized>(&self, sealer: &S) -> Result<SealedBlob, PiiError> {
        let state = self.state.lock().map_err(|_| PiiError::StateUnavailable)?;
        let mut bytes = Zeroizing::new(Vec::new());
        append(&mut bytes, MAGIC)?;
        field(&mut bytes, self.session.to_string().as_bytes())?;
        field(&mut bytes, self.frame.to_string().as_bytes())?;
        field(&mut bytes, self.session_tag.as_bytes())?;
        append(&mut bytes, &state.next_token.to_le_bytes())?;
        append(
            &mut bytes,
            &u32::try_from(state.entries.len())
                .map_err(|_| PiiError::Capacity)?
                .to_le_bytes(),
        )?;
        for entry in &state.entries {
            field(&mut bytes, entry.token.as_bytes())?;
            field(&mut bytes, entry.secret.as_bytes())?;
        }
        drop(state);
        sealer.seal(&bytes)
    }

    /// Authenticate and reconstruct an exact marker map under caller-supplied current limits.
    ///
    /// Caller identities must match the sealed identities; copied snapshots never widen ownership.
    /// Marker syntax, duplicates, UTF-8, complete consumption and aggregate bounds are checked before
    /// the new vault becomes visible. Host key custody and rollback prevention remain explicit.
    /// # Errors
    /// Authentication/format/ownership errors, invalid limits, or source capacity exceeding new bounds.
    pub fn import_sealed<S: Sealer + ?Sized>(
        session: SessionId,
        frame: FrameId,
        entropy: EntropyCfg,
        limits: PiiLimits,
        sealer: &S,
        blob: &SealedBlob,
    ) -> Result<Self, PiiError> {
        if blob.ciphertext().len() > MAX_SNAPSHOT_BYTES.saturating_add(4096) {
            return Err(PiiError::Capacity);
        }
        let bytes = sealer.open(blob)?;
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(PiiError::Capacity);
        }
        let mut reader = Reader {
            bytes: &bytes,
            position: 0,
        };
        if reader.take(MAGIC.len())? != MAGIC
            || reader.text(36)? != session.to_string()
            || reader.text(36)? != frame.to_string()
        {
            return Err(PiiError::Snapshot);
        }
        let tag = reader.text(32)?;
        if tag.len() != 32
            || !tag
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(PiiError::Snapshot);
        }
        let next = u64::from_le_bytes(reader.take(8)?.try_into().map_err(|_| PiiError::Snapshot)?);
        let count = u32::from_le_bytes(reader.take(4)?.try_into().map_err(|_| PiiError::Snapshot)?);
        let count = usize::try_from(count).map_err(|_| PiiError::Capacity)?;
        if count > limits.max_entries
            || u64::try_from(count).map_err(|_| PiiError::Capacity)? > next
        {
            return Err(PiiError::Capacity);
        }
        let mut vault = Self::new(session, frame, entropy, limits)?;
        let mut state = VaultState {
            next_token: next,
            ..VaultState::default()
        };
        state
            .entries
            .try_reserve(count)
            .map_err(|_| PiiError::Capacity)?;
        state
            .token_to_entry
            .try_reserve(count)
            .map_err(|_| PiiError::Capacity)?;
        let mut values = HashSet::new();
        values.try_reserve(count).map_err(|_| PiiError::Capacity)?;
        for index in 0..count {
            let token = reader.text(128)?;
            let secret = reader.text(limits.max_secret_bytes)?;
            if secret.is_empty()
                || !valid_marker(token, tag)
                || !values.insert(secret)
                || state.token_to_entry.contains_key(token)
            {
                return Err(PiiError::Snapshot);
            }
            state.secret_bytes = state
                .secret_bytes
                .checked_add(secret.len())
                .ok_or(PiiError::CounterOverflow)?;
            if state.secret_bytes > limits.max_secret_bytes {
                return Err(PiiError::Capacity);
            }
            let mut value = Zeroizing::new(String::new());
            value
                .try_reserve(secret.len())
                .map_err(|_| PiiError::Capacity)?;
            value.push_str(secret);
            state.token_to_entry.insert(token.to_owned(), index);
            state.entries.push(VaultEntry {
                secret: value,
                token: token.to_owned(),
            });
        }
        if reader.position != bytes.len() {
            return Err(PiiError::Snapshot);
        }
        vault.session_tag = tag.to_owned();
        vault.state = Mutex::new(state);
        Ok(vault)
    }
}
fn valid_marker(token: &str, tag: &str) -> bool {
    let Some(body) = token
        .strip_prefix("<PII1:")
        .and_then(|value| value.strip_suffix('>'))
    else {
        return false;
    };
    let mut fields = body.split(':');
    fields.next() == Some(tag)
        && fields
            .next()
            .is_some_and(|value| value.parse::<u64>().is_ok_and(|id| id.to_string() == value))
        && matches!(fields.next(), Some("email" | "ip" | "key" | "other"))
        && fields.next().is_none()
}
fn append(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), PiiError> {
    if bytes
        .len()
        .checked_add(value.len())
        .ok_or(PiiError::CounterOverflow)?
        > MAX_SNAPSHOT_BYTES
    {
        return Err(PiiError::Capacity);
    }
    bytes
        .try_reserve(value.len())
        .map_err(|_| PiiError::Capacity)?;
    bytes.extend_from_slice(value);
    Ok(())
}
fn field(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), PiiError> {
    append(
        bytes,
        &u32::try_from(value.len())
            .map_err(|_| PiiError::Capacity)?
            .to_le_bytes(),
    )?;
    append(bytes, value)
}
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], PiiError> {
        let end = self.position.checked_add(count).ok_or(PiiError::Snapshot)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(PiiError::Snapshot)?;
        self.position = end;
        Ok(value)
    }
    fn text(&mut self, limit: usize) -> Result<&'a str, PiiError> {
        let len = u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| PiiError::Snapshot)?);
        let len = usize::try_from(len).map_err(|_| PiiError::Snapshot)?;
        if len > limit {
            return Err(PiiError::Capacity);
        }
        std::str::from_utf8(self.take(len)?).map_err(|_| PiiError::Snapshot)
    }
}
