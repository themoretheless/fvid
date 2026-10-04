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
pub fn invalid(s: &str) -> Error {
    Error::Invalid(s.into())
}
/// The envelope was read and understood, and what it names has no arm here. A reader
/// says this rather than `invalid` so a caller can tell a coding it has to implement
/// apart from a file it failed to read: the first is a missing decoder arm, the second
/// a missing or broken demuxer.
pub fn unsupported(s: &str) -> Error {
    Error::Unsupported(s.into())
}

impl From<BitstreamError> for Error {
    fn from(error: BitstreamError) -> Self {
        Self::Invalid(error.0)
    }
}

/// String-bearing codec errors shared without a dependency on a decoder crate.
#[derive(Debug)]
pub struct BitstreamError(pub String);
impl std::fmt::Display for BitstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for BitstreamError {}
impl From<std::io::Error> for BitstreamError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}
/// Checked allocation shared by owned bitstream and frontend domains.
pub fn buffer(size: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    data.try_reserve_exact(size)
        .map_err(|_| invalid("frame allocation failed"))?;
    data.resize(size, 0);
    Ok(data)
}
