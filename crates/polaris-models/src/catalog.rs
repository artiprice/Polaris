//! Deterministic catalog/effort policy with explicit data provenance.
use crate::ModelsError;
use polaris_config::EffortModels;
use polaris_sdk::{
    effort::Effort,
    ids::{MicroUsd, ModelId},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Provider implementation selected by the composition root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Nebius Token Factory OpenAI-compatible API.
    TokenFactory,
    /// An explicitly configured compatible provider, including a local server.
    OpenAiCompatible,
}

/// Task type supported by the configured entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// Text chat completions.
    Chat,
    /// Chat with image input.
    Vision,
    /// Text embedding vectors.
    Embedding,
}

/// Capability provenance; an absent observation is not support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    /// Not checked with the actual provider/checkpoint.
    Unknown,
    /// Explicitly supported by the supplied catalog or a recorded provider probe.
    Supported,
    /// Explicitly unsupported.
    Unsupported,
}

/// Data provenance, distinct from current backend health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogProvenance {
    /// Dated repository snapshot. No claim that the endpoint is available now.
    Snapshot {
        /// Snapshot date as recorded in the plan.
        date: String,
    },
    /// User-configured model and capability metadata.
    Configured,
    /// Catalog data refreshed by the real provider adapter.
    ProviderObserved {
        /// Provider adapter's observation timestamp.
        at: polaris_sdk::contracts::Timestamp,
    },
}

/// Last observed provider availability, never inferred from an id being registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// No current provider health/catalog observation.
    Unknown,
    /// Explicitly observed as available.
    Available,
    /// Explicitly observed as unavailable.
    Unavailable,
}

/// Configured provider pricing in micro-dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceTable {
    /// Uncached input rate.
    pub input_per_million: MicroUsd,
    /// Output rate.
    pub output_per_million: MicroUsd,
    /// Cached input rate, absent when the provider does not supply it.
    pub cached_input_per_million: Option<MicroUsd>,
}

/// Explicit metadata for one model; no credentials or bearer tokens live here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpec {
    /// Exact provider id.
    pub id: ModelId,
    /// Chat, vision or embeddings.
    pub kind: ModelKind,
    /// Configured provider family.
    pub provider: ProviderKind,
    /// Configured/observed token context limit, absent when contradictory or unknown.
    pub context_tokens: Option<u32>,
    /// Tool-call support evidence.
    pub tools: CapabilitySupport,
    /// Reasoning-control support evidence.
    pub reasoning: CapabilitySupport,
    /// User-supplied pricing; no invented prices.
    pub price: Option<PriceTable>,
    /// Metadata source.
    pub provenance: CatalogProvenance,
    /// Last known provider availability.
    pub availability: Availability,
    /// Timestamp of the availability observation, independent of metadata provenance.
    pub availability_observed_at: Option<polaris_sdk::contracts::Timestamp>,
}

/// Reasoning intent. The provider adapter must validate how this request is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningBudget {
    /// Provider default behavior.
    Default,
    /// Higher reasoning request; actual provider support remains separately recorded.
    High,
}

/// Configured selection, not an execution permit or a promise of current provider health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChoice {
    /// Exact registered id.
    pub model: ModelId,
    /// Desired reasoning policy.
    pub reasoning: ReasoningBudget,
}

/// Inclusive effort override. Earlier declared ranges win overlaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEffort {
    /// Exact id or a registered alias.
    pub model: String,
    /// Lowest effort covered.
    pub min: Effort,
    /// Highest effort covered.
    pub max: Effort,
    /// Requested reasoning policy.
    pub reasoning: ReasoningBudget,
}

/// Overlap/gap report for an applied effort override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideReport {
    /// Efforts covered by multiple declarations; the first declaration won.
    pub overlaps: Vec<Effort>,
    /// Efforts which retained the registry's configured base mapping.
    pub defaults: Vec<Effort>,
}

/// Explicit registry input; endpoint/credential objects belong to provider adapters.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    /// Registered model metadata.
    pub models: Vec<ModelSpec>,
    /// Alias to exact registered id.
    pub aliases: Vec<(String, ModelId)>,
    /// Complete base effort mapping.
    pub efforts: EffortModels,
}

/// Cascade planning only; no speculative-decoding or latency claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CascadePlan {
    /// Lower-effort draft selection.
    pub draft: ModelChoice,
    /// Requested verification selection.
    pub verify: ModelChoice,
    /// True if config maps both phases to one identical model/reasoning selection.
    pub same_selection: bool,
}

/// Single source of configured model identities and deterministic effort policy.
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    models: BTreeMap<ModelId, ModelSpec>,
    aliases: BTreeMap<String, ModelId>,
    base: BTreeMap<Effort, ModelChoice>,
    overrides: BTreeMap<Effort, ModelChoice>,
}

