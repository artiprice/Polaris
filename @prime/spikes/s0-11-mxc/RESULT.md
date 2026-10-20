# Spike S0-11: OpenShell `mxc` driver on Windows

RESULT: FAIL -> WSL2 tiers only

Short answer: The `mxc` driver is not usable on this machine today. The blocker is not the Windows build: this host meets the MXC build floors. The blockers are (1) OpenShell v0.1.4 publishes no Windows gateway binary, (2) the OpenShell docs say Windows MXC is "Coming soon", and (3) the driver's default backend needs `wxc-exec.exe`, which is not installed and whose install path needs an elevated host-prep step that this non-admin session cannot run.

Labels: VERIFIED = observed in this session. LIKELY = inferred from docs/code, not run. UNVERIFIED = not checked.

## Host facts (VERIFIED)
- `Get-CimInstance Win32_OperatingSystem`: Caption `Microsoft Windows 11 Enterprise LTSC`, Version `10.0.26100`, BuildNumber `26100`.
- Registry (`HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion`): `UBR=9550`, `DisplayVersion=24H2`, `EditionID=EnterpriseS`, `ProductName=Windows 10 Enterprise LTSC 2024`.
- Admin: `False` (WindowsPrincipal check).
- `Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All` (and VirtualMachinePlatform, Containers, Microsoft-Windows-Subsystem-Linux, HypervisorPlatform): `ERROR: The requested operation requires elevation.` Feature state could not be read.
- `Get-Command wxc-exec`: not found. `C:\mxc\wxc-exec.exe`: not present. A recursive search of `C:\Program Files`, `C:\Program Files (x86)` and `C:\Windows\System32` for `wxc*` returned nothing.

## What MXC is (VERIFIED, repo `microsoft/mxc`)
- Repo `https://github.com/microsoft/mxc`, MIT license, not archived, last push 2026-10-20. Description: "Policy-driven, layered isolation and containment".
- README: "MXC is a sandboxed code execution system for running untrusted code ... on Windows, Linux, and macOS." Windows default backend is `processcontainer` (AppContainer). Other backends include `isolation_session`, `wslc`, `windows_sandbox`.
- "Non-SDK consumption: Platform-specific executor binaries, such as `wxc-exec.exe`, accept JSON container-creation requests." So `wxc-exec.exe` is MXC's native executor.
- Obtain paths:
  - Release `v1.0.1` (2026-10-17, not prerelease) asset `mxc-release-binaries.zip` (542,051,775 bytes). Release notes say it "contains signed binaries that can be useful for testing." Download URL: `https://github.com/microsoft/mxc/releases/download/v1.0.1/mxc-release-binaries.zip`. Not downloaded or executed in this spike.
  - SDKs: crates.io `mxc-sdk`, npm `@microsoft/mxc-sdk`, NuGet `Microsoft.Mxc.Sdk`. The Node and .NET packages include native runtime assets.
- Windows OS-version table (`docs/backends/process-container/os-version-support.md`, VERIFIED):
  - 24H2 (build 26100): Process Isolation `26100.9278`; Session Isolation `26100.9550`.
  - 25H2: `26200.9278` / `26200.9550`. 26H2: `26300.9550` for both. 26H1: `28000.2804` / `28000.3086`.
  - No edition restriction is listed. The table cites no LTSC rule either way (edition support UNVERIFIED).
- Host preparation (`docs/backends/process-container/host-prep.md`, VERIFIED): `wxc-host-prep.exe` (shipped with `wxc-exec.exe`) has `requireAdministrator` in release builds. Its subcommands `prepare-system-drive` and `prepare-null-device` are needed for the AppContainer + DACL tier. `wxc-exec.exe` itself never elevates. This blocks non-admin use of that tier.

## OpenShell `openshell-driver-mxc` at v0.1.4 (VERIFIED, `crates/openshell-driver-mxc/README.md`)
- Prerequisites for live runs: "Windows 11 Insider build ≥ 26300.8553"; `IsoSessionApp.dll` present and registered; `wxc-exec.exe` built with `--features isolation_session`.
- Default backend `process_container` needs `wxc-exec.exe` (config key `wxc_exec_path`). The `isolation_session` backend is opt-in.
- Limitations (capability matrix): network middleware rejected; interactive exec/connect/forward not supported; no bundled agent image (relies on a Windows host install); restart durability absent (in-memory registry); concurrent sandboxes limited on `isolation_session` v1 (single session).
- Off-box smoke test: `OPENSHELL_MXC_MOCK_WXC=1` uses an in-process mock shim, with no `wxc-exec` needed. This tests wiring only, not enforcement.

