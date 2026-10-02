//! Streaming WAVE loudness and true peak, with owned PCM decoding.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{LoudnessStats, MediaPlan, PlanStep, PlanStream};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
/// Standard sample rates qualified for owned loudness and true-peak measurement.
pub fn qualified_rate(rate: u32) -> bool {
    matches!(
        rate,
        8000 | 12000
            | 16000
            | 22050
            | 24000
            | 32000
            | 44100
            | 48000
            | 88200
            | 96000
            | 176400
            | 192000
            | 352800
            | 384000
    )
}
fn policies(options: &CopyOptions) -> bool {
    (options.streams.is_empty() || options.streams == [0])
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
}
fn check(options: &CopyOptions) -> Result<()> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)
}
pub(crate) fn weights(info: &crate::owned_wave_inspect::WaveInfo) -> Result<Vec<f64>> {
    let mask = if info.channel_mask == 0 {
        match info.channels {
            1 => 4,
            2 => 3,
            _ => return Err("loudness requires an explicit multichannel speaker mask".into()),
        }
    } else {
        info.channel_mask
    };
    if mask & !0x7ff != 0 {
        return Err("owned loudness speaker positions are not yet qualified".into());
    }
    Ok((0..11)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| match bit {
            3 => 0.,
            4 | 5 | 8 | 9 | 10 => 1.41,
            _ => 1.,
        })
        .collect())
}
pub(crate) fn supports(source: &Path, options: &CopyOptions) -> bool {
    policies(options)
        && File::open(source).is_ok_and(|mut file| {
            crate::owned_wave_inspect::inspect(&mut file, None)
                .is_ok_and(|info| qualified_rate(info.sample_rate) && weights(&info).is_ok())
        })
}
struct Input {
    file: File,
    info: crate::owned_wave_inspect::WaveInfo,
    weights: Vec<f64>,
    capacity: usize,
    size: usize,
}
fn preflight(source: &Path, options: &CopyOptions) -> Result<Input> {
    if !policies(options) {
        return Err("owned WAVE loudness requires stream 0 and no metadata edits".into());
    }
    check(options)?;
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let info = crate::owned_wave_inspect::inspect(&mut file, options.cancel.as_ref())
        .map_err(|e| e.to_string())?;
    if !qualified_rate(info.sample_rate) {
        return Err(
            "owned WAVE true-peak measurement requires a qualified standard PCM rate".into(),
        );
    }
    let weights = weights(&info)?;
    let frame = usize::from(info.block);
    let capacity = (4096 * frame).min(options.max_packet_bytes / frame * frame);
    if capacity == 0 {
        return Err("PCM packet limit cannot hold one sample frame".into());
    }
    let size = (u128::from(info.data_bytes)).min(
        options
            .max_packets
            .map(|n| u128::from(n) * capacity as u128)
            .unwrap_or(u128::from(info.data_bytes)),
    ) as usize;
    if size == 0 {
        return Err("loudness requires at least one audio frame".into());
    }
    let frames = size / frame;
    let rate = info.sample_rate as usize;
    let hop = rate / 10;
    let integrated_blocks = if frames < rate * 2 / 5 {
        0
    } else {
        1 + (frames - rate * 2 / 5) / hop
    };
    let short_blocks = if frames < rate * 3 {
        0
    } else {
        1 + (frames - rate * 3) / hop
    };
    // Every observed block could occupy a distinct histogram bin. 512 bytes
    // per bin conservatively includes tree nodes and report percentile scratch.
    // Ring storage is 3 s + 400 ms of energy; PCM buffers hold one block only.
    let estimated = (rate * 3 + rate * 2 / 5) as u128 * 8
        + capacity as u128
        + (capacity / frame) as u128 * u128::from(info.channels) * 8
        + u128::from(info.channels) * 512
        + (integrated_blocks + short_blocks) as u128 * 512
        + 16384;
    if options
        .max_controlled_bytes
        .is_some_and(|max| estimated > max as u128)
    {
        return Err(format!(
            "controlled memory budget exceeded: need {estimated} bytes"
        ));
    }
    check(options)?;
    Ok(Input {
        file,
        info,
        weights,
        capacity,
        size,
    })
}
/// Measure stored PCM directly; no FFmpeg decoder, filter or sample-rate converter.
/// Packet limits count aligned read blocks, and measurement includes their prefix.
/// True peak uses the owned Annex-2 FIR at >=48 kHz and sinc FIR below. Transient peaks may differ
/// from another implementation's interpolation filter; bit equivalence is not promised.
pub fn measure_loudness(source: &Path, options: &CopyOptions) -> Result<LoudnessStats> {
    Ok(measure_for_normalization(source, false, options)?.stats)
}
pub(crate) struct NormalizationMeasurement {
    pub stats: LoudnessStats,
    pub integrated_lufs: Option<f64>,
    pub relative_thresh: f64,
    pub weights: Vec<f64>,
}
pub(crate) fn measure_for_normalization(
    source: &Path,
    dual_mono: bool,
    options: &CopyOptions,
) -> Result<NormalizationMeasurement> {
    let mut input = preflight(source, options)?;
    if dual_mono && input.info.channels == 1 {
        input.weights[0] *= 2.;
    }
    let mut meter = crate::owned_loudness::LoudnessMeter::new_with_true_peak(
        input.info.sample_rate,
        &input.weights,
    )?;
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    let emit = |event| {
        if let Some(hook) = &options.progress {
            hook.emit(event);
        }
    };
    emit(event);
    check(options)?;
    let mut buffer = vec![0; input.capacity];
    let mut pcm = Vec::with_capacity(input.capacity / usize::from(input.info.bits_per_sample / 8));
    input
        .file
        .seek(SeekFrom::Start(input.info.data_offset))
        .map_err(|e| e.to_string())?;
    let width = usize::from(input.info.bits_per_sample / 8);
    let mut remaining = input.size;
    while remaining > 0 {
        check(options)?;
        let bytes = remaining.min(buffer.len());
        input
            .file
            .read_exact(&mut buffer[..bytes])
            .map_err(|e| e.to_string())?;
        pcm.clear();
        for sample in buffer[..bytes].chunks_exact(width) {
            let value = if input.info.float {
                if width == 4 {
                    f64::from(f32::from_le_bytes(sample.try_into().unwrap()))
                } else {
                    f64::from_le_bytes(sample.try_into().unwrap())
                }
            } else {
                use crate::owned_pcm_integer::Format;
                match width {
                    1 => Format::U8.decode(sample)?,
                    2 => Format::I16.decode(sample)?,
                    3 => Format::I32.decode(&[0, sample[0], sample[1], sample[2]])?,
                    _ => Format::I32.decode(sample)?,
                }
            };
            pcm.push(value);
        }
        meter.push(&pcm)?;
        remaining -= bytes;
        event.packets += 1;
        event.payload_bytes += bytes as u64;
        emit(event);
        check(options)?;
    }
    check(options)?;
    meter.finish();
    let report = meter.report();
    let peak = meter.true_peak_report().ok_or("true-peak meter missing")?;
    let (low, high) = meter.range_bounds().unwrap_or((0., 0.));
    check(options)?;
    let result = LoudnessStats {
        backend: "owned streaming WAVE loudness",
        sample_frames: report.sample_frames,
        sample_rate: input.info.sample_rate as i32,
        channels: i32::from(input.info.channels),
        integrated_lufs: report.integrated_lufs.unwrap_or(-70.),
        range_lu: report.range_lu.unwrap_or(0.),
        lra_low_lufs: low,
        lra_high_lufs: high,
        true_peak_dbfs: peak.true_peak_dbfs.unwrap_or(f64::NEG_INFINITY),
        sample_peak_dbfs: report.sample_peak_dbfs.unwrap_or(f64::NEG_INFINITY),
    };
    emit(ProgressEvent {
        done: true,
        ..event
    });
    Ok(NormalizationMeasurement {
        stats: result,
        integrated_lufs: report.integrated_lufs,
        relative_thresh: meter.relative_gate_lufs(),
        weights: input.weights,
    })
}
pub fn plan_loudness(source: &Path, options: &CopyOptions) -> Result<MediaPlan> {
    let input = preflight(source, options)?;
    Ok(MediaPlan{command:"loudness".into(),input:source.into(),inputs:vec![source.into()],streams:vec![PlanStream{index:0,media_type:"audio".into(),codec:input.info.codec(),disposition:"analyze".into()}],
        steps:vec![PlanStep{action:"read".into(),detail:format!("read {} PCM bytes at {} Hz in aligned blocks of at most {} bytes",input.size,input.info.sample_rate,input.capacity)},
        PlanStep{action:"analyze".into(),detail:"owned K-weighting, integrated loudness gating, loudness-range percentiles and owned true-peak FIR; no output file".into()}],graph:None,
        notes:vec!["backend: owned WAVE; no external decoder or filter".into(),"metadata-only plan; PCM numeric validity and signal measurements are checked during execution".into()]})
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
    fn files(name: &str) -> Files {
        let dir =
            std::env::temp_dir().join(format!("fvid-wave-loudness-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Files(dir)
    }
    #[test]
    fn all_pcm_precisions_have_identical_full_loudness_results() {
        let files = files("precision");
        let samples: Vec<f64> = (0..48000)
            .map(|i| if i % 4 < 2 { 0.25 } else { -0.25 })
            .collect();
        let mut previous: Option<LoudnessStats> = None;
        for (bits, float) in [
            (8u16, false),
            (16, false),
            (24, false),
            (32, false),
            (32, true),
            (64, true),
        ] {
            let path = files.0.join(format!("{bits}-{float}.wav"));
            if float && bits == 32 {
                let pcm: Vec<f32> = samples.iter().map(|s| *s as f32).collect();
                crate::owned_wav_file::write_wav_f32le(&path, 48000, 1, &pcm).unwrap();
            } else if float {
                crate::owned_wav_file::write_wav_f64le(&path, 48000, 1, &samples).unwrap();
            } else {
                use crate::owned_pcm_integer::Format;
                let format = match bits {
                    8 => Format::U8,
                    16 => Format::I16,
                    _ => Format::I32,
                };
                let mut pcm = Vec::new();
                for sample in &samples {
                    let mut encoded = [0; 4];
                    format
                        .encode(*sample, &mut encoded[..format.bytes()])
                        .unwrap();
                    if bits == 24 {
                        pcm.extend_from_slice(&encoded[1..4]);
                    } else {
                        pcm.extend_from_slice(&encoded[..format.bytes()]);
                    }
                }
                crate::owned_wav_file::write_wav_integer_le(&path, 48000, 1, bits, &pcm, 4)
                    .unwrap();
            }
            assert!(supports(&path, &CopyOptions::default()));
            let actual = crate::measure_loudness(&path, &CopyOptions::default()).unwrap();
            assert_eq!(actual.backend, "owned streaming WAVE loudness");
            assert_eq!(actual.sample_frames, 48000);
            assert_eq!(actual.sample_rate, 48000);
            assert_eq!(actual.channels, 1);
            assert_eq!(actual.range_lu, 0.);
            assert_eq!(actual.lra_low_lufs, 0.);
            assert_eq!(actual.lra_high_lufs, 0.);
            assert!((actual.sample_peak_dbfs - 20. * 0.25f64.log10()).abs() < 1e-12);
            assert!(actual.true_peak_dbfs > actual.sample_peak_dbfs + 2.9);
            assert!(actual.integrated_lufs.is_finite());
            if let Some(expected) = &previous {
                assert_eq!(actual.integrated_lufs, expected.integrated_lufs);
                assert_eq!(actual.true_peak_dbfs, expected.true_peak_dbfs);
                assert_eq!(actual.sample_peak_dbfs, expected.sample_peak_dbfs);
            }
            previous = Some(actual);
            let options = CopyOptions {
                max_packet_bytes: usize::from(bits / 8) * 48,
                max_packets: Some(2),
                progress: Some(fvid_control::ProgressHook::new(|event| {
                    if event.done {
                        assert_eq!(event.packets, 2);
                    }
                })),
                ..Default::default()
            };
            let prefix = crate::measure_loudness(&path, &options).unwrap();
            assert_eq!(prefix.sample_frames, 96);
            assert_eq!(prefix.integrated_lufs, -70.);
            let plan = crate::plan_loudness(&path, &options).unwrap();
            assert_eq!(plan.command, "loudness");
            assert!(plan.graph.is_none());
            assert_eq!(plan.streams[0].disposition, "analyze");
        }
    }
    #[test]
    fn silence_lfe_peak_and_numeric_refusal_are_explicit() {
        let files = files("silence");
        let silence = files.0.join("silence.wav");
        crate::owned_wav_file::write_wav_f32le(&silence, 48000, 1, &vec![0.; 24000]).unwrap();
        let result = crate::measure_loudness(&silence, &CopyOptions::default()).unwrap();
        assert_eq!(result.integrated_lufs, -70.);
        assert_eq!(result.range_lu, 0.);
        assert_eq!(result.true_peak_dbfs, f64::NEG_INFINITY);
        assert_eq!(result.sample_peak_dbfs, f64::NEG_INFINITY);
        let lfe = files.0.join("lfe.wav");
        let pcm: Vec<f64> = (0..48000)
            .flat_map(|i| [0., 0., if i % 4 < 2 { 0.25 } else { -0.25 }])
            .collect();
        crate::owned_wav_file::write_wav_f64le_with_side_data_checked(
            &lfe,
            48000,
            3,
            &pcm,
            0xb,
            &[],
            || Ok(()),
        )
        .unwrap();
        let result = crate::measure_loudness(&lfe, &CopyOptions::default()).unwrap();
        assert_eq!(result.integrated_lufs, -70.);
        assert!(result.true_peak_dbfs > result.sample_peak_dbfs + 2.9);
        let invalid = files.0.join("nan.wav");
        crate::owned_wav_file::write_wav_f64le(&invalid, 48000, 1, &[0., f64::NAN]).unwrap();
        assert!(
            crate::measure_loudness(&invalid, &CopyOptions::default())
                .unwrap_err()
                .contains("finite PCM")
        );
    }
    #[test]
    fn range_bounds_controls_and_metadata_only_plan_are_owned() {
        let files = files("control");
        let source = files.0.join("range.wav");
        let pcm: Vec<f32> = (0..48000 * 6)
            .map(|i| {
                let gain = if i < 48000 * 3 { 0.1 } else { 0.02 };
                (gain * (std::f64::consts::TAU * 1000. * i as f64 / 48000.).sin()) as f32
            })
            .collect();
        crate::owned_wav_file::write_wav_f32le(&source, 48000, 1, &pcm).unwrap();
        let result = crate::measure_loudness(&source, &CopyOptions::default()).unwrap();
        assert_eq!(result.sample_frames, 288000);
        assert!(result.range_lu > 1.);
        assert!(result.lra_high_lufs > result.lra_low_lufs);
        assert!((result.range_lu - (result.lra_high_lufs - result.lra_low_lufs)).abs() < 1e-12);
        let budget = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        assert!(
            crate::measure_loudness(&source, &budget)
                .unwrap_err()
                .contains("controlled memory")
        );
        let cancelled = fvid_control::CancelFlag::default();
        let flag = cancelled.clone();
        let options = CopyOptions {
            cancel: Some(cancelled),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                assert!(!event.done);
                if event.packets == 1 {
                    flag.cancel();
                }
            })),
            ..Default::default()
        };
        assert!(
            crate::measure_loudness(&source, &options)
                .unwrap_err()
                .contains("cancelled")
        );
        let options = CopyOptions {
            progress: Some(fvid_control::ProgressHook::new(|_| {
                panic!("plan must not read PCM or emit execution progress")
            })),
            ..Default::default()
        };
        crate::plan_loudness(&source, &options).unwrap();
        assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 1);
    }
    #[test]
    fn standard_rates_keep_original_clock_and_chunk_independent_peaks() {
        let files = files("rates");
        for rate in [
            8000, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 88200, 96000, 176400, 192000,
            352800, 384000,
        ] {
            let path = files.0.join(format!("{rate}.wav"));
            let pcm: Vec<f64> = (0..rate * 2 / 5)
                .map(|i| if i % 4 < 2 { 0.25 } else { -0.25 })
                .collect();
            crate::owned_wav_file::write_wav_f64le(&path, rate as i32, 1, &pcm).unwrap();
            assert!(supports(&path, &CopyOptions::default()));
            let whole = crate::measure_loudness(&path, &CopyOptions::default()).unwrap();
            let split = crate::measure_loudness(
                &path,
                &CopyOptions {
                    max_packet_bytes: 97 * 8,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(whole.sample_frames, pcm.len() as u64);
            assert_eq!(whole.sample_rate, rate as i32);
            assert!(whole.integrated_lufs.is_finite() && whole.integrated_lufs > -70.);
            assert!(whole.true_peak_dbfs - whole.sample_peak_dbfs > 2.9);
            assert_eq!(whole.integrated_lufs, split.integrated_lufs);
            assert_eq!(whole.true_peak_dbfs, split.true_peak_dbfs);
            assert!(
                crate::plan_loudness(&path, &CopyOptions::default())
                    .unwrap()
                    .steps[0]
                    .detail
                    .contains(&format!("{rate} Hz"))
            );
        }
    }
}
