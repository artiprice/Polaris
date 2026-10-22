//! Local JSONL export only. Remote API requests are absent until a real API is available.
use crate::{DataLabError, Masked, MessageRole, OptIn, ResidualGuard, Trajectory};
use polaris_sdk::ids::{Cid, SessionId};
use polaris_storage_cas::{
    Cas,
    atomic::{RetryPolicy, write_atomic},
    content_id,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use zeroize::Zeroizing;

/// Local interchange format. These are not claims about a remote provider's accepted schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Conversations with `from`/`value`, preserving tool metadata.
    ShareGptJsonl,
    /// Messages with `role`/`content`, preserving tool metadata.
    ChatMlJsonl,
}

/// Bounded local export settings.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    /// Destination directory controlled by the session owner.
    pub output_directory: PathBuf,
    /// Largest individual JSONL record.
    pub max_line_bytes: usize,
    /// Largest atomically committed export file.
    pub max_file_bytes: usize,
    /// Largest number of trajectories included in one export call.
    pub max_batch: usize,
}

/// Consent-checked preview of exact masked JSONL bytes before a local write.
#[derive(Debug, Clone)]
pub struct ExportPreview {
    /// Number of trajectory records.
    pub trajectories: usize,
    /// Total UTF-8 bytes, including JSONL newline delimiters.
    pub bytes: usize,
    /// Selected local format.
    pub format: ExportFormat,
    /// First complete, already masked record. Never a sample from raw data.
    pub masked_line: String,
}

/// Receipt for an actual atomic local export and digest-verified CAS copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReceipt {
    /// Session whose consent allowed this write.
    pub session: SessionId,
    /// Actual local output path.
    pub path: PathBuf,
    /// Content identity of the exact JSONL file bytes.
    pub content: Cid,
    /// Persisted consent record used at commit time.
    pub consent: Cid,
    /// Trajectories included in the file.
    pub trajectories: usize,
    /// Actual number of bytes written.
    pub bytes: usize,
}

/// Local exporter holding a real CAS handle and revocable session consent.
#[derive(Debug)]
pub struct TrajectoryExporter {
    config: ExportConfig,
    cas: Arc<Cas>,
    optin: OptIn,
    guard: ResidualGuard,
}

impl TrajectoryExporter {
    /// Initialize a local output directory and real residual scanner; makes no network request.
    ///
    /// # Errors
    /// Invalid limits, revoked consent, filesystem or scanner initialization failure.
    pub fn new(config: ExportConfig, cas: Arc<Cas>, optin: OptIn) -> Result<Self, DataLabError> {
        if config.max_line_bytes == 0
            || config.max_file_bytes < config.max_line_bytes
            || config.max_batch == 0
        {
            return Err(DataLabError::Limit);
        }
        let _permit = optin.permit(optin.session())?;
        fs::create_dir_all(&config.output_directory)?;
        let root = fs::canonicalize(&config.output_directory)?;
        if !root.is_dir() {
            return Err(DataLabError::Schema("export directory"));
        }
        let mut config = config;
        config.output_directory = root;
        Ok(Self {
            config,
            cas,
            optin: optin.clone(),
            guard: ResidualGuard::new()?,
        })
    }

    /// Inspect exact masked output without writing a file or transmitting data.
    ///
    /// # Errors
    /// Consent, residual scan, serialization or processing-bound failure.
    pub fn preview(
        &self,
        trajectories: &[&Trajectory<Masked>],
        format: ExportFormat,
    ) -> Result<ExportPreview, DataLabError> {
        let _permit = self.optin.permit(self.optin.session())?;
        let bytes = self.encode_batch(trajectories, format)?;
        let line = bytes
            .split(|byte| *byte == b'\n')
            .next()
            .ok_or(DataLabError::Schema("empty preview"))?;
        let masked_line = std::str::from_utf8(line)
            .map_err(|_| DataLabError::Schema("preview encoding"))?
            .to_owned();
        Ok(ExportPreview {
            trajectories: trajectories.len(),
            bytes: bytes.len(),
            format,
            masked_line,
        })
    }

    /// Persist one bounded JSONL file. Consent is held through the atomic commit so revocation
    /// cannot race a write that has already acquired its permit. The caller owns the private
    /// output directory and its ACLs; no code in this crate uploads these files.
    ///
    /// # Errors
    /// Consent revocation, invalid artifact binding, residual data, CAS or filesystem failure.
    pub fn write_local(
        &self,
        trajectories: &[&Trajectory<Masked>],
        format: ExportFormat,
    ) -> Result<ExportReceipt, DataLabError> {
        let permit = self.optin.permit(self.optin.session())?;
        let bytes = self.encode_batch(trajectories, format)?;
        let content = content_id(&bytes);
        let stored = self.cas.put(&bytes)?;
        if stored != content || self.cas.get(&stored)?.as_ref() != bytes.as_slice() {
            return Err(DataLabError::DigestMismatch);
        }
        let prefix = match format {
            ExportFormat::ShareGptJsonl => "sharegpt",
            ExportFormat::ChatMlJsonl => "chatml",
        };
        let filename = format!(
            "{}-{}-{}.jsonl",
            prefix,
            self.optin.session(),
            content.to_hex()
        );
        let path = self.config.output_directory.join(filename);
        ensure_child(&self.config.output_directory, &path)?;
        write_atomic(&path, &bytes, &RetryPolicy::default())?;
        let actual = Zeroizing::new(fs::read(&path)?);
        if content_id(&actual) != content || actual.as_slice() != bytes.as_slice() {
            return Err(DataLabError::DigestMismatch);
        }
        Ok(ExportReceipt {
            session: self.optin.session(),
            path,
            content,
            consent: permit.record(),
            trajectories: trajectories.len(),
            bytes: bytes.len(),
        })
    }