## OpenShell Windows gateway (VERIFIED)
- `gh api repos/NVIDIA/OpenShell/releases/tags/v0.1.4 --jq '.assets[].name'`: no Windows asset. Assets are Linux (`gnu`, `musl`, rpm, deb, snap), macOS (`aarch64-apple-darwin`), a Python wheel, and `openshell.rb`. There is no `.exe`, `.zip` or `windows` name.
- `docs/about/support-matrix.mdx` (v0.1.4): "Windows (WSL 2 + Docker Desktop) | x86_64 | Experimental." Standalone gateway binaries are published for "Linux x86_64 (amd64)", "Linux aarch64 (arm64)" and "macOS Apple Silicon" only.
- `docs/how-it-works/sandboxes/runtimes.mdx`: "| Windows MXC | `mxc` | Coming soon. |" This contradicts the driver README, which describes a working driver.
- `.github/workflows/windows-msvc.yml` has a "Build binaries" job on `windows-2025` and `windows-11-arm` running `mise run --skip-tools windows:build:<arch>` with `continue-on-error: true`. Its output is not published as a release asset (VERIFIED). The `windows:build:*` task definition was not found in `mise.toml` (UNVERIFIED where it is defined).

## Verdict reasoning
| Gate | Required | This host | Result |
|---|---|---|---|
| Windows build (process isolation) | 26100.9278+ | 26100.9550 | Meets (VERIFIED) |
| Windows build (session isolation) | 26100.9550+ | 26100.9550 | Meets (VERIFIED) |
| `isolation_session` backend | Insider 26300.8553+ and `IsoSessionApp.dll` | GA build, not Insider | Fails (documented; `IsoSessionApp.dll` not checked) |
| `wxc-exec.exe` present | Needed for `process_container` | Not present | Fails now; obtainable from release zip (UNVERIFIED that it runs) |
| Admin for AppContainer+DACL prep | `wxc-host-prep.exe` elevated | Not admin | Blocked until elevated (VERIFIED) |
| OpenShell Windows gateway | Needed to host the driver | None published for v0.1.4 | Fails (VERIFIED) |
| Docs status | Supported | "Coming soon" | Fails (VERIFIED) |

Any single failing gate blocks the driver. The gateway gate is decisive: even with `wxc-exec.exe` and admin rights, no Windows `openshell-gateway` binary exists to run the driver.

## Viable local path (VERIFIED from docs)
OpenShell's Windows support is WSL2 + Docker Desktop (Experimental). The `mxc` driver is not available. Local execution on this machine should use WSL2 tiers E1/E2, consistent with the kill criterion.

## What would be needed to enable MXC later
1. Run `wxc-host-prep prepare-system-drive` and `prepare-null-device` from an elevated shell (admin rights required).
2. Download `mxc-release-binaries.zip` (v1.0.1), verify its signature, extract `wxc-exec.exe`, then run `wxc-exec --probe` and the probe script `crates/openshell-driver-mxc/examples/probe-mxc-host.ps1` to get a capability report (UNVERIFIED on this host).
3. Obtain or build a Windows `openshell-gateway` (no v0.1.4 release asset; `windows-msvc.yml` build is unpublished and `continue-on-error`).
4. Confirm with NVIDIA whether `mxc` is supported for Polaris, given the "Coming soon" docs status.
5. For `isolation_session` only: an Insider build of at least 26300.8553 and `IsoSessionApp.dll`. Not applicable to the current GA host.
6. Check with `Get-WindowsOptionalFeature` from an elevated shell (feature state not read in this spike).

## Sources
- https://github.com/microsoft/mxc (README, `docs/backends/process-container/os-version-support.md`, `docs/backends/process-container/host-prep.md`, release v1.0.1)
- https://github.com/NVIDIA/OpenShell/blob/v0.1.4/crates/openshell-driver-mxc/README.md
- https://github.com/NVIDIA/OpenShell/blob/v0.1.4/docs/about/support-matrix.mdx
- https://github.com/NVIDIA/OpenShell/blob/v0.1.4/docs/how-it-works/sandboxes/runtimes.mdx
- https://github.com/NVIDIA/OpenShell/blob/v0.1.4/.github/workflows/windows-msvc.yml
- GitHub release `NVIDIA/OpenShell v0.1.4` asset list (`gh api`)
