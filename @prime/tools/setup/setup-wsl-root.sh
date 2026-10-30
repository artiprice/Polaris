#!/usr/bin/env bash
# =============================================================================
# Polaris — OpenShell setup for a WSL2 Ubuntu distro, run as root so no sudo password is needed:
#
#   wsl -d Ubuntu -u root -- bash /mnt/d/Polaris/@prime/tools/setup/setup-wsl-root.sh <user>
#
# Same result as `setup-wsl.sh --openshell-only` for <user> (the distro's default user):
#   1. apt over IPv4 when IPv6 is unreachable, base packages
#   2. Docker Engine (Docker's apt repo) + <user> in the `docker` group
#   3. OpenShell CLI + gateway (pinned v0.1.4, checksum-verified by its installer) for <user>
#   4. linger for <user> and the docker compute-driver pin in its gateway.env
# Idempotent: every step checks before acting.
# =============================================================================
set -euo pipefail

TARGET_USER="${1:?usage: setup-wsl-root.sh <user>}"
OPENSHELL_VERSION="${OPENSHELL_VERSION:-v0.1.4}"
export DEBIAN_FRONTEND=noninteractive
log() { printf '\n== %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

[ "$(id -u)" -eq 0 ] || { echo "run as root (wsl -u root)"; exit 1; }
id "$TARGET_USER" >/dev/null 2>&1 || { echo "no such user: $TARGET_USER"; exit 1; }
TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
TARGET_UID="$(id -u "$TARGET_USER")"
[ "$(ps -p 1 -o comm=)" = "systemd" ] || { echo "systemd is not PID 1: add [boot] systemd=true to /etc/wsl.conf and wsl --shutdown"; exit 1; }

log "1/4 base packages"
if ! curl -6 -s -o /dev/null -m 5 http://archive.ubuntu.com/ 2>/dev/null; then
  echo 'Acquire::ForceIPv4 "true";' > /etc/apt/apt.conf.d/99polaris-force-ipv4
  echo "apt: IPv6 unreachable, forcing IPv4"
fi
apt-get update -qq
apt-get install -y -qq ca-certificates curl gnupg build-essential pkg-config libssl-dev git jq unzip bc >/dev/null

log "2/4 Docker Engine"
if ! have docker || ! docker version --format '{{.Server.Version}}' >/dev/null 2>&1; then
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
  # shellcheck disable=SC1091
  codename="$(. /etc/os-release && echo "${UBUNTU_CODENAME:-$VERSION_CODENAME}")"
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu ${codename} stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update -qq
  apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin >/dev/null
  systemctl enable --now docker >/dev/null 2>&1
fi
id -nG "$TARGET_USER" | grep -qw docker || usermod -aG docker "$TARGET_USER"
echo "docker: $(docker version --format '{{.Server.Version}}')"

log "3/4 OpenShell ${OPENSHELL_VERSION} for ${TARGET_USER}"
loginctl enable-linger "$TARGET_USER"
# The user's systemd instance must be up before the installer enables the gateway user service.
systemctl start "user@${TARGET_UID}.service" || true
if ! have openshell || ! openshell --version 2>/dev/null | grep -q "${OPENSHELL_VERSION#v}"; then
  tmp="$(mktemp -d)"
  curl -fsSL "https://raw.githubusercontent.com/NVIDIA/OpenShell/${OPENSHELL_VERSION}/install.sh" -o "$tmp/install.sh"
  SUDO_USER="$TARGET_USER" OPENSHELL_VERSION="$OPENSHELL_VERSION" sh "$tmp/install.sh"
  rm -rf "$tmp"
fi
echo "openshell: $(openshell --version)"

log "4/4 compute-driver pin"
as_user() {
  sudo -u "$TARGET_USER" env HOME="$TARGET_HOME" XDG_RUNTIME_DIR="/run/user/${TARGET_UID}" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${TARGET_UID}/bus" "$@"
}
genv="${TARGET_HOME}/.config/openshell/gateway.env"
if ! grep -qx 'OPENSHELL_COMPUTE_DRIVER=docker' "$genv" 2>/dev/null; then
  as_user mkdir -p "$(dirname "$genv")"
  sed -i '/^OPENSHELL_COMPUTE_DRIVER=/d' "$genv" 2>/dev/null || true
  echo 'OPENSHELL_COMPUTE_DRIVER=docker' | as_user tee -a "$genv" >/dev/null
fi
# The gateway runs as the user, who joined `docker` above: restart it so it gets the group.
as_user systemctl --user restart openshell-gateway || true
sleep 3
echo "gateway: $(as_user systemctl --user is-active openshell-gateway 2>&1)"
log "done"
