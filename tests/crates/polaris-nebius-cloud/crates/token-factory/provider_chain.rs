//! Provider selection chain: order, skip reasons, local probing and conservative counting.
use polaris_config::{Layer, LayerInput, PolarisConfig, ProviderName, TrustGrant, load};
use polaris_nebius_cloud_token_factory::{
    ProviderCredentials, SkipReason, SkippedProvider, select_provider,
};
use polaris_sdk::{
    effort::Effort,
    ids::ModelId,
    registry::{ChatMessage, ChatModel, ChatRequest, ChatRole, ModelError, PromptTokenizer},
    secrets::{SecretLocation, SecretSource, SecretSourceError},
};
use secrecy::SecretString;
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Fixture source keyed by environment name; it performs no host credential access.
struct Env(BTreeMap<&'static str, &'static str>);
impl SecretSource for Env {
    fn load(&self, location: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError> {
        Ok(match location {
            SecretLocation::Environment(name) => self
                .0
                .get(name.as_str())
                .map(|value| SecretString::from(*value)),
            SecretLocation::Keyring(_) => None,
        })
    }
    fn store(&self, _: &SecretLocation, _: &SecretString) -> Result<(), SecretSourceError> {
        Err(SecretSourceError::Unsupported)
    }
    fn delete(&self, _: &SecretLocation) -> Result<bool, SecretSourceError> {
        Err(SecretSourceError::Unsupported)
    }
}

fn config(extra: &str) -> Result<PolarisConfig, Box<dyn std::error::Error>> {
    let layers = if extra.is_empty() {
        Vec::new()
    } else {
        vec![LayerInput::parse(Layer::Global, extra, None)?]
    };
    Ok(load(&layers, &TrustGrant::none())?.config)
}

#[tokio::test]
async fn first_provider_with_a_key_is_selected_and_earlier_ones_are_explained() -> TestResult {
    let config = config("")?;
    let env = Env(BTreeMap::from([("OPENROUTER_API_KEY", "synthetic-key")]));
    let choice = select_provider(
        &config,
        Effort::Medium,
        ProviderCredentials::resolve(&config, &env),
    )
    .await;
    let selected = choice.selected.ok_or("expected a provider")?;
    assert_eq!(selected.provider, ProviderName::OpenRouter);
    assert_eq!(
        selected.model.model().as_str(),
        "nvidia/nemotron-3.5-lightning:free"
    );
    assert_eq!(
        choice.skipped,
        [SkippedProvider {
            provider: ProviderName::TokenFactory,
            reason: SkipReason::CredentialMissing,
        }]
    );
    assert!(!format!("{:?}", selected.model).contains("synthetic-key"));
    Ok(())
}

#[tokio::test]
async fn effort_selects_the_mapped_model() -> TestResult {
    let config = config("")?;
    let env = Env(BTreeMap::from([("OPENROUTER_API_KEY", "synthetic-key")]));
    let choice = select_provider(
        &config,
        Effort::Max,
        ProviderCredentials::resolve(&config, &env),
    )
    .await;
    let selected = choice.selected.ok_or("expected a provider")?;
    assert_eq!(
        selected.model.model().as_str(),
        "nvidia/nemotron-3-ultra-550b-a55b:free"
    );
    Ok(())
}

#[tokio::test]
async fn no_usable_provider_means_offline_with_every_reason() -> TestResult {
    let config = config("")?;
    let choice = select_provider(
        &config,
        Effort::Medium,
        ProviderCredentials::resolve(&config, &Env(BTreeMap::new())),
    )
    .await;
    assert!(choice.selected.is_none());
    assert_eq!(
        choice.skipped,
        [
            SkippedProvider {
                provider: ProviderName::TokenFactory,
                reason: SkipReason::CredentialMissing,
            },
            SkippedProvider {
                provider: ProviderName::OpenRouter,
                reason: SkipReason::CredentialMissing,
            },
            SkippedProvider {
                provider: ProviderName::Local,
                reason: SkipReason::NotConfigured,
            },
        ]
    );
    Ok(())
}

#[tokio::test]
async fn empty_order_is_offline_without_probing() -> TestResult {
    let config = config("[provider]\norder = []\n")?;
    let env = Env(BTreeMap::from([("OPENROUTER_API_KEY", "synthetic-key")]));
    let choice = select_provider(
        &config,
        Effort::Low,
        ProviderCredentials::resolve(&config, &env),
    )
    .await;
    assert!(choice.selected.is_none());
    assert!(choice.skipped.is_empty());
    Ok(())
}

#[tokio::test]
async fn unreachable_local_server_is_skipped() -> TestResult {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let config = config(&format!(
        "[provider]\norder = [\"local\"]\n[local]\nbase_url = \"http://127.0.0.1:{port}/v1/\"\nmodel = \"tiny\"\n"
    ))?;
    let choice = select_provider(
        &config,
        Effort::Low,
        ProviderCredentials::resolve(&config, &Env(BTreeMap::new())),
    )
    .await;
    assert!(choice.selected.is_none());
    assert_eq!(
        choice.skipped,
        [SkippedProvider {
            provider: ProviderName::Local,
            reason: SkipReason::Unreachable,
        }]
    );
    Ok(())
}

/// Serve exactly one `/v1/models` response listing `models`.
async fn one_shot_models_server(
    models: &'static str,
) -> Result<(u16, tokio::task::JoinHandle<()>), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let task = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buffer = [0_u8; 4096];
            let _read = stream.read(&mut buffer).await;
            let body = format!("{{\"object\":\"list\",\"data\":[{models}]}}");
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _written = stream.write_all(response.as_bytes()).await;
        }
    });
    Ok((port, task))
}

