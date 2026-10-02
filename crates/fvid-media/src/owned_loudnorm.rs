//! Linear loudness normalization with owned WAVE decoding and publication.
//! Dynamic loudness compression is deliberately a separate implementation.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{LoudnormStats, resolve_loudnorm_args};
use std::path::Path;
type Result<T> = std::result::Result<T, String>;

/// Return a gain only when the requested parameters select linear processing.
/// Offset is a dynamic-mode parameter: linear gain is target I minus measured I.
fn linear_gain(args: &str) -> Result<Option<f64>> {
    let (mut target_i, mut target_tp, mut target_lra) = (-24., -2., 7.);
    let (mut measured_i, mut measured_tp, mut measured_lra, mut threshold) = (0., 99., 0., -70.);
    let mut linear = true;
    for argument in args.split(':') {
        let (key, value) = argument
            .split_once('=')
            .ok_or("expected loudnorm key=value")?;
        if matches!(key, "linear" | "dual_mono") {
            let flag = match value {
                "true" | "1" => true,
                "false" | "0" => false,
                _ => return Err("invalid loudnorm boolean".into()),
            };
            if key == "linear" {
                linear = flag;
            }
            continue;
        }
        if key == "print_format" {
            if !matches!(value, "none" | "json" | "summary") {
                return Err("invalid loudnorm print format".into());
            }
            // Printing measurements needs its own meter and remains outside this path.
            if value != "none" {
                return Ok(None);
            }
            continue;
        }
        let number: f64 = value.parse().map_err(|_| "invalid loudnorm number")?;
        let (slot, low, high) = match key {
            "I" | "i" => (&mut target_i, -70., -5.),
            "TP" | "tp" => (&mut target_tp, -9., 0.),
            "LRA" | "lra" => (&mut target_lra, 1., 50.),
            "measured_I" | "measured_i" => (&mut measured_i, -99., 0.),
            "measured_TP" | "measured_tp" => (&mut measured_tp, -99., 99.),
            "measured_LRA" | "measured_lra" => (&mut measured_lra, 0., 99.),
            "measured_thresh" => (&mut threshold, -99., 0.),
            "offset" => {
                if !number.is_finite() || !(-99. ..=99.).contains(&number) {
                    return Err("loudnorm offset out of range".into());
                }
                continue;
            }
            _ => return Err(format!("unknown loudnorm option {key}")),
        };
        if !number.is_finite() || number < low || number > high {
            return Err(format!("loudnorm {key} out of range"));
        }
        *slot = number;
    }
    let gain_db = target_i - measured_i;
    Ok((linear
        && measured_i != 0.
        && measured_tp != 99.
        && measured_lra != 0.
        && threshold != -70.
        && measured_tp + gain_db <= target_tp
        && measured_lra <= target_lra)
        .then(|| 10f64.powf(gain_db / 20.)))
}

fn policies(options: &CopyOptions) -> bool {
    (options.streams.is_empty() || options.streams == [0])
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
}
#[cfg(feature = "legacy-ffmpeg")]
pub(crate) fn supports(
    source: &Path,
    destination: &Path,
    args: &str,
    options: &CopyOptions,
) -> bool {
    policies(options)
        && destination.extension().and_then(|s| s.to_str()) == Some("wav")
        && linear_gain(args).is_ok_and(|gain| gain.is_some())
        && std::fs::File::open(source)
            .is_ok_and(|mut file| crate::owned_wave_inspect::inspect(&mut file, None).is_ok())
}

