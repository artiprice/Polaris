//! Spike S0-7: blake3-manifest workspace sync from Windows into an OpenShell sandbox.
//!
//! Transport: tar chunks (<= CHUNK_BYTES) piped through `ExecSandbox` stdin into
//! `tar -xf - -C <dest>` (no upload RPC exists). Verification: sha256 of every file on
//! both sides. Usage: `s0-7-sync [FILES]` (default 10000).

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use openshell_sdk::{DeleteOptions, EdgeAuthInterceptor, ExecOptions, OpenShellClient, SandboxSpec};
use sha2::{Digest, Sha256};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

type AnyResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const GATEWAY: &str = "https://localhost:17670";
const MTLS_DIR: &str = r"\\wsl.localhost\Ubuntu-24.04\home\dev\.config\openshell\gateways\openshell\mtls";
const SANDBOX: &str = "polaris-s0-7";
const DEST: &str = "/sandbox/ws";
const CHUNK_BYTES: usize = 896 * 1024; // gateway rejects requests > 1 MiB decoded (OutOfRange, measured)
const DENY: &[&str] = &[".env", ".git/config"];
const DENY_EXT: &[&str] = &["pem", "key"];

/// Deterministic synthetic repository: nested dirs, text with LF and CRLF, binary blobs,
/// a shell script, plus secrets that the deny-list must keep out.
fn generate(root: &Path, files: usize) -> AnyResult<()> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..files {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let dir = root.join(format!("crate{:02}/src/mod{:02}", i % 40, (i / 40) % 25));
        fs::create_dir_all(&dir)?;
        let size = 512 + usize::try_from(seed % 7680)?;
        let path = match i % 10 {
            0 => dir.join(format!("blob{i}.bin")),
            1 => dir.join(format!("win{i}.txt")),
            _ => dir.join(format!("file{i}.rs")),
        };
        let body: Vec<u8> = match i % 10 {
            0 => (0..size).map(|k| u8::try_from((k * 31 + i) % 256).unwrap_or(0)).collect(),
            1 => "line with crlf\r\n".repeat(size / 16).into_bytes(),
            _ => format!("// file {i}\nfn f{i}() -> u64 {{ {seed} }}\n").repeat(size / 40 + 1).into_bytes(),
        };
        fs::write(path, body)?;
    }
    fs::write(root.join("run.sh"), "#!/bin/sh\necho ok\n")?;
    fs::write(root.join(".env"), "NEBIUS_API_KEY=fake-not-a-key\n")?;
    fs::write(root.join("server.pem"), "-----BEGIN FAKE-----\n")?;
    fs::create_dir_all(root.join(".git"))?;
    fs::write(root.join(".git/config"), "[core]\n")?;
    Ok(())
}

fn rel(root: &Path, p: &Path) -> AnyResult<String> {
    Ok(p.strip_prefix(root)?.to_string_lossy().replace('\\', "/"))
}

fn denied(rel: &str) -> bool {
    DENY.contains(&rel)
        || Path::new(rel).extension().and_then(|e| e.to_str()).is_some_and(|e| DENY_EXT.contains(&e))
}

/// Manifest: relative path -> blake3 (hex). Returns (manifest, denied count).
fn manifest(root: &Path) -> AnyResult<(BTreeMap<String, String>, usize)> {
    let mut m = BTreeMap::new();
    let mut skipped = 0;
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let r = rel(root, entry.path())?;
        if denied(&r) {
            skipped += 1;
            continue;
        }
        let bytes = fs::read(entry.path())?;
        m.insert(r, blake3::hash(&bytes).to_hex().to_string());
    }
    Ok((m, skipped))
}

fn sha256_local(root: &Path, files: &BTreeMap<String, String>) -> AnyResult<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for r in files.keys() {
        let digest = Sha256::digest(fs::read(root.join(r))?);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        out.insert(r.clone(), hex);
    }
    Ok(out)
}

/// Build tar chunks of at most CHUNK_BYTES each from the given relative paths.
fn tar_chunks(root: &Path, paths: &[String]) -> AnyResult<Vec<Vec<u8>>> {
    let mut chunks = Vec::new();
    let mut builder = tar::Builder::new(Vec::new());
    let mut pending = 0usize;
    for r in paths {
        let full = root.join(r);
        let len = usize::try_from(fs::metadata(&full)?.len())?;
        if pending > 0 && pending + len + 1024 > CHUNK_BYTES {
            chunks.push(builder.into_inner()?);
            builder = tar::Builder::new(Vec::new());
            pending = 0;
        }
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(len)?);
        header.set_mode(if r.ends_with(".sh") { 0o755 } else { 0o644 });
        header.set_cksum();
        builder.append_data(&mut header, r, fs::File::open(&full)?)?;
        pending += len + 1024;
    }
    chunks.push(builder.into_inner()?);
    Ok(chunks)
}

async fn exec(client: &OpenShellClient, script: &str, stdin: Option<Vec<u8>>) -> AnyResult<(i32, String)> {
    let mut opts = ExecOptions::default();
    opts.no_login_shell = true;
    opts.stdin = stdin;
    let cmd = vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()];
    let out = client.exec(SANDBOX, &cmd, opts).await?;
    Ok((out.exit_code, String::from_utf8_lossy(&out.stdout).into_owned()))
}

