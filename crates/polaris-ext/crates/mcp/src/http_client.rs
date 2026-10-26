use std::{collections::HashMap, io, sync::Arc};

use bytes::{Bytes, BytesMut};
use futures::{StreamExt, stream::BoxStream};
use http::{HeaderName, HeaderValue, StatusCode};
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::streamable_http_client::{
        StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
    },
};
use sse_stream::{Error as SseError, Sse, SseStream};

use crate::frame::MAX_JSONRPC_FRAME_BYTES;

/// Reqwest-backed MCP HTTP client that bounds JSON and SSE bodies before parsing.
#[derive(Clone)]
pub(crate) struct BoundedHttpClient {
    client: reqwest::Client,
}

impl BoundedHttpClient {
    pub(crate) fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn request(
        &self,
        method: reqwest::Method,
        uri: &str,
        session_id: Option<&str>,
        last_event_id: Option<&str>,
        auth_header: Option<&str>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<reqwest::RequestBuilder, HttpClientError> {
        let mut request = self
            .client
            .request(method, uri)
            .header("accept", "text/event-stream, application/json");
        if let Some(session_id) = session_id {
            request = request.header("mcp-session-id", session_id);
        }
        if let Some(last_event_id) = last_event_id {
            request = request.header("last-event-id", last_event_id);
        }
        if let Some(auth_header) = auth_header {
            request = request.bearer_auth(auth_header);
        }
        for (name, value) in custom_headers {
            request = request.header(name, value);
        }
        Ok(request)
    }
}

impl StreamableHttpClient for BoundedHttpClient {
    type Error = HttpClientError;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.post_message_with_max_sse_event_size(
            uri,
            message,
            session_id,
            auth_header,
            custom_headers,
            MAX_JSONRPC_FRAME_BYTES,
        )
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let session_was_attached = session_id.is_some();
        let request = self
            .request(
                reqwest::Method::POST,
                &uri,
                session_id.as_deref(),
                None,
                auth_header.as_deref(),
                custom_headers,
            )
            .map_err(StreamableHttpError::Client)?;
        let body = serde_json::to_vec(&message).map_err(StreamableHttpError::Deserialize)?;
        let response = request
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| StreamableHttpError::Client(HttpClientError::Transport))?;
        let status = response.status();
        if matches!(status, StatusCode::ACCEPTED | StatusCode::NO_CONTENT) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if status == StatusCode::NOT_FOUND && session_was_attached {
            return Err(StreamableHttpError::SessionExpired);
        }
        if status.is_success()
            && response.content_length() == Some(0)
            && matches!(
                message,
                ClientJsonRpcMessage::Notification(_)
                    | ClientJsonRpcMessage::Response(_)
                    | ClientJsonRpcMessage::Error(_)
            )
        {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if !status.is_success() {
            return Err(StreamableHttpError::UnexpectedServerResponse(
                "MCP HTTP request failed".into(),
            ));
        }
        match content_type.as_deref() {
            Some(value) if value.starts_with("text/event-stream") => {
                let events = bounded_sse(response.bytes_stream(), max_sse_event_size);
                Ok(StreamableHttpPostResponse::Sse(events, session_id))
            }
            Some(value) if value.starts_with("application/json") => {
                let body = bounded_body(response, MAX_JSONRPC_FRAME_BYTES).await?;
                match serde_json::from_slice::<ServerJsonRpcMessage>(&body) {
                    Ok(message) => Ok(StreamableHttpPostResponse::Json(message, session_id)),
                    Err(_) => Err(StreamableHttpError::UnexpectedServerResponse(
                        "MCP JSON response was invalid or oversized".into(),
                    )),
                }
            }
            _ => Err(StreamableHttpError::UnexpectedContentType(content_type)),
        }
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let request = self
            .request(
                reqwest::Method::DELETE,
                &uri,
                Some(&session_id),
                None,
                auth_header.as_deref(),
                custom_headers,
            )
            .map_err(StreamableHttpError::Client)?;
        let response = request
            .send()
            .await
            .map_err(|_| StreamableHttpError::Client(HttpClientError::Transport))?;
        if response.status() == StatusCode::METHOD_NOT_ALLOWED || response.status().is_success() {
            Ok(())
        } else {
            Err(StreamableHttpError::UnexpectedServerResponse(
                "MCP session deletion failed".into(),
            ))
        }
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.get_stream_with_max_sse_event_size(
            uri,
            session_id,
            last_event_id,
            auth_header,
            custom_headers,
            MAX_JSONRPC_FRAME_BYTES,
        )
        .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let request = self
            .request(
                reqwest::Method::GET,
                &uri,
                session_id.as_deref(),
                last_event_id.as_deref(),
                auth_header.as_deref(),
                custom_headers,
            )
            .map_err(StreamableHttpError::Client)?;
        let response = request
            .send()
            .await
            .map_err(|_| StreamableHttpError::Client(HttpClientError::Transport))?;
        if response.status() == StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        if !response.status().is_success() {
            return Err(StreamableHttpError::UnexpectedServerResponse(
                "MCP event stream request failed".into(),
            ));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if !content_type
            .as_deref()
            .is_some_and(|value| value.starts_with("text/event-stream"))
        {
            return Err(StreamableHttpError::UnexpectedContentType(content_type));
        }
        Ok(bounded_sse(response.bytes_stream(), max_sse_event_size))
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HttpClientError {
    #[error("HTTP transport failed")]
    Transport,
}

async fn bounded_body(
    response: reqwest::Response,
    maximum: usize,
) -> Result<Bytes, StreamableHttpError<HttpClientError>> {
    if let Some(content_length) = response.content_length() {
        match usize::try_from(content_length) {
            Ok(length) if length <= maximum => {}
            Ok(_) | Err(_) => {
                return Err(StreamableHttpError::UnexpectedServerResponse(
                    "MCP response exceeds configured bound".into(),
                ));
            }
        }
    }
    let mut stream = response.bytes_stream();
    let mut body = BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| StreamableHttpError::Client(HttpClientError::Transport))?;
        let new_size = body.len().checked_add(chunk.len());
        if match new_size {
            Some(size) => size > maximum,
            None => true,
        } {
            return Err(StreamableHttpError::UnexpectedServerResponse(
                "MCP response exceeds configured bound".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

fn bounded_sse<S>(stream: S, maximum: usize) -> BoxStream<'static, Result<Sse, SseError>>
where
    S: futures::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    let limited = stream.scan((0_usize, false, false), move |state, chunk| {
        let result = match chunk {
            Err(_) => Err(SseError::Body(Box::new(io::Error::other(
                "MCP event stream failed",
            )))),
            Ok(bytes) => {
                let mut exceeded = false;
                for byte in bytes.iter().copied() {
                    state.0 = match state.0.checked_add(1) {
                        Some(size) => size,
                        None => {
                            exceeded = true;
                            break;
                        }
                    };
                    if state.0 > maximum {
                        exceeded = true;
                        break;
                    }
                    if state.2 && byte == b'\n' {
                        state.2 = false;
                        continue;
                    }
                    state.2 = false;
                    if byte == b'\r' || byte == b'\n' {
                        if !state.1 {
                            state.0 = 0;
                        } else {
                            state.1 = false;
                        }
                        state.2 = byte == b'\r';
                    } else {
                        state.1 = true;
                    }
                }
                if exceeded {
                    Err(SseError::Body(Box::new(io::Error::other(
                        "MCP event exceeds configured bound",
                    ))))
                } else {
                    Ok(bytes)
                }
            }
        };
        async move { Some(result) }
    });
    SseStream::from_bytes_stream(limited).boxed()
}

pub(crate) fn is_reserved_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "accept" | "content-type" | "last-event-id" | "mcp-protocol-version" | "mcp-session-id"
    )
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use futures::StreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::bounded_body;

    fn run<F: Future<Output = ()>>(future: F) -> Result<(), std::io::Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(future);
        Ok(())
    }

    #[test]
    fn oversized_http_json_body_is_rejected_before_parsing() -> Result<(), std::io::Error> {
        run(async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await;
            assert!(listener.is_ok());
            let listener = match listener {
                Ok(listener) => listener,
                Err(_) => return,
            };
            let address = listener.local_addr();
            assert!(address.is_ok());
            let address = match address {
                Ok(address) => address,
                Err(_) => return,
            };
            let server = tokio::spawn(async move {
                let accepted = listener.accept().await;
                if let Ok((mut stream, _)) = accepted {
                    let mut request_bytes = [0_u8; 1024];
                    let _read_result = stream.read(&mut request_bytes).await;
                    let body = vec![b' '; 1024];
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _header_result = stream.write_all(headers.as_bytes()).await;
                    let _body_result = stream.write_all(&body).await;
                }
            });
            let response = reqwest::Client::new()
                .get(format!("http://{address}/oversized"))
                .send()
                .await;
            assert!(response.is_ok());
            if let Ok(response) = response {
                let bounded = bounded_body(response, 128).await;
                assert!(matches!(
                bounded,
                Err(rmcp::transport::streamable_http_client::StreamableHttpError::UnexpectedServerResponse(_))
            ));
            }
            let server_result = server.await;
            assert!(server_result.is_ok());
        })
    }

    #[test]
    fn oversized_chunked_http_json_body_is_rejected_without_content_length()
    -> Result<(), std::io::Error> {
        run(async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await;
            assert!(listener.is_ok());
            let listener = match listener {
                Ok(listener) => listener,
                Err(_) => return,
            };
            let address = listener.local_addr();
            assert!(address.is_ok());
            let address = match address {
                Ok(address) => address,
                Err(_) => return,
            };
            let server = tokio::spawn(async move {
                let accepted = listener.accept().await;
                if let Ok((mut stream, _)) = accepted {
                    let mut request_bytes = [0_u8; 1024];
                    let _read_result = stream.read(&mut request_bytes).await;
                    let body = vec![b' '; 1024];
                    let headers = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n400\r\n";
                    let _header_result = stream.write_all(headers).await;
                    let _body_result = stream.write_all(&body).await;
                    let _terminator_result = stream.write_all(b"\r\n0\r\n\r\n").await;
                }
            });
            let response = reqwest::Client::new()
                .get(format!("http://{address}/oversized-chunked"))
                .send()
                .await;
            assert!(response.is_ok());
            if let Ok(response) = response {
                let bounded = bounded_body(response, 128).await;
                assert!(matches!(
                bounded,
                Err(rmcp::transport::streamable_http_client::StreamableHttpError::UnexpectedServerResponse(_))
            ));
            }
            let server_result = server.await;
            assert!(server_result.is_ok());
        })
    }

