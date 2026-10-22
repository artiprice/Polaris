//! Pure, byte-preserving application of one-file unified patches.

/// Unified patch parsing or stale-context failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    /// The patch header or hunk counts are malformed or unsupported.
    #[error("malformed or unsupported unified patch")]
    Malformed,
    /// A context or deletion line does not match the current base bytes.
    #[error("patch context does not match current file contents")]
    Stale,
    /// Hunk offsets exceed the source or arithmetic bounds.
    #[error("patch hunk is outside the source bounds")]
    Bounds,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceLine {
    content: Vec<u8>,
    ending: Vec<u8>,
}

/// Apply a unified patch to source bytes without writing to disk.
///
/// The function accepts hunk-only patches or a single-file unified diff. Every context and delete
/// line must match exactly. Existing line endings and final-newline state are retained; newly
/// added lines use the nearest source line ending, or LF for an empty source. The returned bytes
/// are only a proposal candidate and do not authorize persistence.
pub fn apply_unified_patch(base: &[u8], patch: &str) -> Result<Vec<u8>, PatchError> {
    let source = split_source(base);
    let default_ending = match source.iter().find(|line| !line.ending.is_empty()) {
        Some(line) => line.ending.clone(),
        None => b"\n".to_vec(),
    };
    let patch_lines: Vec<&str> = patch.lines().collect();
    let mut source_cursor = 0_usize;
    let mut patch_cursor = 0_usize;
    let mut output = Vec::new();
    let mut saw_hunk = false;
    let mut saw_diff_header = false;
    let mut saw_old_header = false;
    let mut saw_new_header = false;
    let mut old_header_path: Option<String> = None;
    let mut new_header_path: Option<String> = None;

    while patch_cursor < patch_lines.len() {
        let line = patch_lines
            .get(patch_cursor)
            .copied()
            .ok_or(PatchError::Malformed)?;
        if !line.starts_with("@@ ") {
            if line.starts_with("diff --git ") {
                if saw_hunk || saw_diff_header || saw_old_header || saw_new_header {
                    return Err(PatchError::Malformed);
                }
                validate_git_file_pair(line)?;
                saw_diff_header = true;
                patch_cursor = patch_cursor.saturating_add(1);
                continue;
            }
            if let Some(path) = line.strip_prefix("--- ") {
                if saw_hunk || saw_old_header || saw_new_header {
                    return Err(PatchError::Malformed);
                }
                old_header_path = Some(parse_patch_path(path)?);
                saw_old_header = true;
                patch_cursor = patch_cursor.saturating_add(1);
                continue;
            }
            if let Some(path) = line.strip_prefix("+++ ") {
                if saw_hunk || !saw_old_header || saw_new_header {
                    return Err(PatchError::Malformed);
                }
                new_header_path = Some(parse_patch_path(path)?);
                saw_new_header = true;
                patch_cursor = patch_cursor.saturating_add(1);
                continue;
            }
            if line.starts_with("index ") && !saw_hunk && saw_diff_header && !saw_old_header {
                patch_cursor = patch_cursor.saturating_add(1);
                continue;
            }
            if line.is_empty() && !saw_hunk {
                patch_cursor = patch_cursor.saturating_add(1);
                continue;
            }
            return Err(PatchError::Malformed);
        }
        if saw_old_header != saw_new_header {
            return Err(PatchError::Malformed);
        }
        if !header_paths_match(old_header_path.as_deref(), new_header_path.as_deref()) {
            return Err(PatchError::Malformed);
        }
        saw_hunk = true;
        let (old_start, old_count, new_start, expected_new_count) = parse_hunk_header(line)?;
        let target = if old_count == 0 {
            old_start
        } else {
            old_start.checked_sub(1).ok_or(PatchError::Bounds)?
        };
        if target < source_cursor || target > source.len() {
            return Err(PatchError::Bounds);
        }
        let output_prefix_lines = output
            .len()
            .checked_add(target.saturating_sub(source_cursor))
            .ok_or(PatchError::Bounds)?;
        let expected_new_start = if expected_new_count == 0 {
            output_prefix_lines
        } else {
            output_prefix_lines
                .checked_add(1)
                .ok_or(PatchError::Bounds)?
        };
        if new_start != expected_new_start {
            return Err(PatchError::Malformed);
        }
        output.extend(
            source
                .get(source_cursor..target)
                .ok_or(PatchError::Bounds)?
                .iter()
                .cloned(),
        );
        source_cursor = target;
        patch_cursor = patch_cursor.saturating_add(1);
        let mut consumed_old = 0_usize;
        let mut produced_new = 0_usize;
        let mut deleted_ending: Option<Vec<u8>> = None;
        let mut no_newline_target: Option<usize> = None;
        let mut deleted_without_newline = false;

        while consumed_old < old_count
            || produced_new < expected_new_count
            || patch_lines.get(patch_cursor) == Some(&"\\ No newline at end of file")
        {
            let patch_line = patch_lines
                .get(patch_cursor)
                .copied()
                .ok_or(PatchError::Malformed)?;
            patch_cursor = patch_cursor.saturating_add(1);
            if patch_line == "\\ No newline at end of file" {
                if let Some(index) = no_newline_target.take() {
                    let line = output.get_mut(index).ok_or(PatchError::Malformed)?;
                    line.ending.clear();
                    continue;
                }
                if deleted_without_newline {
                    deleted_without_newline = false;
                    continue;
                }
                return Err(PatchError::Malformed);
            }
            let patch_bytes = patch_line.as_bytes();
            let marker = patch_bytes.first().copied().ok_or(PatchError::Malformed)?;
            let text = patch_line.get(1..).ok_or(PatchError::Malformed)?;
            match marker {
                b' ' | b'-' => {
                    if consumed_old >= old_count {
                        return Err(PatchError::Malformed);
                    }
                    let source_line = source.get(source_cursor).ok_or(PatchError::Bounds)?;
                    if source_line.content.as_slice() != text.as_bytes() {
                        return Err(PatchError::Stale);
                    }
                    source_cursor = source_cursor.saturating_add(1);
                    consumed_old = consumed_old.saturating_add(1);
                    if marker == b' ' {
                        if produced_new >= expected_new_count {
                            return Err(PatchError::Malformed);
                        }
                        output.push(source_line.clone());
                        no_newline_target = Some(output.len().saturating_sub(1));
                        produced_new = produced_new.saturating_add(1);
                        deleted_ending = None;
                        deleted_without_newline = false;
                    } else {
                        deleted_ending = Some(source_line.ending.clone());
                        no_newline_target = None;
                        deleted_without_newline = source_line.ending.is_empty();
                    }
                }
                b'+' => {
                    if produced_new >= expected_new_count {
                        return Err(PatchError::Malformed);
                    }
                    let ending = match deleted_ending.take() {
                        Some(ending) => ending,
                        None => ending_near(&source, source_cursor, &default_ending),
                    };
                    output.push(SourceLine {
                        content: text.as_bytes().to_vec(),
                        ending,
                    });
                    no_newline_target = Some(output.len().saturating_sub(1));
                    produced_new = produced_new.saturating_add(1);
                    deleted_without_newline = false;
                }
                _ => return Err(PatchError::Malformed),
            }
        }
        if consumed_old != old_count || produced_new != expected_new_count {
            return Err(PatchError::Malformed);
        }
    }
    if saw_old_header != saw_new_header {
        return Err(PatchError::Malformed);
    }
    if !saw_hunk {
        return Err(PatchError::Malformed);
    }
    output.extend(
        source
            .get(source_cursor..)
            .ok_or(PatchError::Bounds)?
            .iter()
            .cloned(),
    );
    let capacity = output
        .iter()
        .try_fold(0_usize, |total, line| {
            total
                .checked_add(line.content.len())?
                .checked_add(line.ending.len())
        })
        .ok_or(PatchError::Bounds)?;
    let mut bytes = Vec::with_capacity(capacity);
    for line in output {
        bytes.extend_from_slice(&line.content);
        bytes.extend_from_slice(&line.ending);
    }
    Ok(bytes)
}

