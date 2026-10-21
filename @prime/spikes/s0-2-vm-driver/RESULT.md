# S0-2 — `/dev/kvm` in WSL2 + OpenShell `vm` driver (libkrun)

**RESULT: PASS** · raw output: `RESULT.raw.txt` (scripts `inspect.sh`, `run.sh`) · run 2026-10-20T14:07 ·
labels: VERIFIED / LIKELY / UNVERIFIED

| | |
|---|---|
| Question | Is `/dev/kvm` available in WSL2 Ubuntu on the Ryzen 7 2700, and does the OpenShell `vm` driver boot a sandbox? |
| Kill criterion | no `/dev/kvm`, or the vm driver fails |
| Fallback | E4 remote gateway on Nebius AI Cloud — **not needed for E2** |

## Measured (VERIFIED, `RESULT.raw.txt`)

| Step | Result |
|---|---|
| `/dev/kvm` | present; user `dev` in group `kvm`; no `.wslconfig` change was needed |
| driver binary | shipped in the 0.1.4 package: `/usr/libexec/openshell/openshell-driver-vm` |
| switch driver | `OPENSHELL_COMPUTE_DRIVER=vm` → gateway log `Using compute driver driver=vm` |
| first create → Ready | **27.5 s** (image resolved from the local container engine, rootfs ext4 prepared, VM launched) |
| guest | kernel `Linux 6.12.76`, Ubuntu 24.04.5, user `ubuntu`, cwd `/sandbox`; `landlock_abi=n/a` inside the microVM |
| exec | 216 ms; exit-code propagation OK (7) |
| upload | byte-exact (sha256 `3287856b…78e7a1`) |
| delete | OK |
| restore | driver set back to `docker` (the default for daily work) |

## Findings

1. **E2 microVM tier works locally** (VERIFIED) — no remote VM needed for isolation experiments.
2. **Create is ~14× slower than docker** (27.5 s vs ~2 s, VERIFIED; first run, cold image) — the vm tier is for
   untrusted/browser work (P7), not per-command use. Warm-create time was not measured (UNVERIFIED).
3. **The gateway serves one driver at a time** (VERIFIED: switching needs `gateway.env` + restart). Running E1 and E2
   side by side needs either a second gateway instance or a restart — to be designed in P7 (LIKELY: second instance
   on another port).
4. A transient `concurrent modification detected during sandbox reconciliation` WARN appeared on create; the sandbox
   still reached Ready (VERIFIED). Watch for it in stress tests.
5. **Per PC:** `/dev/kvm` depends on CPU virtualization being enabled in firmware (member-setup id `wsl-kvm`).

Conditions: WSL 2.6.3, kernel 6.6.87.2, OpenShell 0.1.4, Docker Engine 29.9.1, Ryzen 7 2700, 16 GB RAM.
