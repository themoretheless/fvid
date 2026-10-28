//! Owned WAVE/AAC-LC extraction and shared PCM DSP, independent of libav.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{io::Write, path::Path};
type Result<T> = std::result::Result<T, String>;

/// Read-only admission for the owned export's format and policy support.
pub(crate) fn supports(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    if destination.extension().and_then(|s| s.to_str()) != Some("wav")
        || !single_audio_options(options)
        || options
            .cancel
            .as_ref()
            .is_some_and(fvid_control::CancelFlag::is_cancelled)
        || options.max_packet_bytes == 0
        || options.max_packets == Some(0)
        || options.max_controlled_bytes == Some(0)
        || crate::owned_budget::check_rss_budget(options).is_err()
    {
        return false;
    }
    if crate::owned_g711_wave_export::recognizes(source,options).unwrap_or(false) {
        return crate::owned_g711_wave_export::supports(source,transform,options);
    }
    if crate::owned_ogg_opus_export::recognizes(source).unwrap_or(false) {
        return match crate::owned_ogg_audio_export::coding(source, options) {
            Ok(crate::owned_ogg_audio_export::Backend::Vorbis) => {
                crate::owned_ogg_vorbis_export::supports(source, transform, options)
            }
            Ok(crate::owned_ogg_audio_export::Backend::Opus) => {
                crate::owned_ogg_opus_export::supports(source, transform, options)
            }
            Err(error) => error.starts_with("controlled memory budget exceeded:"),
            Ok(crate::owned_ogg_audio_export::Backend::Joint) => {
                crate::owned_ogg_joint_export::supports(source, transform, options)
            }
        };
    }
    if crate::owned_mp4_audio_export::recognizes(source, options).unwrap_or(false) {
        return crate::owned_mp4_audio_export::supports(source, transform, options);
    }
    if crate::owned_matroska_pcm_export::recognizes(source, options).unwrap_or(false) {
        return crate::owned_matroska_pcm_export::supports(source, transform, options);
    }
    if crate::owned_matroska_aac_export::recognizes(source, options).unwrap_or(false) {
        return crate::owned_matroska_aac_export::supports(source, transform, options);
    }
    if crate::owned_matroska_opus_export::recognizes(source, options).unwrap_or(false) {
        return crate::owned_matroska_opus_export::supports(source, transform, options);
    }
    if crate::owned_matroska_flac_export::recognizes(source, options).unwrap_or(false) {
        return crate::owned_matroska_flac_export::supports(source, transform, options);
    }
    if crate::owned_matroska_mp3_export::recognizes(source, options).unwrap_or(false) {
        return crate::owned_matroska_mp3_export::supports(source, transform, options);
    }
    if crate::owned_matroska_alac_export::recognizes(source, options).unwrap_or(false) {
        return crate::owned_matroska_alac_export::supports(source, transform, options);
    }
    if crate::owned_raw_flac::recognizes(source).unwrap_or(false) {
        return crate::owned_raw_flac::supports(source, transform, options);
    }
    if crate::owned_raw_mp3::recognizes(source).unwrap_or(false) {
        return crate::owned_raw_mp3::supports(source, transform, options);
    }
    if crate::owned_adts_export::recognizes(source).unwrap_or(false) {
        return simple_options(options)
            && crate::owned_adts_export::supports(source, transform, options);
    }
    if !simple_options(options) {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(source) else {
        return false;
    };
    crate::owned_wave_inspect::inspect(&mut file, options.cancel.as_ref()).is_ok_and(|info| {
        if info.sample_frames == 0 || !(1..=255).contains(&info.channels) {
            return false;
        }
        let frame_bytes = usize::from(info.block);
        let capacity = if frame_bytes == 0 {
            0
        } else {
            (4096 * frame_bytes).min(options.max_packet_bytes / frame_bytes * frame_bytes)
        };
        if capacity == 0 {
            return false;
        }
        let Ok(full_size) = usize::try_from(info.data_bytes) else {
            return false;
        };
        let limit = options
            .max_packets
            .map(|packets| u128::from(packets) * capacity as u128)
            .unwrap_or(full_size as u128);
        let size = (full_size as u128).min(limit) as usize;
        if size == 0 {
            return false;
        }
        if let Some((from, to)) = transform.interval {
            if from < 0 || to <= from {
                return false;
            }
            let frames_read = size / frame_bytes;
            let Ok(out_rate) =
                u128::try_from(transform.sample_rate.unwrap_or(info.sample_rate as i32))
            else {
                return false;
            };
            let Some(output_frames) = (frames_read as u128)
                .checked_mul(out_rate)
                .and_then(|n| usize::try_from(n.div_ceil(u128::from(info.sample_rate))).ok())
            else {
                return false;
            };
            let boundary = |time: i64| -> Option<usize> {
                (time as u128)
                    .checked_mul(out_rate)
                    .and_then(|n| usize::try_from(n.div_ceil(1_000_000)).ok())
                    .map(|n| n.min(output_frames))
            };
            let (Some(first), Some(last)) = (boundary(from), boundary(to)) else {
                return false;
            };
            if last <= first {
                return false;
            }
        }
        let channels = transform.channels.unwrap_or(i32::from(info.channels));
        let changes_channels = channels != i32::from(info.channels);
        let conversion = !changes_channels
            || (info.channels <= 8 && matches!(channels, 1 | 2))
            || (matches!(info.channels, 1 | 2) && (1..=8).contains(&channels));
        let layout = !changes_channels
            || (info.channels <= 2 && info.channel_mask == 0)
            || (info.channel_mask != 0
                && crate::owned_pcm_channels::rematrix_layout_supported(
                    info.channels,
                    info.channel_mask,
                ));
        conversion && layout
    })
}
pub(crate) fn simple_options(options: &CopyOptions) -> bool {
    (options.streams.is_empty() || options.streams == [0]) && single_audio_options(options)
}
pub(crate) fn single_audio_options(options: &CopyOptions) -> bool {
    options.streams.len() <= 1
        && options
            .metadata_set
            .iter()
            .all(|(key, _)| crate::owned_wave_metadata::tag_for_key(key).is_ok())
        && options
            .metadata_delete
            .iter()
            .all(|key| crate::owned_wave_metadata::tag_for_key(key).is_ok())
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
}
pub(crate) fn validate_request(
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<f64> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if !single_audio_options(options) {
        return Err(
            "owned audio export does not yet implement these control/metadata policies".into(),
        );
    }
    if transform
        .interval
        .is_some_and(|(from, to)| from < 0 || to <= from)
    {
        return Err("decode-audio interval requires 0 <= from < to".into());
    }
    if transform
        .sample_rate
        .is_some_and(|rate| !(8000..=384000).contains(&rate))
    {
        return Err("sample rate must be within 8000..=384000".into());
    }
    if transform
        .channels
        .is_some_and(|channels| !(1..=255).contains(&channels))
    {
        return Err("channels must be within 1..=255".into());
    }
    if options.metadata_set.len() + options.metadata_delete.len() > 64 {
        return Err("at most 64 container metadata mutations".into());
    }
    if options
        .metadata_set
        .iter()
        .any(|(_, value)| value.contains('\0'))
    {
        return Err("embedded NUL in metadata value".into());
    }
    let gain = transform.volume.unwrap_or(1.0);
    if !gain.is_finite() || !(0.0..=64.0).contains(&gain) {
        return Err("volume must be a finite linear gain within 0..=64".into());
    }
    Ok(gain)
}
/// Export WAVE samples through the owned resampler/rematrix and WAV writer.
/// Integer PCM retains its decoded precision: packed 24-bit input exports as
/// left-aligned signed 32-bit PCM. Integer DSP uses double precision and
/// saturating ties-to-even quantization.
/// For WAVE, packet limits apply to frame-aligned input I/O blocks (up to
/// 4096 sample frames); `max_packets` exports the prefix read before that limit.
pub fn decode_audio_transformed(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    let gain = validate_request(transform, options)?;
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err("owned WAVE export requires .wav output".into());
    }
    if crate::owned_g711_wave_export::recognizes(source,options)? {
        return crate::owned_g711_wave_export::apply(source,destination,transform,options);
    }
    if crate::owned_ogg_opus_export::recognizes(source)? {
        return match crate::owned_ogg_audio_export::coding(source, options)? {
            crate::owned_ogg_audio_export::Backend::Vorbis => {
                crate::owned_ogg_vorbis_export::apply(source, destination, transform, options)
            }
            crate::owned_ogg_audio_export::Backend::Opus => {
                crate::owned_ogg_opus_export::apply(source, destination, transform, options)
            }
            crate::owned_ogg_audio_export::Backend::Joint => {
                crate::owned_ogg_joint_export::apply(source, destination, transform, options)
            }
        };
    }
    if crate::owned_mp4_audio_export::recognizes(source, options)? {
        return crate::owned_mp4_audio_export::apply(source, destination, transform, options);
    }
    if crate::owned_matroska_pcm_export::recognizes(source, options)? {
        return crate::owned_matroska_pcm_export::apply(source, destination, transform, options);
    }
    if crate::owned_matroska_aac_export::recognizes(source, options)? {
        return crate::owned_matroska_aac_export::apply(source, destination, transform, options);
    }
    if crate::owned_matroska_opus_export::recognizes(source, options)? {
        return crate::owned_matroska_opus_export::apply(source, destination, transform, options);
    }
    if crate::owned_matroska_flac_export::recognizes(source, options)? {
        return crate::owned_matroska_flac_export::apply(source, destination, transform, options);
    }
    if crate::owned_matroska_mp3_export::recognizes(source, options)? {
        return crate::owned_matroska_mp3_export::apply(source, destination, transform, options);
    }
    if crate::owned_matroska_alac_export::recognizes(source, options)? {
        return crate::owned_matroska_alac_export::apply(source, destination, transform, options);
    }
    if crate::owned_raw_flac::recognizes(source)? {
        return crate::owned_raw_flac::apply(source,destination,transform,options);
    }
    if crate::owned_raw_mp3::recognizes(source)? {
        return crate::owned_raw_mp3::apply(source, destination, transform, options);
    }
    if crate::owned_adts_export::recognizes(source)? {
        if !simple_options(options) {
            return Err("ADTS has only stream 0".into());
        }
        return crate::owned_adts_export::apply(source, destination, transform, options);
    }
    if !simple_options(options) {
        return Err("WAVE has only stream 0".into());
    }
    let check = || {
        if options
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.is_cancelled())
        {
            Err("media operation cancelled".to_owned())
        } else {
            crate::owned_budget::check_rss_budget(options)
        }
    };
    let mut event = fvid_control::ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    check()?;
    if let Some(hook) = &options.progress {
        hook.emit(event);
    }
    check()?;
    let (input, pcm) = crate::owned_audio_mix::read_wave_with_admission(
        source,
        None,
        false,
        options.cancel.as_ref(),
        options.max_packet_bytes,
        options.max_packets,
        |file, info, size| {
            if let Some(max) = options.max_controlled_bytes {
                let metadata = crate::owned_wave_inspect::info_chunks_bytes(
                    file,
                    info,
                    options.cancel.as_ref(),
                )
                .map_err(|e| e.to_string())?;
                let estimated = crate::owned_budget::estimate_float_wave_export_bytes(
                    info, size, metadata, transform, options,
                )?;
                if estimated > max {
                    return Err(format!(
                        "controlled memory budget exceeded: need {estimated} bytes, limit {max}"
                    ));
                }
            }
            Ok(())
        },
        |bytes| {
            event.packets = event
                .packets
                .checked_add(1)
                .ok_or("audio packet count overflow")?;
            event.payload_bytes = event
                .payload_bytes
                .checked_add(bytes as u64)
                .ok_or("audio byte count overflow")?;
            if let Some(hook) = &options.progress {
                hook.emit(event);
            }
            check()
        },
    )?;
    let integer = match input.sample_format.as_str() {
        "u8" => Some(crate::owned_pcm_integer::Format::U8),
        "s16" => Some(crate::owned_pcm_integer::Format::I16),
        "s32" => Some(crate::owned_pcm_integer::Format::I32),
        _ => None,
    };
    let mut source_file = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let source_info = crate::owned_wave_inspect::inspect(&mut source_file, options.cancel.as_ref())
        .map_err(|e| e.to_string())?;
    let input_width = usize::from(source_info.bits_per_sample / 8);
    let rate = transform.sample_rate.unwrap_or(input.sample_rate);
    let channels = transform.channels.unwrap_or(input.channels);
    if channels != input.channels {
        let mut file = std::fs::File::open(source).map_err(|e| e.to_string())?;
        let info = crate::owned_wave_inspect::inspect(&mut file, options.cancel.as_ref())
            .map_err(|e| e.to_string())?;
        if !(info.channels <= 2 && info.channel_mask == 0)
            && !(info.channel_mask != 0
                && crate::owned_pcm_channels::rematrix_layout_supported(
                    info.channels,
                    info.channel_mask,
                ))
        {
            return Err("owned rematrix requires a standard explicit speaker layout".into());
        }
    }
    let output_mask = if channels == input.channels {
        source_info.channel_mask
    } else {
        crate::owned_pcm_channels::standard_mask(channels as u16).unwrap_or(0) as u32
    };
    // INFO is independent of sample representation and speaker assignment.
    let info_chunks = crate::owned_wave_inspect::info_chunks(
        &mut source_file,
        &source_info,
        options.cancel.as_ref(),
    )
    .map_err(|e| e.to_string())?;
    let info_chunks = crate::owned_wave_metadata::edit_info_chunks(
        &info_chunks,
        &options.metadata_delete,
        &options.metadata_set,
    )?;
    let width = if integer.is_some() || input.sample_format == "dbl" {
        8
    } else {
        4
    };
    let bytes = if width == 8 {
        let mut resampler = crate::owned_resample_f64::Resampler::new(
            Vec::new(),
            input.sample_rate as u32,
            rate as u32,
            channels as u16,
        )
        .map_err(|e| e.to_string())?;
        let mut matrix = crate::owned_pcm_gain_f64::PcmGain::with_mask(
            &mut resampler,
            1.,
            input.channels as u16,
            channels as u16,
            source_info.effective_channel_mask(),
        )?;
        for block in pcm.chunks(4096 * input.channels as usize * input_width) {
            if let Some(hook) = &options.progress {
                hook.emit(event);
            }
            check()?;
            if let Some(format) = integer {
                let mut normalized = Vec::with_capacity(block.len() / input_width * 8);
                for sample in block.chunks_exact(input_width) {
                    let value = if input_width == 3 {
                        // Packed signed 24-bit PCM is decoded as left-aligned S32.
                        format.decode(&[0, sample[0], sample[1], sample[2]])?
                    } else {
                        format.decode(sample)?
                    };
                    normalized.extend_from_slice(&value.to_le_bytes());
                }
                matrix.write_all(&normalized).map_err(|e| e.to_string())?;
            } else {
                matrix.write_all(block).map_err(|e| e.to_string())?;
            }
        }
        if !matrix.frame_complete() {
            return Err("incomplete PCM channel frame".into());
        }
        check()?;
        resampler.finish().map_err(|e| e.to_string())?;
        check()?;
        resampler.take_output()
    } else {
        let mut resampler = crate::owned_resample::Resampler::new(
            Vec::new(),
            input.sample_rate as u32,
            rate as u32,
            channels as u16,
        )
        .map_err(|e| e.to_string())?;
        let mut matrix = crate::owned_pcm_gain::PcmGain::with_mask(
            &mut resampler,
            1.,
            input.channels as u16,
            channels as u16,
            source_info.effective_channel_mask(),
        )?;
        for block in pcm.chunks(4096 * input.channels as usize * 4) {
            if let Some(hook) = &options.progress {
                hook.emit(event);
            }
            check()?;
            matrix.write_all(block).map_err(|e| e.to_string())?;
        }
        if !matrix.frame_complete() {
            return Err("incomplete PCM channel frame".into());
        }
        check()?;
        resampler.finish().map_err(|e| e.to_string())?;
        check()?;
        resampler.take_output()
    };
    let frames = bytes.len() / (channels as usize * width);
    let boundary = |time: i64| -> Result<usize> {
        usize::try_from((time as u128 * rate as u128).div_ceil(1_000_000))
            .map(|n| n.min(frames))
            .map_err(|_| "audio interval overflow".into())
    };
    let (first, last) = match transform.interval {
        Some((from, to)) => (boundary(from)?, boundary(to)?),
        None => (0, frames),
    };
    if last <= first {
        return Err("no decoded audio samples".into());
    }
    let selected = &bytes[first * channels as usize * width..last * channels as usize * width];
    check()?;
    if let Some(hook) = &options.progress {
        hook.emit(event);
    }
    if let Some(format) = integer {
        let mut samples = Vec::with_capacity(selected.len() / 8 * format.bytes());
        for block in selected.chunks(4096 * channels as usize * 8) {
            check()?;
            for sample in block.chunks_exact(8) {
                let mut encoded = [0u8; 8];
                format.encode(
                    f64::from_le_bytes(sample.try_into().unwrap()) * gain,
                    &mut encoded[..format.bytes()],
                )?;
                samples.extend_from_slice(&encoded[..format.bytes()]);
            }
        }
        crate::owned_wav_file::write_wav_integer_le_checked(
            destination,
            rate,
            channels,
            (format.bytes() * 8) as u16,
            &samples,
            output_mask,
            &info_chunks,
            check,
        )?;
    } else if width == 8 {
        let samples: Vec<_> = selected
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()) * gain)
            .collect();
        if samples.iter().any(|s| !s.is_finite()) {
            return Err("PCM gain overflow".into());
        }
        crate::owned_wav_file::write_wav_f64le_with_side_data_checked(
            destination,
            rate,
            channels,
            &samples,
            output_mask,
            &info_chunks,
            check,
        )?;
    } else {
        let samples: Vec<_> = selected
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()) * gain as f32)
            .collect();
        if samples.iter().any(|s| !s.is_finite()) {
            return Err("PCM gain overflow".into());
        }
        crate::owned_wav_file::write_wav_f32le_with_side_data_checked(
            destination,
            rate,
            channels,
            &samples,
            output_mask,
            &info_chunks,
            check,
        )?;
    }
    if let Some(hook) = &options.progress {
        hook.emit(fvid_control::ProgressEvent {
            done: true,
            ..event
        });
    }
    Ok(AudioDecodeStats {
        sample_frames: (last - first) as u64,
        decoded_frames: input.decoded_frames,
        sample_rate: rate,
        channels,
        sample_format: input.sample_format,
        planar_interleave_bytes: 0,
        decode_errors: 0,
    })
}
pub fn decode_audio(
    source: &Path,
    destination: &Path,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_audio_transformed(source, destination, Default::default(), options)
}
pub fn decode_audio_interval(
    source: &Path,
    destination: &Path,
    interval: Option<(i64, i64)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_audio_transformed(
        source,
        destination,
        AudioDecodeTransform {
            interval,
            ..Default::default()
        },
        options,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Files(std::path::PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn public_wave_export_preserves_identity_and_applies_output_clock_interval() {
        let files =
            Files(std::env::temp_dir().join(format!("fvid-owned-export-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&files.0);
        std::fs::create_dir(&files.0).unwrap();
        let source = files.0.join("source.wav");
        let pcm: Vec<f32> = (0..997).flat_map(|_| [0.25, 0.75]).collect();
        crate::owned_wav_file::write_wav_f32le(&source, 48000, 2, &pcm).unwrap();
        let identity = files.0.join("identity.wav");
        crate::decode_audio(&source, &identity, &CopyOptions::default()).unwrap();
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&identity).unwrap()
        );
        let output = files.0.join("output.wav");
        let transform = AudioDecodeTransform {
            interval: Some((63, 10125)),
            sample_rate: Some(16000),
            channels: Some(1),
            volume: Some(0.5),
        };
        assert!(supports(
            &source,
            &output,
            transform,
            &CopyOptions::default()
        ));
        let stats =
            crate::decode_audio_transformed(&source, &output, transform, &CopyOptions::default())
                .unwrap();
        assert_eq!(stats.sample_frames, 160); // ceil(162) - ceil(1.008).
        assert_eq!(stats.channels, 1);
        assert_eq!(stats.sample_rate, 16000);
        let (_, data) = crate::owned_audio_mix::decode_float_wave(&output).unwrap();
        for sample in data.chunks_exact(4) {
            assert!((f32::from_le_bytes(sample.try_into().unwrap()) - 0.25).abs() < 1e-6);
        }
        let absent = files.0.join("absent.wav");
        assert!(
            decode_audio_interval(
                &source,
                &absent,
                Some((1_000_000, 2_000_000)),
                &CopyOptions::default()
            )
            .is_err()
        );
        assert!(!absent.exists());
        let limited = CopyOptions {
            stream_metadata_set: vec![(0, "title".into(), "unmapped".into())],
            ..Default::default()
        };
        assert!(!supports(&source, &absent, transform, &limited));
        assert!(decode_audio(&source, &absent, &limited).is_err());
        let original = std::fs::read(&output).unwrap();
        assert!(decode_audio(&source, &output, &CopyOptions::default()).is_err());
        assert_eq!(std::fs::read(output).unwrap(), original);
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[test]
    fn progress_counts_source_blocks_and_cancel_never_publishes_output() {
        let directory =
            std::env::temp_dir().join(format!("fvid-export-controls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("source.wav");
        crate::owned_wav_file::write_wav_f32le(&source, 48000, 2, &vec![0.25; 8193 * 2]).unwrap();
        let output = directory.join("output.wav");
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();
        let published = output.clone();
        let hook = fvid_control::ProgressHook::new(move |event| {
            if event.done {
                assert!(published.exists());
            }
            recorded.lock().unwrap().push(event);
        });
        let options = CopyOptions {
            progress: Some(hook),
            ..Default::default()
        };
        assert!(supports(&source, &output, Default::default(), &options));
        let stats = crate::decode_audio(&source, &output, &options).unwrap();
        assert_eq!(stats.sample_frames, 8193);
        let events = events.lock().unwrap();
        assert_eq!(events.first().unwrap().packets, 0);
        assert_eq!(events.last().unwrap().packets, 3);
        assert_eq!(events.last().unwrap().payload_bytes, 8193 * 2 * 4);
        assert!(events.last().unwrap().done);
        assert_eq!(events.iter().filter(|event| event.done).count(), 1);
        drop(events);
        for mode in 0..3 {
            let flag = fvid_control::CancelFlag::new();
            if mode == 0 {
                flag.cancel();
            }
            let cancellation = flag.clone();
            let at_end = std::sync::atomic::AtomicU64::new(0);
            let output = directory.join(format!("cancelled-{mode}.wav"));
            let options = CopyOptions {
                cancel: Some(flag),
                progress: Some(fvid_control::ProgressHook::new(move |event| {
                    assert!(!event.done);
                    if mode == 1 && event.packets > 0
                        || mode == 2
                            && event.packets == 3
                            && at_end.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 1
                    {
                        cancellation.cancel();
                    }
                })),
                ..Default::default()
            };
            assert!(supports(&source, &output, Default::default(), &options));
            let error = crate::decode_audio(&source, &output, &options).unwrap_err();
            assert!(error.contains("cancelled"));
            assert!(!output.exists());
        }
        let output = directory.join("writer-cancelled.wav");
        let mut checks = 0;
        let result =
            crate::owned_wav_file::write_wav_f32le_checked(&output, 48000, 2, &[0.25; 8], || {
                checks += 1;
                if checks == 2 {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            });
        assert!(result.is_err());
        assert!(!output.exists());
        assert!(std::fs::read_dir(&directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod double_tests {
    use super::*;
    #[test]
    fn double_wave_identity_and_transformed_export_retain_sub_float32_precision() {
        let dir =
            std::env::temp_dir().join(format!("fvid-double-wave-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let source = dir.join("source.wav");
        let identity = dir.join("identity.wav");
        let value = 0.12345678901234567f64;
        assert_ne!(value, value as f32 as f64);
        crate::owned_wav_file::write_wav_f64le(&source, 48000, 2, &vec![value; 997 * 2]).unwrap();
        assert!(supports(
            &source,
            &identity,
            Default::default(),
            &CopyOptions::default()
        ));
        let stats = crate::decode_audio(&source, &identity, &CopyOptions::default()).unwrap();
        assert_eq!(stats.sample_format, "dbl");
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&identity).unwrap()
        );
        let output = dir.join("output.wav");
        let transform = AudioDecodeTransform {
            interval: Some((63, 10125)),
            sample_rate: Some(16000),
            channels: Some(1),
            volume: Some(0.5),
        };
        let stats =
            crate::decode_audio_transformed(&source, &output, transform, &CopyOptions::default())
                .unwrap();
        assert_eq!(stats.sample_frames, 160);
        assert_eq!(stats.sample_format, "dbl");
        let mut file = std::fs::File::open(&output).unwrap();
        let info = crate::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert_eq!(info.bits_per_sample, 64);
        assert_eq!(info.channels, 1);
        let (_, pcm) =
            crate::owned_audio_mix::read_float_wave_controlled(&output, None, None, |_| Ok(()))
                .unwrap();
        for sample in pcm.chunks_exact(8) {
            let actual = f64::from_le_bytes(sample.try_into().unwrap());
            assert!((actual - value * 0.5).abs() < 1e-14);
            assert_ne!(actual, actual as f32 as f64);
        }
        let mut file = std::fs::File::open(&source).unwrap();
        assert_eq!(
            crate::owned_wave_inspect::inspect(&mut file, None)
                .unwrap()
                .bits_per_sample,
            64
        );
        assert!(crate::owned_audio_mix::decode_float_wave(&source).is_err());
        let mut bytes = std::fs::read(&source).unwrap();
        bytes.extend_from_slice(b"LIST");
        bytes.extend_from_slice(&18u32.to_le_bytes());
        bytes.extend_from_slice(b"INFOINAM");
        bytes.extend_from_slice(&6u32.to_le_bytes());
        bytes.extend_from_slice(b"title\0");
        let size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        std::fs::write(&source, bytes).unwrap();
        assert_eq!(
            crate::owned_probe::probe_wave(&source).unwrap().metadata["title"],
            "title"
        );
        assert!(supports(
            &source,
            &output,
            Default::default(),
            &CopyOptions::default()
        ));
        let tagged = dir.join("tagged.wav");
        crate::decode_audio(&source, &tagged, &CopyOptions::default()).unwrap();
        assert_eq!(
            crate::owned_probe::probe_wave(&tagged).unwrap().metadata["title"],
            "title"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod side_data_tests {
    use super::*;
    #[test]
    fn double_surround_identity_retains_explicit_masks_and_raw_info_chunks() {
        let dir =
            std::env::temp_dir().join(format!("fvid-double-wave-side-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let mut chunks = Vec::new();
        chunks.extend_from_slice(b"LIST");
        chunks.extend_from_slice(&30u32.to_le_bytes());
        chunks.extend_from_slice(b"INFOINAM");
        chunks.extend_from_slice(&6u32.to_le_bytes());
        chunks.extend_from_slice(b"title\0");
        chunks.extend_from_slice(b"QWER");
        chunks.extend_from_slice(&3u32.to_le_bytes());
        chunks.extend_from_slice(b"abc");
        chunks.push(0xee);
        for (channels, mask) in [(6, 0x3f), (6, 0x60f), (7, 0x70f), (8, 0x63f)] {
            let source = dir.join(format!("source-{channels}-{mask}.wav"));
            let output = dir.join(format!("output-{channels}-{mask}.wav"));
            let pcm: Vec<f64> = (0..97 * channels).map(|i| i as f64 / 997.0).collect();
            crate::owned_wav_file::write_wav_f64le_with_side_data_checked(
                &source,
                48000,
                channels,
                &pcm,
                mask,
                &chunks,
                || Ok(()),
            )
            .unwrap();
            assert!(supports(
                &source,
                &output,
                Default::default(),
                &CopyOptions::default()
            ));
            crate::decode_audio(&source, &output, &CopyOptions::default()).unwrap();
            assert_eq!(
                std::fs::read(&source).unwrap(),
                std::fs::read(&output).unwrap()
            );
            let mut file = std::fs::File::open(&output).unwrap();
            let info = crate::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert_eq!(info.channel_mask, mask);
            assert_eq!(info.bits_per_sample, 64);
            assert_eq!(info.sample_frames, 97);
            assert_eq!(
                crate::owned_wave_inspect::info_chunks(&mut file, &info, None).unwrap(),
                chunks
            );
            let description = crate::owned_probe::probe_wave(&output).unwrap();
            assert_eq!(description.metadata["title"], "title");
            assert_eq!(description.metadata["QWER"], "abc");
            if mask == 0x3f || channels >= 7 {
                let mono = dir.join(format!("mono-{channels}.wav"));
                let transform = AudioDecodeTransform {
                    channels: Some(1),
                    sample_rate: Some(16000),
                    ..Default::default()
                };
                crate::decode_audio_transformed(&source, &mono, transform, &CopyOptions::default())
                    .unwrap();
                let mut file = std::fs::File::open(&mono).unwrap();
                let info = crate::owned_wave_inspect::inspect(&mut file, None).unwrap();
                assert_eq!(info.channel_mask, 4);
                assert_eq!(info.sample_frames, 33);
                assert_eq!(
                    crate::owned_wave_inspect::info_chunks(&mut file, &info, None).unwrap(),
                    chunks
                );
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    #[test]
    fn public_float_wave_export_edits_tags_without_changing_pcm_or_losing_opaque_info() {
        let dir =
            std::env::temp_dir().join(format!("fvid-wave-metadata-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let initial = crate::owned_wave_metadata::edit_info_chunks(
            &[],
            &[],
            &[
                ("title".into(), "old".into()),
                ("artist".into(), "artist".into()),
                ("QWER".into(), "opaque".into()),
            ],
        )
        .unwrap();
        for bits in [32, 64] {
            let source = dir.join(format!("source-{bits}.wav"));
            let output = dir.join(format!("output-{bits}.wav"));
            if bits == 32 {
                crate::owned_wav_file::write_wav_f32le_with_side_data_checked(
                    &source,
                    48000,
                    2,
                    &[0.25, -0.5, 0.5, 0.75],
                    0,
                    &initial,
                    || Ok(()),
                )
                .unwrap();
            } else {
                crate::owned_wav_file::write_wav_f64le_with_side_data_checked(
                    &source,
                    48000,
                    2,
                    &[0.25, -0.5, 0.5, 0.75],
                    0,
                    &initial,
                    || Ok(()),
                )
                .unwrap();
            }
            let options = CopyOptions {
                metadata_delete: vec!["artist".into()],
                metadata_set: vec![
                    ("title".into(), "Новый".into()),
                    ("comment".into(), "comment".into()),
                    ("artist".into(), "transient".into()),
                ],
                ..Default::default()
            };
            assert!(supports(&source, &output, Default::default(), &options));
            crate::decode_audio(&source, &output, &options).unwrap();
            let info = crate::owned_probe::probe_wave(&output).unwrap();
            assert_eq!(info.metadata["title"], "Новый");
            assert_eq!(info.metadata["comment"], "comment");
            assert_eq!(info.metadata["QWER"], "opaque");
            assert!(!info.metadata.contains_key("artist"));
            let (_, before) =
                crate::owned_audio_mix::read_float_wave_controlled(&source, None, None, |_| Ok(()))
                    .unwrap();
            let (_, after) =
                crate::owned_audio_mix::read_float_wave_controlled(&output, None, None, |_| Ok(()))
                    .unwrap();
            assert_eq!(before, after);
            let invalid = dir.join(format!("invalid-{bits}.wav"));
            let options = CopyOptions {
                metadata_set: vec![("title".into(), "bad\0value".into())],
                ..Default::default()
            };
            assert!(decode_audio(&source, &invalid, &options).is_err());
            assert!(!invalid.exists());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[test]
    fn packet_limits_bound_blocks_and_export_the_exact_prefix_for_both_float_widths() {
        let dir =
            std::env::temp_dir().join(format!("fvid-wave-limit-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        for bits in [32, 64] {
            let source = dir.join(format!("source-{bits}.wav"));
            let output = dir.join(format!("output-{bits}.wav"));
            if bits == 32 {
                crate::owned_wav_file::write_wav_f32le(
                    &source,
                    48000,
                    2,
                    &(0..34).map(|i| i as f32 / 64.).collect::<Vec<_>>(),
                )
                .unwrap();
            } else {
                crate::owned_wav_file::write_wav_f64le(
                    &source,
                    48000,
                    2,
                    &(0..34).map(|i| i as f64 / 64.).collect::<Vec<_>>(),
                )
                .unwrap();
            }
            let frame_bytes = 2 * bits as usize / 8;
            let block_limit = frame_bytes * 3 + 1;
            let events = Arc::new(Mutex::new(Vec::new()));
            let recorded = events.clone();
            let options = CopyOptions {
                max_packet_bytes: block_limit,
                max_packets: Some(2),
                progress: Some(fvid_control::ProgressHook::new(move |event| {
                    recorded.lock().unwrap().push(event)
                })),
                ..Default::default()
            };
            assert!(supports(&source, &output, Default::default(), &options));
            let stats = crate::decode_audio(&source, &output, &options).unwrap();
            assert_eq!(stats.sample_frames, 6);
            assert_eq!(stats.decoded_frames, 2);
            let (_, original) =
                crate::owned_audio_mix::read_float_wave_controlled(&source, None, None, |_| Ok(()))
                    .unwrap();
            let (_, actual) =
                crate::owned_audio_mix::read_float_wave_controlled(&output, None, None, |_| Ok(()))
                    .unwrap();
            assert_eq!(actual, &original[..6 * frame_bytes]);
            let events = events.lock().unwrap();
            assert_eq!(events.last().unwrap().packets, 2);
            assert_eq!(
                events.last().unwrap().payload_bytes,
                (6 * frame_bytes) as u64
            );
            assert!(events.last().unwrap().done);
            let distinct: Vec<_> = events
                .windows(2)
                .filter(|pair| pair[0].packets != pair[1].packets)
                .collect();
            assert_eq!(distinct.len(), 2);
            for pair in distinct {
                assert!(pair[1].payload_bytes - pair[0].payload_bytes <= block_limit as u64);
            }
            drop(events);
            for (limit, packets) in [(frame_bytes - 1, None), (frame_bytes, Some(0))] {
                let invalid = dir.join(format!("invalid-{bits}-{limit}.wav"));
                let options = CopyOptions {
                    max_packet_bytes: limit,
                    max_packets: packets,
                    ..Default::default()
                };
                assert!(crate::decode_audio(&source, &invalid, &options).is_err());
                assert!(!invalid.exists());
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod rss_tests {
    use super::*;
    #[test]
    fn public_wave_export_checks_requested_process_limit_without_legacy_admission() {
        let dir = std::env::temp_dir().join(format!("fvid-wave-rss-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let source = dir.join("source.wav");
        let absent = dir.join("absent.wav");
        crate::owned_wav_file::write_wav_f32le(&source, 48000, 2, &[0.25; 16]).unwrap();
        let options = CopyOptions {
            max_rss_bytes: Some(0),
            ..Default::default()
        };
        assert!(supports(&source, &absent, Default::default(), &options));
        let error = crate::decode_audio(&source, &absent, &options).unwrap_err();
        assert!(error.contains("rss"));
        assert!(!absent.exists());
        if crate::owned_budget::process_rss_bytes().is_some() {
            let output = dir.join("output.wav");
            let options = CopyOptions {
                max_rss_bytes: Some(u64::MAX),
                ..Default::default()
            };
            assert!(supports(&source, &output, Default::default(), &options));
            crate::decode_audio(&source, &output, &options).unwrap();
            assert_eq!(
                std::fs::read(&source).unwrap(),
                std::fs::read(&output).unwrap()
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[test]
    fn requested_controlled_limit_is_checked_before_pcm_reads_and_large_buffers() {
        let dir = std::env::temp_dir().join(format!("fvid-wave-admission-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        for bits in [32, 64] {
            let source = dir.join(format!("source-{bits}.wav"));
            let absent = dir.join(format!("absent-{bits}.wav"));
            if bits == 32 {
                crate::owned_wav_file::write_wav_f32le(&source, 48000, 2, &[0.25; 34]).unwrap();
            } else {
                crate::owned_wav_file::write_wav_f64le(&source, 48000, 2, &[0.25; 34]).unwrap();
            }
            let events = Arc::new(Mutex::new(Vec::new()));
            let recorded = events.clone();
            let options = CopyOptions {
                max_controlled_bytes: Some(1),
                progress: Some(fvid_control::ProgressHook::new(move |event| {
                    recorded.lock().unwrap().push(event)
                })),
                ..Default::default()
            };
            assert!(supports(&source, &absent, Default::default(), &options));
            let error = crate::decode_audio(&source, &absent, &options).unwrap_err();
            assert!(error.contains("controlled memory budget exceeded"));
            assert!(!absent.exists());
            assert!(
                events
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|event| event.packets == 0 && event.payload_bytes == 0 && !event.done)
            );
            let output = dir.join(format!("output-{bits}.wav"));
            let options = CopyOptions {
                max_controlled_bytes: Some(64 * 1024),
                ..Default::default()
            };
            crate::decode_audio(&source, &output, &options).unwrap();
            assert_eq!(
                std::fs::read(&source).unwrap(),
                std::fs::read(output).unwrap()
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod integer_tests {
    use super::*;
    #[test]
    fn integer_public_export_retains_precision_and_transforms_without_libav() {
        let dir = std::env::temp_dir().join(format!("fvid-int-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for bits in [8u16, 16, 24, 32] {
            let source = dir.join(format!("source-{bits}.wav"));
            let width = usize::from(bits / 8);
            let raw: Vec<u8> = (0..15 * width).map(|i| (i * 73 + 17) as u8).collect();
            crate::owned_wav_file::write_wav_integer_le(&source, 48000, 1, bits, &raw, 4).unwrap();
            let output = dir.join(format!("identity-{bits}.wav"));
            assert!(supports(
                &source,
                &output,
                Default::default(),
                &CopyOptions::default()
            ));
            let stats = crate::decode_audio(&source, &output, &CopyOptions::default()).unwrap();
            let mut file = std::fs::File::open(&output).unwrap();
            let info = crate::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert!(!info.float);
            assert_eq!(info.bits_per_sample, if bits == 24 { 32 } else { bits });
            assert_eq!(info.channel_mask, 4);
            assert_eq!(stats.sample_frames, 15);
            let bytes = std::fs::read(&output).unwrap();
            let expected = if bits == 24 {
                raw.chunks_exact(3)
                    .flat_map(|s| [0, s[0], s[1], s[2]])
                    .collect()
            } else {
                raw.clone()
            };
            assert_eq!(
                &bytes[info.data_offset as usize..info.data_offset as usize + expected.len()],
                expected
            );
            let output = dir.join(format!("interval-{bits}.wav"));
            let stats = crate::decode_audio_interval(
                &source,
                &output,
                Some((21, 105)),
                &CopyOptions::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, 4); // ceil(1.008)=2 .. ceil(5.04)=6
            let bytes = std::fs::read(&output).unwrap();
            let mut cursor = std::io::Cursor::new(&bytes);
            let info = crate::owned_wave_inspect::inspect(&mut cursor, None).unwrap();
            let out_width = usize::from(info.bits_per_sample / 8);
            assert_eq!(
                &bytes[info.data_offset as usize..info.data_offset as usize + 4 * out_width],
                &expected[2 * out_width..6 * out_width]
            );
            let limited = dir.join(format!("limited-{bits}.wav"));
            let stats = crate::decode_audio(
                &source,
                &limited,
                &CopyOptions {
                    max_packet_bytes: width * 3,
                    max_packets: Some(2),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(stats.sample_frames, 6);
            assert_eq!(stats.decoded_frames, 2);
            let raw_limited = std::fs::read(&limited).unwrap();
            let mut cursor = std::io::Cursor::new(&raw_limited);
            let info = crate::owned_wave_inspect::inspect(&mut cursor, None).unwrap();
            assert_eq!(
                &raw_limited[info.data_offset as usize..info.data_offset as usize + 6 * out_width],
                &expected[..6 * out_width]
            );
            let tagged = dir.join(format!("tagged-{bits}.wav"));
            crate::decode_audio(
                &source,
                &tagged,
                &CopyOptions {
                    metadata_set: vec![("title".into(), "own PCM".into())],
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                crate::owned_probe::probe_wave(&tagged)
                    .unwrap()
                    .metadata
                    .get("title")
                    .map(String::as_str),
                Some("own PCM")
            );
            let rejected = dir.join(format!("budget-{bits}.wav"));
            assert!(
                crate::decode_audio(
                    &source,
                    &rejected,
                    &CopyOptions {
                        max_controlled_bytes: Some(1),
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .contains("controlled memory")
            );
            assert!(!rejected.exists());
        }
        let source = dir.join("dc.wav");
        let raw: Vec<u8> = (0..480).flat_map(|_| 8192i16.to_le_bytes()).collect();
        crate::owned_wav_file::write_wav_integer_le(&source, 48000, 1, 16, &raw, 4).unwrap();
        let output = dir.join("dc-transformed.wav");
        let stats = crate::decode_audio_transformed(
            &source,
            &output,
            AudioDecodeTransform {
                sample_rate: Some(16000),
                channels: Some(2),
                volume: Some(2.),
                ..Default::default()
            },
            &CopyOptions::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 160);
        assert_eq!(stats.sample_format, "s16");
        let bytes = std::fs::read(&output).unwrap();
        let mut cursor = std::io::Cursor::new(&bytes);
        let info = crate::owned_wave_inspect::inspect(&mut cursor, None).unwrap();
        assert_eq!(info.channels, 2);
        assert_eq!(info.channel_mask, 3);
        for sample in bytes
            [info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize]
            .chunks_exact(2)
        {
            assert_eq!(i16::from_le_bytes(sample.try_into().unwrap()), 16384);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn raw_flac_mp3_and_ogg_supports_validate_budget_and_rematrix_layouts() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/wave-ogg-flac-plan");
        let dst = std::path::Path::new("out.wav");
        let flac_quad = root.join("flac-4ch-quad.flac");
        let flac_unassigned = root.join("flac-4ch-unassigned.flac");
        let mp3 = root.join("raw-stereo.mp3");
        let vorbis_5ch = root.join("ogg-vorbis-chained-5ch-0x37.ogg");
        let joint_5ch = root.join("ogg-joint-chained-5ch-0x37.ogg");
        let vorbis_discrete = root.join("ogg-vorbis-chained-8ch-0x73f.ogg");
        let joint_discrete = root.join("ogg-joint-chained-8ch-0x73f.ogg");

        let tight = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        for src in [
            &flac_quad,
            &flac_unassigned,
            &mp3,
            &vorbis_5ch,
            &joint_5ch,
            &vorbis_discrete,
            &joint_discrete,
        ] {
            assert!(
                supports(src, dst, Default::default(), &tight),
                "tight budget must remain owned for {}",
                src.display()
            );
        }

        let downmix = AudioDecodeTransform {
            channels: Some(2),
            ..Default::default()
        };
        assert!(supports(&flac_quad, dst, downmix, &CopyOptions::default()));
        assert!(supports(&vorbis_5ch, dst, downmix, &CopyOptions::default()));
        assert!(supports(&joint_5ch, dst, downmix, &CopyOptions::default()));
        assert!(!supports(
            &flac_unassigned,
            dst,
            downmix,
            &CopyOptions::default()
        ));
        assert!(!supports(
            &vorbis_discrete,
            dst,
            downmix,
            &CopyOptions::default()
        ));
        assert!(!supports(
            &joint_discrete,
            dst,
            downmix,
            &CopyOptions::default()
        ));

        let upmix_6 = AudioDecodeTransform {
            channels: Some(6),
            ..Default::default()
        };
        assert!(!supports(&flac_quad, dst, upmix_6, &CopyOptions::default()));
        assert!(supports(&mp3, dst, upmix_6, &CopyOptions::default()));
    }
}
