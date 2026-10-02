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
        info.float && info.bits_per_sample == 32 && conversion && layout
    })
}
fn simple_options(options: &CopyOptions) -> bool {
    options.max_packet_bytes == CopyOptions::default().max_packet_bytes
        && (options.streams.is_empty() || options.streams == [0])
        && options.max_packets.is_none()
        && options.max_controlled_bytes.is_none()
        && options.max_rss_bytes.is_none()
        && options.cancel.is_none()
        && options.progress.is_none()
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
    let (input, pcm) = crate::owned_audio_mix::decode_float_wave(source)?;
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
    matrix.write_all(&pcm).map_err(|e| e.to_string())?;
    if !matrix.frame_complete() {
        return Err("incomplete PCM channel frame".into());
    }
    resampler.finish().map_err(|e| e.to_string())?;
    let bytes = resampler.take_output();
    let frames = bytes.len() / (channels as usize * 4);
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
    let samples: Vec<_> = bytes[first * channels as usize * 4..last * channels as usize * 4]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()) * gain as f32)
        .collect();
    if samples.iter().any(|s| !s.is_finite()) {
        return Err("PCM gain overflow".into());
    }
    crate::owned_wav_file::write_wav_f32le(destination, rate, channels, &samples)?;
    Ok(AudioDecodeStats {
        sample_frames: (last - first) as u64,
        decoded_frames: input.decoded_frames,
        sample_rate: rate,
        channels,
        sample_format: "flt".into(),
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
