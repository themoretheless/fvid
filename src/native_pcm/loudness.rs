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
    Ok(measure_file_with_options_and_kind(source, weights, options)?.0)
}

pub(super) fn measure_file_with_options_and_kind(
    source: &std::path::Path,
    weights: &[f64],
    options: &fvid_media::CopyOptions,
) -> crate::Result<(IntegratedLoudness, bool)> {
    use crate::{invalid, native_media};
    use std::io::{BufReader, Read, Seek, SeekFrom, Write};
    if options
        .cancel
        .as_ref()
        .is_some_and(crate::media_control::CancelFlag::is_cancelled)
    {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("loudness requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
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
    let selected = options.streams.first().copied();
    let cancel = options.cancel.as_ref();
    let progress = options.progress.as_ref();
    let mut control = native_media::DecodeProgress::new_with_limits(
        cancel,
        progress,
        options.max_rss_bytes,
        options.max_packets,
    )?;
    let mut input = Some(BufReader::new(std::fs::File::open(source)?));
    let mut head = [0u8; 12];
    let mut head_len = 0;
    while head_len < head.len() {
        match input.as_mut().unwrap().read(&mut head[head_len..]) {
            Ok(0) => break,
            Ok(n) => head_len += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    if head_len < 8 {
        return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
    }
    let prefix: [u8; 8] = head[..8].try_into().unwrap();
    input.as_mut().unwrap().seek(SeekFrom::Start(0))?;
    enum Input<'a> {
        Wave(BufReader<std::fs::File>, super::WaveInfo),
        Mp4(Box<crate::container::mp4::Mp4Reader<BufReader<std::fs::File>>>),
        Mka(Box<crate::container::webm::WebmReader<BufReader<std::fs::File>>>),
        Adts(Box<crate::native_media::NegotiatedAdts<BufReader<std::fs::File>>>),
        _Marker(std::marker::PhantomData<&'a ()>),
    }
    let (input, rate, channels) = if super::is_wave_reader(input.as_mut().unwrap())? {
        let mut input = input.take().unwrap();
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
    } else if {
        let standalone = fvid_media::owned_container_loudness::recognizes_standalone_from_head(
            source, &head, head_len, options,
        )
        .map_err(|e| invalid(&e))?;
        standalone
            || ((crate::container::mp4::recognizes_prefix(&prefix)
                || prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]))
                && (options.max_controlled_bytes.is_some()
                    || {
                        drop(input.take());
                        native_media::constant_audio_source_info_selected_with_options(
                            source, selected, options,
                        )
                        .is_err()
                    }))
    } {
        drop(input.take());
        let (rate, channels, mask, codec, _, precomputed_info) =
            fvid_media::owned_container_loudness::geometry_with_frames_unqualified(source, options)
                .map_err(|e| invalid(&e))?;
        if weights.len() != usize::from(channels) {
            return Err(invalid("channel weights must match selected audio stream"));
        }
        LoudnessMeter::new(rate, weights).map_err(|e| invalid(&e))?;
        if let Some(max) = options.max_controlled_bytes {
            let frame = usize::from(channels.max(1)) * 4;
            let capacity = 65536 / frame * frame;
            let rate_sz = rate as usize;
            let estimated = (rate_sz * 3 + rate_sz * 2 / 5) as u128 * 8
                + capacity as u128
                + (capacity / frame) as u128 * u128::from(channels) * 8
                + u128::from(channels) * 512
                + 16384;
            if estimated > max as u128 {
                return Err(invalid(&format!(
                    "controlled memory budget exceeded: need {estimated} bytes"
                )));
            }
        }
        let spool =
            fvid_media::owned_container_loudness::decode_to_wave_from_geometry_with_info(
                source,
                rate,
                channels,
                mask,
                &codec,
                precomputed_info,
                options,
            )
            .map_err(|e| invalid(&e))?;
        let mut pcm_options = fvid_media::CopyOptions::default();
        pcm_options.cancel = options.cancel.clone();
        pcm_options.max_controlled_bytes = options.max_controlled_bytes;
        pcm_options.max_rss_bytes = options.max_rss_bytes;
        let report = measure_file_with_options(&spool.wave, weights, &pcm_options)?;
        control.check()?;
        if let Some(hook) = progress {
            hook.emit(crate::media_control::ProgressEvent {
                done: true,
                ..spool.progress
            });
        }
        return Ok((report, false));
    } else if crate::container::mp4::recognizes_prefix(&prefix) {
        let input = match input.take() {
            Some(r) => r,
            None => BufReader::new(std::fs::File::open(source)?),
        };
        let mut reader = crate::container::mp4::Mp4Reader::open(
            input,
            crate::container::mp4::Limits::default(),
        )?;
        reader.restrict_packet_bytes(options.max_packet_bytes);
        let index = native_media::mp4_audio_index(&reader, selected)?;
        let in_band_ps = native_media::negotiate_mp4_ps(&mut reader, index, options.max_packets, || control.check())?;
        let track = &reader.tracks()[index];
        let decoder = if in_band_ps {
            crate::native_audio_decoder::PacketPcmDecoder::with_in_band_ps(track)?
        } else {
            crate::native_audio_decoder::PacketPcmDecoder::new(track)?
        };
        let (rate, channels) = (decoder.sample_rate(), decoder.channels());
        (Input::Mp4(Box::new(reader)), rate, channels)
    } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let input = match input.take() {
            Some(r) => r,
            None => BufReader::new(std::fs::File::open(source)?),
        };
        let limits = crate::container::webm::Limits::default();
        let mut reader = crate::container::webm::WebmReader::open(input, limits)?;
        reader.restrict_packet_bytes(options.max_packet_bytes);
        let index = native_media::matroska_audio_index(&reader, selected)?;
        let in_band_ps =
            native_media::negotiate_matroska_ps(&mut reader, index, options.max_packets, || control.check())?;
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
        if let Some(max) = options.max_controlled_bytes {
            drop(input.take());
            let (rate, channels) =
                fvid_media::owned_audio_plan::source_geometry(source, options)
                    .map_err(|e| invalid(&e))?;
            if weights.len() != usize::from(channels) {
                return Err(invalid("channel weights must match selected audio stream"));
            }
            LoudnessMeter::new(rate, weights).map_err(|e| invalid(&e))?;
            let frame = usize::from(channels.max(1)) * 4;
            let capacity = 65536 / frame * frame;
            let rate_sz = rate as usize;
            let estimated = (rate_sz * 3 + rate_sz * 2 / 5) as u128 * 8
                + capacity as u128
                + (capacity / frame) as u128 * u128::from(channels) * 8
                + u128::from(channels) * 512
                + 16384;
            if estimated > max as u128 {
                return Err(invalid(&format!(
                    "controlled memory budget exceeded: need {estimated} bytes"
                )));
            }
            let spool =
                fvid_media::owned_container_loudness::decode_adts_to_wave(source, options)
                    .map_err(|e| invalid(&e))?;
            if weights.len() != usize::from(spool.stats.channels) {
                return Err(invalid("channel weights must match selected audio stream"));
            }
            LoudnessMeter::new(spool.stats.sample_rate, weights).map_err(|e| invalid(&e))?;
            let mut pcm_options = fvid_media::CopyOptions::default();
            pcm_options.cancel = options.cancel.clone();
            pcm_options.max_controlled_bytes = options.max_controlled_bytes;
            pcm_options.max_rss_bytes = options.max_rss_bytes;
            let report = measure_file_with_options(&spool.wave, weights, &pcm_options)?;
            control.check()?;
            if let Some(hook) = progress {
                hook.emit(crate::media_control::ProgressEvent {
                    done: true,
                    ..spool.progress
                });
            }
            return Ok((report, false));
        }
        let reader = crate::container::adts::StreamReader::open_with_packet_limit(
            input.take().unwrap(),
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
            let frame_bytes = self.channels.saturating_mul(4);
            if self.byte_count == 0
                && self.channel == 0
                && frame_bytes > 0
                && bytes.len() % frame_bytes == 0
            {
                for raw_frame in bytes.chunks_exact(frame_bytes) {
                    for (ch, sample_bytes) in raw_frame.chunks_exact(4).enumerate() {
                        self.frame[ch] =
                            f64::from(f32::from_le_bytes(sample_bytes.try_into().unwrap()));
                    }
                    self.meter
                        .push(&self.frame[..self.channels])
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                }
                return Ok(bytes.len());
            }
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
    Ok((report, true))
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
    if options
        .cancel
        .as_ref()
        .is_some_and(crate::media_control::CancelFlag::is_cancelled)
    {
        return Err(crate::invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(crate::invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(crate::invalid("loudness requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(crate::invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| crate::invalid(&e))?;
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
    let selected = options.streams.first().copied();
    let mut file = Some(std::fs::File::open(source)?);
    let channels = if super::is_wave_reader(file.as_mut().unwrap())? {
        if selected.is_some_and(|index| index != 0) {
            return Err(crate::invalid("WAVE has only stream 0"));
        }
        let info = super::inspect(
            file.as_mut().unwrap(),
            options.cancel.as_ref(),
        )?;
        let frame = usize::from(info.frame_bytes());
        if frame == 0 || (65536usize.min(options.max_packet_bytes) / frame * frame) == 0 {
            return Err(crate::invalid(
                "packet byte limit is smaller than one PCM sample frame",
            ));
        }
        info.channels
    } else if {
        drop(file.take());
        options.max_controlled_bytes.is_none()
    } && let Ok(info) = crate::native_media::constant_audio_source_info_selected_with_options(source, selected, options)
    {
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
