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
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    options.cancel = cancel.cloned();
    options.progress = progress.cloned();
    measure_file_with_options(source, weights, &options)
}

/// Measure loudness while enforcing packet, memory, and process RSS budgets.
pub fn measure_file_with_options(
    source: &std::path::Path,
    weights: &[f64],
    options: &fvid_media::CopyOptions,
) -> crate::Result<IntegratedLoudness> {
    use crate::{invalid, native_media};
    use std::io::{BufReader, Read, Seek, SeekFrom, Write};
    if options.streams.len() > 1
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(invalid(
            "owned loudness requires at most one audio stream and no metadata edits",
        ));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("loudness requires at least one packet"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let selected = options.streams.first().copied();
    let cancel = options.cancel.as_ref();
    let progress = options.progress.as_ref();
    let mut control = native_media::DecodeProgress::new_with_limits(
        cancel,
        progress,
        options.max_rss_bytes,
        options.max_packets,
    )?;
    let mut input = BufReader::new(std::fs::File::open(source)?);
    let mut prefix = [0u8; 8];
    input.read_exact(&mut prefix)?;
    input.seek(SeekFrom::Start(0))?;
    enum Input<'a> {
        Wave(BufReader<std::fs::File>, super::WaveInfo),
        Mp4(Box<crate::container::mp4::Mp4Reader<BufReader<std::fs::File>>>),
        Mka(Box<crate::container::webm::WebmReader<BufReader<std::fs::File>>>),
        Adts(Box<crate::native_media::NegotiatedAdts<BufReader<std::fs::File>>>),
        _Marker(std::marker::PhantomData<&'a ()>),
    }
    let (input, rate, channels) = if super::is_wave(source)? {
        if selected.is_some_and(|s| s != 0) {
            return Err(invalid("WAVE has only stream 0"));
        }
        let info = super::inspect(&mut input, cancel)?;
        info.validate_decode()?;
        let frame = usize::from(info.block);
        let capacity = 65536usize.min(options.max_packet_bytes) / frame * frame;
        if capacity == 0 {
            return Err(invalid("packet byte limit is smaller than one PCM sample frame"));
        }
        if let Some(max) = options.max_controlled_bytes {
            let size = (u128::from(info.data_bytes)).min(
                options
                    .max_packets
                    .map(|n| u128::from(n) * capacity as u128)
                    .unwrap_or(u128::from(info.data_bytes)),
            ) as usize;
            let frames = size / frame;
            let rate_sz = info.sample_rate as usize;
            let hop = (rate_sz / 10).max(1);
            let integrated_blocks = if frames < rate_sz * 2 / 5 {
                0
            } else {
                1 + (frames - rate_sz * 2 / 5) / hop
            };
            let short_blocks = if frames < rate_sz * 3 {
                0
            } else {
                1 + (frames - rate_sz * 3) / hop
            };
            let estimated = (rate_sz * 3 + rate_sz * 2 / 5) as u128 * 8
                + capacity as u128
                + (capacity / frame) as u128 * u128::from(info.channels) * 8
                + u128::from(info.channels) * 512
                + (integrated_blocks + short_blocks) as u128 * 512
                + 16384;
            if estimated > max as u128 {
                return Err(invalid(&format!(
                    "controlled memory budget exceeded: need {estimated} bytes"
                )));
            }
        }
        let (rate, channels) = (info.sample_rate, info.channels);
        (Input::Wave(input, info), rate, channels)
    } else if fvid_media::owned_container_loudness::recognizes_standalone_audio(source)
        .map_err(|e| invalid(&e))?
        || ((crate::container::mp4::recognizes_prefix(&prefix)
            || prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]))
            && (native_media::audio_source_info_selected(source, selected).is_err()
                || options.max_controlled_bytes.is_some()))
    {
        let (rate, channels) =
            fvid_media::owned_audio_plan::source_geometry(source, options)
                .map_err(|e| invalid(&e))?;
        if weights.len() != usize::from(channels) {
            return Err(invalid("channel weights must match selected audio stream"));
        }
        LoudnessMeter::new(rate, weights).map_err(|e| invalid(&e))?;
        let spool = fvid_media::owned_container_loudness::decode_to_wave_unqualified(
            source, options,
        )
        .map_err(|e| invalid(&e))?;
        let mut pcm_options = fvid_media::CopyOptions::default();
        pcm_options.cancel = options.cancel.clone();
        pcm_options.max_rss_bytes = options.max_rss_bytes;
        let report = measure_file_with_options(&spool.wave, weights, &pcm_options)?;
        control.check()?;
        if let Some(hook) = progress {
            hook.emit(crate::media_control::ProgressEvent {
                done: true,
                ..spool.progress
            });
        }
        return Ok(report);
    } else if crate::container::mp4::recognizes_prefix(&prefix) {
        let mut limits = crate::container::mp4::Limits::default();
        limits.packet_bytes = limits.packet_bytes.min(options.max_packet_bytes);
        let mut reader = crate::container::mp4::Mp4Reader::open(input, limits)?;
        let index = native_media::mp4_audio_index(&reader, selected)?;
        let in_band_ps = native_media::negotiate_mp4_ps(&mut reader, index, || control.check())?;
        let track = &reader.tracks()[index];
        let decoder = if in_band_ps {
            crate::native_audio_decoder::PacketPcmDecoder::with_in_band_ps(track)?
        } else {
            crate::native_audio_decoder::PacketPcmDecoder::new(track)?
        };
        let (rate, channels) = (decoder.sample_rate(), decoder.channels());
        (Input::Mp4(Box::new(reader)), rate, channels)
    } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let limits = crate::container::webm::Limits::default();
        let mut reader = crate::container::webm::WebmReader::open(input, limits)?;
        reader.restrict_packet_bytes(options.max_packet_bytes);
        let index = native_media::matroska_audio_index(&reader, selected)?;
        let in_band_ps =
            native_media::negotiate_matroska_ps(&mut reader, index, || control.check())?;
        let track = &reader.tracks[index];
        let decoder = if in_band_ps {
            crate::native_audio_decoder::PacketPcmDecoder::Ps(Box::new(
                crate::codec::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(
                    &track.codec_private,
                    u32::try_from(track.sample_rate)
                        .map_err(|_| invalid("AAC output clock overflow"))?,
                )
                .map_err(|e| invalid(&e.0))?,
            ))
        } else {
            crate::native_audio_decoder::PacketPcmDecoder::from_matroska(track)?
        };
        let (rate, channels) = (decoder.sample_rate(), decoder.channels());
        (Input::Mka(Box::new(reader)), rate, channels)
    } else {
        if selected.is_some_and(|s| s != 0) {
            return Err(invalid("ADTS has only stream 0"));
        }
        if options.max_controlled_bytes.is_some() {
            let spool =
                fvid_media::owned_container_loudness::decode_adts_to_wave(source, options)
                    .map_err(|e| invalid(&e))?;
            if weights.len() != usize::from(spool.stats.channels) {
                return Err(invalid("channel weights must match selected audio stream"));
            }
            LoudnessMeter::new(spool.stats.sample_rate, weights).map_err(|e| invalid(&e))?;
            let mut pcm_options = fvid_media::CopyOptions::default();
            pcm_options.cancel = options.cancel.clone();
            pcm_options.max_rss_bytes = options.max_rss_bytes;
            let report = measure_file_with_options(&spool.wave, weights, &pcm_options)?;
            control.check()?;
            if let Some(hook) = progress {
                hook.emit(crate::media_control::ProgressEvent {
                    done: true,
                    ..spool.progress
                });
            }
            return Ok(report);
        }
        let reader = crate::container::adts::StreamReader::open_with_packet_limit(
            input,
            options.max_packet_bytes,
        )?;
        let negotiated = native_media::negotiate_adts_aac_reader(reader, None, &mut control)?;
        let (rate, channels) = (negotiated.sample_rate(), negotiated.channels());
        (Input::Adts(Box::new(negotiated)), rate, channels)
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
        Input::Wave(reader, info) => super::decode_reader(
            reader,
            info,
            &mut sink,
            None,
            &mut control,
            Some(options.max_packet_bytes),
        )?,
        Input::Mp4(reader) => native_media::decode_mp4_audio_reader_controlled(
            *reader,
            &mut sink,
            None,
            selected,
            &mut control,
        )?,
        Input::Mka(reader) => native_media::decode_matroska_audio_reader_controlled(
            *reader,
            &mut sink,
            None,
            selected,
            &mut control,
        )?,
        Input::Adts(negotiated) => negotiated.decode(&mut sink, &mut control)?,
        Input::_Marker(_) => unreachable!(),
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
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    default_weights_with_options(source, &options)
}

