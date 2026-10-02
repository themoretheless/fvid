//! Loudness normalization with owned WAVE decoding and publication.
//! Dynamic loudness compression is deliberately a separate implementation.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{LoudnormStats, resolve_loudnorm_args};
use std::path::Path;
type Result<T> = std::result::Result<T, String>;
mod report;
use report::{measure_phase, output_report, print_report, report_memory};

/// Return a gain only when the requested parameters select linear processing.
/// Offset is a dynamic-mode parameter: linear gain is target I minus measured I.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Print {
    None,
    Json,
    Summary,
}
struct Parameters {
    target_i: f64,
    target_tp: f64,
    target_lra: f64,
    offset: f64,
    dual_mono: bool,
    print: Print,
    gain: Option<f64>,
}
#[cfg(test)]
fn linear_gain(args: &str) -> Result<Option<f64>> {
    Ok(parse(args)?.gain)
}
fn parse(args: &str) -> Result<Parameters> {
    let (mut target_i, mut target_tp, mut target_lra) = (-24., -2., 7.);
    let (mut measured_i, mut measured_tp, mut measured_lra, mut threshold) = (0., 99., 0., -70.);
    let mut offset = 0.;
    let mut linear = true;
    let mut dual_mono = false;
    let mut print = Print::None;
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
            // Printing measurements needs its own report and remains outside this path.
            print = match value {
                "json" => Print::Json,
                "summary" => Print::Summary,
                _ => Print::None,
            };
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
                offset = number;
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
        target_lra,
        offset,
        dual_mono,
        print,
        gain: (linear
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
pub(crate) fn supports(
    source: &Path,
    destination: &Path,
    args: &str,
    options: &CopyOptions,
) -> bool {
    policies(options)
        && destination.extension().and_then(|s| s.to_str()) == Some("wav")
        && parse(args).is_ok_and(|params| {
            (params.print == Print::None || crate::owned_wave_loudness::supports(source, options))
                && std::fs::File::open(source).is_ok_and(|mut file| {
                    crate::owned_wave_inspect::inspect(&mut file, None)
                        .is_ok_and(|info| params.gain.is_some() || dynamic_input(&info))
                })
        })
}

/// Resolve defaults and validate the same parameter names/ranges as execution.
pub fn validate_request(args: Option<&str>) -> Result<String> {
    let resolved = resolve_loudnorm_args(args)?;
    parse(&resolved)?;
    Ok(resolved)
}

/// Whether execution measures and prints an input/output normalization report.
pub fn report_requested(args: Option<&str>) -> Result<bool> {
    Ok(parse(&resolve_loudnorm_args(args)?)?.print != Print::None)
}

/// Whether this request has an owned WAVE route. Admission and cancellation
/// remain execution checks; a false result lets migrating callers retain other
/// formats without changing their behavior.
pub fn supports_request(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> bool {
    let Ok(args) = resolve_loudnorm_args(args) else {
        return false;
    };
    if dual_pass {
        supports_dual(source, destination, &args, options)
    } else {
        supports(source, destination, &args, options)
    }
}

/// Metadata-only plan for the owned WAVE normalizer. Does not decode samples,
/// measure loudness, emit progress or create output files.
pub fn plan_loudnorm(
    source: &Path,
    args: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> Result<fvid_media_info::MediaPlan> {
    if crate::owned_adts_export::recognizes(source)? {
        return crate::owned_adts_loudnorm::plan_loudnorm(source, args, dual_pass, options);
    }
    use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
    let resolved = validate_request(args)?;
    if !supports_request(
        source,
        Path::new("output.wav"),
        Some(&resolved),
        dual_pass,
        options,
    ) {
        return Err("request has no owned WAVE loudnorm route".into());
    }
    let info = crate::owned_probe::probe_wave(source).map_err(|e| e.to_string())?;
    let report = report_requested(Some(&resolved))?;
    let mut steps = vec![PlanStep {
        action: "decode".into(),
        detail: "owned WAVE reader retaining PCM precision and speaker layout for f64 processing"
            .into(),
    }];
    if dual_pass || report {
        steps.push(PlanStep { action: "analyze".into(), detail: if dual_pass {
            "owned K-weighting, loudness gating, LRA and true peak; select measured linear gain when range/peak margins permit"
        } else {
            "owned input loudness/true-peak measurement for the requested report; retain the configured normalization mode"
        }.into() });
    }
    steps.push(PlanStep { action: "filter".into(), detail: format!("owned loudnorm targets {resolved}; eligible measured linear gain, otherwise owned dynamic controller and linked true-peak limiter") });
    if report {
        steps.push(PlanStep { action: "analyze-output".into(), detail: "owned output loudness/true-peak measurement before publication; print JSON/summary after successful publication".into() });
    }
    steps.push(PlanStep { action: "write".into(), detail: "float32 WAVE; linear retains source clock, dynamic uses 192 kHz; atomic publication without overwrite; report/completion only after publication".into() });
    let mut notes = vec![
        "normalization backend: fvid; dynamic PCM is not claimed equivalent to libavfilter".into(),
        "measurement values, mode selection, packet contents and memory admission are execution checks".into(),
    ];
    if let Some(maximum) = options.max_packets {
        notes.push(format!(
            "read at most {maximum} original WAVE PCM blocks per pass"
        ));
    }
    Ok(MediaPlan {
        command: "loudnorm".into(),
        input: source.into(),
        inputs: vec![source.into()],
        streams: vec![PlanStream {
            index: 0,
            media_type: "audio".into(),
            codec: info.streams[0].codec.clone(),
            disposition: "decode".into(),
        }],
        steps,
        graph: None,
        notes,
    })
}

pub fn apply_loudnorm(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    if crate::owned_adts_export::recognizes(source)? {
        return crate::owned_adts_loudnorm::apply(source, destination, args, false, options);
    }
    let args = resolve_loudnorm_args(args)?;
    let params = parse(&args)?;
    if params.print != Print::None {
        let (measurement, export_options) = measure_phase(source, params.dual_mono, options)?;
        return apply_resolved(
            source,
            destination,
            args,
            params,
            &export_options,
            Some(&measurement),
        );
    }
    apply_resolved(source, destination, args, params, options, None)
}
/// Measure with the owned meter, then select a linear gain when the measured
/// range/true peak permit it; otherwise use the owned dynamic controller.
pub fn apply_loudnorm_dual(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    if crate::owned_adts_export::recognizes(source)? {
        return crate::owned_adts_loudnorm::apply(source, destination, args, true, options);
    }
    let base = resolve_loudnorm_args(args)?;
    let original = parse(&base)?;
    let (measurement, export_options) = measure_phase(source, original.dual_mono, options)?;
    let clean = base
        .split(':')
        .filter(|item| !item.starts_with("print_format="))
        .collect::<Vec<_>>()
        .join(":");
    let measured_i = measurement.integrated_lufs.unwrap_or(-70.).clamp(-99., 0.);
    let measured_tp = measurement.stats.true_peak_dbfs.clamp(-99., 99.);
    let mut pass2 = format!(
        "{clean}:measured_I={measured_i}:measured_TP={measured_tp}:measured_LRA={}:measured_thresh={}:linear=true",
        measurement.stats.range_lu.clamp(0., 99.),
        measurement.relative_thresh.clamp(-99., 0.)
    );
    match original.print {
        Print::Json => pass2.push_str(":print_format=json"),
        Print::Summary => pass2.push_str(":print_format=summary"),
        Print::None => {}
    }
    let params = parse(&pass2)?;
    let mut stats = apply_resolved(
        source,
        destination,
        pass2,
        params,
        &export_options,
        Some(&measurement),
    )?;
    stats.dual_pass = true;
    Ok(stats)
}
pub(crate) fn supports_dual(
    source: &Path,
    destination: &Path,
    args: &str,
    options: &CopyOptions,
) -> bool {
    destination.extension().and_then(|s| s.to_str()) == Some("wav")
        && parse(args).is_ok()
        && crate::owned_wave_loudness::supports(source, options)
}
fn apply_resolved(
    source: &Path,
    destination: &Path,
    args: String,
    params: Parameters,
    options: &CopyOptions,
    measurement: Option<&crate::owned_wave_loudness::NormalizationMeasurement>,
) -> Result<LoudnormStats> {
    if params.gain.is_none() {
        return apply_dynamic(source, destination, args, params, options, measurement);
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
            let report_bytes = if params.print == Print::None {
                0
            } else {
                usize::try_from(report_memory(
                    info.sample_rate,
                    (size / usize::from(info.block)) as u64,
                    info.channels,
                ))
                .map_err(|_| "loudnorm report memory overflow")?
            };
            let estimate = size
                .checked_add(output)
                .and_then(|n| n.checked_add(report_bytes))
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
    let report = output_report(
        &output,
        input.sample_rate as u32,
        &params,
        measurement,
        &check,
    )?;
    crate::owned_wav_file::write_wav_f32le_with_side_data_checked(
        destination,
        input.sample_rate,
        input.channels,
        &output,
        mask,
        &[],
        check,
    )?;
    if let Some(report) = report {
        print_report(params.print, report, params.target_i, "linear");
    }
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
fn dynamic_input(info: &crate::owned_wave_inspect::WaveInfo) -> bool {
    crate::owned_wave_loudness::weights(info).is_ok() && (8000..=384000).contains(&info.sample_rate)
}
#[cfg(test)]
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
fn apply_dynamic(
    source: &Path,
    destination: &Path,
    args: String,
    params: Parameters,
    options: &CopyOptions,
    measurement: Option<&crate::owned_wave_loudness::NormalizationMeasurement>,
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
            if !dynamic_input(info) {
                return Err(
                    "owned dynamic loudnorm requires a qualified rate and speaker layout".into(),
                );
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
                + 2 * frames.min(4096) as u128 * u128::from(info.channels) * 8
                + samples as u128 / u128::from(info.channels) * 8
                // Four anchor vectors plus both meter histograms (every block
                // may occupy a distinct bin; 512 bytes per tree node/scratch).
                + (samples as u128 / u128::from(info.channels)).div_ceil(19200) * (40 + 1024)
                + 19200 * u128::from(info.channels) * 8;
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
    let global = meter.histogram_integrated_lufs();
    drop(meter);
    let output = if frames >= 576000 {
        crate::owned_dynamic_loudnorm::process(
            &data,
            &weights,
            global,
            params.target_i,
            params.target_lra,
            params.target_tp,
            params.offset,
            &check,
        )?
    } else {
        let desired = global
            .map(|level| 10f64.powf((params.target_i - level) / 20.))
            .unwrap_or(f64::INFINITY);
        let gain = if peak == 0. {
            1.
        } else {
            desired.min(10f64.powf(params.target_tp / 20.) / peak)
        };
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
        output
    };
    drop(data);
    let report = output_report(&output, 192000, &params, measurement, &check)?;
    crate::owned_wav_file::write_wav_f32le_with_side_data_checked(
        destination,
        192000,
        input.channels,
        &output,
        mask,
        &[],
        check,
    )?;
    if let Some(report) = report {
        print_report(
            params.print,
            report,
            params.target_i,
            if frames >= 576000 {
                "dynamic"
            } else {
                "linear"
            },
        );
    }
    event.done = true;
    if let Some(hook) = &options.progress {
        hook.emit(event);
    }
    Ok(LoudnormStats {
        backend: if frames >= 576000 {
            "fvid dynamic loudnorm"
        } else {
            "fvid short loudnorm"
        },
        sample_frames: frames,
        sample_rate: 192000,
        channels: input.channels,
        args,
        dual_pass: false,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn public_wave_plan_uses_owned_backend_with_or_without_legacy() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/loudnorm-dual.wav");
        let options = CopyOptions {
            max_packets: Some(3),
            progress: Some(fvid_control::ProgressHook::new(|_| {
                panic!("plan emitted progress")
            })),
            ..Default::default()
        };
        for dual in [false, true] {
            let plan =
                crate::plan_loudnorm(&source, Some("I=-16:print_format=json"), dual, &options)
                    .unwrap();
            assert!(plan.graph.is_none());
            assert_eq!(plan.streams[0].codec, "pcm_s16le");
            assert_eq!(
                plan.steps
                    .iter()
                    .map(|s| s.action.as_str())
                    .collect::<Vec<_>>(),
                ["decode", "analyze", "filter", "analyze-output", "write"]
            );
            assert!(
                plan.notes
                    .iter()
                    .any(|s| s.contains("3 original WAVE PCM blocks"))
            );
            let own =
                super::plan_loudnorm(&source, Some("I=-16:print_format=json"), dual, &options)
                    .unwrap();
            assert_eq!(
                serde_json::to_value(plan).unwrap(),
                serde_json::to_value(own).unwrap()
            );
        }
        let no_report = crate::plan_loudnorm(
            &source,
            Some("print_format=json:print_format=none"),
            false,
            &options,
        )
        .unwrap();
        assert_eq!(no_report.steps.len(), 3);
        for args in ["I=0", "unknown=1", "print_format=maybe"] {
            assert!(super::plan_loudnorm(&source, Some(args), false, &options).is_err());
        }
    }

    use super::*;
    const ARGS: &str = "I=-16:TP=-1.5:LRA=11:measured_I=-22:measured_TP=-12:measured_LRA=2:measured_thresh=-32:linear=true";
    #[test]
    fn dual_measurement_cancel_does_not_publish_or_signal_completion() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/loudnorm-dual.wav");
        let dest =
            std::env::temp_dir().join(format!("fvid-dual-cancel-{}.wav", std::process::id()));
        let _ = std::fs::remove_file(&dest);
        let cancel = fvid_control::CancelFlag::default();
        let hook_cancel = cancel.clone();
        let options = CopyOptions {
            cancel: Some(cancel),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                assert!(!event.done, "measurement must not finish the export");
                if event.payload_bytes >= 384000 * 2 {
                    hook_cancel.cancel();
                }
            })),
            ..CopyOptions::default()
        };
        assert!(apply_loudnorm_dual(&source, &dest, None, &options).is_err());
        assert!(!dest.exists());
    }

    #[test]
    fn dual_zero_lra_uses_dynamic_fallback_without_ffmpeg() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/loudnorm-dynamic.wav");
        let dest =
            std::env::temp_dir().join(format!("fvid-dual-dynamic-{}.wav", std::process::id()));
        let stats = apply_loudnorm_dual(&source, &dest, None, &CopyOptions::default()).unwrap();
        assert!(stats.dual_pass);
        assert_eq!(stats.backend, "fvid dynamic loudnorm");
        assert_eq!(stats.sample_frames, 576000);
        std::fs::remove_file(dest).unwrap();
    }

    #[test]
    fn feedback_fixture_recovers_loudness_between_isolated_peaks() {
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = fixtures.join("loudnorm-feedback.wav");
        let video = std::fs::read(fixtures.join("loudnorm-feedback.y4m")).unwrap();
        assert_eq!(
            video.len(),
            b"YUV4MPEG2 W16 H16 F30:1 Ip A1:1 C420jpeg\n".len() + 90 * 390
        );
        let dest = std::env::temp_dir().join(format!(
            "fvid-feedback-regression-{}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&dest);
        let stats = crate::apply_loudnorm(&source, &dest, None, &CopyOptions::default()).unwrap();
        assert_eq!(stats.backend, "fvid dynamic loudnorm");
        assert_eq!(stats.sample_frames, 576000);
        let report =
            crate::owned_wave_loudness::measure_loudness(&dest, &CopyOptions::default()).unwrap();
        assert!(
            (report.integrated_lufs + 16.).abs() < 0.2,
            "{}",
            report.integrated_lufs
        );
        assert!(
            report.true_peak_dbfs <= -1.5 + 1e-5,
            "{}",
            report.true_peak_dbfs
        );
        std::fs::remove_file(dest).unwrap();
    }
    #[test]
    fn long_dynamic_fixture_accepts_public_processing_at_three_seconds() {
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = fixtures.join("loudnorm-dynamic.wav");
        let mut file = std::fs::File::open(&source).unwrap();
        let info = crate::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert!(!short_input(&info, &CopyOptions::default()));
        let video = std::fs::read(fixtures.join("loudnorm-dynamic.y4m")).unwrap();
        let header = b"YUV4MPEG2 W16 H16 F30:1 Ip A1:1 C420jpeg\n";
        assert!(video.starts_with(header));
        assert_eq!(video.len() - header.len(), 90 * 390);
        let dest = std::env::temp_dir().join(format!(
            "fvid-dynamic-regression-{}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&dest);
        let options = CopyOptions {
            max_controlled_bytes: Some(24 * 1024 * 1024),
            ..CopyOptions::default()
        };
        // Admission precedes retained PCM and DSP allocations.
        assert!(
            crate::apply_loudnorm(
                &source,
                &dest,
                None,
                &CopyOptions {
                    max_controlled_bytes: Some(8),
                    ..CopyOptions::default()
                }
            )
            .is_err()
        );
        assert!(!dest.exists());
        let stats = crate::apply_loudnorm(&source, &dest, None, &options).unwrap();
        assert_eq!(stats.backend, "fvid dynamic loudnorm");
        assert_eq!(stats.sample_frames, 576000);
        assert_eq!(stats.sample_rate, 192000);
        let measured =
            crate::owned_wave_loudness::measure_loudness(&dest, &CopyOptions::default()).unwrap();
        assert!(
            (measured.integrated_lufs + 16.).abs() < 0.3,
            "{}",
            measured.integrated_lufs
        );
        assert!(
            measured.true_peak_dbfs <= -1.5 + 1e-5,
            "{}",
            measured.true_peak_dbfs
        );
        let (_, bytes) = crate::owned_audio_mix::decode_float_wave(&dest).unwrap();
        assert_eq!(bytes.len(), 576000 * 4);
        assert!(bytes.chunks_exact(4).enumerate().all(|(i, sample)| {
            let value = f32::from_le_bytes(sample.try_into().unwrap());
            value.is_finite() && if i % 2 == 0 { value > 0. } else { value < 0. }
        }));
        std::fs::remove_file(dest).unwrap();
    }
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
            linear_gain(&format!("{ARGS}:print_format=json:print_format=none")).unwrap(),
            Some(expected)
        );
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
