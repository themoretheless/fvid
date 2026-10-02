//! Owned Apple Lossless packet reconstruction; no foreign codec backend.
#![forbid(unsafe_code)]
#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) } }
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error { Error(message.into()) }
include!("owned_alac_impl.rs");

impl AlacDecoder {
    pub fn sample_rate(&self) -> u32 { self.sample_rate }
    pub fn channels(&self) -> u16 { self.channels as u16 }
}
