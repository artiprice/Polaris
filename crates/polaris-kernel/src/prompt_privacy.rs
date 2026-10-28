//! Optional local semantic PII inspection before any provider-bound tokenization.
use crate::KernelPromptError;
use polaris_sdk::{
    registry::{ChatMessage, ChatRequest},
    system_one::PiiSpanDetector,
};
use polaris_security::pii::PiiVault;
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

/// Total semantic work allowed during one complete prompt assembly, including archived history.
#[derive(Debug, Clone, Copy)]
pub struct SemanticPrivacyLimits {
    /// Total detector wall-clock allowance; remaining fields fall back to DFA/entropy after it.
    pub deadline: Duration,
    /// Maximum nonempty string fields submitted to NER; remaining fields still receive DFA.
    pub max_fields: usize,
}
impl Default for SemanticPrivacyLimits {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(5),
            max_fields: 512,
        }
    }
}
/// Actual semantic coverage for the latest successfully sanitized assembly, not a DLP guarantee.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SemanticPrivacyReport {
    /// Nonempty fields for which detection succeeded, including legitimate empty span results.
    pub ner_fields: usize,
    /// Nonempty fields receiving deterministic fallback due to bounds or unavailable NER.
    pub degraded_fields: usize,
}
type PrivacyFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SemanticPrivacyReport, KernelPromptError>> + Send + 'a>>;
trait PromptSanitizer: Send + Sync {
    fn texts<'a>(&'a self, vault: &'a PiiVault, texts: &'a mut [String]) -> PrivacyFuture<'a>;
    fn sanitize<'a>(
        &'a self,
        vault: &'a PiiVault,
        request: &'a mut ChatRequest,
        history: &'a mut [ChatMessage],
    ) -> PrivacyFuture<'a>;
}
struct Adapter<D> {
    detector: Arc<D>,
    limits: SemanticPrivacyLimits,
}
pub(crate) struct SemanticPrivacy {
    sanitizer: Arc<dyn PromptSanitizer>,
    report: Mutex<Option<SemanticPrivacyReport>>,
}
impl SemanticPrivacy {
    pub(crate) async fn sanitize_texts(
        &self,
        vault: &PiiVault,
        texts: &mut [String],
    ) -> Result<(), KernelPromptError> {
        let _report = self.sanitizer.texts(vault, texts).await?;
        Ok(())
    }
    pub(crate) fn new<D: PiiSpanDetector + 'static>(
        detector: Arc<D>,
        limits: SemanticPrivacyLimits,
    ) -> Result<Self, KernelPromptError> {
        if limits.deadline.is_zero()
            || limits.deadline > Duration::from_secs(120)
            || limits.max_fields == 0
            || limits.max_fields > 4096
        {
            return Err(KernelPromptError::Limits);
        }
        Ok(Self {
            sanitizer: Arc::new(Adapter { detector, limits }),
            report: Mutex::new(None),
        })
    }
    pub(crate) fn report(&self) -> Result<Option<SemanticPrivacyReport>, KernelPromptError> {
        Ok(*self.report.lock().map_err(|_| KernelPromptError::Privacy)?)
    }
    pub(crate) async fn sanitize(
        &self,
        vault: &PiiVault,
        request: &mut ChatRequest,
        history: &mut [ChatMessage],
    ) -> Result<(), KernelPromptError> {
        *self.report.lock().map_err(|_| KernelPromptError::Privacy)? = None;
        let report = self.sanitizer.sanitize(vault, request, history).await?;
        *self.report.lock().map_err(|_| KernelPromptError::Privacy)? = Some(report);
        Ok(())
    }
}
struct Work<'a, D> {
    detector: &'a D,
    vault: &'a PiiVault,
    deadline: Instant,
    remaining: usize,
    report: SemanticPrivacyReport,
}
impl<D: PiiSpanDetector> Work<'_, D> {
    async fn text(&mut self, text: &mut String) -> Result<(), KernelPromptError> {
        if text.is_empty() {
            return Ok(());
        }
        let time_left = self.deadline.checked_duration_since(Instant::now());
        let masked = match time_left {
            Some(timeout) if !timeout.is_zero() && self.remaining > 0 => {
                self.remaining = self
                    .remaining
                    .checked_sub(1)
                    .ok_or(KernelPromptError::Limits)?;
                self.vault.mask_with_ner(self.detector, text, timeout).await
            }
            Some(_) | None => self.vault.mask(text),
        }
        .map_err(|_| KernelPromptError::Privacy)?;
        if masked.report.ner_used && !masked.report.degraded {
            self.report.ner_fields = self
                .report
                .ner_fields
                .checked_add(1)
                .ok_or(KernelPromptError::Limits)?;
        } else {
            self.report.degraded_fields = self
                .report
                .degraded_fields
                .checked_add(1)
                .ok_or(KernelPromptError::Limits)?;
        }
        *text = masked.text;
        Ok(())
    }
    fn value<'a>(
        &'a mut self,
        value: &'a mut Value,
    ) -> Pin<Box<dyn Future<Output = Result<(), KernelPromptError>> + Send + 'a>> {
        Box::pin(async move {
            match value {
                Value::String(text) => self.text(text).await?,
                Value::Array(values) => {
                    for value in values {
                        self.value(value).await?;
                    }
                }
                Value::Object(values) => {
                    for value in values.values_mut() {
                        self.value(value).await?;
                    }
                }
                Value::Null | Value::Bool(_) | Value::Number(_) => {}
            }
            Ok(())
        })
    }
    async fn json(&mut self, text: &mut String) -> Result<(), KernelPromptError> {
        let mut value: Value =
            serde_json::from_str(text).map_err(|_| KernelPromptError::History)?;
        self.value(&mut value).await?;
        *text = serde_json::to_string(&value).map_err(|_| KernelPromptError::History)?;
        Ok(())
    }
    async fn message(&mut self, message: &mut ChatMessage) -> Result<(), KernelPromptError> {
        self.text(&mut message.content).await?;
        if let Some(reasoning) = message.reasoning.as_mut() {
            self.text(reasoning).await?;
        }
        for call in &mut message.tool_calls {
            self.json(&mut call.arguments_json).await?;
        }
        Ok(())
    }
}
impl<D: PiiSpanDetector + 'static> PromptSanitizer for Adapter<D> {
    fn texts<'a>(&'a self, vault: &'a PiiVault, texts: &'a mut [String]) -> PrivacyFuture<'a> {
        Box::pin(async move {
            let deadline = Instant::now()
                .checked_add(self.limits.deadline)
                .ok_or(KernelPromptError::Limits)?;
            let mut work = Work {
                detector: self.detector.as_ref(),
                vault,
                deadline,
                remaining: self.limits.max_fields,
                report: SemanticPrivacyReport::default(),
            };
            for text in texts {
                work.text(text).await?;
            }
            Ok(work.report)
        })
    }
    fn sanitize<'a>(
        &'a self,
        vault: &'a PiiVault,
        request: &'a mut ChatRequest,
        history: &'a mut [ChatMessage],
    ) -> PrivacyFuture<'a> {
        Box::pin(async move {
            let deadline = Instant::now()
                .checked_add(self.limits.deadline)
                .ok_or(KernelPromptError::Limits)?;
            let mut work = Work {
                detector: self.detector.as_ref(),
                vault,
                deadline,
                remaining: self.limits.max_fields,
                report: SemanticPrivacyReport::default(),
            };
            for message in &mut request.messages {
                work.message(message).await?;
            }
            for message in history {
                work.message(message).await?;
            }
            for tool in &mut request.tools {
                let mut description = tool.description.to_string();
                work.text(&mut description).await?;
                tool.description = description.into();
                let mut schema = tool.input_schema_json.to_string();
                work.json(&mut schema).await?;
                tool.input_schema_json = schema.into();
            }
            Ok(work.report)
        })
    }
}
