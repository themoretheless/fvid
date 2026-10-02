//! Loudness normalization with owned WAVE decoding and publication.
//! Dynamic loudness compression is deliberately a separate implementation.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{LoudnormStats, resolve_loudnorm_args};
use std::path::Path;
type Result<T> = std::result::Result<T, String>;

/// Return a gain only when the requested parameters select linear processing.
/// Offset is a dynamic-mode parameter: linear gain is target I minus measured I.
struct Parameters {
    target_i: f64,
    target_tp: f64,
    dual_mono: bool,
    print: bool,
    gain: Option<f64>,
}
#[cfg(test)]
fn linear_gain(args: &str) -> Result<Option<f64>> {
    Ok(parse(args)?.gain)
}
fn parse(args: &str) -> Result<Parameters> {
    let (mut target_i, mut target_tp, mut target_lra) = (-24., -2., 7.);
    let (mut measured_i, mut measured_tp, mut measured_lra, mut threshold) = (0., 99., 0., -70.);
    let mut linear = true;
    let mut dual_mono = false;
    let mut print = false;
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
            } else {
                dual_mono = flag;
            }
            continue;
        }
        if key == "print_format" {
            if !matches!(value, "none" | "json" | "summary") {
                return Err("invalid loudnorm print format".into());
            }
            // Printing measurements needs its own meter and remains outside this path.
            if value != "none" {
                print = true;
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
    Ok(Parameters {
        target_i,
        target_tp,
        dual_mono,
        print,
        gain: (linear
            && !print
            && measured_i != 0.
            && measured_tp != 99.
            && measured_lra != 0.
            && threshold != -70.
            && measured_tp + gain_db <= target_tp
            && measured_lra <= target_lra)
            .then(|| 10f64.powf(gain_db / 20.)),
    })
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
        && parse(args).is_ok_and(|params| {
            !params.print
                && std::fs::File::open(source).is_ok_and(|mut file| {
                    crate::owned_wave_inspect::inspect(&mut file, None)
                        .is_ok_and(|info| params.gain.is_some() || short_input(&info, options))
                })
        })
}

pub fn apply_loudnorm(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    let args = resolve_loudnorm_args(args)?;
    let params = parse(&args)?;
    if params.print {
        return Err("owned loudnorm measurement printing is not yet implemented".into());
    }
    if params.gain.is_none() {
        return apply_short(source, destination, args, params, options);
    }
    let gain = params.gain.unwrap();
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

/// The dynamic filter uses a whole-file gain for recordings below its 3 s
/// initial window, measured after conversion to its 192 kHz processing clock.
fn short_input(info: &crate::owned_wave_inspect::WaveInfo, options: &CopyOptions) -> bool {
    let frame = usize::from(info.block);
    let capacity = (4096 * frame).min(options.max_packet_bytes / frame * frame);
    let frames = u128::from(info.sample_frames).min(
        options
            .max_packets
            .map(|n| u128::from(n) * (capacity / frame) as u128)
            .unwrap_or(u128::MAX),
    );
    frames > 0
        && (frames * 192000).div_ceil(u128::from(info.sample_rate)) < 576000
        && crate::owned_wave_loudness::weights(info).is_ok()
        && (8000..=384000).contains(&info.sample_rate)
}
fn apply_short(
    source: &Path,
    destination: &Path,
    args: String,
    params: Parameters,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    use std::io::Write;
    if !policies(options) || destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err("owned loudnorm requires WAVE output, stream 0 and no metadata edits".into());
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
    let mut width = 0;
    let mut float = false;
    let mut mask = 0;
    let mut weights = Vec::new();
    let mut samples = 0usize;
    let (input, raw) = crate::owned_audio_mix::read_wave_with_admission(
        source,
        None,
        false,
        options.cancel.as_ref(),
        options.max_packet_bytes,
        options.max_packets,
        |_, info, size| {
            if !short_input(info, options) {
                return Err("owned long-form dynamic loudnorm is not yet implemented".into());
            }
            width = usize::from(info.bits_per_sample / 8);
            float = info.float;
            mask = info.channel_mask;
            weights = crate::owned_wave_loudness::weights(info)?;
            if params.dual_mono && info.channels == 1 {
                weights[0] *= 2.;
            }
            let frames = size / usize::from(info.block);
            samples = usize::try_from(
                (frames as u128 * 192000).div_ceil(u128::from(info.sample_rate))
                    * u128::from(info.channels),
            )
            .map_err(|_| "loudnorm output size overflow")?;
            // Raw input + exact output double storage + f32 publication buffer,
            // bounded resampler queues/scratch and the meter's 3.4 s energy rings.
            let estimate = (size as u128)
                + samples as u128 * 12
                + 192000 * 34 / 10 * 8
                + 512 * 1024
                + 2 * frames.min(4096) as u128 * u128::from(info.channels) * 8;
            if options
                .max_controlled_bytes
                .is_some_and(|limit| estimate > limit as u128)
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
    let mut data = Vec::new();
    data.try_reserve_exact(samples.checked_mul(8).ok_or("loudnorm size overflow")?)
        .map_err(|_| "cannot allocate loudnorm processing buffer")?;
    let mut resampler = crate::owned_resample_f64::Resampler::new(
        &mut data,
        input.sample_rate as u32,
        192000,
        input.channels as u16,
    )
    .map_err(|e| e.to_string())?;
    for block in raw.chunks(width * input.channels as usize * 4096) {
        check()?;
        let mut pcm = Vec::with_capacity(block.len() / width * 8);
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
            pcm.extend_from_slice(&value.to_le_bytes());
        }
        resampler.write_all(&pcm).map_err(|e| e.to_string())?;
    }
    let frames = resampler.finish().map_err(|e| e.to_string())?;
    drop(resampler);
    drop(raw);
    let mut meter = crate::owned_loudness::LoudnessMeter::new(192000, &weights)?;
    let mut peak = 0f64;
    for block in data.chunks(8 * input.channels as usize * 4096) {
        check()?;
        let pcm: Vec<f64> = block
            .chunks_exact(8)
            .map(|bytes| f64::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        for &value in &pcm {
            peak = peak.max(value.abs());
        }
        meter.push(&pcm)?;
    }
    let desired = meter
        .histogram_integrated_lufs()
        .map(|level| 10f64.powf((params.target_i - level) / 20.))
        .unwrap_or(f64::INFINITY);
    let gain = if peak == 0. {
        1.
    } else {
        desired.min(10f64.powf(params.target_tp / 20.) / peak)
    };
    drop(meter);
    let mut output = Vec::new();
    output
        .try_reserve_exact(samples)
        .map_err(|_| "cannot allocate loudnorm output")?;
    for block in data.chunks(8 * input.channels as usize * 4096) {
        check()?;
        for bytes in block.chunks_exact(8) {
            let value = (f64::from_le_bytes(bytes.try_into().unwrap()) * gain) as f32;
            if !value.is_finite() {
                return Err("non-finite loudnorm output".into());
            }
            output.push(value);
        }
    }
    drop(data);
    crate::owned_wav_file::write_wav_f32le_with_side_data_checked(
        destination,
        192000,
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
        backend: "fvid short loudnorm",
        sample_frames: frames,
        sample_rate: 192000,
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
    fn histogram_fixture_accepts_expected_gain_instead_of_exact_energy_gain() {
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = fixtures.join("loudnorm-histogram.wav");
        let video = std::fs::read(fixtures.join("loudnorm-histogram.y4m")).unwrap();
        let header = b"YUV4MPEG2 W16 H16 F30:1 Ip A1:1 C420jpeg\n";
        assert!(video.starts_with(header));
        assert_eq!(video.len() - header.len(), 15 * (6 + 384));
        let dest = std::env::temp_dir().join(format!(
            "fvid-histogram-regression-{}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&dest);
        let result = crate::apply_loudnorm(&source, &dest, None, &CopyOptions::default()).unwrap();
        assert_eq!(result.sample_frames, 96000);
        assert_eq!(result.backend, "fvid short loudnorm");
        let (_, output) = crate::owned_audio_mix::decode_float_wave(&dest).unwrap();
        // Independent bin-center oracle; exact-energy gain differs by about 0.02 dB.
        let amplitude = (328. / 32768. * 10f64.powf((-16. + 36.65) / 20.)) as f32;
        let source_pcm: Vec<f64> = (0..96000)
            .map(|i| {
                if i % 2 == 0 {
                    328. / 32768.
                } else {
                    -328. / 32768.
                }
            })
            .collect();
        let mut previous = crate::owned_loudness::LoudnessMeter::new(192000, &[1.]).unwrap();
        previous.push(&source_pcm).unwrap();
        let previous_gain = 10f64.powf((-16. - previous.report().integrated_lufs.unwrap()) / 20.);
        assert!(
            ((328. / 32768. * previous_gain) as f32 - amplitude).abs() > 0.0002,
            "fixture must distinguish the former exact-energy gain decision"
        );
        for (i, sample) in output.chunks_exact(4).enumerate() {
            let expected = if i % 2 == 0 { amplitude } else { -amplitude };
            assert_eq!(
                f32::from_le_bytes(sample.try_into().unwrap()),
                expected,
                "sample {i}"
            );
        }
        std::fs::remove_file(dest).unwrap();
    }
    #[test]
    fn short_dynamic_peak_gain_silence_dual_mono_and_duration_boundary() {
        let dir = std::env::temp_dir().join(format!("fvid-short-loudnorm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.wav");
        let output = dir.join("out.wav");
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&output);
        // Too short for a 400 ms loudness block: the peak ceiling determines gain.
        crate::owned_wav_file::write_wav_f64le(&source, 192000, 2, &[0.5, -0.25, 0., 0.125])
            .unwrap();
        let stats = crate::apply_loudnorm(&source, &output, None, &CopyOptions::default()).unwrap();
        assert_eq!(stats.backend, "fvid short loudnorm");
        assert_eq!(stats.sample_rate, 192000);
        assert_eq!(stats.sample_frames, 2);
        let (_, pcm) = crate::owned_audio_mix::decode_float_wave(&output).unwrap();
        let peak = 10f64.powf(-1.5 / 20.);
        for (sample, fraction) in pcm.chunks_exact(4).zip([1., -0.5, 0., 0.25]) {
            assert_eq!(
                f32::from_le_bytes(sample.try_into().unwrap()),
                (peak * fraction) as f32
            );
        }
        std::fs::remove_file(&source).unwrap();
        std::fs::remove_file(&output).unwrap();
        let tone: Vec<f64> = (0..96000)
            .map(|i| 0.01 * (std::f64::consts::TAU * 1000. * i as f64 / 192000.).sin())
            .collect();
        crate::owned_wav_file::write_wav_f64le(&source, 192000, 1, &tone).unwrap();
        apply_loudnorm(&source, &output, None, &CopyOptions::default()).unwrap();
        let (_, normal) = crate::owned_audio_mix::decode_float_wave(&output).unwrap();
        std::fs::remove_file(&output).unwrap();
        apply_loudnorm(
            &source,
            &output,
            Some("I=-16:TP=-1.5:LRA=11:dual_mono=true"),
            &CopyOptions::default(),
        )
        .unwrap();
        let (_, dual) = crate::owned_audio_mix::decode_float_wave(&output).unwrap();
        for (a, b) in normal.chunks_exact(4).zip(dual.chunks_exact(4)) {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            // 0.1 LU histogram decisions quantize the exact +3.0103 LU shift.
            if a.abs() > 1e-5 {
                assert!((20. * (b / a).log10() + 10. * 2f32.log10()).abs() < 0.1);
            }
        }
        std::fs::remove_file(&source).unwrap();
        std::fs::remove_file(&output).unwrap();
        crate::owned_wav_file::write_wav_f32le(&source, 8000, 1, &[0.; 4]).unwrap();
        assert_eq!(
            apply_loudnorm(&source, &output, None, &CopyOptions::default())
                .unwrap()
                .sample_frames,
            96
        );
        let (_, pcm) = crate::owned_audio_mix::decode_float_wave(&output).unwrap();
        assert!(
            pcm.chunks_exact(4)
                .all(|v| f32::from_le_bytes(v.try_into().unwrap()) == 0.)
        );
        let mut file = std::fs::File::open(&source).unwrap();
        let mut info = crate::owned_wave_inspect::inspect(&mut file, None).unwrap();
        info.sample_frames = 24000;
        assert!(!short_input(&info, &CopyOptions::default()));
        info.sample_frames = 23999;
        assert!(short_input(&info, &CopyOptions::default()));
        std::fs::remove_dir_all(dir).unwrap();
    }
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
