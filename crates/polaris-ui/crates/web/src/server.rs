//! Loopback HTTP server: the embedded app, a JSON command API and a Server-Sent Events stream.
//!
//! Every `/api` call needs the per-run token (header `Authorization: Bearer …`, or `?token=` for
//! the event stream, which browsers open without custom headers) and a `Host` header naming the
//! bound loopback address, which defeats DNS rebinding. The page itself holds no secret: it reads
//! the token from the URL fragment, which browsers never send to servers or logs.
use crate::assets;
use crate::backend::{BackendConfig, Command, Publisher, SessionRow, WebEvent};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures::Stream;
use polaris_sdk::effort::Effort;
use serde::Deserialize;
use std::{convert::Infallible, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

/// Undelivered events kept per open page before it must resynchronise.
const EVENT_BACKLOG: usize = 1024;
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// Web UI failures.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    /// The address is not a loopback address.
    #[error("the web UI binds only to loopback addresses, not {0}")]
    NotLoopback(SocketAddr),
    /// Binding or serving failed.
    #[error("web server: {0}")]
    Io(#[from] std::io::Error),
    /// The random token could not be generated.
    #[error("no randomness for the access token")]
    Random,
}

/// Where to listen and which sessions to serve.
#[derive(Debug, Clone)]
pub struct WebOptions {
    /// Loopback address (port 0 picks a free one).
    pub bind: SocketAddr,
    /// Session store root.
    pub state_dir: PathBuf,
    /// Workspace of new sessions.
    pub workspace: PathBuf,
    /// Layered configuration.
    pub config: polaris_config::PolarisConfig,
    /// Dotenv file with provider keys.
    pub env_file: Option<PathBuf>,
}

#[derive(Clone)]
struct AppState {
    token: Arc<str>,
    hosts: Arc<[String]>,
    commands: mpsc::Sender<Command>,
    publisher: Publisher,
    state_dir: Arc<PathBuf>,
    /// Turns true when the server stops; open event streams end so shutdown completes.
    closing: watch::Receiver<bool>,
}

/// A bound, not yet serving web UI.
pub struct WebServer {
    listener: tokio::net::TcpListener,
    router: Router,
    url: String,
    backend: tokio::task::JoinHandle<()>,
    closing: watch::Sender<bool>,
}

impl std::fmt::Debug for WebServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebServer").finish_non_exhaustive()
    }
}

fn random_token() -> Result<String, WebError> {
    let mut bytes = [0_u8; 24];
    getrandom::fill(&mut bytes).map_err(|_| WebError::Random)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl WebServer {
    /// Bind `options.bind` and start the session owner.
    /// # Errors
    /// A non-loopback address, a busy port or no randomness for the token.
    pub async fn bind(options: WebOptions) -> Result<Self, WebError> {
        if !options.bind.ip().is_loopback() {
            return Err(WebError::NotLoopback(options.bind));
        }
        let listener = tokio::net::TcpListener::bind(options.bind).await?;
        let address = listener.local_addr()?;
        let token = random_token()?;
        let publisher = Publisher::new(EVENT_BACKLOG);
        let (commands, receiver) = mpsc::channel(64);
        let backend = tokio::spawn(crate::backend::run(
            BackendConfig {
                state_dir: options.state_dir.clone(),
                workspace: options.workspace,
                config: options.config,
                env_file: options.env_file,
            },
            publisher.clone(),
            receiver,
        ));
        let port = address.port();
        let hosts: Arc<[String]> =
            Arc::from(vec![address.to_string(), format!("localhost:{port}")]);
        let (closing, closing_rx) = watch::channel(false);
        let router = router(AppState {
            token: Arc::from(token.as_str()),
            hosts,
            commands,
            publisher,
            state_dir: Arc::new(options.state_dir),
            closing: closing_rx,
        });
        Ok(Self {
            listener,
            router,
            url: format!("http://{address}/#token={token}"),
            backend,
            closing,
        })
    }

    /// The address to open, including the access token in the fragment.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Serve until `shutdown` resolves; then stop the session owner.
    /// # Errors
    /// The server failed.
    pub async fn serve(
        self,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<(), WebError> {
        let Self {
            listener,
            router,
            backend,
            closing,
            ..
        } = self;
        let served = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                shutdown.await;
                // End the open event streams; graceful shutdown waits for every response.
                let _closed = closing.send(true);
            })
            .await;
        // Dropping the router dropped the command sender; the owner closes its session.
        if tokio::time::timeout(Duration::from_secs(15), backend)
            .await
            .is_err()
        {
            tracing::warn!("web session owner did not stop in time");
        }
        served.map_err(WebError::Io)
    }
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.css", get(stylesheet))
        .route("/app.js", get(script))
        .route("/fonts/{name}", get(font))
        .route("/api/events", get(events))
        .route("/api/sessions", get(sessions))
        .route("/api/stats", get(stats))
        .route("/api/open", post(open))
        .route("/api/submit", post(submit))
        .route("/api/interrupt", post(interrupt))
        .route("/api/effort", post(effort))
        .route("/api/close", post(close))
        .route("/api/review", post(review))
        .with_state(state)
}

