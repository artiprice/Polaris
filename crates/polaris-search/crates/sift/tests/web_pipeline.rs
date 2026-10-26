#![forbid(unsafe_code)]

use polaris_ratelimit::{Quota, ShedPolicy};
use polaris_sdk::{
    backends::OneOf2,
    registry::{DecisionEngine, DecisionError, RedactStats, Redactor},
    system_one::{
        Answer, Decision, DecisionInput, Probability, Question, QuestionAnswer, QuestionId,
        QuestionKind,
    },
};
use polaris_search_providers::{ProviderRuntime, Searxng, SearxngConfig, Tavily, TavilyConfig};
use polaris_search_sift::{
    Locator, RaceMode, RaceOutcome, RacePolicy, RaceSource, SemaphoreSet, SiftConfig, SiftQuery,
    SiftQuestions, Sifter, SourceId, SourcePool, SourceStatus, TokenBudget, Trust, WebSearchSource,
    recover_dropped,
};
use polaris_storage_cas::{Cas, CasConfig};
use std::{error::Error, io, net::SocketAddr, num::NonZeroU32, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

const QUERY_CANARY: &str = "private-canary-token";
const MASKED_QUERY: &str = "masked-canary-token";
const EXPECTED_RESULT_URL: &str = "https://search.example.test/article/42";
const FIXTURE_RESULT: &str = r#"{"results":[{"url":"https://search.example.test/article/42","title":"Synthetic retrieval result","content":"Deployment behavior from the fixture."}]}"#;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type WebProvider = OneOf2<Tavily, Searxng>;
type WebSource = WebSearchSource<WebProvider>;

/// SDK Redactor fixture that masks one synthetic canary before either provider sends a request.
#[derive(Debug)]
struct CanaryRedactor;

impl Redactor for CanaryRedactor {
    fn redact_into(&self, input: &[u8], output: &mut Vec<u8>) -> RedactStats {
        let text = String::from_utf8_lossy(input);
        let redacted = text.replace(QUERY_CANARY, MASKED_QUERY);
        let replacements = text
            .matches(QUERY_CANARY)
            .fold(0_u32, |count, _match| count.saturating_add(1));
        output.extend_from_slice(redacted.as_bytes());
        RedactStats {
            bytes_in: input.len(),
            replacements,
        }
    }
}

/// Clearly labelled S1 fixture: it returns a valid low relevance score to force failed-open
/// retrieval fallback when one provider also fails.
struct S1FixtureEngine;

impl DecisionEngine for S1FixtureEngine {
    async fn decide(
        &self,
        schema: &[Question],
        _input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        let low_relevance =
            Probability::new(0.2).map_err(|_| DecisionError::InvalidSchema("fixture".into()))?;
        let density =
            Probability::new(0.8).map_err(|_| DecisionError::InvalidSchema("fixture".into()))?;
        let answers = schema
            .iter()
            .map(|question| QuestionAnswer {
                question: question.id.clone(),
                answer: match question.kind {
                    QuestionKind::Noul { .. } => Answer::Noul { yes: low_relevance },
                    QuestionKind::Score { .. } => Answer::Score {
                        value: 4,
                        confidence: density,
                    },
                    QuestionKind::Choice { .. } => Answer::Noul { yes: low_relevance },
                },
            })
            .collect();
        Ok(Decision {
            answers,
            latency_us: 0,
        })
    }
}

/// Owns the synthetic HTTP server; dropping an unfinished fixture aborts its task.
struct ProviderFixture {
    address: SocketAddr,
    task: Option<JoinHandle<io::Result<Vec<String>>>>,
}

impl ProviderFixture {
    async fn start() -> TestResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(2);
            for _request_index in 0..2 {
                let accepted = tokio::time::timeout(Duration::from_secs(8), listener.accept())
                    .await
                    .map_err(|_| io::Error::other("fixture accept timed out"))??;
                let (mut stream, _) = accepted;
                let request =
                    tokio::time::timeout(Duration::from_secs(8), read_request(&mut stream))
                        .await
                        .map_err(|_| io::Error::other("fixture request read timed out"))??;
                let is_tavily = request.starts_with("POST ");
                let (status, body) = if is_tavily {
                    (
                        "503 Service Unavailable",
                        r#"{"error":"synthetic failure"}"#,
                    )
                } else {
                    ("200 OK", FIXTURE_RESULT)
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await?;
                requests.push(request);
            }
            Ok(requests)
        });
        Ok(Self {
            address,
            task: Some(task),
        })
    }

    async fn finish(mut self) -> TestResult<Vec<String>> {
        let mut task = self
            .task
            .take()
            .ok_or_else(|| io::Error::other("fixture task was already joined"))?;
        let joined = match tokio::time::timeout(Duration::from_secs(10), &mut task).await {
            Ok(result) => result,
            Err(_) => {
                task.abort();
                let _join_result = task.await;
                return Err(io::Error::other("fixture server did not finish").into());
            }
        };
        Ok(joined??)
    }
}

