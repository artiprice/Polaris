//! Bounded read-only listener ownership queries for local model startup.

use polaris_sdk::registry::PlatformError;
use std::{net::SocketAddr, path::Path};

#[cfg(target_os = "linux")]
const MAX_TABLE_BYTES: usize = 4 * 1024 * 1024;
#[cfg(target_os = "linux")]
const MAX_PROCESS_FDS: usize = 65_536;

#[cfg(windows)]
pub(super) fn owns_exact_root_listener(
    root_pid: u32,
    address: SocketAddr,
) -> Result<bool, PlatformError> {
    use std::time::Duration;

    use crate::run_probe;

    let current_dir = std::env::current_dir().map_err(platform_io)?;
    let output = run_probe(
        Path::new(r"C:\Windows\System32\netstat.exe"),
        &["-ano".to_owned(), "-p".to_owned(), "tcp".to_owned()],
        &current_dir,
        Duration::from_secs(2),
        256 * 1024,
    )?;
    if output.timed_out
        || output.stdout_truncated
        || output.exit_code != Some(0)
    {
        return Err(PlatformError::Io(
            "bounded listener ownership query did not complete".to_owned(),
        ));
    }
    parse_netstat_listener_owner(&String::from_utf8_lossy(&output.stdout), root_pid, address)
}

#[cfg(windows)]
fn parse_netstat_listener_owner(
    text: &str,
    root_pid: u32,
    address: SocketAddr,
) -> Result<bool, PlatformError> {
    let mut matching = 0_u8;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let protocol = fields.next();
        if protocol != Some("TCP") {
            continue;
        }
        let local = fields.next();
        let remote = fields.next();
        let state = fields.next();
        let pid = fields.next();
        let (Some(local), Some(_remote), Some(state), Some(pid)) = (local, remote, state, pid)
        else {
            continue;
        };
        if state != "LISTENING" {
            continue;
        }
        let parsed = local.parse::<SocketAddr>();
        let local_address = match parsed {
            Ok(value) => value,
            Err(_) => {
                if local_port(local) == Some(address.port()) {
                    return Ok(false);
                }
                continue;
            }
        };
        if local_address.port() != address.port() {
            continue;
        }
        if local_address != address {
            return Ok(false);
        }
        let owner_pid = match pid.parse::<u32>() {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        if owner_pid != root_pid {
            return Ok(false);
        }
        matching = matching.checked_add(1).ok_or_else(|| {
            PlatformError::Io("listener ownership row count overflow".to_owned())
        })?;
        if matching > 1 {
            return Ok(false);
        }
    }
    Ok(matching == 1)
}

#[cfg(windows)]
fn local_port(value: &str) -> Option<u16> {
    value
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
}

#[cfg(target_os = "linux")]
pub(super) fn owns_exact_root_listener(
    root_pid: u32,
    address: SocketAddr,
) -> Result<bool, PlatformError> {
    let ipv4 = read_proc_table(Path::new("/proc/net/tcp"), address)?;
    let ipv6 = read_proc_table(Path::new("/proc/net/tcp6"), address)?;
    let mut matching = ipv4;
    for inode in ipv6 {
        if !matching.is_empty() {
            return Ok(false);
        }
        matching.push(inode);
    }
    if matching.len() != 1 {
        return Ok(false);
    }
    let inode = matching.first().copied().ok_or_else(|| {
        PlatformError::Io("listener ownership observation was incomplete".to_owned())
    })?;
    process_owns_inode(root_pid, inode)
}

#[cfg(target_os = "linux")]
fn read_proc_table(path: &Path, requested: SocketAddr) -> Result<Vec<u64>, PlatformError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(platform_io)?;
    let read_limit = MAX_TABLE_BYTES.checked_add(1).ok_or_else(|| {
        PlatformError::Io("listener table bound overflow".to_owned())
    })?;
    let mut contents = String::new();
    file.by_ref()
        .take(u64::try_from(read_limit).map_err(|_| {
            PlatformError::Io("listener table bound exceeds the read limit type".to_owned())
        })?)
        .read_to_string(&mut contents)
        .map_err(platform_io)?;
    if contents.len() > MAX_TABLE_BYTES {
        return Err(PlatformError::Io(
            "listener table exceeded its scan bound".to_owned(),
        ));
    }
    parse_proc_table(&contents, requested)
}

