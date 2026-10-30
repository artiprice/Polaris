#!/usr/bin/env bash
# =============================================================================
# Polaris — per-member WSL2 setup (run INSIDE the Ubuntu-24.04 distro as the
# default non-root user; needs passwordless or interactive sudo).
#
#   wsl -d Ubuntu-24.04 -- bash /mnt/c/<repo>/@prime/tools/setup/setup-wsl.sh [--with-kani]
#
# Idempotent: every step checks before acting. Installs:
#   1. base build packages (build-essential, pkg-config, libssl-dev, git, jq, unzip, bc)
#   2. Docker Engine (Docker's apt repo) + user in `docker` group      (OpenShell docker driver)
#   3. OpenShell CLI + gateway (pinned v0.1.4, checksum-verified .deb) (P0-ENV-2)
#   4. loginctl linger for the user (gateway user service outlives shells) and the
#      compute-driver pin in ~/.config/openshell/gateway.env (avoids the boot race where
#      driver auto-detection runs before dockerd is up)
#   5. rustup + Rust 1.99.0 (rustfmt, clippy) + nightly (rust-src, miri)
#   6. cargo-binstall, cargo-nextest, cargo-fuzz (nightly use)
#   7. optional: Kani verifier (--with-kani; large download)
# See @prime/PHASES.md "Team member setup" for the Windows-side steps.
# =============================================================================
set -euo pipefail

OPENSHELL_VERSION="${OPENSHELL_VERSION:-v0.1.4}"
RUST_VERSION="1.99.0"
WITH_KANI=0
OPENSHELL_ONLY=0
[ "${1:-}" = "--with-kani" ] && WITH_KANI=1
# --openshell-only: steps 1-4 (packages, Docker Engine, OpenShell, linger); the sandbox image
# builds its own toolchain inside Docker, so Rust in the distro is not needed for it.
[ "${1:-}" = "--openshell-only" ] && OPENSHELL_ONLY=1

log() { printf '\n== %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }
export DEBIAN_FRONTEND=noninteractive

if [ "$(id -u)" -eq 0 ]; then
  echo "run as the default non-root user (not root)"; exit 1
fi

log "1/7 base packages"
# WSL networks without IPv6 routing make apt hang on AAAA mirrors (curl falls back, apt does not).
if ! curl -6 -s -o /dev/null -m 5 http://archive.ubuntu.com/ 2>/dev/null; then
  echo 'Acquire::ForceIPv4 "true";' | sudo tee /etc/apt/apt.conf.d/99polaris-force-ipv4 >/dev/null
  echo "apt: IPv6 unreachable, forcing IPv4"
fi
sudo apt-get update -qq
sudo apt-get install -y -qq ca-certificates curl gnupg build-essential pkg-config libssl-dev git jq unzip bc >/dev/null

log "2/7 Docker Engine"
if ! have docker || ! docker version --format '{{.Server.Version}}' >/dev/null 2>&1; then
  sudo install -m 0755 -d /etc/apt/keyrings
  sudo curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
  sudo chmod a+r /etc/apt/keyrings/docker.asc
  # shellcheck disable=SC1091
  codename="$(. /etc/os-release && echo "${UBUNTU_CODENAME:-$VERSION_CODENAME}")"
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu ${codename} stable" \
    | sudo tee /etc/apt/sources.list.d/docker.list >/dev/null
  sudo apt-get update -qq
  sudo apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin >/dev/null
  sudo systemctl enable --now docker >/dev/null 2>&1
fi
id -nG "$USER" | grep -qw docker || sudo usermod -aG docker "$USER"
echo "docker: $(sudo docker version --format '{{.Server.Version}}')"

log "3/7 OpenShell ${OPENSHELL_VERSION}"
if ! have openshell || ! openshell --version 2>/dev/null | grep -q "${OPENSHELL_VERSION#v}"; then
  tmp="$(mktemp -d)"
  curl -fsSL "https://raw.githubusercontent.com/NVIDIA/OpenShell/${OPENSHELL_VERSION}/install.sh" -o "$tmp/install.sh"
  OPENSHELL_VERSION="$OPENSHELL_VERSION" sh "$tmp/install.sh"
  rm -rf "$tmp"
fi
echo "openshell: $(openshell --version)"

log "4/7 linger (gateway survives logout) + compute-driver pin"
sudo loginctl enable-linger "$USER"
genv="${XDG_CONFIG_HOME:-$HOME/.config}/openshell/gateway.env"
if ! grep -qx 'OPENSHELL_COMPUTE_DRIVER=docker' "$genv" 2>/dev/null; then
  mkdir -p "$(dirname "$genv")"
  sed -i '/^OPENSHELL_COMPUTE_DRIVER=/d' "$genv" 2>/dev/null || true
  echo 'OPENSHELL_COMPUTE_DRIVER=docker' >> "$genv"
  systemctl --user restart openshell-gateway 2>/dev/null || true
fi
echo "gateway: $(systemctl --user is-active openshell-gateway 2>&1); driver pin: $(grep -c '^OPENSHELL_COMPUTE_DRIVER=docker$' "$genv")"

if [ "$OPENSHELL_ONLY" -eq 1 ]; then
  log "done (--openshell-only) — open a NEW wsl session so the docker group applies"
  exit 0
fi

log "5/7 Rust ${RUST_VERSION} + nightly"
if ! have rustup && [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION" -c rustfmt -c clippy
fi
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
rustup toolchain install "$RUST_VERSION" --profile minimal -c rustfmt -c clippy >/dev/null
rustup toolchain install nightly --profile minimal -c rust-src -c miri >/dev/null
rustup default "$RUST_VERSION" >/dev/null
echo "rustc: $(rustc --version); nightly: $(rustc +nightly --version)"

log "6/7 cargo tools"
if ! have cargo-binstall; then
  curl -fsSL "https://github.com/cargo-bins/cargo-binstall/releases/latest/download/cargo-binstall-x86_64-unknown-linux-musl.tgz" | tar -xz -C "$HOME/.cargo/bin" cargo-binstall
fi
# Note: the binstall cargo-fuzz is the musl build and defaults --target to musl; always run
# `cargo fuzz run --target x86_64-unknown-linux-gnu …` (verify-proofs.sh does; PLAN W20).
cargo binstall --no-confirm cargo-nextest cargo-fuzz >/dev/null
echo "nextest: $(cargo nextest --version | head -1); fuzz: $(cargo fuzz --version)"

log "7/7 Kani (optional)"
if [ "$WITH_KANI" -eq 1 ]; then
  have cargo-kani || cargo install --locked kani-verifier
  cargo kani setup
  echo "kani: $(cargo kani --version)"
else
  echo "skipped (pass --with-kani)"
fi

log "done — open a NEW wsl session so the docker group applies"
