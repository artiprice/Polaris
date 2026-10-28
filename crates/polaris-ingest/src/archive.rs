use std::collections::HashSet;
use std::io::{Cursor, Read};

use polaris_sdk::registry::IngestError;
use zip::ZipArchive;

use crate::IngestLimits;

pub(super) fn validate(bytes: &[u8], limits: IngestLimits) -> Result<(), IngestError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| malformed("invalid ZIP archive", error))?;
    let count = archive.len();
    if count > limits.max_archive_entries {
        return Err(IngestError::Malformed(format!(
            "archive has {count} entries; limit is {}",
            limits.max_archive_entries
        )));
    }
    let mut total = 0_u64;
    let mut normalized_names = HashSet::new();
    for index in 0..count {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| malformed("invalid ZIP member", error))?;
        let name = entry
            .name()
            .map_err(|error| malformed("invalid ZIP member name", error))?
            .into_owned();
        if unsafe_path(&name) {
            return Err(IngestError::Malformed(format!(
                "unsafe archive member path: {name}"
            )));
        }
        if !normalized_names.insert(normalized_name(&name)) {
            return Err(IngestError::Malformed(format!(
                "duplicate normalized archive member path: {name}"
            )));
        }
        let declared = entry.size();
        let compressed = entry.compressed_size();
        if declared > limits.max_entry_bytes {
            return Err(IngestError::TooLarge {
                bytes: declared,
                limit: limits.max_entry_bytes,
            });
        }
        if compressed == 0 && declared > 0 {
            return Err(IngestError::Malformed(format!(
                "archive member {name} has impossible compressed size"
            )));
        }
        let maximum_uncompressed = compressed.checked_mul(limits.max_compression_ratio);
        if maximum_uncompressed.is_some_and(|maximum| declared > maximum) {
            return Err(IngestError::Malformed(format!(
                "archive member {name} exceeds compression-ratio limit"
            )));
        }
        let mut member_total = 0_u64;
        let mut buffer = vec![0_u8; 8_192];
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|error| malformed("cannot decompress ZIP member", error))?;
            if read == 0 {
                break;
            }
            let read = u64::try_from(read).map_err(|_| {
                IngestError::Malformed("decompressed byte count overflow".to_owned())
            })?;
            member_total = member_total
                .checked_add(read)
                .ok_or_else(|| IngestError::Malformed("member size overflow".to_owned()))?;
            total = total
                .checked_add(read)
                .ok_or_else(|| IngestError::Malformed("archive size overflow".to_owned()))?;
            if member_total > limits.max_entry_bytes {
                return Err(IngestError::TooLarge {
                    bytes: member_total,
                    limit: limits.max_entry_bytes,
                });
            }
            if total > limits.max_uncompressed_bytes {
                return Err(IngestError::TooLarge {
                    bytes: total,
                    limit: limits.max_uncompressed_bytes,
                });
            }
        }
        if member_total != declared {
            return Err(IngestError::Malformed(format!(
                "archive member {name} size differs from declared size"
            )));
        }
    }
    Ok(())
}

fn unsafe_path(name: &str) -> bool {
    name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\0')
        || name.contains(':')
        || name.split(['/', '\\']).any(|part| part == "..")
}

fn normalized_name(name: &str) -> String {
    name.split(['/', '\\'])
        .filter(|component| !component.is_empty() && *component != ".")
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("/")
}

fn malformed(context: &str, error: impl std::fmt::Display) -> IngestError {
    IngestError::Malformed(format!("{context}: {error}"))
}
