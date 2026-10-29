// Unit tests of crates/polaris-openshell/crates/client/src/stdio.rs (`tests`); compiled into that module via #[path].
use super::{pump, validate};
use bytes::Bytes;
use openshell_sdk::raw::proto;
use polaris_sdk::{
    interactive::{StdioChunk, StdioRequest},
    registry::SandboxError,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

fn output(bytes: &[u8]) -> Result<proto::ExecSandboxEvent, tonic::Status> {
    Ok(proto::ExecSandboxEvent {
        payload: Some(proto::exec_sandbox_event::Payload::Stdout(
            proto::ExecSandboxStdout {
                data: bytes.to_vec(),
            },
        )),
    })
}
fn exit() -> Result<proto::ExecSandboxEvent, tonic::Status> {
    Ok(proto::ExecSandboxEvent {
        payload: Some(proto::exec_sandbox_event::Payload::Exit(
            proto::ExecSandboxExit { exit_code: 7 },
        )),
    })
}
#[tokio::test]
async fn stdio_input_and_output_preserve_bounded_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let (sender, mut receiver) = mpsc::channel(8);
    let (_stop, stopped) = watch::channel(false);
    let result = pump(
        futures::stream::iter([output(b"a\0b\r\n"), exit()]),
        sender,
        stopped,
        Duration::from_secs(1),
        5,
    )
    .await?;
    assert_eq!(result, 7);
    assert_eq!(
        receiver.recv().await,
        Some(Ok(StdioChunk::Stdout(Bytes::from_static(b"a\0b\r\n"))))
    );
    assert_eq!(receiver.recv().await, Some(Ok(StdioChunk::Exit(7))));
    assert!(receiver.recv().await.is_none());
    Ok(())
}
#[tokio::test]
async fn stdio_exit_requires_clean_stream_end() {
    let (sender, _receiver) = mpsc::channel(8);
    let (_stop, stopped) = watch::channel(false);
    let result = pump(
        futures::stream::iter([
            exit(),
            Err(tonic::Status::internal("synthetic secret canary")),
        ]),
        sender,
        stopped,
        Duration::from_secs(1),
        1024,
    )
    .await;
    assert!(
        matches!(result, Err(SandboxError::Unavailable(message)) if !message.contains("canary"))
    );
}
#[tokio::test]
async fn stdio_rejects_missing_exit_duplicate_exit_and_post_exit_output() {
    for frames in [
        vec![output(b"a")],
        vec![exit(), exit()],
        vec![exit(), output(b"a")],
    ] {
        let (sender, _receiver) = mpsc::channel(8);
        let (_stop, stopped) = watch::channel(false);
        let result = pump(
            futures::stream::iter(frames),
            sender,
            stopped,
            Duration::from_secs(1),
            1024,
        )
        .await;
        assert!(matches!(result, Err(SandboxError::Protocol(_))));
    }
}
#[tokio::test]
async fn stdio_flood_applies_frame_bound_before_capture() {
    let (sender, _receiver) = mpsc::channel(8);
    let (_stop, stopped) = watch::channel(false);
    let result = pump(
        futures::stream::iter([output(&[0; 5]), exit()]),
        sender,
        stopped,
        Duration::from_secs(1),
        4,
    )
    .await;
    assert!(matches!(result, Err(SandboxError::LimitExceeded(_))));
}
#[tokio::test]
async fn stdio_backpressure_cannot_hide_lifetime_deadline() {
    let (sender, _receiver) = mpsc::channel(1);
    let (_stop, stopped) = watch::channel(false);
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        pump(
            futures::stream::iter([output(b"a"), output(b"b"), exit()]),
            sender,
            stopped,
            Duration::from_millis(10),
            4,
        ),
    )
    .await;
    assert!(matches!(result, Ok(Err(SandboxError::Timeout))));
}
#[test]
fn stdio_launch_rejects_invalid_data_before_backend_access() {
    let mut request = StdioRequest {
        argv: vec!["echo".into()],
        workdir: None,
        env: vec![],
        timeout: None,
        output_limit_bytes: None,
    };
    assert!(validate(&request, Duration::from_secs(1)).is_ok());
    assert!(validate(&request, Duration::ZERO).is_err());
    request.argv = vec!["bad\0program".into()];
    assert!(validate(&request, Duration::from_secs(1)).is_err());
    request.argv = vec!["echo".into()];
    request.env = vec![("A".into(), "one".into()), ("A".into(), "two".into())];
    assert!(validate(&request, Duration::from_secs(1)).is_err());
}

