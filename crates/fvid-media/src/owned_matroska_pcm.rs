//! Owned Matroska PCM presentation timeline to caller-owned float32 PCM.
pub use crate::owned_matroska_audio::{AudioDecodeStats, Error};
use fvid_control::CopyOptions;
use std::{
    io::{Read, Seek, Write},
    time::Duration,
};
/// Decode delay, signed padding, gaps and ceil-rounded interval boundaries.
/// Errors may leave partial caller-owned PCM; progress never reports publication.
/// Retained allocation admission is checked before decoding; metadata edits are unsupported.
pub fn decode_matroska_pcm<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> std::result::Result<AudioDecodeStats, Error> {
    crate::owned_matroska_audio::decode_matroska_audio_pcm(source, output, interval, options, "PCM")
}

/// Preserve signed 32-bit integer and IEEE float64 samples as normalized f64.
/// The same presentation clock, packet controls and refusal policies apply.
pub fn decode_matroska_pcm_f64<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> std::result::Result<AudioDecodeStats, Error> {
    precise::decode(source, output, interval, options)
}
mod precise {
    use super::*;
    use crate::owned_matroska_audio::{DecodeProgress, invalid, matroska_audio_index};
    use crate::owned_webm::WebmReader as MatroskaTimelineReader;
    type Result<T> = std::result::Result<T, Error>;
    struct MatroskaTimelineDecoder(crate::owned_pcm_decoder::PcmDecoder);
    impl MatroskaTimelineDecoder {
        const SAMPLE_BYTES: usize = 8;
        fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self> {
            Ok(Self(crate::owned_pcm_decoder::PcmDecoder::from_matroska(
                track,
            )?))
        }
        fn sample_rate(&self) -> u32 {
            self.0.sample_rate()
        }
        fn channels(&self) -> u16 {
            self.0.channels()
        }
        fn delayed(&self) -> bool {
            false
        }
        fn finish(&mut self) -> Result<Option<Vec<f64>>> {
            Ok(None)
        }
        fn decode(&mut self, data: &[u8]) -> Result<Option<Vec<f64>>> {
            Ok(Some(self.0.decode_pcm_f64(data)?))
        }
    }
    pub(super) fn decode<R: Read + Seek>(
        source: R,
        output: &mut impl Write,
        interval: Option<(Duration, Duration)>,
        options: &CopyOptions,
    ) -> Result<AudioDecodeStats> {
        if !options.metadata_set.is_empty()
            || !options.metadata_delete.is_empty()
            || !options.stream_metadata_set.is_empty()
            || !options.stream_metadata_delete.is_empty()
        {
            return Err(invalid("raw PCM stream cannot apply metadata mutations"));
        }
        let selected = match options.streams.as_slice() {
            [] => None,
            [i] => Some(*i),
            _ => return Err(invalid("select exactly one audio stream")),
        };
        let mut control = DecodeProgress {
            options,
            event: fvid_control::ProgressEvent {
                packets: 0,
                payload_bytes: 0,
                done: false,
            },
        };
        control.check()?;
        if let Some(hook) = &options.progress {
            hook.emit(control.event);
        }
        control.check()?;
        let mut reader = crate::owned_matroska_audio::open_audio_reader(
            source,
            options,
            options.max_packet_bytes,
        )?;
        let index = matroska_audio_index(&reader, selected)?;
        crate::owned_matroska_audio::admit_audio_reader(&mut reader, index, options)?;
        decode_matroska_audio_reader_controlled(reader, output, interval, selected, &mut control)
    }
    include!("owned_matroska_audio_timeline_impl.rs");
}
