//! Domain-separated identity binding for a proposed mutation.

use polaris_sdk::ids::{Cid, CommandId, FrameId};

/// Failure to encode a length-delimited mutation binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MutationBindingError {
    /// A field length could not be represented by the fixed-width binding format.
    #[error("mutation binding field is too large")]
    FieldTooLarge,
}

/// Compute the capability fact hash for one exact command and pair of source snapshots.
///
/// Capability issuers that pin a mutation in `Scope::diff_blake3` must use this function's output.
/// The SDK command body separately carries the BLAKE3 CID of the exact diff bytes; the verifier
/// checks that CID before hashing this stronger binding. This binds the command id and frame,
/// normalized target, diff CID, and old/new source CIDs into one token fact.
///
/// # Errors
/// Returns [`MutationBindingError::FieldTooLarge`] if a length cannot fit the fixed-width format.
pub fn mutation_digest(
    command: CommandId,
    agent: FrameId,
    normalized_target: &str,
    diff: Cid,
    old_source: Cid,
    new_source: Cid,
) -> Result<Cid, MutationBindingError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"polaris.hybrid-mutation.v1\0");
    update_field(&mut hasher, b"command", command.as_uuid().as_bytes())?;
    update_field(&mut hasher, b"agent", agent.as_uuid().as_bytes())?;
    update_field(&mut hasher, b"target", normalized_target.as_bytes())?;
    update_field(&mut hasher, b"diff", diff.as_bytes())?;
    update_field(&mut hasher, b"old_source", old_source.as_bytes())?;
    update_field(&mut hasher, b"new_source", new_source.as_bytes())?;
    Ok(Cid::from_bytes(*hasher.finalize().as_bytes()))
}

fn update_field(
    hasher: &mut blake3::Hasher,
    name: &[u8],
    value: &[u8],
) -> Result<(), MutationBindingError> {
    let length = u64::try_from(value.len()).map_err(|_| MutationBindingError::FieldTooLarge)?;
    hasher.update(name);
    hasher.update(&[0]);
    hasher.update(&length.to_le_bytes());
    hasher.update(value);
    Ok(())
}
