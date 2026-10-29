use std::error::Error;

use polaris_config::EffortModels;
use polaris_models::{
    Availability, CapabilitySupport, CatalogProvenance, ModelChoice, ModelEffort, ModelKind,
    ModelRegistry, ModelSpec, ProviderKind, ReasoningBudget, RegistryConfig,
};
use polaris_sdk::{contracts::Timestamp, effort::Effort, ids::ModelId};

const NANO: &str = "nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B";
const LIGHTNING: &str = "nvidia/Nemotron-3_5-Lightning";
const SUPER: &str = "nvidia/nemotron-3-super-120b-a12b";
const ULTRA: &str = "nvidia/Nemotron-3-Ultra-550b-a55b";
const VISION: &str = "nvidia/Nemotron-3.5-Super-VL";
const EMBEDDING: &str = "Qwen/Qwen3-Embedding-8B";

fn model_id(value: &str) -> Result<ModelId, Box<dyn Error>> {
    Ok(ModelId::new(value)?)
}

fn spec(value: &str, kind: ModelKind) -> Result<ModelSpec, Box<dyn Error>> {
    Ok(ModelSpec {
        id: model_id(value)?,
        kind,
        provider: ProviderKind::OpenAiCompatible,
        context_tokens: None,
        tools: CapabilitySupport::Unknown,
        reasoning: CapabilitySupport::Unknown,
        price: None,
        provenance: CatalogProvenance::Configured,
        availability: Availability::Unknown,
        availability_observed_at: None,
    })
}

fn config(
    models: Vec<ModelSpec>,
    aliases: Vec<(String, ModelId)>,
) -> Result<RegistryConfig, Box<dyn Error>> {
    Ok(RegistryConfig {
        models,
        aliases,
        efforts: EffortModels {
            low: model_id(NANO)?,
            medium: model_id(LIGHTNING)?,
            high: model_id(SUPER)?,
            xhigh: model_id(SUPER)?,
            max: model_id(ULTRA)?,
        },
    })
}

#[test]
fn builtin_contains_dated_snapshot_ids_and_effort_mapping() -> Result<(), Box<dyn Error>> {
    let registry = ModelRegistry::builtin()?;
    let ids: Vec<&str> = registry.models().map(|model| model.id.as_str()).collect();
    let expected = [NANO, LIGHTNING, SUPER, ULTRA, VISION, EMBEDDING];
    assert_eq!(
        ids.len(),
        expected.len(),
        "built-in snapshot row count changed"
    );
    for expected_id in expected {
        assert!(
            ids.contains(&expected_id),
            "missing snapshot id: {expected_id}"
        );
        let row = registry.get(&model_id(expected_id)?);
        assert!(row.is_some(), "snapshot id must resolve through get");
        let row = row.ok_or("snapshot row unexpectedly absent")?;
        assert_eq!(
            row.provider,
            ProviderKind::TokenFactory,
            "snapshot provider changed"
        );
        assert_eq!(
            row.context_tokens, None,
            "unverified context must remain unknown"
        );
        assert_eq!(
            row.tools,
            CapabilitySupport::Unknown,
            "capability must remain unverified"
        );
        assert_eq!(
            row.reasoning,
            CapabilitySupport::Unknown,
            "capability must remain unverified"
        );
        assert_eq!(row.price, None, "snapshot must not invent prices");
        assert_eq!(
            row.availability,
            Availability::Unknown,
            "snapshot is not a live health check"
        );
        assert_eq!(
            row.availability_observed_at, None,
            "no provider health observation exists"
        );
        assert_eq!(
            row.provenance,
            CatalogProvenance::Snapshot {
                date: "2026-10-20".to_owned()
            },
            "snapshot provenance date changed"
        );
    }
    assert_eq!(
        registry.resolve("nano")?,
        model_id(NANO)?,
        "nano alias changed"
    );
    assert_eq!(
        registry.resolve("lightning")?,
        model_id(LIGHTNING)?,
        "lightning alias changed"
    );
    assert_eq!(
        registry.resolve("super")?,
        model_id(SUPER)?,
        "super alias changed"
    );
    assert_eq!(
        registry.resolve("ultra")?,
        model_id(ULTRA)?,
        "ultra alias changed"
    );
    assert_eq!(
        registry.resolve("vl")?,
        model_id(VISION)?,
        "vision alias changed"
    );
    assert_eq!(
        registry.resolve("embed")?,
        model_id(EMBEDDING)?,
        "embedding alias changed"
    );
    let low = registry.model_for(Effort::Low)?;
    let medium = registry.model_for(Effort::Medium)?;
    let high = registry.model_for(Effort::High)?;
    let xhigh = registry.model_for(Effort::XHigh)?;
    let max = registry.model_for(Effort::Max)?;
    assert_eq!(low.model, model_id(NANO)?, "low default changed");
    assert_eq!(medium.model, model_id(LIGHTNING)?, "medium default changed");
    assert_eq!(high.model, model_id(SUPER)?, "high default changed");
    assert_eq!(xhigh.model, model_id(SUPER)?, "xhigh default model changed");
    assert_eq!(
        xhigh.reasoning,
        ReasoningBudget::High,
        "xhigh reasoning budget changed"
    );
    assert_eq!(max.model, model_id(ULTRA)?, "max default changed");
    assert_eq!(
        registry.resolve("not-registered"),
        Err(polaris_models::ModelsError::UnknownModel),
        "unknown exact ids must not be implicitly registered"
    );
    Ok(())
}