impl Drop for ProviderFixture {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn read_request(stream: &mut TcpStream) -> io::Result<String> {
    let mut header = Vec::new();
    loop {
        let byte = stream.read_u8().await?;
        header.push(byte);
        if header.len() > 16_384 {
            return Err(io::Error::other("fixture request header exceeded bound"));
        }
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let header_text = std::str::from_utf8(&header).map_err(io::Error::other)?;
    let content_length = match header_text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse::<usize>().ok()
        } else {
            None
        }
    }) {
        Some(length) if length <= 16_384 => length,
        Some(_) => return Err(io::Error::other("fixture request body exceeded bound")),
        None => 0,
    };
    let mut body = vec![0_u8; content_length];
    stream.read_exact(&mut body).await?;
    let mut request = header;
    request.extend_from_slice(&body);
    String::from_utf8(request).map_err(io::Error::other)
}

fn runtime() -> TestResult<ProviderRuntime> {
    let quota = Quota {
        rpm: NonZeroU32::new(60).ok_or("invalid fixture request quota")?,
        tpm: NonZeroU32::new(60).ok_or("invalid fixture token quota")?,
        burst_requests: 4,
        burst_tokens: 4,
    };
    Ok(ProviderRuntime::new(quota, 1, ShedPolicy::default())?)
}

fn sifter() -> TestResult<Sifter<S1FixtureEngine>> {
    let config = SiftConfig {
        questions: SiftQuestions {
            relevance: Question {
                id: QuestionId::new("fixture-relevance")?,
                kind: QuestionKind::Noul {
                    statement: "Does this result support the retrieval goal?".into(),
                },
            },
            density: Question {
                id: QuestionId::new("fixture-density")?,
                kind: QuestionKind::Score { min: 0, max: 10 },
            },
        },
        accept_relevance: Probability::new(0.6)?,
        max_candidates: 8,
        batch_size: 2,
        max_chunk_chars: 4_096,
        decision_deadline: Duration::from_secs(2),
    };
    Ok(Sifter::new(S1FixtureEngine, config)?)
}