async fn push(client: &OpenShellClient, root: &Path, paths: &[String]) -> AnyResult<(usize, usize)> {
    let chunks = tar_chunks(root, paths)?;
    let mut bytes = 0;
    for c in &chunks {
        bytes += c.len();
        let (code, _) = exec(client, &format!("mkdir -p {DEST} && tar -xf - -C {DEST}"), Some(c.clone())).await?;
        if code != 0 {
            return Err(format!("remote tar exited {code}").into());
        }
    }
    Ok((chunks.len(), bytes))
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let files: usize = std::env::args().nth(1).map_or(Ok(10_000), |a| a.parse())?;
    if rustls::crypto::aws_lc_rs::default_provider().install_default().is_err() {
        return Err("crypto provider already installed".into());
    }
    let root: PathBuf = std::env::temp_dir().join("polaris-s0-7").join("ws");

    let t = Instant::now();
    generate(&root, files)?;
    println!("generate: {files} files in {} ms", t.elapsed().as_millis());

    let t = Instant::now();
    let (m1, skipped) = manifest(&root)?;
    println!("manifest #1 (blake3, cold-ish): {} files, {} denied, {} ms", m1.len(), skipped, t.elapsed().as_millis());
    let t = Instant::now();
    let (m2, _) = manifest(&root)?;
    let changed: Vec<&String> = m2.iter().filter(|(k, v)| m1.get(*k) != Some(*v)).map(|(k, _)| k).collect();
    println!("manifest #2 + no-op diff: {} changed, {} ms", changed.len(), t.elapsed().as_millis());

    let dir = PathBuf::from(MTLS_DIR);
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(fs::read(dir.join("ca.crt"))?))
        .identity(Identity::from_pem(fs::read(dir.join("tls.crt"))?, fs::read(dir.join("tls.key"))?))
        .domain_name("localhost");
    let channel = Endpoint::from_shared(GATEWAY)?.tls_config(tls)?.connect().await?;
    let client = OpenShellClient::from_parts(channel, EdgeAuthInterceptor::noop());
    let spec = SandboxSpec {
        name: Some(SANDBOX.to_owned()),
        command: vec!["sleep".to_owned(), "infinity".to_owned()],
        ..SandboxSpec::default()
    };
    let created = client.create_sandbox(spec).await?;
    client.wait_ready(SANDBOX, Duration::from_secs(120)).await?;

    let all: Vec<String> = m1.keys().cloned().collect();
    let t = Instant::now();
    let (n, bytes) = push(&client, &root, &all).await?;
    println!("initial push: {} files, {n} chunks, {:.1} MiB, {} ms", all.len(), bytes as f64 / 1_048_576.0, t.elapsed().as_millis());

    // Byte-exact verification: sha256 of every file on both sides.
    let local = sha256_local(&root, &m1)?;
    let (_, remote_out) = exec(&client, &format!("cd {DEST} && find . -type f -print0 | xargs -0 sha256sum"), None).await?;
    let mut remote = BTreeMap::new();
    for line in remote_out.lines() {
        if let Some((h, p)) = line.split_once("  ") {
            remote.insert(p.trim_start_matches("./").to_owned(), h.to_owned());
        }
    }
    let mismatches = local.iter().filter(|(k, v)| remote.get(*k) != Some(*v)).count();
    let leaked: Vec<&String> = remote.keys().filter(|k| denied(k)).collect();
    println!("verify: local={} remote={} mismatches={mismatches} denied-files-in-sandbox={}", local.len(), remote.len(), leaked.len());
    let (_, mode) = exec(&client, &format!("stat -c %a {DEST}/run.sh; od -c {DEST}/crate01/src/mod00/win1.txt | head -1"), None).await?;
    println!("fidelity sample: {}", mode.replace('\n', " | "));

    // Incremental: modify 3 files, re-manifest, push only the delta.
    for r in all.iter().take(3) {
        let p = root.join(r);
        let mut b = fs::read(&p)?;
        b.extend_from_slice(b"\n// edited\n");
        fs::write(&p, b)?;
    }
    let t = Instant::now();
    let (m3, _) = manifest(&root)?;
    let delta: Vec<String> = m3.iter().filter(|(k, v)| m1.get(*k) != Some(*v)).map(|(k, _)| k.clone()).collect();
    let manifest_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let (n, bytes) = push(&client, &root, &delta).await?;
    println!("incremental: manifest+diff {manifest_ms} ms, {} changed files -> push {n} chunk(s) {bytes} B in {} ms", delta.len(), t.elapsed().as_millis());

    client.delete_sandbox(SANDBOX, DeleteOptions::default()).await?;
    client.wait_deleted(SANDBOX, Duration::from_secs(60), Some(&created.id)).await?;
    fs::remove_dir_all(root.parent().unwrap_or(&root))?;

    let pass = mismatches == 0 && leaked.is_empty() && local.len() == remote.len();
    println!("RESULT: {}", if pass { "PASS" } else { "FAIL" });
    if pass { Ok(()) } else { Err("verification failed".into()) }
}
