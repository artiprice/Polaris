//! Spike S0-12 guest: the in-sandbox side of the host<->guest contract.
//!
//! `polaris-guest handshake <host_protocol_version>` prints one JSON line describing the guest and
//! exits 0 when the versions match, 3 when they do not (the host must refuse to drive it).

use std::process::ExitCode;

const PROTOCOL_VERSION: u32 = 1;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next().map(|v| v.parse::<u32>())) {
        (Some("handshake"), Some(Ok(host))) => {
            println!(
                "{{\"protocol_version\":{PROTOCOL_VERSION},\"guest_version\":\"{}\",\"os\":\"{}\",\"arch\":\"{}\",\"pid\":{}}}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                std::process::id()
            );
            if host == PROTOCOL_VERSION {
                ExitCode::SUCCESS
            } else {
                eprintln!("protocol mismatch: host {host}, guest {PROTOCOL_VERSION}");
                ExitCode::from(3)
            }
        }
        _ => {
            eprintln!("usage: polaris-guest handshake <host_protocol_version>");
            ExitCode::from(2)
        }
    }
}