    #[test]
    fn sse_parser_accepts_split_crlf_and_multiline_events() -> Result<(), std::io::Error> {
        run(async {
            let chunks = futures::stream::iter([
                Ok::<_, reqwest::Error>(bytes::Bytes::from_static(b"data: first\r")),
                Ok(bytes::Bytes::from_static(b"\ndata: second\r")),
                Ok(bytes::Bytes::from_static(b"\n\r")),
                Ok(bytes::Bytes::from_static(b"\n")),
            ]);
            let mut events = super::bounded_sse(chunks, 64);
            let event = events.next().await;
            assert!(matches!(&event, Some(Ok(_))));
            if let Some(Ok(event)) = event {
                assert_eq!(event.data.as_deref(), Some("first\nsecond"));
            }
        })
    }

    #[test]
    fn sse_event_limit_spans_fragmented_chunks() -> Result<(), std::io::Error> {
        run(async {
            let chunks = futures::stream::iter([
                Ok::<_, reqwest::Error>(bytes::Bytes::from_static(b"data: abc")),
                Ok(bytes::Bytes::from_static(b"def\n\n")),
            ]);
            let mut events = super::bounded_sse(chunks, 8);
            let event = events.next().await;
            assert!(matches!(event, Some(Err(_))));
        })
    }
}
