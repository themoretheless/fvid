//! Owned PCM packet decoding; sample arithmetic shared with the frontend.
#![forbid(unsafe_code)]
#[derive(Debug)]
pub struct Error(String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_pcm_decoder_impl.rs");
impl PcmDecoder {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    /// Validate Matroska PCM geometry and byte order before packet decoding.
    pub fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self> {
        if track.kind != 2 {
            return Err(invalid("selected stream is not audio"));
        }
        let rate = u32::try_from(track.sample_rate)
            .map_err(|_| invalid("Matroska audio rate overflow"))?;
        let channels = u16::try_from(track.channels)
            .map_err(|_| invalid("Matroska audio channel count overflow"))?;
        if channels == 0 || channels > 64 {
            return Err(invalid("PCM channels must be 1..64"));
        }
        let bits = u8::try_from(track.bit_depth).map_err(|_| invalid("PCM bit depth overflow"))?;
        let format = match track.codec.as_str() {
            "A_PCM/FLOAT/IEEE" => PcmFormat::Float { bits },
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" if bits == 8 => PcmFormat::Unsigned8,
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" => PcmFormat::Int {
                bits,
                big_endian: track.codec == "A_PCM/INT/BIG",
            },
            _ => return Err(invalid("selected Matroska audio stream is not PCM")),
        };
        Self::new(format, rate, channels)
    }
}
