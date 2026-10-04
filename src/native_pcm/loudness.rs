//! File adapters for the owned fvid-media PCM loudness meter.
pub use fvid_media::owned_loudness::{IntegratedLoudness, LoudnessMeter};

/// Analyze a selected owned audio stream without creating a decoded temporary file.
/// Channel weights must match the stream layout and are never guessed.
pub fn measure_file(
    source: &std::path::Path,
    selected: Option<usize>,
    weights: &[f64],
    cancel: Option<&crate::media_control::CancelFlag>,
) -> crate::Result<IntegratedLoudness> {
    measure_file_controlled(source, selected, weights, cancel, None)
}

/// Stream packet/byte progress and honor cancellation before a final done event.
pub fn measure_file_controlled(
    source: &std::path::Path,
    selected: Option<usize>,
    weights: &[f64],
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> crate::Result<IntegratedLoudness> {
    use crate::{invalid, native_media};
    use std::io::{BufReader, Read, Seek, SeekFrom, Write};
    let mut control = native_media::DecodeProgress::new(cancel, progress)?;
    let mut input = BufReader::new(std::fs::File::open(source)?);
    let mut prefix = [0u8; 8];
    input.read_exact(&mut prefix)?;
    input.seek(SeekFrom::Start(0))?;
    enum Input {
        Wave(BufReader<std::fs::File>, super::WaveInfo),
        Mp4(crate::container::mp4::Mp4Reader<BufReader<std::fs::File>>),
        Mka(crate::container::webm::WebmReader<BufReader<std::fs::File>>),
        Adts(crate::container::adts::StreamReader<BufReader<std::fs::File>>),
    }
    let (input, rate, channels) = if &prefix[..4] == b"RIFF" {
        if selected.is_some_and(|s| s != 0) {
            return Err(invalid("WAVE has only stream 0"));
        }
        let info = super::inspect(&mut input, cancel)?;
        info.validate_decode()?;
        let (rate, channels) = (info.sample_rate, info.channels);
        (Input::Wave(input, info), rate, channels)
    } else if crate::container::mp4::recognizes_prefix(&prefix) {
        let reader = crate::container::mp4::Mp4Reader::open(input, Default::default())?;
        let index = native_media::mp4_audio_index(&reader, selected)?;
        let decoder = crate::native_audio_decoder::PacketPcmDecoder::new(&reader.tracks()[index])?;
        let (rate, channels) = (decoder.sample_rate(), decoder.channels());
        (Input::Mp4(reader), rate, channels)
    } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let reader = crate::container::webm::WebmReader::open(input, Default::default())?;
        let index = native_media::matroska_audio_index(&reader, selected)?;
        let decoder =
            crate::native_audio_decoder::PacketPcmDecoder::from_matroska(&reader.tracks[index])?;
        let (rate, channels) = (decoder.sample_rate(), decoder.channels());
        (Input::Mka(reader), rate, channels)
    } else {
        if selected.is_some_and(|s| s != 0) {
            return Err(invalid("ADTS has only stream 0"));
        }
        let reader = crate::container::adts::StreamReader::open(input)?;
        let config = reader.configuration();
        let (rate, channels) = (config.sample_rate, config.channels);
        (Input::Adts(reader), rate, channels)
    };
    if weights.len() != usize::from(channels) {
        return Err(invalid("channel weights must match selected audio stream"));
    }
    struct Sink {
        meter: LoudnessMeter,
        bytes: [u8; 4],
        byte_count: usize,
        frame: [f64; 64],
        channel: usize,
        channels: usize,
    }
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            for byte in bytes {
                self.bytes[self.byte_count] = *byte;
                self.byte_count += 1;
                if self.byte_count == 4 {
                    self.byte_count = 0;
                    self.frame[self.channel] = f64::from(f32::from_le_bytes(self.bytes));
                    self.channel += 1;
                    if self.channel == self.channels {
                        self.meter
                            .push(&self.frame[..self.channels])
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                        self.channel = 0;
                    }
                }
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink {
        meter: LoudnessMeter::new(rate, weights).map_err(|e| invalid(&e))?,
        bytes: [0; 4],
        byte_count: 0,
        frame: [0.0; 64],
        channel: 0,
        channels: weights.len(),
    };
    let stats = match input {
        Input::Wave(reader, info) => {
            super::decode_reader(reader, info, &mut sink, None, &mut control, None)?
        }
        Input::Mp4(reader) => native_media::decode_mp4_audio_reader_controlled(
            reader,
            &mut sink,
            None,
            selected,
            &mut control,
        )?,
        Input::Mka(reader) => native_media::decode_matroska_audio_reader_controlled(
            reader,
            &mut sink,
            None,
            selected,
            &mut control,
        )?,
        Input::Adts(reader) => {
            native_media::decode_adts_aac_reader_controlled(reader, &mut sink, None, &mut control)?
        }
    };
    control.check()?;
    let report = sink.meter.report();
    if sink.byte_count != 0 || sink.channel != 0 || stats.sample_frames != report.sample_frames {
        return Err(invalid("incomplete or inconsistent loudness PCM stream"));
    }
    control.emit(false);
    control.check()?;
    control.emit(true);
    Ok(report)
}


/// Unit energy weights for conventional mono/stereo input. Multichannel callers
/// must supply explicit layout weights to `measure_loudness_file`.
pub fn default_weights(
    source: &std::path::Path,
    selected: Option<usize>,
) -> crate::Result<Vec<f64>> {
    let channels = if super::is_wave(source)? {
        if selected.is_some_and(|index| index != 0) {
            return Err(crate::invalid("WAVE has only stream 0"));
        }
        super::inspect(&mut std::fs::File::open(source)?, None)?.channels
    } else {
        crate::native_media::audio_source_info_selected(source, selected)?.channels
    };
    match channels {
        1 | 2 => Ok(vec![1.0; usize::from(channels)]),
        _ => Err(crate::invalid(
            "multichannel loudness requires explicit channel weights in stream order",
        )),
    }
}
