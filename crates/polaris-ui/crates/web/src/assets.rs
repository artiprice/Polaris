//! The embedded web app: one page, one stylesheet, one script and the desktop's fonts.

/// The app shell.
pub const INDEX: &str = include_str!("../assets/index.html");
/// Styles, including the welcome atmosphere (gradients and canvas stars, no images).
pub const STYLE: &str = include_str!("../assets/app.css");
/// The page logic: API calls, the event stream, Markdown and panels.
pub const SCRIPT: &str = include_str!("../assets/app.js");

const INTER_REGULAR: &[u8] = include_bytes!("../../gui/assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../gui/assets/fonts/Inter-Medium.ttf");
const SPACE_GROTESK: &[u8] = include_bytes!("../../gui/assets/fonts/space-grotesk.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../../gui/assets/fonts/jetbrains-mono.ttf");

/// A bundled font by its URL name.
#[must_use]
pub fn font(name: &str) -> Option<&'static [u8]> {
    match name {
        "inter-regular.ttf" => Some(INTER_REGULAR),
        "inter-medium.ttf" => Some(INTER_MEDIUM),
        "space-grotesk.ttf" => Some(SPACE_GROTESK),
        "jetbrains-mono.ttf" => Some(JETBRAINS_MONO),
        _ => None,
    }
}
