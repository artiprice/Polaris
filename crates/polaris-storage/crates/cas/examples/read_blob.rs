//! Read one verified CAS blob for a headless evidence audit.
use polaris_sdk::ids::Cid;
use polaris_storage_cas::{Cas, CasConfig};
use std::{io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("usage: read_blob CAS_ROOT CID")?);
    let cid = args.next().ok_or("usage: read_blob CAS_ROOT CID")?;
    if args.next().is_some() || !root.is_dir() {
        return Err("expected an existing CAS directory and exactly one content ID".into());
    }
    let cid = Cid::from_hex(cid.to_str().ok_or("content ID must be ASCII hexadecimal")?)?;
    let mut config = CasConfig::new(root);
    config.max_blob_bytes = 8 * 1024 * 1024;
    let bytes = Cas::open(config)?.get(&cid)?;
    std::io::stdout().lock().write_all(&bytes)?;
    Ok(())
}
