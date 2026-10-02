//! Owned float WAVE PCM extraction and DSP, independent of libav.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{io::Write, path::Path};
type Result<T> = std::result::Result<T, String>;

/// Existing legacy entrypoints use this only when all requested policies are
/// implemented by this path; other formats/policies retain their current backend.
pub(crate) fn supports(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") || !simple_options(options) {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(source) else {
        return false;
    };
    crate::owned_wave_inspect::inspect(&mut file, None).is_ok_and(|info| {
        let channels = transform.channels.unwrap_or(i32::from(info.channels));
        let changes_channels = channels != i32::from(info.channels);
        let conversion = !changes_channels
            || (info.channels <= 8 && matches!(channels, 1 | 2))
            || (matches!(info.channels, 1 | 2) && (1..=8).contains(&channels));
        let layout = !changes_channels
            || (info.channels <= 2 && info.channel_mask == 0)
            || crate::owned_pcm_channels::standard_mask(info.channels)
                .is_some_and(|mask| mask == u64::from(info.channel_mask));
        // The legacy double WAV muxer also retains tags and explicit surround
        // masks. Keep those cases there until the owned writer retains them.
        let precision = info.bits_per_sample == 32
            || (info.bits_per_sample == 64
                && info.channels <= 2
                && channels <= 2
                && (info.channel_mask == 0
                    || crate::owned_pcm_channels::standard_mask(info.channels)
                        .is_some_and(|mask| mask == u64::from(info.channel_mask)))
                && crate::owned_probe::probe_wave(source)
                    .is_ok_and(|info| info.metadata.is_empty()));
        info.float && precision && conversion && layout
    })
}
fn simple_options(options: &CopyOptions) -> bool {
    options.max_packet_bytes == CopyOptions::default().max_packet_bytes
        && (options.streams.is_empty() || options.streams == [0])
        && options.max_packets.is_none()
        && options.max_controlled_bytes.is_none()
        && options.max_rss_bytes.is_none()
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
}
/// Export float WAVE samples through the owned resampler/rematrix and WAV writer.
pub fn decode_audio_transformed(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if !simple_options(options) {
        return Err(
            "owned float WAVE export does not yet implement these control/metadata policies".into(),
        );
    }
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err("owned float WAVE export requires .wav output".into());
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
        .is_some_and(|channels| !(1..=64).contains(&channels))
    {
        return Err("channels must be within 1..=64".into());
    }
    let gain = transform.volume.unwrap_or(1.0);
    if !gain.is_finite() || !(0.0..=64.0).contains(&gain) {
        return Err("volume must be a finite linear gain within 0..=64".into());
    }
    let check = || {
        if options
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.is_cancelled())
        {
            Err("media operation cancelled".to_owned())
        } else {
            Ok(())
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
    let (input, pcm) = crate::owned_audio_mix::read_float_wave_controlled(
        source,
        None,
        options.cancel.as_ref(),
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
    let rate = transform.sample_rate.unwrap_or(input.sample_rate);
    let channels = transform.channels.unwrap_or(input.channels);
    if channels != input.channels {
        let mut file = std::fs::File::open(source).map_err(|e| e.to_string())?;
        let info =
            crate::owned_wave_inspect::inspect(&mut file, None).map_err(|e| e.to_string())?;
        if !(info.channel_mask == 0 && info.channels <= 2
            || crate::owned_pcm_channels::standard_mask(info.channels)
                .is_some_and(|mask| mask == u64::from(info.channel_mask)))
        {
            return Err("owned rematrix requires a standard explicit speaker layout".into());
        }
    }
    let width = if input.sample_format == "dbl" { 8 } else { 4 };
    let bytes = if width == 8 {
        let mut resampler = crate::owned_resample_f64::Resampler::new(
            Vec::new(),
            input.sample_rate as u32,
            rate as u32,
            channels as u16,
        )
        .map_err(|e| e.to_string())?;
        let mut matrix = crate::owned_pcm_gain_f64::PcmGain::new(
            &mut resampler,
            1.,
            input.channels as u16,
            channels as u16,
        )?;
        for block in pcm.chunks(4096 * input.channels as usize * 8) {
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
    } else {
        let mut resampler = crate::owned_resample::Resampler::new(
            Vec::new(),
            input.sample_rate as u32,
            rate as u32,
            channels as u16,
        )
        .map_err(|e| e.to_string())?;
        let mut matrix = crate::owned_pcm_gain::PcmGain::new(
            &mut resampler,
            1.,
            input.channels as u16,
            channels as u16,
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
    if width == 8 {
        let samples: Vec<_> = selected
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()) * gain)
            .collect();
        if samples.iter().any(|s| !s.is_finite()) {
            return Err("PCM gain overflow".into());
        }
        crate::owned_wav_file::write_wav_f64le_checked(
            destination,
            rate,
            channels,
            &samples,
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
        crate::owned_wav_file::write_wav_f32le_checked(
            destination,
            rate,
            channels,
            &samples,
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
            max_packets: Some(1),
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
        assert!(!supports(
            &source,
            &output,
            Default::default(),
            &CopyOptions::default()
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
