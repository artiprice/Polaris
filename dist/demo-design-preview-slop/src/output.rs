//! Color-depth policy and session-local Windows VT output mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorDepth {
    #[default]
    Auto,
    TrueColor,
    Ansi,
}
impl std::str::FromStr for ColorDepth {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "truecolor" => Ok(Self::TrueColor),
            "ansi" => Ok(Self::Ansi),
            _ => Err("--color-depth requires auto, truecolor, or ansi"),
        }
    }
}
impl ColorDepth {
    /// Explicit depth overrides automatic hints, including deterministic dumps.
    pub fn resolve_rgb(self, rgb_hint: bool, windows_vt: bool, dump: bool) -> bool {
        match self {
            Self::Auto => rgb_hint || windows_vt || dump,
            Self::TrueColor => true,
            Self::Ansi => false,
        }
    }
}

/// Own the actual stdout console mode for the duration of one terminal session.
/// Failure to enable VT is a capability fallback, not a startup failure.
pub struct ConsoleOutputMode {
    #[cfg(windows)]
    mode: Option<WindowsOutputMode>,
}
impl ConsoleOutputMode {
    pub fn enter() -> Self {
        Self {
            #[cfg(windows)]
            mode: WindowsOutputMode::enter().ok(),
        }
    }
    pub fn vt_available(&self) -> bool {
        #[cfg(windows)]
        {
            self.mode.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}
#[cfg(windows)]
struct WindowsOutputMode {
    console: std::fs::File,
    original: u32,
}
#[cfg(windows)]
impl WindowsOutputMode {
    fn enter() -> std::io::Result<Self> {
        use std::os::windows::io::AsHandle;
        Self::for_file(std::fs::File::from(
            std::io::stdout().as_handle().try_clone_to_owned()?,
        ))
    }
    fn for_file(console: std::fs::File) -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::{
            consoleapi::{GetConsoleMode, SetConsoleMode},
            wincon::{ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING},
        };
        let mut original = 0;
        // SAFETY: console owns this live handle; original is writable. Only
        // output-processing bits are added, preserving all inherited flags.
        unsafe {
            if GetConsoleMode(console.as_raw_handle().cast(), &mut original) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if SetConsoleMode(
                console.as_raw_handle().cast(),
                original | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(Self { console, original })
    }
}
#[cfg(windows)]
impl Drop for WindowsOutputMode {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: the owned duplicate remains live, including on setup failure
        // and unwinding. Restore the exact original mode after terminal teardown.
        unsafe {
            winapi::um::consoleapi::SetConsoleMode(
                self.console.as_raw_handle().cast(),
                self.original,
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modern_windows_vt_does_not_require_terminal_environment_hints() {
        assert!(!ColorDepth::Auto.resolve_rgb(false, false, false));
        assert!(ColorDepth::Auto.resolve_rgb(false, true, false));
        assert!(ColorDepth::Auto.resolve_rgb(true, false, false));
        assert!(ColorDepth::Auto.resolve_rgb(false, false, true));
    }
    #[test]
    fn explicit_depth_takes_precedence_over_every_automatic_capability() {
        for hint in [false, true] {
            for vt in [false, true] {
                for dump in [false, true] {
                    assert!(ColorDepth::TrueColor.resolve_rgb(hint, vt, dump));
                    assert!(!ColorDepth::Ansi.resolve_rgb(hint, vt, dump));
                }
            }
        }
    }
    #[test]
    fn depth_values_are_validated_without_changing_existing_color_switches() {
        for (s, depth) in [
            ("auto", ColorDepth::Auto),
            ("truecolor", ColorDepth::TrueColor),
            ("ansi", ColorDepth::Ansi),
        ] {
            assert_eq!(s.parse::<ColorDepth>(), Ok(depth));
        }
        for bad in ["", "always", "never", "rgb", "256", "TRUECOLOR"] {
            assert!(bad.parse::<ColorDepth>().is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn non_console_output_handle_is_rejected_without_modifying_it() {
        let exe = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        assert!(WindowsOutputMode::for_file(exe).is_err());
    }
}