#[tokio::test]
async fn local_server_is_selected_only_when_it_serves_the_model() -> TestResult {
    let (port, server) =
        one_shot_models_server(r#"{"id":"tiny","object":"model"},{"id":"other","object":"model"}"#)
            .await?;
    let settings = config(&format!(
        "[provider]\norder = [\"local\"]\n[local]\nbase_url = \"http://127.0.0.1:{port}/v1/\"\nmodel = \"tiny\"\n"
    ))?;
    let choice = select_provider(
        &settings,
        Effort::Low,
        ProviderCredentials::resolve(&settings, &Env(BTreeMap::new())),
    )
    .await;
    server.await?;
    let selected = choice.selected.ok_or("expected the local provider")?;
    assert_eq!(selected.provider, ProviderName::Local);
    assert_eq!(selected.model.max_context_tokens().get(), 8192);

    let (port, server) = one_shot_models_server(r#"{"id":"other","object":"model"}"#).await?;
    let settings = config(&format!(
        "[provider]\norder = [\"local\"]\n[local]\nbase_url = \"http://127.0.0.1:{port}/v1/\"\nmodel = \"tiny\"\n"
    ))?;
    let choice = select_provider(
        &settings,
        Effort::Low,
        ProviderCredentials::resolve(&settings, &Env(BTreeMap::new())),
    )
    .await;
    server.await?;
    assert!(choice.selected.is_none());
    assert_eq!(
        choice.skipped,
        [SkippedProvider {
            provider: ProviderName::Local,
            reason: SkipReason::ModelNotServed,
        }]
    );
    Ok(())
}

fn request(model: &str, content: &str) -> Result<ChatRequest, Box<dyn std::error::Error>> {
    Ok(ChatRequest {
        model: ModelId::new(model)?,
        messages: vec![ChatMessage::text(ChatRole::User, content.to_owned())],
        tools: Vec::new(),
        max_output_tokens: None,
        effort: Effort::Medium,
    })
}

#[tokio::test]
async fn hosted_count_is_a_conservative_upper_bound() -> TestResult {
    let config = config("")?;
    let env = Env(BTreeMap::from([("OPENROUTER_API_KEY", "synthetic-key")]));
    let choice = select_provider(
        &config,
        Effort::Medium,
        ProviderCredentials::resolve(&config, &env),
    )
    .await;
    let model = choice.selected.ok_or("expected a provider")?.model;
    let content = "fn main() { println!(\"héllo\"); }".repeat(50);
    let count = model
        .count_prompt(&request(model.model().as_str(), &content)?)
        .await?;
    let bytes = u32::try_from(content.len())?;
    assert!(count.input_tokens.get() >= bytes, "bound covers every byte");
    assert_eq!(count.max_context_tokens.get(), 262_144);

    let mismatch = model.stream(request("some/other-model", "hi")?).await;
    assert!(matches!(mismatch, Err(ModelError::BadRequest(_))));
    Ok(())
}
