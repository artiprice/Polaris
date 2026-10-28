//! Bounded file reading with encoding and newline metadata.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use encoding_rs::{Encoding, UTF_8};
use serde::{Deserialize, Serialize};

use crate::{
    ToolLimits,
    path::{PathError, resolve_in_workspace},
};

/// Read-file tool input. Offsets and limits are measured in original source bytes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileInput {
    /// Workspace-relative file path.
    pub path: String,
    /// Starting byte offset, defaulting to the beginning.
    #[serde(default)]
    pub offset: u64,
    /// Requested bytes, capped by [`ToolLimits::max_read_bytes`].
    #[serde(default)]
    pub limit: Option<u64>,
}

/// Bounded file contents plus source encoding and line-ending facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadFileOutput {
    /// Normalized workspace-relative path.
    pub path: String,
    /// Decoded text when the file is textual; absent for binary data.
    pub content: Option<String>,
    /// Whether a NUL appeared in the first 8 KiB.
    pub binary: bool,
    /// Detected source encoding label.
    pub encoding: &'static str,
    /// Number of original file bytes returned in the window.
    pub bytes: u64,
    /// Whether bytes remain beyond this returned window.
    pub truncated: bool,
    /// CRLF sequences in the returned text window.
    pub crlf_lines: u64,
    /// LF sequences not preceded by CR in the returned text window.
    pub lf_lines: u64,
    /// Standalone CR sequences in the returned text window.
    pub cr_lines: u64,
}

/// Read one workspace file under the configured byte cap.
pub fn read_file(
    root: &Path,
    input: &ReadFileInput,
    limits: &ToolLimits,
) -> Result<ReadFileOutput, polaris_sdk::registry::ToolError> {
    read_file_authorized(root, input, limits, |_| Ok(()))
}

pub(crate) fn read_file_authorized<F>(
    root: &Path,
    input: &ReadFileInput,
    limits: &ToolLimits,
    mut authorize: F,
) -> Result<ReadFileOutput, polaris_sdk::registry::ToolError>
where
    F: FnMut(&str) -> Result<(), polaris_sdk::registry::ToolError>,
{
    authorize(&input.path)?;
    let resolved = resolve_in_workspace(root, &input.path).map_err(path_tool_error)?;
    let metadata = std::fs::symlink_metadata(&resolved)
        .map_err(|_| polaris_sdk::registry::ToolError::Io("file metadata failed".to_owned()))?;
    if !metadata.file_type().is_file() {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "path is not a regular file".to_owned(),
        ));
    }
    let mut file = File::open(&resolved)
        .map_err(|_| polaris_sdk::registry::ToolError::Io("file open failed".to_owned()))?;
    let metadata = file
        .metadata()
        .map_err(|_| polaris_sdk::registry::ToolError::Io("file metadata failed".to_owned()))?;
    if !metadata.is_file() {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "path is not a regular file".to_owned(),
        ));
    }
    let size = metadata.len();
    let mut prefix = vec![
        0_u8;
        usize::try_from(size.min(8_192).min(limits.max_read_bytes))
            .map_err(|_| too_large(limits.max_read_bytes))?
    ];
    file.read_exact(&mut prefix)
        .map_err(|_| polaris_sdk::registry::ToolError::Io("file read failed".to_owned()))?;
    let encoding = detect_encoding(&prefix);
    let has_bom = encoding.name() != UTF_8.name() || prefix.starts_with(b"\xEF\xBB\xBF");
    if !has_bom && prefix.contains(&0) {
        return Ok(ReadFileOutput {
            path: input.path.replace('\\', "/"),
            content: None,
            binary: true,
            encoding: "binary",
            bytes: 0,
            truncated: size > 0,
            crlf_lines: 0,
            lf_lines: 0,
            cr_lines: 0,
        });
    }
    let requested = match input.limit {
        Some(limit) => limit.min(limits.max_read_bytes),
        None => limits.max_read_bytes,
    };
    let available = size.saturating_sub(input.offset);
    let take = available.min(requested);
    let capacity = usize::try_from(take).map_err(|_| too_large(limits.max_read_bytes))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.seek(SeekFrom::Start(input.offset))
        .map_err(|_| polaris_sdk::registry::ToolError::Io("file seek failed".to_owned()))?;
    file.take(take)
        .read_to_end(&mut bytes)
        .map_err(|_| polaris_sdk::registry::ToolError::Io("file read failed".to_owned()))?;
    let content_bytes = if input.offset == 0 {
        strip_bom(&bytes, encoding)
    } else {
        bytes.as_slice()
    };
    let content = encoding.decode(content_bytes).0.into_owned();
    let (crlf_lines, lf_lines, cr_lines) = count_line_endings(&content);
    let byte_count = u64::try_from(bytes.len()).map_err(|_| too_large(limits.max_read_bytes))?;
    Ok(ReadFileOutput {
        path: input.path.replace('\\', "/"),
        content: Some(content),
        binary: false,
        encoding: encoding.name(),
        bytes: byte_count,
        truncated: input.offset.saturating_add(byte_count) < size,
        crlf_lines,
        lf_lines,
        cr_lines,
    })
}

fn detect_encoding(prefix: &[u8]) -> &'static Encoding {
    match Encoding::for_bom(prefix) {
        Some((encoding, _)) => encoding,
        None => UTF_8,
    }
}

fn strip_bom<'a>(bytes: &'a [u8], encoding: &'static Encoding) -> &'a [u8] {
    let bom: &[u8] = match encoding.name() {
        "UTF-8" => b"\xEF\xBB\xBF",
        "UTF-16LE" => b"\xFF\xFE",
        "UTF-16BE" => b"\xFE\xFF",
        _ => &[],
    };
    match bytes.strip_prefix(bom) {
        Some(rest) => rest,
        None => bytes,
    }
}

fn count_line_endings(text: &str) -> (u64, u64, u64) {
    let mut crlf = 0_u64;
    let mut lf = 0_u64;
    let mut cr = 0_u64;
    let mut previous_cr = false;
    for character in text.chars() {
        match character {
            '\r' => {
                if previous_cr {
                    cr = cr.saturating_add(1);
                }
                previous_cr = true;
            }
            '\n' => {
                if previous_cr {
                    crlf = crlf.saturating_add(1);
                } else {
                    lf = lf.saturating_add(1);
                }
                previous_cr = false;
            }
            _ => {
                if previous_cr {
                    cr = cr.saturating_add(1);
                }
                previous_cr = false;
            }
        }
    }
    if previous_cr {
        cr = cr.saturating_add(1);
    }
    (crlf, lf, cr)
}

fn too_large(limit: u64) -> polaris_sdk::registry::ToolError {
    polaris_sdk::registry::ToolError::InvalidInput(format!(
        "requested file window exceeds {limit} bytes"
    ))
}

pub(crate) fn path_tool_error(error: PathError) -> polaris_sdk::registry::ToolError {
    use polaris_sdk::registry::ToolError;
    match error {
        PathError::Invalid(reason) => ToolError::InvalidInput(reason.to_owned()),
        PathError::NotFound => ToolError::NotFound("workspace path".to_owned()),
        PathError::Escape | PathError::SecretDenied => {
            ToolError::Forbidden("workspace path denied".to_owned())
        }
        PathError::Io => ToolError::Io("workspace path resolution failed".to_owned()),
    }
}
