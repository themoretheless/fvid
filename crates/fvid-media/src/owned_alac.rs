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

impl AlacDecoder {
    /// Validate a container track and initialize owned packet reconstruction.
    /// The caller retains responsibility for presentation delay/discard padding.
    pub fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self> {
        if track.kind != 2 || track.codec != "A_ALAC" {
            return Err(invalid("selected Matroska track is not ALAC audio"));
        }
        let rate = u32::try_from(track.sample_rate)
            .map_err(|_| invalid("ALAC sample rate exceeds decoder geometry"))?;
        let channels = u16::try_from(track.channels)
            .map_err(|_| invalid("ALAC channel count exceeds decoder geometry"))?;
        if track.codec_private.len() < 24
            || u32::from_be_bytes(track.codec_private[20..24].try_into().unwrap()) != rate {
            return Err(invalid("ALAC cookie and track sample rates disagree"));
        }
        Self::new(&track.codec_private, rate, channels)
    }
}