#[tokio::test]
async fn real_web_providers_redact_race_preserve_provenance_and_recover_dropped_chunks()
-> TestResult {
    let fixture = ProviderFixture::start().await?;
    let fixture_address = fixture.address;
    let redactor: Arc<dyn Redactor> = Arc::new(CanaryRedactor);
    let tavily_config = TavilyConfig {
        base_url: format!("http://{fixture_address}/").parse()?,
        api_key: "synthetic-fixture-key".into(),
        timeout: Duration::from_secs(3),
        max_response_bytes: 16_384,
        allow_http_loopback: true,
    };
    let searx_base_url = tavily_config.base_url.clone();
    let tavily = Tavily::new(tavily_config, runtime()?, Arc::clone(&redactor))?;
    let searxng = Searxng::new(
        SearxngConfig {
            base_url: searx_base_url,
            engines: vec!["general".to_owned()],
            timeout: Duration::from_secs(3),
            max_response_bytes: 16_384,
            allow_http_loopback: true,
        },
        runtime()?,
        Arc::clone(&redactor),
    )?;
    let tavily_source = Arc::new(WebSource::new(
        SourceId::new("tavily-fixture")?,
        OneOf2::A(tavily),
    ));
    let searx_source = Arc::new(WebSource::new(
        SourceId::new("searx-fixture")?,
        OneOf2::B(searxng),
    ));
    let sources = vec![
        RaceSource {
            source: tavily_source,
            pool: SourcePool::Web,
        },
        RaceSource {
            source: searx_source,
            pool: SourcePool::Web,
        },
    ];
    let query = SiftQuery {
        text: format!("synthetic retrieval for {QUERY_CANARY}"),
        goal: "deployment behavior from a synthetic web search".to_owned(),
        max_chunks: 4,
    };
    let policy = RacePolicy {
        mode: RaceMode::BestWithinDeadline,
        deadline: Duration::from_secs(5),
        accept: Probability::new(0.6)?,
        max_validations: 8,
    };
    let semaphores = SemaphoreSet::new(1, 1, 2)?;
    let sifter = sifter()?;
    let race = sifter
        .race(
            &query,
            &sources,
            &semaphores,
            &policy,
            &CancellationToken::new(),
        )
        .await?;
    let fallback = match race {
        RaceOutcome::FailedOpen { fallback, sources } => {
            let failed_count = sources
                .iter()
                .filter(|source| source.status == SourceStatus::Failed)
                .count();
            let retrieved_count = sources
                .iter()
                .filter(|source| source.status == SourceStatus::Retrieved)
                .count();
            if failed_count != 1 || retrieved_count != 1 {
                return Err("race did not record one provider failure and one retrieval".into());
            }
            fallback
        }
        _ => return Err("low-scored result with a failed provider did not fail open".into()),
    };
    if fallback.len() != 1 {
        return Err("failed-open race did not preserve exactly one synthetic result".into());
    }
    let fallback_chunk = fallback
        .first()
        .ok_or_else(|| io::Error::other("failed-open fallback was empty"))?;
    if fallback_chunk.source.as_str() != "searx-fixture"
        || fallback_chunk.locator != Locator::Url(EXPECTED_RESULT_URL.to_owned())
        || fallback_chunk.trust != Trust::Untrusted
    {
        return Err("web source changed URL provenance, source id, or untrusted trust".into());
    }

    let requests = fixture.finish().await?;
    if requests.len() != 2 {
        return Err("fixture did not observe both provider requests".into());
    }
    for request in &requests {
        if request.contains(QUERY_CANARY) || !request.contains(MASKED_QUERY) {
            return Err("raw canary reached a provider or redacted query was absent".into());
        }
    }
    let tavily_request = requests.iter().any(|request| {
        request.starts_with("POST /search ")
            && request
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-fixture-key")
            && request.contains(MASKED_QUERY)
    });
    let searx_request = requests
        .iter()
        .any(|request| request.starts_with("GET /search?") && request.contains(MASKED_QUERY));
    if !tavily_request || !searx_request {
        return Err("fixture did not observe both concrete provider request formats".into());
    }
    let distilled = sifter
        .sift(&query.goal, fallback, TokenBudget::new(0))
        .await?;
    if distilled.dropped.len() != 1 {
        return Err("S1 fixture did not produce one recoverable dropped chunk".into());
    }
    let directory = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(directory.path().join("cas")))?;
    let persisted = distilled.persist_dropped(&cas)?;
    let recovered = recover_dropped(&cas, &persisted.manifest_ref)?;
    if recovered != distilled.dropped
        || recovered
            .first()
            .is_none_or(|chunk| chunk.trust != Trust::Untrusted)
    {
        return Err("CAS recovery changed dropped chunk text or untrusted provenance".into());
    }
    Ok(())
}