#[test]
fn overrides_are_first_wins_and_report_gaps() -> Result<(), Box<dyn Error>> {
    let mut registry = ModelRegistry::builtin()?;
    let report = registry.apply_subagent_models(&[
        ModelEffort {
            model: "lightning".to_owned(),
            min: Effort::Low,
            max: Effort::High,
            reasoning: ReasoningBudget::Default,
        },
        ModelEffort {
            model: "ultra".to_owned(),
            min: Effort::High,
            max: Effort::XHigh,
            reasoning: ReasoningBudget::High,
        },
    ])?;
    assert_eq!(
        report.overlaps,
        vec![Effort::High],
        "inclusive overlap report changed"
    );
    assert_eq!(
        report.defaults,
        vec![Effort::Max],
        "uncovered effort report changed"
    );
    assert_eq!(
        registry.model_for(Effort::Low)?.model,
        model_id(LIGHTNING)?,
        "first range not applied"
    );
    assert_eq!(
        registry.model_for(Effort::High)?.model,
        model_id(LIGHTNING)?,
        "first declaration must win overlap"
    );
    assert_eq!(
        registry.model_for(Effort::XHigh)?.model,
        model_id(ULTRA)?,
        "second non-overlapping effort not applied"
    );
    assert_eq!(
        registry.model_for(Effort::Max)?.model,
        model_id(ULTRA)?,
        "gap must retain base mapping"
    );
    Ok(())
}

#[test]
fn invalid_override_is_atomic() -> Result<(), Box<dyn Error>> {
    let mut registry = ModelRegistry::builtin()?;
    let before = registry.apply_subagent_models(&[ModelEffort {
        model: "lightning".to_owned(),
        min: Effort::Low,
        max: Effort::Low,
        reasoning: ReasoningBudget::Default,
    }])?;
    let prior_low = registry.model_for(Effort::Low)?;
    let prior_high = registry.model_for(Effort::High)?;
    let result = registry.apply_subagent_models(&[
        ModelEffort {
            model: "ultra".to_owned(),
            min: Effort::Max,
            max: Effort::Max,
            reasoning: ReasoningBudget::Default,
        },
        ModelEffort {
            model: "unknown-model".to_owned(),
            min: Effort::Low,
            max: Effort::High,
            reasoning: ReasoningBudget::Default,
        },
    ]);
    assert!(
        result.is_err(),
        "unknown model must reject the whole update"
    );
    assert_eq!(
        registry.model_for(Effort::Low)?,
        prior_low,
        "failed update partially replaced low"
    );
    assert_eq!(
        registry.model_for(Effort::High)?,
        prior_high,
        "failed update partially replaced high"
    );
    assert_eq!(
        before.defaults,
        vec![Effort::Medium, Effort::High, Effort::XHigh, Effort::Max],
        "precondition changed"
    );
    Ok(())
}

