#!/usr/bin/env sh
# State and recent log of the user's OpenShell gateway service.
systemctl --user is-active openshell-gateway
systemctl --user status openshell-gateway --no-pager 2>&1 | head -12
journalctl --user -u openshell-gateway --no-pager -n 25 2>&1 | tail -25
id
