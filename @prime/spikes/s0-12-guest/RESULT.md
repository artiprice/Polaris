# S0-12 — Guest build in WSL2, sandbox image, `protocol_version` handshake, browser screenshot

**RESULT: PASS** for guest build, image packaging, handshake and **headless** Chromium screenshots;
**Xvfb (headed) fails under the default OpenShell policy** → the browser loop uses headless Chromium over CDP.
Raw output: `RESULT.raw.txt` (run 2), `policy-probe.raw.txt`, `browser-debug.raw.txt`, `browser-debug2.raw.txt` ·
runs 2026-10-20 · labels: VERIFIED / LIKELY / UNVERIFIED

| | |
|---|---|
| Question | `polaris-guest` build in WSL2, packaging into the sandbox image, `protocol_version` handshake, Xvfb + Chromium screenshot |
| Kill criterion | image build or handshake fails |
| Fallback | exec-only guest without browser — **not needed** (headless browser works) |

## Measured (VERIFIED)

| Step | Result |
|---|---|
| guest build in WSL2 (`cargo build --release`, std only) | ELF x86-64 PIE, 342,008 B, 2.9 s cold / 0.14 s warm |
| image `polaris/s0-12:dev` (base `nvcr.io/nvidia/base/ubuntu:24.04` + guest + Xvfb/x11-apps + Chrome for Testing 155.0.8059.84 + deps) | 1.24 GB; build 155 s cold, 58 s with cached apt layer |
| sandbox from the local image (`openshell sandbox create --from polaris/s0-12:dev`) | Ready in 5.0 s (run 1) / 14.2 s (run 2, under load) |
| handshake `polaris-guest handshake 1` | `{"protocol_version":1,"guest_version":"0.0.1","os":"linux","arch":"x86_64",…}`, exit 0 |
| handshake with host version 2 | `protocol mismatch: host 2, guest 1`, **exit 3** (host refuses) |
| sandbox identity | user `ubuntu`, `HOME=/sandbox`, cwd `/sandbox`; `NoNewPrivs: 1`, `Seccomp: 2` (filter) |
| headless Chromium screenshot (`--headless=new --no-sandbox --no-zygote --disable-gpu --disable-dev-shm-usage`) | **OK**: `/sandbox/out/test.html` → 800×600 PNG (13,383 B) rendered correctly (inspected); `data:` URL also OK |
| headed Chromium under Xvfb | **FAIL**: Xvfb aborts — `XKB: Failed to compile keymap … Failed to activate virtual core keyboard` (as non-root; `xkbcomp` 1.4.6 itself runs). Not diagnosed further (time-boxed) |

## Findings — default OpenShell sandbox policy (VERIFIED by probes)

| Path / action | Result |
|---|---|
| exec from `/opt` | **denied** (exit 126) |
| read from `/opt` | **denied** (`cat` fails; Chrome `net::ERR_ACCESS_DENIED` for `file:///opt/...`) |
| exec + read under `/usr` (`/usr/local/bin`, `/usr/lib`, `/usr/share`) | allowed |
| write `/tmp`, `/sandbox` | allowed |
| `openshell sandbox download` | only paths inside the workspace `/sandbox` ("outside the sandbox workspace") |
| Chromium zygote (user-namespace setup) | **denied**: `FATAL:sandbox/linux/services/credentials.cc … Operation not permitted` even with `--no-sandbox`; fixed by `--no-zygote` |
| `--single-process` | crashes (exit 133) — do not use |

## Consequences

1. **P2-X-1 image rules:** install everything under `/usr` (never `/opt`); artifacts the host fetches go under
   `/sandbox`; Chromium flags `--headless=new --no-sandbox --no-zygote --disable-dev-shm-usage`. Chromium's own
   sandbox is replaced by the OpenShell sandbox (Landlock + seccomp) — record in the threat model.
2. **P7 browser loop:** headless Chromium driven over CDP (`chromiumoxide`) for screenshots, a11y tree and input;
   Xvfb only if a later need appears (needs a custom policy or a fix for the keymap compile; UNVERIFIED cause).
3. **Image size 1.24 GB** (Chrome + GTK deps) — per-PC build (member-setup `wsl-sandbox-image`) unless the team
   agrees on a registry. Chrome for Testing is a Google build; whether it may be redistributed in a published
   image is a **team decision** (open-source Chromium builds are the alternative) — local builds only for now.
4. Default driver `docker`; the vm driver (S0-2) was not used here (UNVERIFIED for this image).

Conditions: WSL2 Ubuntu 24.04.5, Docker Engine 29.9.1, OpenShell 0.1.4 (docker driver, default policy), Rust 1.99.0 (WSL2).
