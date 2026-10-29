// Unit tests of apps/polaris-worker/src/main.rs (`tests`); compiled into that module via #[path].
use std::io::Cursor;
use std::sync::mpsc;
use std::time::Duration;

use super::{idle_policy, parse_options, read_worker_credential_from, shutdown_runtime};

#[test]
fn idle_cli_policy_is_bounded_and_does_not_echo_bad_values()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for value in [
        "0",
        "-1",
        "86400001",
        "18446744073709551616",
        "private-value",
    ] {
        let options = parse_options(&["--idle-ttl-ms".to_owned(), value.to_owned()])?;
        let error = idle_policy(&options)
            .err()
            .ok_or("invalid idle policy accepted")?;
        assert!(
            !error.to_string().contains(value),
            "bad input must not be echoed"
        );
    }
    let options = parse_options(&["--idle-ttl-ms".to_owned(), "25".to_owned()])?;
    assert_eq!(idle_policy(&options)?.idle_ttl(), Duration::from_millis(25));
    assert_eq!(
        idle_policy(&std::collections::HashMap::new())?.idle_ttl(),
        Duration::from_secs(45)
    );
    Ok(())
}

#[test]
fn bootstrap_rejects_short_and_non_utf8_secrets() {
    assert!(read_worker_credential_from(&mut Cursor::new(b"short".to_vec())).is_err());
    assert!(read_worker_credential_from(&mut Cursor::new(vec![0xff; 64])).is_err());
    assert!(read_worker_credential_from(&mut Cursor::new(vec![b'g'; 64])).is_err());
}

#[test]
fn runtime_shutdown_wait_is_bounded_while_blocking_work_remains()
-> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()?;
    let (started_sender, started_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let (blocking_finished_sender, blocking_finished_receiver) = mpsc::channel();
    let _blocking = runtime.spawn_blocking(move || {
        let _ = started_sender.send(());
        let _ = release_receiver.recv();
        let _ = blocking_finished_sender.send(());
    });
    if started_receiver
        .recv_timeout(Duration::from_secs(2))
        .is_err()
    {
        let _ = release_sender.send(());
        return Err("blocking task did not start".into());
    }
    let (finished_sender, finished_receiver) = mpsc::channel();
    let shutdown = std::thread::Builder::new()
        .name("worker-runtime-shutdown-test".to_owned())
        .spawn(move || {
            shutdown_runtime(runtime, Duration::from_millis(250));
            let _ = finished_sender.send(());
        })?;
    let returned_before_release = finished_receiver
        .recv_timeout(Duration::from_secs(2))
        .is_ok();
    let _ = release_sender.send(());
    let blocker_completed = blocking_finished_receiver
        .recv_timeout(Duration::from_secs(2))
        .is_ok();
    shutdown
        .join()
        .map_err(|_| std::io::Error::other("shutdown thread panicked"))?;
    assert!(
        returned_before_release,
        "runtime shutdown must return within its configured wait without preempting blocking work"
    );
    assert!(
        blocker_completed,
        "the started blocking task must be able to finish after the bounded wait returns"
    );
    Ok(())
}