#[test]
fn custom_registry_is_bounded_and_rejects_unsafe_aliases() -> Result<(), Box<dyn Error>> {
    let custom = model_id("local/ollama-qwen")?;
    let registry = ModelRegistry::with_config(RegistryConfig {
        models: vec![
            spec("local/ollama-qwen", ModelKind::Chat)?,
            spec("local/vision", ModelKind::Vision)?,
            spec("local/embed", ModelKind::Embedding)?,
        ],
        aliases: vec![("local-chat".to_owned(), custom.clone())],
        efforts: EffortModels {
            low: custom.clone(),
            medium: custom.clone(),
            high: custom.clone(),
            xhigh: custom.clone(),
            max: custom.clone(),
        },
    })?;
    assert_eq!(
        registry.resolve("local-chat")?,
        custom,
        "custom alias resolution changed"
    );
    assert!(
        registry.resolve("../secret").is_err(),
        "path-like arbitrary name must not resolve"
    );

    let mut too_many = Vec::new();
    for index in 0..=4096_u32 {
        too_many.push(spec(&format!("model-{index}"), ModelKind::Chat)?);
    }
    let over_limit = config(too_many, Vec::new())?;
    assert!(
        ModelRegistry::with_config(over_limit).is_err(),
        "model count bound must be enforced"
    );

    let invalid_alias = config(
        vec![spec("local/chat", ModelKind::Chat)?],
        vec![("../key".to_owned(), model_id("local/chat")?)],
    )?;
    assert!(
        ModelRegistry::with_config(invalid_alias).is_err(),
        "unsafe alias must be rejected"
    );

    let alias_target = model_id("local/chat")?;
    let mut too_many_aliases = Vec::new();
    for index in 0..=4096_u32 {
        too_many_aliases.push((format!("alias-{index}"), alias_target.clone()));
    }
    let alias_limit = config(vec![spec("local/chat", ModelKind::Chat)?], too_many_aliases)?;
    assert!(
        ModelRegistry::with_config(alias_limit).is_err(),
        "alias count bound must be enforced"
    );

    let embedding_effort = RegistryConfig {
        models: vec![spec("local/embed", ModelKind::Embedding)?],
        aliases: Vec::new(),
        efforts: EffortModels {
            low: model_id("local/embed")?,
            medium: model_id("local/embed")?,
            high: model_id("local/embed")?,
            xhigh: model_id("local/embed")?,
            max: model_id("local/embed")?,
        },
    };
    assert!(
        ModelRegistry::with_config(embedding_effort).is_err(),
        "embedding cannot serve chat effort"
    );
    Ok(())
}

#[test]
fn constrained_selection_enforces_effort_model_and_availability() -> Result<(), Box<dyn Error>> {
    let mut registry = ModelRegistry::builtin()?;
    let nano = model_id(NANO)?;
    let permitted =
        registry.constrained_model(Effort::Low, &[Effort::Low], std::slice::from_ref(&nano))?;
    assert_eq!(permitted.model, nano, "allowed selection changed");
    assert_eq!(
        registry.constrained_model(Effort::High, &[Effort::Low], &[]),
        Err(polaris_models::ModelsError::EffortDenied),
        "effort outside allowlist must be denied"
    );
    assert_eq!(
        registry.constrained_model(Effort::Low, &[Effort::Low], &[model_id(ULTRA)?]),
        Err(polaris_models::ModelsError::EffortDenied),
        "model outside allowlist must be denied"
    );
    registry.observe_availability(
        &[(nano.clone(), Availability::Unavailable)],
        Timestamp::from_second(100)?,
    )?;
    assert_eq!(
        registry.constrained_model(Effort::Low, &[Effort::Low], &[]),
        Err(polaris_models::ModelsError::Load(
            polaris_models::LoadError::Unavailable
        )),
        "explicitly unavailable model must be denied"
    );
    assert_eq!(
        registry
            .get(&model_id(LIGHTNING)?)
            .map(|row| row.availability),
        Some(Availability::Unknown),
        "unobserved model must remain unknown"
    );
    Ok(())
}

