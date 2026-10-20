//! Owned Matroska MPEG Layers I/II/III presentation decode.
pub use crate::owned_matroska_audio::{AudioDecodeStats, Error};
use fvid_control::CopyOptions;
use std::{
    io::{Read, Seek, Write},
    time::Duration,
};
pub fn decode_matroska_mpeg_pcm<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> std::result::Result<AudioDecodeStats, Error> {
    crate::owned_matroska_audio::decode_matroska_audio_pcm(
        source, output, interval, options, "MPEG",
    )
}