impl ModelRegistry {
    /// Build the dated PLAN catalog without asserting live provider availability.
    ///
    /// # Errors
    /// Invalid static id or inconsistent catalog metadata.
    pub fn builtin() -> Result<Self, ModelsError> {
        let nano = id("nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B")?;
        let lightning = id("nvidia/Nemotron-3_5-Lightning")?;
        let super_model = id("nvidia/nemotron-3-super-120b-a12b")?;
        let ultra = id("nvidia/Nemotron-3-Ultra-550b-a55b")?;
        let vision = id("nvidia/Nemotron-3.5-Super-VL")?;
        let embedding = id("Qwen/Qwen3-Embedding-8B")?;
        let rows = [
            ("nano", nano.clone(), ModelKind::Chat),
            ("lightning", lightning.clone(), ModelKind::Chat),
            ("super", super_model.clone(), ModelKind::Chat),
            ("ultra", ultra.clone(), ModelKind::Chat),
            ("vl", vision, ModelKind::Vision),
            // The PLAN records contradictory embedding context metadata, so no bound is invented.
            ("embed", embedding, ModelKind::Embedding),
        ];
        let models = rows
            .iter()
            .map(|(_, id, kind)| ModelSpec {
                id: id.clone(),
                kind: *kind,
                provider: ProviderKind::TokenFactory,
                // K/M shorthand in the plan is not an exact provider token limit. Preserve unknown
                // until the adapter supplies precise data instead of guessing decimal/binary units.
                context_tokens: None,
                tools: CapabilitySupport::Unknown,
                reasoning: CapabilitySupport::Unknown,
                price: None,
                provenance: CatalogProvenance::Snapshot {
                    date: "2026-10-20".to_owned(),
                },
                availability: Availability::Unknown,
                availability_observed_at: None,
            })
            .collect();
        let aliases = rows
            .into_iter()
            .map(|(alias, id, _)| (alias.to_owned(), id))
            .collect();
        Self::with_config(RegistryConfig {
            models,
            aliases,
            efforts: EffortModels {
                low: nano,
                medium: lightning,
                high: super_model.clone(),
                xhigh: super_model,
                max: ultra,
            },
        })
    }

    /// Construct from explicit metadata. All mapped models must be registered chat models.
    ///
    /// # Errors
    /// Duplicates, invalid aliases, non-chat effort selections or exceeded registry bounds.
    pub fn with_config(config: RegistryConfig) -> Result<Self, ModelsError> {
        if config.models.is_empty() || config.models.len() > 4096 || config.aliases.len() > 4096 {
            return Err(ModelsError::Config("catalog size"));
        }
        let mut models = BTreeMap::new();
        for model in config.models {
            if model.context_tokens == Some(0) {
                return Err(ModelsError::Config("zero context limit"));
            }
            if models.insert(model.id.clone(), model).is_some() {
                return Err(ModelsError::Config("duplicate model"));
            }
        }
        let mut aliases = BTreeMap::new();
        for (alias, id) in config.aliases {
            if !valid_alias(&alias)
                || !models.contains_key(&id)
                || models.keys().any(|model| model.as_str() == alias)
            {
                return Err(ModelsError::Config(
                    "alias does not identify a unique model",
                ));
            }
            if aliases.insert(alias, id).is_some() {
                return Err(ModelsError::Config("duplicate alias"));
            }
        }
        let mut base = BTreeMap::new();
        for effort in Effort::ALL {
            let model = config.efforts.model(effort);
            let spec = models.get(model).ok_or(ModelsError::UnknownModel)?;
            if !matches!(spec.kind, ModelKind::Chat | ModelKind::Vision) {
                return Err(ModelsError::Config("non-chat effort model"));
            }
            let _old = base.insert(
                effort,
                ModelChoice {
                    model: model.clone(),
                    reasoning: match effort {
                        Effort::XHigh => ReasoningBudget::High,
                        Effort::Low | Effort::Medium | Effort::High | Effort::Max => {
                            ReasoningBudget::Default
                        }
                    },
                },
            );
        }
        Ok(Self {
            models,
            aliases,
            base,
            overrides: BTreeMap::new(),
        })
    }

    /// Metadata for an exact registered id.
    #[must_use]
    pub fn get(&self, model: &ModelId) -> Option<&ModelSpec> {
        self.models.get(model)
    }

    /// Deterministic catalog listing, ordered by exact id.
    pub fn models(&self) -> impl Iterator<Item = &ModelSpec> {
        self.models.values()
    }

    /// Resolve a registered alias or exact id without accepting arbitrary unknown provider names.
    ///
    /// # Errors
    /// Unknown/invalid names.
    pub fn resolve(&self, name: &str) -> Result<ModelId, ModelsError> {
        if let Some(model) = self.aliases.get(name) {
            return Ok(model.clone());
        }
        let model = id(name)?;
        if self.models.contains_key(&model) {
            Ok(model)
        } else {
            Err(ModelsError::UnknownModel)
        }
    }

