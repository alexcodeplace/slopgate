use std::fmt;

/// Typed engine error. Core never panics on bad input nor calls process::exit —
/// it returns this; only slopgate-rs maps it to an exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlopError {
    Io(String),
    Parse(String),
    Tool(String),
}

impl fmt::Display for SlopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlopError::Io(m) => write!(f, "io: {m}"),
            SlopError::Parse(m) => write!(f, "parse: {m}"),
            SlopError::Tool(m) => write!(f, "tool: {m}"),
        }
    }
}
impl std::error::Error for SlopError {}
