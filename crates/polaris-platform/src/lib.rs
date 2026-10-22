//! # polaris-platform
//!
//! Facade over native process/PTY/filesystem adapters and WSL2 host management.
//!
//! ## Operating-system behavior
//!
//! | Capability | Windows host | Linux / WSL2 | macOS |
//! |---|---|---|---|
//! | Process tree | `process-wrap` Job Object with kill-on-drop (VERIFIED by Windows lifecycle integration tests). | Process group with kill-on-drop (VERIFIED by Linux lifecycle run recorded in P1 evidence). | Process group is the planned adapter route (UNVERIFIED). |
//! | Graceful stop | Child stdin closure followed by Job Object force kill after the grace interval; CTRL_BREAK is not used (VERIFIED by implementation). | SIGTERM, grace interval, then group kill (VERIFIED by implementation; host runtime matrix is separate). | Same intended Unix path (UNVERIFIED). |
//! | PTY | ConPTY via `portable-pty` (implementation present; live smoke test is environment-dependent). | `openpty` via `portable-pty` (implementation present; live smoke test is environment-dependent). | `openpty` via `portable-pty` (UNVERIFIED). |
//! | Filesystem watch | Native Windows notifications; callback overflow reports `RescanRequired` (VERIFIED by implementation). | inotify backend; overflow reports `RescanRequired` (VERIFIED by implementation). Windows edits under `/mnt/c` may not be observed. | FSEvents via `notify` (UNVERIFIED). |
//! | WSL distro management | `wsl.exe` output decoding, distro listing, argv execution, config checks, and keepalive (VERIFIED by implementation). | Not applicable. | Not applicable. |
//!
//! A child cannot depend on this facade. `polaris-kernel` owns composition between the two
//! adapters, and `polaris-platform-process` is the implementation point for SDK `PlatformOps`.
//!
//! ```
//! use polaris_platform::{process, wsl};
//!
//! let _platform = process::ProcessPlatform::new();
//! let _install_plan = wsl::Wsl::provision_ubuntu_plan();
//! ```

pub use polaris_platform_process as process;
pub use polaris_platform_wsl as wsl;