    /// Resolve an effort under the current configured override layer.
    ///
    /// # Errors
    /// Inconsistent registry mapping.
    pub fn model_for(&self, effort: Effort) -> Result<ModelChoice, ModelsError> {
        let choice = match self.overrides.get(&effort) {
            Some(choice) => choice,
            None => self
                .base
                .get(&effort)
                .ok_or(ModelsError::Config("effort gap"))?,
        };
        Ok(choice.clone())
    }

    /// Select only within an explicit session/frame effort and model allowlist.
    ///
    /// `allowed_efforts` is mandatory. An empty `allowed_models` slice means no additional model
    /// filter; callers requiring a model-specific policy must supply that filter. This selects
    /// configuration and does not issue an authorization witness or establish provider support.
    ///
    /// # Errors
    /// Empty/denied effort constraints, a denied model filter, or an observed unavailable model;
    /// no automatic escalation outside the supplied constraints.
    pub fn constrained_model(
        &self,
        effort: Effort,
        allowed_efforts: &[Effort],
        allowed_models: &[ModelId],
    ) -> Result<ModelChoice, ModelsError> {
        if !allowed_efforts.contains(&effort) {
            return Err(ModelsError::EffortDenied);
        }
        let choice = self.model_for(effort)?;
        if !allowed_models.is_empty() && !allowed_models.contains(&choice.model) {
            return Err(ModelsError::EffortDenied);
        }
        if self
            .get(&choice.model)
            .is_some_and(|spec| spec.availability == Availability::Unavailable)
        {
            return Err(ModelsError::Load(crate::LoadError::Unavailable));
        }
        Ok(choice)
    }

    /// Replace the override layer atomically after validating every declaration.
    ///
    /// # Errors
    /// Unknown models, reversed ranges, non-chat targets or too many declarations.
    pub fn apply_subagent_models(
        &mut self,
        declarations: &[ModelEffort],
    ) -> Result<OverrideReport, ModelsError> {
        if declarations.len() > 256 {
            return Err(ModelsError::Config("too many effort overrides"));
        }
        let mut overrides = BTreeMap::new();
        let mut overlaps = BTreeSet::new();
        for declaration in declarations {
            if declaration.min > declaration.max {
                return Err(ModelsError::Config("reversed effort range"));
            }
            let model = self.resolve(&declaration.model)?;
            let spec = self.get(&model).ok_or(ModelsError::UnknownModel)?;
            if spec.kind == ModelKind::Embedding {
                return Err(ModelsError::Config("embedding cannot serve chat effort"));
            }
            for effort in Effort::ALL {
                if effort < declaration.min || effort > declaration.max {
                    continue;
                }
                match overrides.entry(effort) {
                    std::collections::btree_map::Entry::Occupied(_) => {
                        let _inserted = overlaps.insert(effort);
                    }
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        let _choice = entry.insert(ModelChoice {
                            model: model.clone(),
                            reasoning: declaration.reasoning,
                        });
                    }
                }
            }
        }
        let defaults = Effort::ALL
            .into_iter()
            .filter(|effort| !overrides.contains_key(effort))
            .collect();
        self.overrides = overrides;
        Ok(OverrideReport {
            overlaps: overlaps.into_iter().collect(),
            defaults,
        })
    }

    /// Build a two-call cascade plan from the configured lower-effort and requested selections.
    ///
    /// # Errors
    /// Inconsistent effort mapping.
    pub fn cascade_for(&self, effort: Effort) -> Result<CascadePlan, ModelsError> {
        let draft_effort = match effort {
            Effort::Low | Effort::Medium => Effort::Low,
            Effort::High => Effort::Medium,
            Effort::XHigh => Effort::High,
            Effort::Max => Effort::XHigh,
        };
        let draft = self.model_for(draft_effort)?;
        let verify = self.model_for(effort)?;
        Ok(CascadePlan {
            same_selection: draft == verify,
            draft,
            verify,
        })
    }

    /// Apply actual adapter observations without treating an unknown/omitted entry as available.
    ///
    /// # Errors
    /// Unknown ids or duplicate observation rows.
    pub fn observe_availability(
        &mut self,
        observations: &[(ModelId, Availability)],
        at: polaris_sdk::contracts::Timestamp,
    ) -> Result<(), ModelsError> {
        let mut seen = BTreeSet::new();
        for (model, _) in observations {
            let spec = self.models.get(model).ok_or(ModelsError::UnknownModel)?;
            if !seen.insert(model) {
                return Err(ModelsError::Config("duplicate availability observation"));
            }
            if spec
                .availability_observed_at
                .is_some_and(|previous| previous > at)
            {
                return Err(ModelsError::Config("stale availability observation"));
            }
        }
        for (model, availability) in observations {
            let spec = self
                .models
                .get_mut(model)
                .ok_or(ModelsError::UnknownModel)?;
            spec.availability = *availability;
            spec.availability_observed_at = Some(at);
        }
        Ok(())
    }
}

fn valid_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= 64
        && alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn id(name: &str) -> Result<ModelId, ModelsError> {
    ModelId::new(name).map_err(|_| ModelsError::UnknownModel)
}
