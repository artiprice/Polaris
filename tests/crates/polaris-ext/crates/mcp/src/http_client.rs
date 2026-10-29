// Unit tests of crates/polaris-ext/crates/mcp/src/http_client.rs (`tests`); compiled into that module via #[path].
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
