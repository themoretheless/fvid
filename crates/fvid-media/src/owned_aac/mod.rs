//! Owned AAC synthesis and transforms. No foreign decoder or libav dependency.
#![forbid(unsafe_code)]
pub mod aac_imdct;
pub mod aac_coupling;
pub mod aac_synthesis;
pub mod aac_tns;
#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(&self.0)}
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T,Error>;
fn invalid(message:&str)->Error {Error(message.into())}
mod aac_band_tables;
pub mod aac_bands;

pub mod bits;
pub mod aac_pce;
pub mod config;
pub mod adts;
fn unsupported(message: &str) -> Error { Error(message.into()) }
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self { Self(error.to_string()) }
}
