//! Owned Matroska Opus presentation timeline to caller-owned float32 PCM.
pub use crate::owned_matroska_audio::{AudioDecodeStats, Error};
use fvid_control::CopyOptions;
use std::{
    io::{Read, Seek, Write},
    time::Duration,
};
/// Decode delay, signed padding, gaps and ceil-rounded interval boundaries.
/// Errors may leave partial caller-owned PCM; progress never reports publication.
/// Controlled allocation admission covers decoder scratch, packet/index payload
/// and cloned track metadata. Metadata mutations remain unsupported.
pub fn decode_matroska_opus_pcm<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> std::result::Result<AudioDecodeStats, Error> {
    crate::owned_matroska_audio::decode_matroska_audio_pcm(
        source, output, interval, options, "A_OPUS",
    )
}