#[test]
fn availability_observations_validate_atomically_and_monotonically() -> Result<(), Box<dyn Error>> {
    let mut registry = ModelRegistry::builtin()?;
    let nano = model_id(NANO)?;
    let lightning = model_id(LIGHTNING)?;
    registry.observe_availability(
        &[(nano.clone(), Availability::Available)],
        Timestamp::from_second(20)?,
    )?;
    let unknown = model_id("missing/model")?;
    let result = registry.observe_availability(
        &[
            (lightning.clone(), Availability::Unavailable),
            (unknown, Availability::Available),
        ],
        Timestamp::from_second(30)?,
    );
    assert!(result.is_err(), "unknown observation must reject batch");
    assert_eq!(
        registry.get(&lightning).map(|row| row.availability),
        Some(Availability::Unknown),
        "batch changed a row before detecting unknown id"
    );
    assert!(
        registry
            .observe_availability(
                &[
                    (lightning.clone(), Availability::Available),
                    (lightning.clone(), Availability::Unavailable)
                ],
                Timestamp::from_second(30)?
            )
            .is_err(),
        "duplicate observation must reject batch"
    );
    assert_eq!(
        registry.get(&lightning).map(|row| row.availability),
        Some(Availability::Unknown),
        "duplicate batch partially mutated state"
    );
    assert!(
        registry
            .observe_availability(
                &[(nano.clone(), Availability::Unavailable)],
                Timestamp::from_second(19)?
            )
            .is_err(),
        "stale observation must reject"
    );
    assert_eq!(
        registry.get(&nano).map(|row| row.availability),
        Some(Availability::Available),
        "stale observation changed prior availability"
    );
    registry.observe_availability(
        &[(lightning.clone(), Availability::Available)],
        Timestamp::from_second(30)?,
    )?;
    let observed = registry.get(&lightning).ok_or("observed row absent")?;
    assert_eq!(
        observed.availability,
        Availability::Available,
        "fresh observation not applied"
    );
    assert_eq!(
        observed.availability_observed_at,
        Some(Timestamp::from_second(30)?),
        "observation timestamp not applied"
    );
    Ok(())
}

#[test]
fn cascade_uses_lower_effort_draft_and_requested_verification() -> Result<(), Box<dyn Error>> {
    let registry = ModelRegistry::builtin()?;
    let high = registry.cascade_for(Effort::High)?;
    assert_eq!(
        high.draft.model,
        model_id(LIGHTNING)?,
        "high effort draft must use lower medium tier"
    );
    assert_eq!(
        high.verify.model,
        model_id(SUPER)?,
        "high effort verify mapping changed"
    );
    assert!(
        !high.same_selection,
        "different model selections must not be marked identical"
    );
    let xhigh = registry.cascade_for(Effort::XHigh)?;
    assert_eq!(
        xhigh.draft.model,
        model_id(SUPER)?,
        "xhigh draft must use lower high tier"
    );
    assert_eq!(
        xhigh.verify.model,
        model_id(SUPER)?,
        "xhigh verification mapping changed"
    );
    assert!(
        !xhigh.same_selection,
        "different reasoning selections must not be marked identical"
    );
    let low = registry.cascade_for(Effort::Low)?;
    assert!(
        low.same_selection,
        "low draft and verify are identical selections"
    );
    assert_eq!(
        low.draft,
        ModelChoice {
            model: model_id(NANO)?,
            reasoning: ReasoningBudget::Default
        },
        "low cascade selection changed"
    );
    Ok(())
}
