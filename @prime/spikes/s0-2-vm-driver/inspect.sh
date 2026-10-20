#!/usr/bin/env bash
# S0-2 step 1: what does the openshell 0.1.4 .deb ship for the vm driver, and how is the driver configured?
set -uo pipefail
echo "== kvm"; ls -l /dev/kvm; id -nG
echo "== package files (driver/vm/krun)"; dpkg -L openshell | grep -Ei 'driver|vm|krun|libexec' | head -30
echo "== gateway.toml"; ls -la ~/.config/openshell/; cat ~/.config/openshell/gateway.toml 2>/dev/null || echo "(no gateway.toml: driver auto-detected)"
echo "== gateway.env"; cat ~/.config/openshell/gateway.env 2>/dev/null || echo "(none)"
echo "== service unit"; systemctl --user cat openshell-gateway 2>/dev/null | sed -n 1,40p
echo "== gateway config help"; openshell-gateway --help 2>&1 | sed -n 1,60p
echo "== recent gateway log (driver lines)"; journalctl --user -u openshell-gateway --no-pager -n 200 2>/dev/null | grep -Ei 'driver|compute|vm|docker|podman' | tail -15