fn split_source(bytes: &[u8]) -> Vec<SourceLine> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte == b'\n' {
            let crlf = index > start && bytes.get(index.saturating_sub(1)) == Some(&b'\r');
            let content_end = if crlf { index.saturating_sub(1) } else { index };
            lines.push(SourceLine {
                content: match bytes.get(start..content_end) {
                    Some(content) => content.to_vec(),
                    None => Vec::new(),
                },
                ending: if crlf {
                    b"\r\n".to_vec()
                } else {
                    b"\n".to_vec()
                },
            });
            start = index.saturating_add(1);
        } else if byte == b'\r' && bytes.get(index.saturating_add(1)) != Some(&b'\n') {
            lines.push(SourceLine {
                content: match bytes.get(start..index) {
                    Some(content) => content.to_vec(),
                    None => Vec::new(),
                },
                ending: b"\r".to_vec(),
            });
            start = index.saturating_add(1);
        }
    }
    if start < bytes.len() {
        lines.push(SourceLine {
            content: match bytes.get(start..) {
                Some(content) => content.to_vec(),
                None => Vec::new(),
            },
            ending: Vec::new(),
        });
    }
    lines
}

fn parse_hunk_header(header: &str) -> Result<(usize, usize, usize, usize), PatchError> {
    let mut parts = header.split_whitespace();
    if parts.next() != Some("@@") {
        return Err(PatchError::Malformed);
    }
    let old = parts
        .next()
        .and_then(|part| part.strip_prefix('-'))
        .ok_or(PatchError::Malformed)?;
    let new = parts
        .next()
        .and_then(|part| part.strip_prefix('+'))
        .ok_or(PatchError::Malformed)?;
    if parts.next() != Some("@@") {
        return Err(PatchError::Malformed);
    }
    let (old_start, old_count) = parse_range(old)?;
    let (new_start, new_count) = parse_range(new)?;
    Ok((old_start, old_count, new_start, new_count))
}

