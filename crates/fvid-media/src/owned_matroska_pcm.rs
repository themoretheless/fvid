//! Owned Matroska PCM presentation timeline to caller-owned float32/float64 PCM.
use crate::owned_audio_control::CopyProgress;
pub use crate::owned_matroska_audio::{AudioDecodeStats, Error};
use crate::owned_matroska_audio::{
    MatroskaSampleDecoder, decode_matroska_audio_reader_generic, invalid, matroska_audio_index,
};
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

pub(crate) fn decode_matroska_pcm_f64_reader<R: Read + Seek>(
    reader: crate::owned_webm::WebmReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> std::result::Result<AudioDecodeStats, Error> {
    precise::decode_reader(reader, output, interval, options)
}

mod precise {
    use super::*;
    use crate::owned_audio_control::AudioDecodeControl;
    use crate::owned_webm::WebmReader as MatroskaTimelineReader;
    type Result<T> = std::result::Result<T, Error>;

    struct PreciseMatroskaPcmDecoder(crate::owned_pcm_decoder::PcmDecoder);

    impl MatroskaSampleDecoder for PreciseMatroskaPcmDecoder {
        type Sample = f64;
        const SAMPLE_BYTES: usize = 8;

        fn push_le_bytes(sample: f64, out: &mut Vec<u8>) {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        fn with_in_band_ps(_: &crate::owned_webm::Track) -> Result<Self> {
            Err(invalid("PCM reader cannot select AAC PS"))
        }
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
        let mut control = CopyProgress::new(options);
        control.check()?;
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
        if !matches!(
            reader.tracks[index].codec.as_str(),
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE"
        ) {
            return Err(invalid(
                "selected Matroska audio stream has a different codec",
            ));
        }
        crate::owned_matroska_audio::admit_audio_reader_with_interval(
            &mut reader,
            index,
            interval,
            options,
        )?;
        decode_matroska_audio_reader_generic::<R, CopyProgress<'_>, PreciseMatroskaPcmDecoder>(
            reader,
            output,
            interval,
            selected,
            &mut control,
        )
    }

    pub(super) fn decode_reader<R: Read + Seek>(
        reader: MatroskaTimelineReader<R>,
        output: &mut impl Write,
        interval: Option<(Duration, Duration)>,
        options: &CopyOptions,
    ) -> Result<AudioDecodeStats> {
        let mut control = CopyProgress::new(options);
        control.check()?;
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
        if let Some(hook) = &options.progress {
            hook.emit(control.event);
        }
        control.check()?;
        decode_matroska_audio_reader_generic::<R, CopyProgress<'_>, PreciseMatroskaPcmDecoder>(
            reader,
            output,
            interval,
            selected,
            &mut control,
        )
    }
}