struct CleanupFixture {
    snapshot: Result<Option<(String, String)>, SandboxError>,
    receipt: Option<String>,
    observed: std::sync::Mutex<Vec<String>>,
}
impl super::CleanupGateway for CleanupFixture {
    async fn inspect(&self, name: &str) -> Result<Option<(String, String)>, SandboxError> {
        self.observed
            .lock()
            .map_err(|_| SandboxError::Timeout)?
            .push(format!("inspect:{name}"));
        self.snapshot.clone()
    }
    async fn delete(&self, name: &str) -> Result<Option<String>, SandboxError> {
        self.observed
            .lock()
            .map_err(|_| SandboxError::Timeout)?
            .push(format!("delete:{name}"));
        Ok(self.receipt.clone())
    }
    async fn await_deleted(&self, name: &str, id: &str) -> Result<(), SandboxError> {
        self.observed
            .lock()
            .map_err(|_| SandboxError::Timeout)?
            .push(format!("wait:{name}:{id}"));
        Ok(())
    }
}
#[tokio::test]
async fn stdio_stop_reaps_only_owned_sandbox() -> Result<(), Box<dyn std::error::Error>> {
    for (snapshot, expected) in [
        (None, vec!["inspect:owned"]),
        (
            Some(("owned".into(), "replacement".into())),
            vec!["inspect:owned"],
        ),
        (
            Some(("owned".into(), "uuid".into())),
            vec!["inspect:owned", "delete:owned", "wait:owned:uuid"],
        ),
    ] {
        let gateway = CleanupFixture {
            snapshot: Ok(snapshot),
            receipt: Some("uuid".into()),
            observed: std::sync::Mutex::new(vec![]),
        };
        super::delete_owned(&gateway, "owned", "uuid").await?;
        assert_eq!(
            *gateway.observed.lock().map_err(|_| "fixture poisoned")?,
            expected
        );
    }
    Ok(())
}
#[tokio::test]
async fn stdio_cleanup_rejects_bad_receipt_and_preserves_retry()
-> Result<(), Box<dyn std::error::Error>> {
    for snapshot in [
        Err(SandboxError::Timeout),
        Ok(Some(("wrong-name".into(), "uuid".into()))),
    ] {
        let gateway = CleanupFixture {
            snapshot,
            receipt: Some("uuid".into()),
            observed: std::sync::Mutex::new(vec![]),
        };
        assert!(
            super::delete_owned(&gateway, "owned", "uuid")
                .await
                .is_err()
        );
        assert_eq!(
            *gateway.observed.lock().map_err(|_| "fixture poisoned")?,
            vec!["inspect:owned"]
        );
    }
    let gateway = CleanupFixture {
        snapshot: Ok(Some(("owned".into(), "uuid".into()))),
        receipt: Some("replacement".into()),
        observed: std::sync::Mutex::new(vec![]),
    };
    assert!(
        super::delete_owned(&gateway, "owned", "uuid")
            .await
            .is_err()
    );
    assert_eq!(
        *gateway.observed.lock().map_err(|_| "fixture poisoned")?,
        vec!["inspect:owned", "delete:owned"]
    );
    // A failed attempt must inspect again, rather than cache a false cleanup acknowledgement.
    assert!(
        super::delete_owned(&gateway, "owned", "uuid")
            .await
            .is_err()
    );
    assert_eq!(
        gateway
            .observed
            .lock()
            .map_err(|_| "fixture poisoned")?
            .len(),
        4
    );
    Ok(())
}