pub fn apply_loudnorm(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    let args = resolve_loudnorm_args(args)?;
    let gain =
        linear_gain(&args)?.ok_or("owned loudnorm dynamic processing is not yet implemented")?;
    if !policies(options) || destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err(
            "owned linear loudnorm requires WAVE output, stream 0 and no metadata edits".into(),
        );
    }
    let check = || {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".to_owned());
        }
        crate::owned_budget::check_rss_budget(options)
    };
    check()?;
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    if let Some(hook) = &options.progress {
        hook.emit(event);
    }
    let mut width = 0usize;
    let mut float = false;
    let mut mask = 0;
    let (input, bytes) = crate::owned_audio_mix::read_wave_with_admission(
        source,
        None,
        false,
        options.cancel.as_ref(),
        options.max_packet_bytes,
        options.max_packets,
        |_, info, size| {
            width = usize::from(info.bits_per_sample / 8);
            float = info.float;
            mask = info.channel_mask;
            let output = (size / width)
                .checked_mul(4)
                .ok_or("loudnorm output size overflow")?;
            let estimate = size
                .checked_add(output)
                .ok_or("loudnorm memory size overflow")?;
            if options
                .max_controlled_bytes
                .is_some_and(|limit| estimate > limit)
            {
                return Err(format!(
                    "controlled memory budget exceeded: need {estimate} bytes"
                ));
            }
            check()
        },
        |size| {
            event.packets += 1;
            event.payload_bytes += size as u64;
            if let Some(hook) = &options.progress {
                hook.emit(event);
            }
            check()
        },
    )?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(bytes.len() / width)
        .map_err(|_| "cannot allocate loudnorm output")?;
    for block in bytes.chunks(width * 4096) {
        check()?;
        for sample in block.chunks_exact(width) {
            use crate::owned_pcm_integer::Format;
            let value = match (float, width) {
                (true, 4) => f64::from(f32::from_le_bytes(sample.try_into().unwrap())),
                (true, 8) => f64::from_le_bytes(sample.try_into().unwrap()),
                (false, 1) => Format::U8.decode(sample)?,
                (false, 2) => Format::I16.decode(sample)?,
                (false, 3) => Format::I32.decode(&[0, sample[0], sample[1], sample[2]])?,
                (false, 4) => Format::I32.decode(sample)?,
                _ => return Err("unsupported loudnorm PCM format".into()),
            };
            let value = (value * gain) as f32;
            if !value.is_finite() {
                return Err("non-finite loudnorm output".into());
            }
            output.push(value);
        }
    }
    drop(bytes);
    crate::owned_wav_file::write_wav_f32le_with_side_data_checked(
        destination,
        input.sample_rate,
        input.channels,
        &output,
        mask,
        &[],
        check,
    )?;
    event.done = true;
    if let Some(hook) = &options.progress {
        hook.emit(event);
    }
    Ok(LoudnormStats {
        backend: "fvid linear loudnorm",
        sample_frames: input.sample_frames,
        sample_rate: input.sample_rate,
        channels: input.channels,
        args,
        dual_pass: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const ARGS: &str = "I=-16:TP=-1.5:LRA=11:measured_I=-22:measured_TP=-12:measured_LRA=2:measured_thresh=-32:linear=true";
    #[test]
    fn public_entrypoint_decodes_integer_depths_and_preserves_layout() {
        let dir = std::env::temp_dir().join(format!("fvid-linear-depths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for bits in [8, 16, 24, 32] {
            let source = dir.join(format!("{bits}.wav"));
            let dest = dir.join(format!("out-{bits}.wav"));
            let _ = std::fs::remove_file(&source);
            let _ = std::fs::remove_file(&dest);
            // Exact quarter-scale values in every integer representation, stereo antiphase.
            let raw: Vec<u8> = [1i32, -1, 0, 1]
                .into_iter()
                .flat_map(|value| match bits {
                    8 => vec![(128 + value * 32) as u8],
                    16 => (value as i16 * 8192).to_le_bytes().to_vec(),
                    24 => (value * 2097152).to_le_bytes()[..3].to_vec(),
                    _ => (value * 536870912).to_le_bytes().to_vec(),
                })
                .collect();
            crate::owned_wav_file::write_wav_integer_le(&source, 48000, 2, bits, &raw, 3).unwrap();
            let stats =
                crate::apply_loudnorm(&source, &dest, Some(ARGS), &CopyOptions::default()).unwrap();
            assert_eq!(stats.backend, "fvid linear loudnorm");
            assert_eq!(stats.sample_frames, 2);
            let mut file = std::fs::File::open(&dest).unwrap();
            assert_eq!(
                crate::owned_wave_inspect::inspect(&mut file, None)
                    .unwrap()
                    .channel_mask,
                3
            );
            let (_, bytes) = crate::owned_audio_mix::decode_float_wave(&dest).unwrap();
            for (actual, value) in bytes.chunks_exact(4).zip([0.25, -0.25, 0., 0.25]) {
                assert_eq!(
                    f32::from_le_bytes(actual.try_into().unwrap()),
                    (value * 10f64.powf(6. / 20.)) as f32
                );
            }
        }
        let source = dir.join("double.wav");
        let dest = dir.join("double-out.wav");
        crate::owned_wav_file::write_wav_f64le(&source, 96000, 1, &[0.1234567890123, -0.2])
            .unwrap();
        let cancel = fvid_control::CancelFlag::default();
        cancel.cancel();
        let options = CopyOptions {
            cancel: Some(cancel),
            ..CopyOptions::default()
        };
        assert!(apply_loudnorm(&source, &dest, Some(ARGS), &options).is_err());
        assert!(!dest.exists());
        apply_loudnorm(&source, &dest, Some(ARGS), &CopyOptions::default()).unwrap();
        let (_, bytes) = crate::owned_audio_mix::decode_float_wave(&dest).unwrap();
        assert_eq!(
            f32::from_le_bytes(bytes[..4].try_into().unwrap()),
            (0.1234567890123 * 10f64.powf(6. / 20.)) as f32
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn linear_mode_preserves_gain_rules_and_dynamic_mode_boundaries() {
        let expected = 10f64.powf(6. / 20.);
        assert_eq!(linear_gain(ARGS).unwrap(), Some(expected));
        assert_eq!(
            linear_gain(&format!("{ARGS}:offset=20")).unwrap(),
            Some(expected)
        );
        assert_eq!(
            linear_gain(&format!("{ARGS}:i=-17")).unwrap(),
            Some(10f64.powf(5. / 20.))
        );
        for args in [
            "I=-16:TP=-1.5:LRA=11".to_owned(),
            format!("{ARGS}:linear=false"),
            format!("{ARGS}:measured_TP=-3"),
            format!("{ARGS}:measured_LRA=12"),
            format!("{ARGS}:measured_LRA=0"),
            format!("{ARGS}:measured_thresh=-70"),
        ] {
            assert_eq!(linear_gain(&args).unwrap(), None, "{args}");
        }
        for suffix in [
            "I=-4",
            "TP=1",
            "measured_I=nan",
            "unknown=2",
            "linear=maybe",
        ] {
            assert!(linear_gain(&format!("{ARGS}:{suffix}")).is_err());
        }
    }
    #[test]
    fn owned_float_export_keeps_rate_frames_gain_and_controls() {
        let dir = std::env::temp_dir().join(format!("fvid-linear-loudnorm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.wav");
        let dest = dir.join("out.wav");
        let _ = std::fs::remove_file(&dest);
        let pcm: Vec<f32> = (0..10003)
            .map(|i| ((i % 251) as f32 - 125.) / 1024.)
            .collect();
        let _ = std::fs::remove_file(&source);
        crate::owned_wav_file::write_wav_f32le(&source, 44100, 1, &pcm).unwrap();
        let stats = apply_loudnorm(&source, &dest, Some(ARGS), &CopyOptions::default()).unwrap();
        assert_eq!(stats.sample_rate, 44100);
        assert_eq!(stats.sample_frames, 10003);
        let (_, bytes) = crate::owned_audio_mix::decode_float_wave(&dest).unwrap();
        for (sample, actual) in pcm.iter().zip(bytes.chunks_exact(4)) {
            assert_eq!(
                f32::from_le_bytes(actual.try_into().unwrap()),
                (f64::from(*sample) * 10f64.powf(6. / 20.)) as f32
            );
        }
        std::fs::remove_file(&dest).unwrap();
        let options = CopyOptions {
            max_packets: Some(1),
            max_packet_bytes: 1000,
            ..CopyOptions::default()
        };
        assert_eq!(
            apply_loudnorm(&source, &dest, Some(ARGS), &options)
                .unwrap()
                .sample_frames,
            250
        );
        std::fs::remove_file(&dest).unwrap();
        let options = CopyOptions {
            max_controlled_bytes: Some(8),
            ..CopyOptions::default()
        };
        assert!(apply_loudnorm(&source, &dest, Some(ARGS), &options).is_err());
        assert!(!dest.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
