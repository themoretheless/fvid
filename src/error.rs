//! Shared error contract and constructors used across library domains.
use std::io;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Invalid(String),
    Unsupported(String),
    Gpu(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Invalid(s) => write!(f, "{s}"),
            Self::Unsupported(s) => write!(f, "{s}"),
            Self::Gpu(s) => write!(f, "GPU: {s}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn invalid(s: &str) -> Error {
    Error::Invalid(s.into())
}
/// The envelope was read and understood, and what it names has no arm here. A reader
/// says this rather than `invalid` so a caller can tell a coding it has to implement
/// apart from a file it failed to read: the first is a missing decoder arm, the second
/// a missing or broken demuxer.
pub(crate) fn unsupported(s: &str) -> Error {
    Error::Unsupported(s.into())
}
