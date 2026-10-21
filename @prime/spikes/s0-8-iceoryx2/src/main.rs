//! Spike S0-8: iceoryx2 0.10 between two Windows processes — round-trip latency, behaviour
//! when the peer is killed (TerminateProcess), dead-node cleanup, reconnect, leftovers.
//!
//! Usage: `s0-8-iceoryx2 [ROUNDS]` (driver; spawns `pong` children) | `pong` | `inspect`.

use core::time::Duration;
use std::error::Error;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use iceoryx2::prelude::*;

type AnyResult<T> = Result<T, Box<dyn Error>>;

const PING: &str = "polaris/s08/ping";
const PONG: &str = "polaris/s08/pong";
const STOP: u64 = u64::MAX;
const ROOT: &str = r"C:\Temp\iceoryx2";

fn pong() -> AnyResult<()> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let ping = node.service_builder(&PING.try_into()?).publish_subscribe::<u64>().open_or_create()?;
    let pong = node.service_builder(&PONG.try_into()?).publish_subscribe::<u64>().open_or_create()?;
    let sub = ping.subscriber_builder().create()?;
    let publ = pong.publisher_builder().create()?;
    loop {
        match sub.receive()? {
            Some(sample) => {
                let v = *sample;
                drop(sample);
                if v == STOP {
                    return Ok(());
                }
                publ.loan_uninit()?.write_payload(v).send()?;
            }
            None => std::thread::yield_now(),
        }
    }
}

fn percentiles(label: &str, mut v: Vec<Duration>, timeouts: usize) {
    v.sort_unstable();
    let at = |q: f64| -> f64 {
        let n = v.len();
        if n == 0 {
            return f64::NAN;
        }
        let i = ((q * (n as f64 - 1.0)).round() as usize).min(n - 1);
        v.get(i).map_or(f64::NAN, |d| d.as_secs_f64() * 1e6)
    };
    println!(
        "{label}: n={} timeouts={timeouts} rtt_us p50={:.1} p95={:.1} p99={:.1} max={:.1}",
        v.len(),
        at(0.50),
        at(0.95),
        at(0.99),
        at(1.0)
    );
}

fn node_census(cleanup: bool) -> AnyResult<(usize, usize, usize)> {
    let (mut alive, mut dead, mut cleaned) = (0, 0, 0);
    Node::<ipc::Service>::list(Config::global_config(), |state| {
        match state {
            NodeState::Alive(_) => alive += 1,
            NodeState::Dead(view) => {
                dead += 1;
                if cleanup {
                    match view.try_remove_stale_resources() {
                        Ok(_) => cleaned += 1,
                        Err(e) => println!("  cleanup failed: {e:?}"),
                    }
                }
            }
            other => println!("  other node state: {other:?}"),
        }
        CallbackProgression::Continue
    })?;
    Ok((alive, dead, cleaned))
}

fn files_under(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |rd| {
        rd.flatten()
            .map(|e| {
                let p = e.path();
                if p.is_dir() { files_under(&p) } else { 1 }
            })
            .sum()
    })
}

fn inspect() -> AnyResult<()> {
    let (alive, dead, _) = node_census(false)?;
    println!("nodes: alive={alive} dead={dead}; files under {ROOT}: {}", files_under(Path::new(ROOT)));
    Ok(())
}

fn driver(rounds: usize) -> AnyResult<bool> {
    let exe = std::env::current_exe()?;
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let ping = node.service_builder(&PING.try_into()?).publish_subscribe::<u64>().open_or_create()?;
    let pongs = node.service_builder(&PONG.try_into()?).publish_subscribe::<u64>().open_or_create()?;
    let publ = ping.publisher_builder().create()?;
    let sub = pongs.subscriber_builder().create()?;

    let rtt = |v: u64, timeout: Duration| -> AnyResult<Option<Duration>> {
        let t = Instant::now();
        publ.loan_uninit()?.write_payload(v).send()?;
        loop {
            if let Some(s) = sub.receive()? {
                if *s == v {
                    return Ok(Some(t.elapsed()));
                }
                continue;
            }
            if t.elapsed() > timeout {
                return Ok(None);
            }
            std::hint::spin_loop();
        }
    };
    let connect = |deadline: Duration, base: u64| -> AnyResult<Option<Duration>> {
        let t = Instant::now();
        let mut seq = base;
        while t.elapsed() < deadline {
            seq += 1;
            if rtt(seq, Duration::from_millis(20))?.is_some() {
                return Ok(Some(t.elapsed()));
            }
        }
        Ok(None)
    };
    let measure = |label: &str, n: usize, base: u64| -> AnyResult<usize> {
        let mut lat = Vec::with_capacity(n);
        let mut timeouts = 0;
        for i in 0..n {
            match rtt(base + i as u64, Duration::from_secs(1))? {
                Some(d) => lat.push(d),
                None => timeouts += 1,
            }
        }
        percentiles(label, lat, timeouts);
        Ok(timeouts)
    };

    // Phase 1: spawn, connect, latency.
    let t = Instant::now();
    let mut child = Command::new(&exe).arg("pong").spawn()?;
    let up = connect(Duration::from_secs(10), 1_000_000_000)?;
    println!("phase1: first echo {:?} after spawn (includes process start)", up.map(|d| d.max(t.elapsed())));
    measure("warmup", 1_000, 2_000_000_000)?;
    let t1 = measure("phase1", rounds, 3_000_000_000)?;

    // Phase 2: kill the peer, keep publishing, census + cleanup.
    child.kill()?;
    let status = child.wait()?;
    println!("phase2: peer killed (exit status {status})");
    let (mut sent, mut send_err) = (0usize, 0usize);
    let t2 = Instant::now();
    while t2.elapsed() < Duration::from_millis(200) {
        let r: AnyResult<usize> = (|| Ok(publ.loan_uninit()?.write_payload(7).send()?))();
        match r {
            Ok(_) => sent += 1,
            Err(e) => {
                send_err += 1;
                if send_err == 1 {
                    println!("  first send error into dead peer: {e}");
                }
            }
        }
    }
    println!("phase2: publish into dead peer for 200 ms: ok={sent} err={send_err}");
    let (alive, dead, cleaned) = node_census(true)?;
    println!("phase2: census alive={alive} dead={dead} cleaned={cleaned}");

    // Phase 3: respawn, reconnect, latency again, graceful stop.
    let t3 = Instant::now();
    let mut child = Command::new(&exe).arg("pong").spawn()?;
    let re = connect(Duration::from_secs(10), 4_000_000_000)?;
    println!("phase3: reconnect {:?} after respawn", re.map(|d| d.max(t3.elapsed())));
    let t4 = measure("phase3", rounds / 4, 5_000_000_000)?;
    publ.loan_uninit()?.write_payload(STOP).send()?;
    let status = child.wait()?;
    println!("phase3: peer stopped gracefully (exit status {status})");

    let pass = up.is_some() && re.is_some() && t1 == 0 && t4 == 0 && send_err == 0 && cleaned == dead;
    Ok(pass)
}

fn main() -> AnyResult<()> {
    match std::env::args().nth(1).as_deref() {
        Some("pong") => pong(),
        Some("inspect") => inspect(),
        arg => {
            let rounds: usize = arg.map_or(Ok(20_000), str::parse)?;
            let pass = driver(rounds)?;
            println!("RESULT: {}", if pass { "PASS" } else { "FAIL" });
            Ok(())
        }
    }
}