#[cfg(target_os = "linux")]
fn parse_proc_table(text: &str, requested: SocketAddr) -> Result<Vec<u64>, PlatformError> {
    let mut inodes = Vec::new();
    for line in text.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let _slot = fields.next();
        let local = fields.next();
        let _remote = fields.next();
        let state = fields.next();
        let _tx_rx = fields.next();
        let _timer = fields.next();
        let _retransmit = fields.next();
        let _uid = fields.next();
        let _timeout = fields.next();
        let inode = fields.next();
        let (Some(local), Some(state), Some(inode)) = (local, state, inode) else {
            return Err(PlatformError::Io(
                "listener table contained a malformed row".to_owned(),
            ));
        };
        if state != "0A" {
            continue;
        }
        let local_address = parse_proc_socket_address(local)?;
        if local_address.port() != requested.port() {
            continue;
        }
        if local_address != requested {
            return Err(PlatformError::Policy(
                "a conflicting listener uses the requested local port".to_owned(),
            ));
        }
        let inode = inode.parse::<u64>().map_err(|_| {
            PlatformError::Io("listener table contained an invalid socket inode".to_owned())
        })?;
        inodes.push(inode);
        if inodes.len() > 1 {
            return Ok(inodes);
        }
    }
    Ok(inodes)
}

#[cfg(target_os = "linux")]
fn parse_proc_socket_address(value: &str) -> Result<SocketAddr, PlatformError> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    let (address, port) = value.rsplit_once(':').ok_or_else(|| {
        PlatformError::Io("listener table contained an invalid socket address".to_owned())
    })?;
    let port = u16::from_str_radix(port, 16).map_err(|_| {
        PlatformError::Io("listener table contained an invalid local port".to_owned())
    })?;
    let ip = match address.len() {
        8 => {
            let value = u32::from_str_radix(address, 16).map_err(|_| {
                PlatformError::Io("listener table contained an invalid IPv4 address".to_owned())
            })?;
            IpAddr::V4(Ipv4Addr::from(value.to_le_bytes()))
        }
        32 => {
            let mut octets = [0_u8; 16];
            for (index, word) in address.as_bytes().chunks_exact(8).enumerate() {
                let word = std::str::from_utf8(word).map_err(|_| {
                    PlatformError::Io("listener table contained invalid address bytes".to_owned())
                })?;
                let value = u32::from_str_radix(word, 16).map_err(|_| {
                    PlatformError::Io("listener table contained an invalid IPv6 address".to_owned())
                })?;
                let start = index.checked_mul(4).ok_or_else(|| {
                    PlatformError::Io("IPv6 listener address offset overflow".to_owned())
                })?;
                let end = start.checked_add(4).ok_or_else(|| {
                    PlatformError::Io("IPv6 listener address range overflow".to_owned())
                })?;
                octets
                    .get_mut(start..end)
                    .ok_or_else(|| {
                        PlatformError::Io("IPv6 listener address was out of bounds".to_owned())
                    })?
                    .copy_from_slice(&value.to_le_bytes());
            }
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        _ => {
            return Err(PlatformError::Io(
                "listener table contained an invalid IP address".to_owned(),
            ));
        }
    };
    Ok(SocketAddr::new(ip, port))
}

#[cfg(target_os = "linux")]
fn process_owns_inode(root_pid: u32, expected_inode: u64) -> Result<bool, PlatformError> {
    let descriptor_dir = Path::new("/proc")
        .join(root_pid.to_string())
        .join("fd");
    let entries = std::fs::read_dir(descriptor_dir).map_err(platform_io)?;
    for (count, entry) in entries.enumerate() {
        if count >= MAX_PROCESS_FDS {
            return Err(PlatformError::Io(
                "process descriptor scan exceeded its bound".to_owned(),
            ));
        }
        let entry = entry.map_err(platform_io)?;
        let target = std::fs::read_link(entry.path()).map_err(platform_io)?;
        if let Some(inode) = target
            .to_string_lossy()
            .strip_prefix("socket:[")
            .and_then(|value| value.strip_suffix(']'))
            .and_then(|value| value.parse::<u64>().ok())
            && inode == expected_inode
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(not(any(target_os = "linux", windows)))]
pub(super) fn owns_exact_root_listener(
    _root_pid: u32,
    _address: SocketAddr,
) -> Result<bool, PlatformError> {
    Ok(false)
}

fn platform_io(error: std::io::Error) -> PlatformError {
    PlatformError::Io(error.to_string())
}
