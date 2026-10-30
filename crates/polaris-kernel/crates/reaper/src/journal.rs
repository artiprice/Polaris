//! Versioned, escaped recovery records with fsynced atomic replacement.
use crate::{LeakEntry, Locator, ReapId, ReaperError, ResourceKind};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

const HEADER: &str = "polaris-reaper-journal v1";
pub(crate) fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
fn unescape(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        out.push(match chars.next()? {
            '\\' => '\\',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(out)
}
pub(crate) fn read(path: &Path) -> Result<(BTreeMap<ReapId, LeakEntry>, usize), ReaperError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((BTreeMap::new(), 0));
        }
        Err(error) => return Err(error.into()),
    };
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some(HEADER) {
        return Err(ReaperError::JournalFormat(
            "unsupported header/version".to_owned(),
        ));
    }
    let mut entries = BTreeMap::new();
    let mut ignored = 0usize;
    for line in lines {
        if !line.ends_with('\n') {
            ignored = ignored.saturating_add(1);
            continue;
        }
        let Some(record) = line.strip_suffix('\n').and_then(parse) else {
            return Err(ReaperError::JournalFormat(
                "malformed complete record".to_owned(),
            ));
        };
        if let Some(previous) = entries.insert(record.id, record.clone())
            && previous != record
        {
            return Err(ReaperError::JournalFormat(
                "conflicting duplicate resource id".to_owned(),
            ));
        }
    }
    Ok((entries, ignored))
}
fn parse(line: &str) -> Option<LeakEntry> {
    let mut fields = line.split('\t');
    if fields.next()? != "R" {
        return None;
    }
    let id = ReapId(fields.next()?.parse().ok()?);
    let kind = ResourceKind::parse(fields.next()?)?;
    let form = fields.next()?;
    let value = unescape(fields.next()?)?;
    if fields.next().is_some() {
        return None;
    }
    let locator = match form {
        "path" => Locator::Path(value.into()),
        "pid" => Locator::Pid(value.parse().ok()?),
        "label" => Locator::Label(value),
        _ => return None,
    };
    Some(LeakEntry { id, kind, locator })
}
pub(crate) fn write(
    path: &Path,
    entries: impl Iterator<Item = LeakEntry>,
) -> Result<(), ReaperError> {
    let mut text = format!("{HEADER}\n");
    let mut count = 0usize;
    for entry in entries {
        let (form, value) = match entry.locator {
            Locator::None => continue,
            Locator::Path(path) => (
                "path",
                path.to_str()
                    .ok_or_else(|| {
                        ReaperError::JournalFormat("non-UTF-8 resource path".to_owned())
                    })?
                    .to_owned(),
            ),
            Locator::Pid(pid) => ("pid", pid.to_string()),
            Locator::Label(label) => ("label", label),
        };
        text.push_str(&format!(
            "R\t{}\t{}\t{form}\t{}\n",
            entry.id.0,
            entry.kind.name(),
            escape(&value)
        ));
        count = count.saturating_add(1);
    }
    if count == 0 {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| ReaperError::JournalFormat("journal has no parent".to_owned()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    let _persisted = temp
        .persist(path)
        .map_err(|error| ReaperError::Journal(error.error))?;
    Ok(())
}
