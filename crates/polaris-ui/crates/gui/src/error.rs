//! Errors of the GUI crate.

/// Why the welcome window could not run.
#[derive(Debug, thiserror::Error)]
pub enum GuiError {
    /// The Slint platform (windowing system or renderer) failed.
    #[error("windowing platform failed: {0}")]
    Platform(#[from] slint::PlatformError),
    /// The ASCII aurora engine could not be built (font or size problem).
    #[error("aurora background failed: {0}")]
    Aurora(#[from] polaris_ui_ascii_aurora::AuroraError),
    /// A configured model id failed validation.
    #[error("invalid model id `{0}`")]
    Model(String),
    /// The configuration offered no model.
    #[error("no model is configured")]
    NoModels,
}
