//! # polaris-guest
//!
//! Linux in-sandbox agent baked into the Polaris sandbox image (layer L7, phase P2/P7). The
//! binary answers the host's `--protocol-version` handshake and serves the workspace sync
//! endpoint (`sync-init`, `sync-batch`, `sync-snapshot`, `sync-cas`) that the OpenShell
//! execution profile uses to mirror a reviewed frame into the sandbox. It builds inside
//! WSL2/Linux only (excluded from Windows default members). Structured exec, screenshots and
//! accessibility trees are P7.