    fn encode_batch(
        &self,
        trajectories: &[&Trajectory<Masked>],
        format: ExportFormat,
    ) -> Result<Zeroizing<Vec<u8>>, DataLabError> {
        if trajectories.is_empty() || trajectories.len() > self.config.max_batch {
            return Err(DataLabError::Limit);
        }
        let mut output = Zeroizing::new(Vec::new());
        let mut seen = std::collections::BTreeSet::new();
        for trajectory in trajectories {
            if trajectory.input.session != self.optin.session() {
                return Err(DataLabError::NotOptedIn);
            }
            let Some(proof) = trajectory.proof else {
                return Err(DataLabError::NotVerified);
            };
            if !seen.insert((trajectory.input.frame, proof)) {
                return Err(DataLabError::Schema("duplicate trajectory"));
            }
            // Only masked artifact copies can occur here; verify their real bytes on every export.
            for cid in [trajectory.input.diff, trajectory.input.test_run] {
                let bytes = self.cas.get(&cid)?;
                if bytes.len() > trajectory.limits.max_content_bytes {
                    return Err(DataLabError::Limit);
                }
                if content_id(&bytes) != cid {
                    return Err(DataLabError::DigestMismatch);
                }
                self.guard.check(&bytes)?;
            }
            let record = encode_record(trajectory, format)?;
            scan_values(&record, &self.guard, 0)?;
            let line = Zeroizing::new(
                serde_json::to_vec(&record).map_err(|_| DataLabError::Schema("JSONL encoding"))?,
            );
            if line.len() > self.config.max_line_bytes {
                return Err(DataLabError::Limit);
            }
            let size = output
                .len()
                .checked_add(line.len())
                .and_then(|size| size.checked_add(1))
                .ok_or(DataLabError::Limit)?;
            if size > self.config.max_file_bytes {
                return Err(DataLabError::Limit);
            }
            output.extend_from_slice(&line);
            output.push(b'\n');
        }
        Ok(output)
    }
}

fn ensure_child(root: &Path, file: &Path) -> Result<(), DataLabError> {
    if file.parent() != Some(root) || file.file_name().is_none() {
        return Err(DataLabError::Schema("export path containment"));
    }
    Ok(())
}

fn encode_record(
    trajectory: &Trajectory<Masked>,
    format: ExportFormat,
) -> Result<Value, DataLabError> {
    let mut messages = Vec::with_capacity(trajectory.input.messages.len());
    for message in &trajectory.input.messages {
        let content = message.content.masked_text()?;
        let calls: Vec<_> = message
            .tool_calls
            .iter()
            .map(|call| {
                Ok(json!({"id": call.id, "type": "function", "function": {
                    "name": call.name, "arguments": call.arguments.masked_text()?
                }}))
            })
            .collect::<Result<_, DataLabError>>()?;
        let role = match (format, message.role) {
            (ExportFormat::ShareGptJsonl, MessageRole::User) => "human",
            (ExportFormat::ShareGptJsonl, MessageRole::Assistant) => "gpt",
            (_, MessageRole::System) => "system",
            (_, MessageRole::Tool) => "tool",
            (ExportFormat::ChatMlJsonl, MessageRole::User) => "user",
            (ExportFormat::ChatMlJsonl, MessageRole::Assistant) => "assistant",
        };
        let mut value = match format {
            ExportFormat::ShareGptJsonl => json!({"from": role, "value": content}),
            ExportFormat::ChatMlJsonl => json!({"role": role, "content": content}),
        };
        let object = value
            .as_object_mut()
            .ok_or(DataLabError::Schema("message encoding"))?;
        if !calls.is_empty() {
            let _old = object.insert("tool_calls".to_owned(), Value::Array(calls));
        }
        if let Some(id) = &message.tool_call_id {
            let _old = object.insert("tool_call_id".to_owned(), json!(id));
        }
        messages.push(value);
    }
    let provenance = &trajectory.input.provenance;
    let mut record = json!({"schema": 1, "session": trajectory.input.session, "frame": trajectory.input.frame,
        "workspace_snapshot": trajectory.input.workspace_snapshot, "proof": trajectory.proof,
        "masked_diff": trajectory.input.diff, "masked_test_run": trajectory.input.test_run,
        "consent": trajectory.consent, "collected_at": trajectory.input.collected_at,
        "provenance": {"repository": provenance.repository, "license": provenance.license,
            "model": provenance.model, "tool_versions": provenance.tool_versions}});
    let object = record
        .as_object_mut()
        .ok_or(DataLabError::Schema("trajectory encoding"))?;
    let key = match format {
        ExportFormat::ShareGptJsonl => "conversations",
        ExportFormat::ChatMlJsonl => "messages",
    };
    let _old = object.insert(key.to_owned(), Value::Array(messages));
    Ok(record)
}

fn scan_values(value: &Value, guard: &ResidualGuard, depth: usize) -> Result<(), DataLabError> {
    if depth > 64 {
        return Err(DataLabError::Limit);
    }
    let next = depth.saturating_add(1);
    match value {
        Value::String(text) => guard.check(text.as_bytes()),
        Value::Array(values) => {
            for value in values {
                scan_values(value, guard, next)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values {
                guard.check(key.as_bytes())?;
                scan_values(value, guard, next)?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}