pub fn default_weights_with_options(
    source: &std::path::Path,
    options: &fvid_media::CopyOptions,
) -> crate::Result<Vec<f64>> {
    if options.streams.len() > 1
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(crate::invalid(
            "owned loudness requires at most one audio stream and no metadata edits",
        ));
    }
    if options.max_packet_bytes == 0 {
        return Err(crate::invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(crate::invalid("loudness requires at least one packet"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| crate::invalid(&e))?;
    let selected = options.streams.first().copied();
    let channels = if super::is_wave(source)? {
        if selected.is_some_and(|index| index != 0) {
            return Err(crate::invalid("WAVE has only stream 0"));
        }
        let info = super::inspect(
            &mut std::fs::File::open(source)?,
            options.cancel.as_ref(),
        )?;
        let frame = usize::from(info.frame_bytes());
        if frame == 0 || (65536usize.min(options.max_packet_bytes) / frame * frame) == 0 {
            return Err(crate::invalid(
                "packet byte limit is smaller than one PCM sample frame",
            ));
        }
        info.channels
    } else if options.max_controlled_bytes.is_none()
        && let Ok(info) = crate::native_media::audio_source_info_selected(source, selected)
    {
        if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes {
            use std::io::{BufReader, Read, Seek, SeekFrom};
            let mut input = BufReader::new(std::fs::File::open(source)?);
            let mut prefix = [0u8; 8];
            input.read_exact(&mut prefix)?;
            input.seek(SeekFrom::Start(0))?;
            if crate::container::mp4::recognizes_prefix(&prefix) {
                let mut limits = crate::container::mp4::Limits::default();
                limits.packet_bytes = limits.packet_bytes.min(options.max_packet_bytes);
                let reader = crate::container::mp4::Mp4Reader::open(input, limits)?;
                if let Some(track) = reader.tracks().get(info.stream_index) {
                    let exceeds = match &track.samples {
                        crate::container::mp4::SampleIndex::Expanded(samples) => samples
                            .iter()
                            .any(|s| (s.size as usize) > options.max_packet_bytes),
                        crate::container::mp4::SampleIndex::Uniform(uniform) => {
                            uniform.count > 0
                                && (uniform.bytes_per_frame as usize) > options.max_packet_bytes
                        }
                    };
                    if exceeds {
                        return Err(crate::invalid("MP4 packet exceeds budget"));
                    }
                }
            } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
                let mut reader = crate::container::webm::WebmReader::open(
                    input,
                    crate::container::webm::Limits::default(),
                )?;
                reader.scan_all()?;
                let Some(track) = reader.tracks.get(info.stream_index) else {
                    return Err(crate::invalid("selected audio stream is absent"));
                };
                let track_number = track.number;
                let mut packets = 0u64;
                for (packet_index, packet) in reader.packets.iter().enumerate() {
                    if packet.track == track_number {
                        if options.max_packets.is_some_and(|limit| packets >= limit) {
                            break;
                        }
                        packets += 1;
                        if reader.packet_payload_size(packet_index)? > options.max_packet_bytes {
                            return Err(crate::invalid("WebM packet exceeds budget"));
                        }
                    }
                }
            } else if crate::container::adts::header(&prefix).is_some() {
                let mut reader = crate::container::adts::StreamReader::open_with_packet_limit(
                    input,
                    options.max_packet_bytes,
                )?;
                let mut packets = 0u64;
                while options.max_packets.is_none_or(|limit| packets < limit) {
                    let Some(_) = reader.next_packet()? else {
                        break;
                    };
                    packets += 1;
                }
            }
        }
        info.channels
    } else {
        let mut geom_options = options.clone();
        geom_options.metadata_set.clear();
        geom_options.metadata_delete.clear();
        geom_options.stream_metadata_set.clear();
        geom_options.stream_metadata_delete.clear();
        fvid_media::owned_audio_plan::source_geometry(source, &geom_options)
            .map_err(|e| crate::invalid(&e))?
            .1
    };
    match channels {
        1 | 2 => Ok(vec![1.0; usize::from(channels)]),
        _ => Err(crate::invalid(
            "multichannel loudness requires explicit channel weights in stream order",
        )),
    }
}
