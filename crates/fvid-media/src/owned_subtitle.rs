//! Owned SRT/ASS to Matroska subtitle conversion and atomic publication.
use crate::owned_matroska::{Encoding, PacketWriter, TrackSpec};
use crate::owned_webm as webm;
use fvid_media_info as media_info;
#[derive(Debug)]
pub struct Error(String);
impl std::fmt::Display for Error { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) } }
impl std::error::Error for Error {}
impl From<std::io::Error> for Error { fn from(e: std::io::Error) -> Self { Self(e.to_string()) } }
impl From<crate::owned_matroska::Error> for Error { fn from(e: crate::owned_matroska::Error) -> Self { Self(e.to_string()) } }
impl From<crate::owned_ebml::Error> for Error { fn from(e: crate::owned_ebml::Error) -> Self { Self(e.to_string()) } }
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error { Error(message.into()) }
fn is_matroska_source(source: &std::path::Path) -> Result<bool> {
    use std::io::Read;
    let mut input = std::fs::File::open(source)?;
    let mut signature = [0; 4];
    match input.read_exact(&mut signature) {
        Ok(()) => Ok(signature == [0x1a, 0x45, 0xdf, 0xa3]),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}
include!("owned_subtitle_impl.rs");