fn validate_git_file_pair(line: &str) -> Result<(), PatchError> {
    let pair = line
        .strip_prefix("diff --git ")
        .ok_or(PatchError::Malformed)?;
    let mut paths = pair.split_whitespace();
    let old = paths.next().ok_or(PatchError::Malformed)?;
    let new = paths.next().ok_or(PatchError::Malformed)?;
    let old_path = old.strip_prefix("a/").ok_or(PatchError::Malformed)?;
    let new_path = new.strip_prefix("b/").ok_or(PatchError::Malformed)?;
    if paths.next().is_some() || old_path.is_empty() || old_path != new_path {
        return Err(PatchError::Malformed);
    }
    Ok(())
}

fn parse_patch_path(path: &str) -> Result<String, PatchError> {
    let path = match path.split('\t').next() {
        Some(path) => path,
        None => return Err(PatchError::Malformed),
    };
    if path.is_empty() || path.contains('\0') || path.contains('\n') {
        return Err(PatchError::Malformed);
    }
    Ok(path.to_owned())
}

fn header_paths_match(old: Option<&str>, new: Option<&str>) -> bool {
    match (old, new) {
        (None, None) => true,
        (Some(old), Some(new)) if old == "/dev/null" || new == "/dev/null" => true,
        (Some(old), Some(new)) => {
            let old = match old.strip_prefix("a/") {
                Some(path) => path,
                None => old,
            };
            let new = match new.strip_prefix("b/") {
                Some(path) => path,
                None => new,
            };
            old == new
        }
        _ => false,
    }
}

fn parse_range(range: &str) -> Result<(usize, usize), PatchError> {
    match range.split_once(',') {
        Some((start, count)) => Ok((
            start.parse().map_err(|_| PatchError::Malformed)?,
            count.parse().map_err(|_| PatchError::Malformed)?,
        )),
        None => Ok((range.parse().map_err(|_| PatchError::Malformed)?, 1)),
    }
}

fn ending_near(source: &[SourceLine], cursor: usize, fallback: &[u8]) -> Vec<u8> {
    if let Some(line) = source.get(cursor).filter(|line| !line.ending.is_empty()) {
        return line.ending.clone();
    }
    if let Some(line) = cursor
        .checked_sub(1)
        .and_then(|index| source.get(index))
        .filter(|line| !line.ending.is_empty())
    {
        return line.ending.clone();
    }
    fallback.to_vec()
}
