#!/usr/bin/env bash
# Prints the OpenShell gateway listener and its server-certificate SANs (used by Polaris on Windows to pick a TLS server name).
set -uo pipefail
for _ in $(seq 1 30); do openshell status 2>/dev/null | grep -q 'Status: Connected' && break; sleep 1; done
echo "gateway service: $(systemctl --user is-active openshell-gateway)"
ss -ltn | awk 'NR==1 || /:17670/'
echo | openssl s_client -connect 127.0.0.1:17670 2>/dev/null | openssl x509 -noout -subject -issuer -ext subjectAltName 2>/dev/null