fn host_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| state.hosts.iter().any(|allowed| allowed == host))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// `Ok` when the request carries the run's token and a bound Host.
fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> Result<(), StatusCode> {
    if !host_allowed(state, headers) {
        return Err(StatusCode::MISDIRECTED_REQUEST);
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match bearer.or(query_token) {
        Some(token) if constant_time_eq(token.as_bytes(), state.token.as_bytes()) => Ok(()),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

fn asset(content_type: &'static str, body: &'static [u8]) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !host_allowed(&state, &headers) {
        return StatusCode::MISDIRECTED_REQUEST.into_response();
    }
    asset("text/html; charset=utf-8", assets::INDEX.as_bytes())
}

async fn stylesheet(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !host_allowed(&state, &headers) {
        return StatusCode::MISDIRECTED_REQUEST.into_response();
    }
    asset("text/css; charset=utf-8", assets::STYLE.as_bytes())
}

async fn script(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !host_allowed(&state, &headers) {
        return StatusCode::MISDIRECTED_REQUEST.into_response();
    }
    asset("text/javascript; charset=utf-8", assets::SCRIPT.as_bytes())
}

async fn font(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !host_allowed(&state, &headers) {
        return StatusCode::MISDIRECTED_REQUEST.into_response();
    }
    match assets::font(&name) {
        Some(bytes) => asset("font/ttf", bytes),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    authorize(&state, &headers, query.token.as_deref())?;
    let (replay, receiver) = state.publisher.subscribe();
    let closing = state.closing.clone();
    let stream = futures::stream::unfold(
        (replay.into_iter(), receiver, closing),
        |(mut replay, mut receiver, mut closing)| async move {
            if *closing.borrow() {
                return None;
            }
            if let Some(event) = replay.next() {
                return Some((Ok(to_sse(&event)), (replay, receiver, closing)));
            }
            let event = tokio::select! {
                received = receiver.recv() => match received {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => WebEvent::Notice {
                        text: "Some updates were skipped; reload the page to resynchronise."
                            .to_owned(),
                    },
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
                _changed = closing.changed() => return None,
            };
            Some((Ok(to_sse(&event)), (replay, receiver, closing)))
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn to_sse(event: &WebEvent) -> Event {
    match Event::default().json_data(event) {
        Ok(event) => event,
        Err(_) => Event::default().data("{\"type\":\"notice\",\"text\":\"unencodable event\"}"),
    }
}

async fn send(state: &AppState, command: Command) -> StatusCode {
    match state.commands.send(command).await {
        Ok(()) => StatusCode::ACCEPTED,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<SessionRow>>, StatusCode> {
    authorize(&state, &headers, None)?;
    let (reply, rows) = oneshot::channel();
    if state.commands.send(Command::List { reply }).await.is_err() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    rows.await
        .map(Json)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

async fn stats(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, StatusCode> {
    authorize(&state, &headers, None)?;
    let rows = polaris_ui_session::list_sessions(&state.state_dir, 10_000)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let now = jiff::Zoned::now();
    Ok(Json(polaris_ui_session::stats::stats_lines(
        &rows,
        now.date(),
        now.time_zone(),
    )))
}

#[derive(Deserialize)]
struct OpenBody {
    resume: Option<String>,
}

async fn open(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OpenBody>,
) -> StatusCode {
    if let Err(status) = authorize(&state, &headers, None) {
        return status;
    }
    send(
        &state,
        Command::Open {
            resume: body.resume,
        },
    )
    .await
}

/// Largest message accepted from the page.
const MAX_SUBMIT_BYTES: usize = 65_536;

#[derive(Deserialize)]
struct SubmitBody {
    text: String,
    #[serde(default)]
    plan: bool,
}

async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SubmitBody>,
) -> StatusCode {
    if let Err(status) = authorize(&state, &headers, None) {
        return status;
    }
    if body.text.trim().is_empty() || body.text.len() > MAX_SUBMIT_BYTES {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    send(
        &state,
        Command::Submit {
            text: body.text,
            plan: body.plan,
        },
    )
    .await
}

/// Longest note accepted with a decision.
const MAX_NOTE_BYTES: usize = 4096;

#[derive(Deserialize)]
struct ReviewBody {
    request_id: String,
    apply: bool,
    #[serde(default)]
    note: Option<String>,
}

async fn review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ReviewBody>,
) -> StatusCode {
    if let Err(status) = authorize(&state, &headers, None) {
        return status;
    }
    if body.request_id.len() > 64
        || body
            .note
            .as_ref()
            .is_some_and(|note| note.len() > MAX_NOTE_BYTES)
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    send(
        &state,
        Command::Review {
            request_id: body.request_id,
            apply: body.apply,
            note: body.note,
        },
    )
    .await
}

async fn interrupt(State(state): State<AppState>, headers: HeaderMap) -> StatusCode {
    if let Err(status) = authorize(&state, &headers, None) {
        return status;
    }
    send(&state, Command::Interrupt).await
}

async fn close(State(state): State<AppState>, headers: HeaderMap) -> StatusCode {
    if let Err(status) = authorize(&state, &headers, None) {
        return status;
    }
    send(&state, Command::Close).await
}

#[derive(Deserialize)]
struct EffortBody {
    effort: String,
}

async fn effort(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EffortBody>,
) -> StatusCode {
    if let Err(status) = authorize(&state, &headers, None) {
        return status;
    }
    match Effort::ALL
        .iter()
        .copied()
        .find(|effort| effort.name().eq_ignore_ascii_case(&body.effort))
    {
        Some(effort) => send(&state, Command::SetEffort(effort)).await,
        None => StatusCode::UNPROCESSABLE_ENTITY,
    }
}
