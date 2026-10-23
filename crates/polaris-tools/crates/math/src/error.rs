//! Errors returned by bounded math operations.

/// A parse, validation, range, or arithmetic failure.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MathError {
    /// The expression or table could not be parsed at this byte offset.
    #[error("invalid input at byte {at}")]
    Parse { at: usize },
    /// Division or remainder by zero.
    #[error("division by zero")]
    DivideByZero,
    /// A checked operation exceeded the supported numeric range.
    #[error("numeric overflow or underflow")]
    Overflow,
    /// A floating-point input or result was NaN or infinite.
    #[error("non-finite number")]
    NonFinite,
    /// Input exceeded a configured length or count bound.
    #[error("input exceeds its configured bound")]
    TooLong,
    /// Expression nesting exceeded the configured depth bound.
    #[error("expression nesting exceeds its configured bound")]
    TooDeep,
    /// The expression used a power exponent beyond its configured bound.
    #[error("power exponent exceeds its configured bound")]
    ExponentLimit,
    /// A statistical input set was empty.
    #[error("statistics input is empty")]
    EmptyInput,
    /// Invalid knot, grid, or interpolation data.
    #[error("invalid interpolation data: {0}")]
    BadKnots(&'static str),
    /// The requested compute capability is absent from the supplied table.
    #[error("unknown compute capability {major}.{minor}")]
    UnknownCapability { major: u8, minor: u8 },
    /// A launch configuration cannot execute on the selected device.
    #[error("invalid launch configuration: {0}")]
    InvalidLaunch(&'static str),
    /// Elapsed time or another duration input is not positive.
    #[error("duration must be positive")]
    InvalidDuration,
    /// The GPU table was missing required provenance.
    #[error("GPU specification row has invalid provenance: {0}")]
    MissingSource(String),
    /// A GPU table did not meet the supported schema version.
    #[error("unsupported GPU specification table version {0}")]
    UnsupportedTableVersion(u32),
}
