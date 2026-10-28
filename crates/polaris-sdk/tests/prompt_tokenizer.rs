//! Tokenizer-dispatch contracts using an explicitly synthetic oracle, not real tokenization.
use polaris_sdk::{
    backends::{OneOf2, OneOf3},
    effort::Effort,
    ids::{ModelId, TokenCount},
    registry::{ChatMessage, ChatRequest, ChatRole, ModelError, PromptTokenCount, PromptTokenizer},
};
use std::{error::Error, sync::Arc};
type TestResult = Result<(), Box<dyn Error>>;
#[derive(Clone, Copy)]
struct SyntheticTokenizer {
    fail: bool,
    input: u32,
    capacity: u32,
}
impl PromptTokenizer for SyntheticTokenizer {
    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        if self.fail || request.model.as_str() != "fixture/model" {
            return Err(ModelError::Timeout);
        }
        Ok(PromptTokenCount {
            input_tokens: TokenCount::new(self.input),
            max_context_tokens: TokenCount::new(self.capacity),
        })
    }
}
fn request() -> Result<ChatRequest, Box<dyn Error>> {
    Ok(ChatRequest {
        model: ModelId::new("fixture/model")?,
        messages: vec![ChatMessage::text(ChatRole::User, "fixture".into())],
        tools: Vec::new(),
        max_output_tokens: None,
        effort: Effort::Low,
    })
}
#[tokio::test]
async fn prompt_tokenizer_reports_exact_prompt_and_model_capacity() -> TestResult {
    let oracle = SyntheticTokenizer {
        fail: false,
        input: 7,
        capacity: 4096,
    };
    let a: OneOf2<SyntheticTokenizer, SyntheticTokenizer> = OneOf2::A(oracle);
    let b: OneOf2<SyntheticTokenizer, SyntheticTokenizer> = OneOf2::B(oracle);
    let c: OneOf3<SyntheticTokenizer, SyntheticTokenizer, SyntheticTokenizer> = OneOf3::C(oracle);
    let d: OneOf3<SyntheticTokenizer, SyntheticTokenizer, SyntheticTokenizer> = OneOf3::A(oracle);
    let e: OneOf3<SyntheticTokenizer, SyntheticTokenizer, SyntheticTokenizer> = OneOf3::B(oracle);
    let expected = oracle.count_prompt(&request()?).await?;
    assert_eq!(
        a.count_prompt(&request()?).await?,
        expected,
        "OneOf2 A forwards actual result"
    );
    assert_eq!(
        b.count_prompt(&request()?).await?,
        expected,
        "OneOf2 B forwards actual result"
    );
    assert_eq!(
        c.count_prompt(&request()?).await?,
        expected,
        "OneOf3 C forwards actual result"
    );
    assert_eq!(
        d.count_prompt(&request()?).await?,
        expected,
        "OneOf3 A forwards actual result"
    );
    assert_eq!(
        e.count_prompt(&request()?).await?,
        expected,
        "OneOf3 B forwards actual result"
    );
    assert_eq!(
        Arc::new(oracle).count_prompt(&request()?).await?,
        expected,
        "Arc forwards actual result"
    );
    Ok(())
}
#[tokio::test]
async fn prompt_tokenizer_errors_when_counting_is_unavailable() -> TestResult {
    let oracle = SyntheticTokenizer {
        fail: true,
        input: 7,
        capacity: 4096,
    };
    let a: OneOf2<SyntheticTokenizer, SyntheticTokenizer> = OneOf2::B(oracle);
    let b: OneOf3<SyntheticTokenizer, SyntheticTokenizer, SyntheticTokenizer> = OneOf3::C(oracle);
    assert_eq!(
        a.count_prompt(&request()?).await,
        Err(ModelError::Timeout),
        "OneOf2 never synthesizes a fallback"
    );
    assert_eq!(
        b.count_prompt(&request()?).await,
        Err(ModelError::Timeout),
        "OneOf3 never synthesizes a fallback"
    );
    assert_eq!(
        Arc::new(oracle).count_prompt(&request()?).await,
        Err(ModelError::Timeout),
        "Arc preserves failure"
    );
    Ok(())
}
#[tokio::test]
async fn prompt_token_count_can_exceed_context_for_trimming() -> TestResult {
    let oracle = SyntheticTokenizer {
        fail: false,
        input: 6000,
        capacity: 4096,
    };
    let count = Arc::new(oracle).count_prompt(&request()?).await?;
    assert_eq!(
        count.input_tokens,
        TokenCount::new(6000),
        "preserve oversized count"
    );
    assert_eq!(
        count.max_context_tokens,
        TokenCount::new(4096),
        "preserve independent capacity"
    );
    Ok(())
}
