#!/usr/bin/env bash
# Own only this gateway, its private state, and a distinct Docker sandbox namespace.
set -euo pipefail
umask 077

mode="${1:-start}"
validate_state() {
  state_dir="${2:?usage: isolated-fault-gateway.sh stop STATE_DIRECTORY}"
  state_dir="$(realpath -- "$state_dir")"
  state_base="$(realpath -- "$HOME/.local/state/polaris")"
  case "$state_dir" in "$state_base"/p2-gateway.*) ;; *) printf 'unexpected state directory\n' >&2; exit 2 ;; esac
  [[ -d "$state_dir" && -O "$state_dir" && "$(stat -c %a "$state_dir")" == 700 ]]
  read -r owned_pid owned_start < "$state_dir/owner.tsv"
  [[ "$owned_pid" =~ ^[0-9]+$ && "$owned_start" =~ ^[0-9]+$ ]]
  same_owner() {
    [[ -r "/proc/$owned_pid/stat" ]] || return 1
    [[ "$(awk '{print $22}' "/proc/$owned_pid/stat")" == "$owned_start" ]] || return 1
    [[ "$(readlink "/proc/$owned_pid/exe")" == /usr/bin/openshell-gateway ]] || return 1
    tr '\0' '\n' < "/proc/$owned_pid/cmdline" | grep -Fxq -- "$state_dir/gateway.toml"
  }
}
launch_gateway() {
  [[ -z "$(ss -H -ltn "sport = :$port")" ]] || { printf 'selected fault gateway port is occupied\n' >&2; exit 2; }
  start_ticks="$(awk '{print $22}' "/proc/$$/stat")"
  printf '%s %s\n' "$$" "$start_ticks" > "$state_dir/owner.tsv"
  printf 'FAULT_GATEWAY state=%s endpoint=https://localhost:%s namespace=%s pid=%s\n' "$state_dir" "$port" "$namespace" "$$"
  exec /usr/bin/env -i HOME="$HOME" PATH=/usr/bin:/bin \
    XDG_CONFIG_HOME="$state_dir/config" XDG_STATE_HOME="$state_dir/state" \
    OPENSHELL_LOCAL_TLS_DIR="$state_dir/tls" \
    /usr/bin/openshell-gateway --config "$state_dir/gateway.toml" \
    --bind-address 127.0.0.1 --port "$port" --name p2-fault --compute-driver docker \
    --db-url "sqlite:$state_dir/gateway.db" --log-level warn \
    >> "$state_dir/gateway.log" 2>&1
}
if [[ "$mode" == stop || "$mode" == crash || "$mode" == resume ]]; then
  validate_state "$@"
  if [[ "$mode" == resume ]]; then
    if same_owner; then printf 'owned gateway is already running\n' >&2; exit 2; fi
    read -r port namespace < "$state_dir/gateway.tsv"
    [[ "$port" =~ ^[0-9]+$ && "$port" -ge 1024 && "$port" -le 65535 ]]
    [[ "$namespace" == "p2-fault-$(basename "$state_dir" | cut -d. -f2)" ]]
    [[ -f "$state_dir/gateway.db" && -f "$state_dir/gateway.toml" && -f "$state_dir/tls/ca.crt" ]]
    launch_gateway
  fi
  if [[ "$mode" == crash ]]; then
    same_owner || { printf 'owned gateway is not running\n' >&2; exit 2; }
    kill -KILL "$owned_pid"
    for _ in {1..100}; do
      if ! same_owner; then printf 'owned gateway killed; private state retained\n'; exit 0; fi
      sleep 0.1
    done
    printf 'owned gateway did not disappear\n' >&2
    exit 2
  fi
  if same_owner; then
    kill -TERM "$owned_pid"
    for _ in {1..100}; do
      if ! same_owner; then printf 'owned gateway stopped\n'; exit 0; fi
      sleep 0.1
    done
    if same_owner; then kill -KILL "$owned_pid"; fi
  fi
  printf 'owned gateway is absent; state retained for recovery\n'
  exit 0
fi
[[ "$mode" == start ]] || { printf 'usage: isolated-fault-gateway.sh start [PORT] | stop|crash|resume STATE_DIRECTORY\n' >&2; exit 2; }
port="${2:-17771}"
[[ "$port" =~ ^[0-9]+$ && "$port" -ge 1024 && "$port" -le 65535 ]]
[[ -x /usr/bin/openshell-gateway ]]
if [[ -n "$(ss -H -ltn "sport = :$port")" ]]; then
  printf 'selected fault gateway port is already occupied\n' >&2
  exit 2
fi
state_base="$HOME/.local/state/polaris"
mkdir -p -- "$state_base"
[[ -O "$state_base" ]]
state_base="$(realpath -- "$state_base")"
state_dir="$(mktemp -d "$state_base/p2-gateway.XXXXXXXXXX")"
trap 'printf "fault gateway setup failed; private diagnostics retained at %s\n" "$state_dir" >&2; if [[ -f "$state_dir/preflight.log" ]]; then tail -n 12 "$state_dir/preflight.log" >&2; fi' ERR
chmod 700 "$state_dir"
namespace="p2-fault-$(basename "$state_dir" | cut -d. -f2)"
printf '%s %s\n' "$port" "$namespace" > "$state_dir/gateway.tsv"
mkdir -m 700 "$state_dir/config" "$state_dir/state" "$state_dir/api-client"

# No --dry-run: PEMs are written directly into the private directory and never printed.
/usr/bin/env -i HOME="$HOME" PATH=/usr/bin:/bin \
  /usr/bin/openshell-gateway generate-certs --output-dir "$state_dir/tls" \
  --server-san localhost --server-san 127.0.0.1 > "$state_dir/certgen.log" 2>&1
cp "$state_dir/tls/ca.crt" "$state_dir/api-client/ca.crt"
cp "$state_dir/tls/client/tls.crt" "$state_dir/api-client/tls.crt"
cp "$state_dir/tls/client/tls.key" "$state_dir/api-client/tls.key"
chmod 600 "$state_dir/api-client/"*
cat > "$state_dir/gateway.toml" <<EOF
[openshell]
version = 2
[openshell.gateway]
guest_tls_ca = "$state_dir/tls/ca.crt"
[openshell.drivers.docker]
allow_driver_config = true
enable_bind_mounts = false
sandbox_label = "$namespace"
grpc_endpoint = "https://127.0.0.1:$port"
EOF
/usr/bin/env -i HOME="$HOME" PATH=/usr/bin:/bin \
  XDG_CONFIG_HOME="$state_dir/config" XDG_STATE_HOME="$state_dir/state" \
  OPENSHELL_LOCAL_TLS_DIR="$state_dir/tls" \
  /usr/bin/openshell-gateway config preflight --path "$state_dir/gateway.toml" \
  > "$state_dir/preflight.log" 2>&1
# These are nonsecret locators. Private TLS and JWT payloads stay in the private directory.
launch_gateway