// No RPC is sent: the lazy client is used only to construct already-cleaned lifecycle state.
fn completed_state(
    result: Result<i32, SandboxError>,
) -> Result<
    (
        std::sync::Arc<super::Lifecycle>,
        mpsc::Receiver<proto::ExecSandboxInput>,
    ),
    Box<dyn std::error::Error>,
> {
    let client = openshell_sdk::OpenShellClient::from_parts(
        tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy(),
        openshell_sdk::EdgeAuthInterceptor::new(None, None)?,
    );
    let (input_tx, input_rx) = mpsc::channel(8);
    let (stopped, _stopped_rx) = watch::channel(false);
    let (_completion_tx, completion) = watch::channel(Some(result));
    Ok((
        std::sync::Arc::new(super::Lifecycle {
            client: std::sync::Arc::new(client),
            name: "fixture".into(),
            id: "fixture-uuid".into(),
            input: tokio::sync::Mutex::new(Some(input_tx)),
            stopped,
            completion,
            cleanup: tokio::sync::Mutex::new(true),
            emergency_started: std::sync::atomic::AtomicBool::new(false),
        }),
        input_rx,
    ))
}
#[tokio::test]
async fn stdio_output_splits_frames_and_keeps_terminal_errors()
-> Result<(), Box<dyn std::error::Error>> {
    use polaris_sdk::interactive::StdioOutput;
    let (state, _input) = completed_state(Ok(7))?;
    let (sender, receiver) = mpsc::channel(8);
    sender
        .send(Ok(StdioChunk::Stdout(Bytes::from_static(b"1234567"))))
        .await?;
    sender
        .send(Ok(StdioChunk::Stderr(Bytes::from_static(b"err"))))
        .await?;
    sender.send(Ok(StdioChunk::Exit(7))).await?;
    drop(sender);
    let mut output = super::Output {
        receiver,
        pending: None,
        state,
        terminal: None,
    };
    for bytes in [b"123".as_slice(), b"456", b"7"] {
        assert_eq!(
            output.next(3).await?,
            Some(StdioChunk::Stdout(Bytes::copy_from_slice(bytes)))
        );
    }
    assert_eq!(
        output.next(3).await?,
        Some(StdioChunk::Stderr(Bytes::from_static(b"err")))
    );
    assert_eq!(output.next(3).await?, Some(StdioChunk::Exit(7)));
    assert_eq!(output.next(3).await?, None);
    assert_eq!(output.next(3).await?, None);
    let (state, _input) = completed_state(Err(SandboxError::Protocol("failed trailer".into())))?;
    let (sender, receiver) = mpsc::channel(1);
    drop(sender);
    let mut output = super::Output {
        receiver,
        pending: None,
        state,
        terminal: None,
    };
    assert!(matches!(
        output.next(3).await,
        Err(SandboxError::Protocol(_))
    ));
    assert!(matches!(
        output.next(3).await,
        Err(SandboxError::Protocol(_))
    ));
    Ok(())
}
#[tokio::test]
async fn stdio_input_drop_closes_sender_and_stop_is_distinct_from_exit()
-> Result<(), Box<dyn std::error::Error>> {
    use polaris_sdk::interactive::{StdioControl, StdioInput};
    let (state, mut receiver) = completed_state(Ok(7))?;
    let mut input = super::Input(std::sync::Arc::clone(&state));
    input.send(Bytes::from_static(b"a\0b")).await?;
    let event = receiver.recv().await.ok_or("missing input")?;
    assert!(
        matches!(event.payload, Some(proto::exec_sandbox_input::Payload::Stdin(bytes)) if bytes == b"a\0b")
    );
    drop(input);
    assert!(receiver.recv().await.is_none());
    let control = super::Control(state);
    assert_eq!(control.wait().await?, 7);
    control.stop(Duration::ZERO).await?;
    assert_eq!(control.wait().await?, 7);
    Ok(())
}
