//! Real setup probes. Every authoritative member-setup item remains visible in the report.
use crate::CliError;
use polaris_config::{PolarisConfig, SecretRef};
use polaris_platform::{
    process::{resolve_program, run_probe},
    wsl::{DistroName, Wsl},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

const INVENTORY: &str = include_str!("../../../@prime/tools/workspace.toml");
const PROBE_CAP: usize = 8192;

pub(crate) async fn collect(workspace: &Path, config: &PolarisConfig) -> Result<Value, CliError> {
    let cwd = std::fs::canonicalize(workspace)?;
    let native_cwd = cwd.clone();
    let tf = config.nebius.api_key.clone();
    let tavily = config.search.tavily_api_key.clone();
    let mut probes = tokio::task::spawn_blocking(move || {
        native_probes(&native_cwd, tf.as_ref(), tavily.as_ref())
    })
    .await
    .map_err(|_| CliError::Unavailable("setup probe owner failed"))?;
    collect_wsl(&mut probes).await;
    assemble(probes)
}
fn assemble(mut probes: BTreeMap<String, Value>) -> Result<Value, CliError> {
    let inventory: toml::Value = toml::from_str(INVENTORY)
        .map_err(|_| CliError::Unavailable("embedded setup inventory invalid"))?;
    let entries = inventory
        .get("member_setup")
        .and_then(toml::Value::as_array)
        .ok_or(CliError::Unavailable("setup inventory missing"))?;
    let mut items = Vec::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or(CliError::Unavailable("setup item has no id"))?;
        let probe = match probes.remove(id) {
            Some(value) => value,
            None => observation("UNKNOWN", "no probe registered for this inventory entry"),
        };
        items.push(json!({"id":id,"component":entry.get("component").and_then(toml::Value::as_str),"required":entry.get("required").and_then(toml::Value::as_bool),"from":entry.get("from").and_then(toml::Value::as_str),"onboarding":entry.get("install").and_then(toml::Value::as_str),"observation":probe}));
    }
    let required_missing = items
        .iter()
        .filter(|item| {
            item.get("required").and_then(serde_json::Value::as_bool) == Some(true)
                && !matches!(
                    item.get("observation")
                        .and_then(|value| value.get("status"))
                        .and_then(serde_json::Value::as_str),
                    Some("PASS" | "NOT_APPLICABLE")
                )
        })
        .count();
    Ok(
        json!({"items":items,"required_missing":required_missing,"ready":required_missing==0,"keys":"presence_only_no_secret_payloads","onboarding":"follow each missing item's onboarding action; provisioning plans do not edit host configuration"}),
    )
}
fn observation(status: &str, detail: &str) -> Value {
    json!({"status":status,"detail":detail})
}
pub(crate) fn is_ready(report: &Value) -> bool {
    report.get("ready").and_then(Value::as_bool) == Some(true)
}
fn native(cwd: &Path, program: &str, args: &[&str], contains: Option<&str>) -> Value {
    let resolved = match resolve_program(std::ffi::OsStr::new(program), cwd) {
        Ok(resolved) => resolved,
        Err(_) => return observation("MISSING", "executable not safely resolvable"),
    };
    let argv = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    match run_probe(
        &resolved.path,
        &argv,
        cwd,
        Duration::from_secs(8),
        PROBE_CAP,
    ) {
        Ok(output) => {
            let out = polaris_tools_rtk::decode_stream(&output.stdout);
            let err = polaris_tools_rtk::decode_stream(&output.stderr);
            let matches = match contains {
                Some(required) => out.contains(required) || err.contains(required),
                None => true,
            };
            let pass = output.exit_code == Some(0)
                && !output.timed_out
                && !output.stdout_truncated
                && !output.stderr_truncated
                && matches;
            json!({"status":if pass{"PASS"}else{"FAIL"},"program":resolved.path,"argv":argv,"exit_code":output.exit_code,"timed_out":output.timed_out,"truncated":output.stdout_truncated||output.stderr_truncated,"stdout":out,"stderr":err})
        }
        Err(_) => observation(
            "FAIL",
            "native probe could not execute within its ownership policy",
        ),
    }
}
fn group(checks: Vec<Value>) -> Value {
    let pass = checks.iter().all(|check| check["status"] == "PASS");
    json!({"status":if pass{"PASS"}else{"FAIL"},"checks":checks})
}
fn credential(reference: Option<&SecretRef>) -> Value {
    match reference {
        None => observation("MISSING", "no credential reference configured"),
        Some(SecretRef::Env(name)) => environment_presence(std::env::var_os(name).as_deref()),
        Some(SecretRef::Keyring { service, user }) => match keyring::Entry::new(service, user)
            .and_then(|entry| entry.inner.get_attributes())
        {
            Ok(_) => observation(
                "PASS",
                "OS credential entry metadata exists; secret payload not requested",
            ),
            Err(keyring::Error::NoEntry) => observation("MISSING", "OS credential entry absent"),
            Err(_) => observation("UNKNOWN", "OS credential metadata query unavailable"),
        },
    }
}
fn environment_presence(value: Option<&std::ffi::OsStr>) -> Value {
    observation(
        if value.is_some_and(|value| !value.is_empty()) {
            "PASS"
        } else {
            "MISSING"
        },
        "environment presence checked; value omitted",
    )
}
fn file(path: PathBuf) -> Value {
    observation(
        if path.is_file() { "PASS" } else { "MISSING" },
        "file presence checked",
    )
}
fn native_probes(
    cwd: &Path,
    tf: Option<&SecretRef>,
    tavily: Option<&SecretRef>,
) -> BTreeMap<String, Value> {
    let mut probes = BTreeMap::new();
    let commands: [(&str, &str, &[&str], Option<&str>); 10] = [
        ("win-git", "git", &["--version"], None),
        ("win-rust", "rustc", &["--version"], Some("1.99.0")),
        ("win-python", "python", &["--version"], Some("3.14")),
        ("win-protoc", "protoc", &["--version"], None),
        ("win-uv", "python", &["-m", "uv", "--version"], None),
        ("win-ollama", "ollama", &["list"], Some("nemotron-mini")),
        ("win-cuda", "nvcc", &["--version"], None),
        (
            "win-wasm32",
            "rustup",
            &["target", "list", "--installed", "--toolchain", "1.99.0"],
            Some("wasm32-unknown-unknown"),
        ),
        ("contree-mcp", "contree-mcp", &["--version"], None),
        (
            "wsl-keepalive",
            "schtasks.exe",
            &["/Query", "/TN", "Polaris WSL keepalive", "/FO", "LIST"],
            None,
        ),
    ];
    for (id, program, args, contains) in commands {
        probes.insert(
            id.into(),
            if cfg!(windows) {
                native(cwd, program, args, contains)
            } else {
                observation("NOT_APPLICABLE", "Windows host setup item")
            },
        );
    }
    probes.insert(
        "win-cmake-nasm".into(),
        if cfg!(windows) {
            group(vec![
                native(cwd, "cmake", &["--version"], None),
                native(cwd, "nasm", &["-v"], None),
                native(cwd, "ninja", &["--version"], None),
            ])
        } else {
            observation("NOT_APPLICABLE", "Windows native build tools")
        },
    );
    probes.insert(
        "win-rust".into(),
        if cfg!(windows) {
            group(vec![
                native(cwd, "rustc", &["--version"], Some("1.99.0")),
                native(cwd, "rustfmt", &["--version"], None),
                native(cwd, "cargo", &["clippy", "--version"], None),
            ])
        } else {
            observation("NOT_APPLICABLE", "Windows Rust toolchain")
        },
    );
    let tools = [
        "cargo-binstall",
        "cargo-nextest",
        "cargo-mutants",
        "cargo-leptos",
        "cargo-deny",
        "cargo-vet",
        "cargo-audit",
    ];
    probes.insert(
        "win-cargo-tools".into(),
        if cfg!(windows) {
            group(
                tools
                    .into_iter()
                    .map(|tool| {
                        if tool == "cargo-mutants" {
                            native(cwd, "cargo", &["mutants", "--version"], None)
                        } else if tool == "cargo-binstall" {
                            native(cwd, tool, &["-V"], None)
                        } else {
                            native(cwd, tool, &["--version"], None)
                        }
                    })
                    .collect(),
            )
        } else {
            observation("NOT_APPLICABLE", "Windows cargo tools")
        },
    );
    let msvc = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .map(|base| base.join("Microsoft Visual Studio/Installer/vswhere.exe"));
    probes.insert(
        "win-msvc".into(),
        if !cfg!(windows) {
            observation("NOT_APPLICABLE", "Windows MSVC")
        } else {
            match msvc {
                Some(path) => native(
                    cwd,
                    &path.to_string_lossy(),
                    &[
                        "-prerelease",
                        "-latest",
                        "-products",
                        "*",
                        "-requires",
                        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                        "-property",
                        "installationPath",
                    ],
                    Some("Visual Studio"),
                ),
                None => observation("MISSING", "Visual Studio installer location unavailable"),
            }
        },
    );
    probes.insert(
        "win-longpaths".into(),
        if cfg!(windows) {
            group(vec![
                native(
                    cwd,
                    "git",
                    &["config", "--get", "core.longpaths"],
                    Some("true"),
                ),
                native(
                    cwd,
                    "reg.exe",
                    &[
                        "query",
                        r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem",
                        "/v",
                        "LongPathsEnabled",
                    ],
                    Some("0x1"),
                ),
            ])
        } else {
            observation("NOT_APPLICABLE", "Windows long path policy")
        },
    );
    probes.insert("win-keyring".into(),json!({"status":"OBSERVED","token_factory":credential(tf),"tavily":credential(tavily),"values":"omitted"}));
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    probes.insert(
        "win-models".into(),
        match home {
            Some(home) => model_files(&home.join(".polaris/models")),
            None => observation("NOT_APPLICABLE", "Windows model directory unavailable"),
        },
    );
    if cfg!(windows) {
        probes.insert("win-cuda".into(), cuda_files());
    }
    probes.insert(
        "win-pdfium".into(),
        match std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
        {
            Some(dir) => file(dir.join("pdfium.dll")),
            None => observation("UNKNOWN", "binary directory unavailable"),
        },
    );
    probes
}
fn model_files(root: &Path) -> Value {
    let mut pending = vec![root.to_path_buf()];
    let mut seen = 0usize;
    let (mut onnx, mut cact, mut runtime) = (false, false, false);
    let mut incomplete = false;
    while let Some(directory) = pending.pop() {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(_) => {
                incomplete = true;
                continue;
            }
        };
        for entry in entries {
            if seen >= 10_000 {
                incomplete = true;
                break;
            }
            seen = seen.saturating_add(1);
            let Ok(entry) = entry else {
                incomplete = true;
                continue;
            };
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                incomplete = true;
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                pending.push(path);
            } else if meta.is_file() && meta.len() > 0 {
                onnx |= path.extension().is_some_and(|ext| ext == "onnx");
                cact |= path.extension().is_some_and(|ext| ext == "cact");
                runtime |= path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("onnxruntime.dll"));
            }
        }
        if seen >= 10_000 {
            break;
        }
    }
    json!({"status":if onnx&&cact&&runtime{"PASS"}else if incomplete{"UNKNOWN"}else{"MISSING"},"onnx_file":onnx,"cact_file":cact,"onnxruntime_dll":runtime,"entries_examined":seen,"incomplete":incomplete,"scope":"nonempty artifact presence only, bounded recursive scan without following symlinks; model load and inference remain unchecked"})
}
fn cuda_files() -> Value {
    let mut runtime = false;
    let mut cudnn = false;
    let mut seen = 0usize;
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths).take(256) {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.take(4096usize.saturating_sub(seen)).flatten() {
                seen = seen.saturating_add(1);
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if !entry
                    .metadata()
                    .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
                {
                    continue;
                }
                runtime |= name.starts_with("cudart64_") && name.ends_with(".dll");
                cudnn |= name.starts_with("cudnn64_") && name.ends_with(".dll");
            }
            if seen >= 4096 {
                break;
            }
        }
    }
    json!({"status":if runtime&&cudnn{"PASS"}else{"UNKNOWN"},"cuda_runtime_dll":runtime,"cudnn_dll":cudnn,"entries_examined":seen,"scope":"DLL file presence on PATH only; loading and CUDA provider execution unchecked"})
}
async fn collect_wsl(probes: &mut BTreeMap<String, Value>) {
    let ids = [
        "wsl-distro",
        "wsl-docker",
        "wsl-openshell",
        "wsl-rust",
        "wsl-kani",
        "wsl-kvm",
        "wsl-sandbox-image",
        "wsl-searxng",
        "wsl-nix",
    ];
    let wsl = match Wsl::locate() {
        Ok(wsl) => wsl,
        Err(_) => {
            for id in ids {
                probes.insert(
                    id.into(),
                    observation(
                        if cfg!(windows) {
                            "MISSING"
                        } else {
                            "NOT_APPLICABLE"
                        },
                        "WSL executable unavailable on this host",
                    ),
                );
            }
            return;
        }
    };
    let distro = match DistroName::new("Ubuntu-24.04") {
        Ok(distro) => distro,
        Err(_) => return,
    };
    probes.insert("wsl-distro".into(),match wsl.doctor().await{Ok(report)=>json!({"status":if report.ubuntu_24_04_installed&&report.distributions.iter().any(|distro|distro.name.as_str()=="Ubuntu-24.04"&&distro.version==Some(2)){"PASS"}else{"FAIL"},"report":report}),Err(_)=>observation("FAIL","WSL doctor failed")});
    let checks: Vec<(&str, &[&str], Option<&str>)> = vec![
        (
            "wsl-docker",
            &["docker", "info", "--format", "{{.ServerVersion}}"],
            None,
        ),
        ("wsl-openshell", &["openshell", "status"], Some("Connected")),
        ("wsl-openshell", &["openshell", "--version"], Some("0.1.4")),
        (
            "wsl-openshell",
            &["systemctl", "--user", "is-active", "openshell-gateway"],
            Some("active"),
        ),
        (
            "wsl-rust",
            &["/home/dev/.cargo/bin/rustc", "--version"],
            Some("1.99.0"),
        ),
        (
            "wsl-rust",
            &["/home/dev/.cargo/bin/rustfmt", "--version"],
            None,
        ),
        (
            "wsl-rust",
            &["/home/dev/.cargo/bin/cargo", "clippy", "--version"],
            None,
        ),
        (
            "wsl-rust",
            &[
                "/home/dev/.cargo/bin/rustup",
                "component",
                "list",
                "--installed",
                "--toolchain",
                "nightly",
            ],
            Some("miri"),
        ),
        (
            "wsl-rust",
            &[
                "/home/dev/.cargo/bin/rustup",
                "component",
                "list",
                "--installed",
                "--toolchain",
                "nightly",
            ],
            Some("rust-src"),
        ),
        (
            "wsl-rust",
            &["/home/dev/.cargo/bin/cargo", "binstall", "-V"],
            None,
        ),
        (
            "wsl-rust",
            &["/home/dev/.cargo/bin/cargo", "nextest", "--version"],
            None,
        ),
        (
            "wsl-rust",
            &["/home/dev/.cargo/bin/cargo", "fuzz", "--version"],
            None,
        ),
        (
            "wsl-kani",
            &["/home/dev/.cargo/bin/cargo", "kani", "--version"],
            None,
        ),
        ("wsl-kvm", &["/usr/bin/test", "-r", "/dev/kvm"], None),
        (
            "wsl-sandbox-image",
            &[
                "docker",
                "image",
                "inspect",
                "polaris-sandbox:p2",
                "--format",
                "{{.Id}}",
            ],
            Some("sha256:"),
        ),
        (
            "wsl-searxng",
            &[
                "docker",
                "inspect",
                "searxng",
                "--format",
                "{{.State.Running}}",
            ],
            Some("true"),
        ),
        (
            "wsl-nix",
            &["/nix/var/nix/profiles/default/bin/nix", "--version"],
            None,
        ),
    ];
    let mut grouped = BTreeMap::<String, Vec<Value>>::new();
    for (id, argv, contains) in checks {
        let mut safe_argv = vec![
            "/usr/bin/env",
            "RUSTUP_AUTO_INSTALL=0",
            "CARGO_NET_OFFLINE=true",
        ];
        safe_argv.extend_from_slice(argv);
        let value = match wsl
            .exec(&distro, &safe_argv, None, Duration::from_secs(15))
            .await
        {
            Ok(output) => {
                let stdout = polaris_tools_rtk::decode_stream(&output.stdout);
                let matches = match contains {
                    Some(value) => stdout.contains(value),
                    None => true,
                };
                json!({"status":if output.exit_code==0&&matches{"PASS"}else{"FAIL"},"argv":argv,"exit_code":output.exit_code,"stdout":stdout,"stderr":polaris_tools_rtk::decode_stream(&output.stderr)})
            }
            Err(error) => observation("FAIL", &format!("WSL argv probe failed: {error}")),
        };
        grouped.entry(id.to_owned()).or_default().push(value);
    }
    for (id, checks) in grouped {
        probes.insert(id, group(checks));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setup_item_has_a_probe() -> Result<(), CliError> {
        let report = assemble(BTreeMap::new())?;
        let inventory: toml::Value =
            toml::from_str(INVENTORY).map_err(|_| CliError::Unavailable("inventory"))?;
        let entries = inventory
            .get("member_setup")
            .and_then(toml::Value::as_array)
            .ok_or(CliError::Unavailable("items"))?;
        let items = report
            .get("items")
            .and_then(serde_json::Value::as_array)
            .ok_or(CliError::Unavailable("report"))?;
        assert_eq!(
            items.len(),
            entries.len(),
            "inventory cannot silently shrink"
        );
        for (item, source) in items.iter().zip(entries) {
            assert_eq!(
                item.get("id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(CliError::Unavailable("report id missing"))?,
                source
                    .get("id")
                    .and_then(toml::Value::as_str)
                    .ok_or(CliError::Unavailable("inventory id missing"))?,
                "every authoritative id retained"
            );
            assert_eq!(
                item.get("observation")
                    .and_then(|value| value.get("status"))
                    .and_then(serde_json::Value::as_str),
                Some("UNKNOWN"),
                "absent probe is not success"
            );
            assert!(
                item.get("onboarding")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|text| !text.is_empty()),
                "concrete action retained"
            );
        }
        assert_eq!(
            report.get("ready").and_then(serde_json::Value::as_bool),
            Some(false),
            "unknown required item prevents readiness"
        );
        assert!(
            !is_ready(&report),
            "unknown required item blocks CLI readiness"
        );
        Ok(())
    }

    #[test]
    fn optional_missing_item_does_not_block_readiness() -> Result<(), CliError> {
        let inventory: toml::Value =
            toml::from_str(INVENTORY).map_err(|_| CliError::Unavailable("inventory"))?;
        let entries = inventory
            .get("member_setup")
            .and_then(toml::Value::as_array)
            .ok_or(CliError::Unavailable("items"))?;
        let mut probes = BTreeMap::new();
        for entry in entries {
            let id = entry
                .get("id")
                .and_then(toml::Value::as_str)
                .ok_or(CliError::Unavailable("setup item has no id"))?;
            probes.insert(
                id.to_owned(),
                observation("PASS", "synthetic contract fixture"),
            );
        }
        probes.insert(
            "win-keyring".to_owned(),
            observation("MISSING", "optional credentials are not configured"),
        );

        let report = assemble(probes)?;

        assert_eq!(
            report.get("ready").and_then(serde_json::Value::as_bool),
            Some(true),
            "optional absence does not block setup"
        );
        assert_eq!(
            report
                .get("required_missing")
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );
        assert!(is_ready(&report));
        Ok(())
    }

    #[test]
    fn credential_metadata_probe_never_renders_secret_values() {
        let secret = std::ffi::OsStr::new("sensitive-value-do-not-render-4950");
        let present = environment_presence(Some(secret));
        assert_eq!(
            present.get("status").and_then(serde_json::Value::as_str),
            Some("PASS"),
            "nonempty is present"
        );
        assert!(
            !present.to_string().contains("4950"),
            "secret payload must never render"
        );
        assert_eq!(
            environment_presence(None)
                .get("status")
                .and_then(serde_json::Value::as_str),
            Some("MISSING"),
            "absent is missing"
        );
        assert_eq!(
            environment_presence(Some(std::ffi::OsStr::new("")))
                .get("status")
                .and_then(serde_json::Value::as_str),
            Some("MISSING"),
            "empty is missing"
        );
    }
}
