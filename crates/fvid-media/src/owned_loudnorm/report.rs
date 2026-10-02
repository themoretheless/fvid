//! Measurement/reporting for owned normalization. No log scraping or FFI.
use super::{Parameters, Print, Result};
use crate::owned_wave_loudness::NormalizationMeasurement;
use fvid_control::{CopyOptions, ProgressHook};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(super) fn measure_phase(
    source: &Path,
    dual_mono: bool,
    options: &CopyOptions,
) -> Result<(NormalizationMeasurement, CopyOptions)> {
    let packets = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let mut measure_options = options.clone();
    if let Some(hook) = &options.progress {
        let hook = hook.clone();
        let packets = packets.clone();
        let bytes = bytes.clone();
        measure_options.progress = Some(ProgressHook::new(move |mut event| {
            packets.store(event.packets, Ordering::Relaxed);
            bytes.store(event.payload_bytes, Ordering::Relaxed);
            event.done = false;
            hook.emit(event);
        }));
    }
    let measurement =
        crate::owned_wave_loudness::measure_for_normalization(source, dual_mono, &measure_options)?;
    let mut export_options = options.clone();
    if let Some(hook) = &options.progress {
        let hook = hook.clone();
        let prefix_packets = packets.load(Ordering::Relaxed);
        let prefix_bytes = bytes.load(Ordering::Relaxed);
        export_options.progress = Some(ProgressHook::new(move |mut event| {
            event.packets = event.packets.saturating_add(prefix_packets);
            event.payload_bytes = event.payload_bytes.saturating_add(prefix_bytes);
            hook.emit(event);
        }));
    }
    Ok((measurement, export_options))
}

pub(super) fn report_memory(rate: u32, frames: u64, channels: u16) -> u128 {
    let hop = u128::from(rate) / 10;
    u128::from(rate) * 34 / 10 * 8
        + 2 * u128::from(frames).div_ceil(hop) * 512
        + u128::from(frames.min(4096)) * u128::from(channels) * 8
        + 512 * 1024
}
pub(super) struct Report {
    input_i: f64,
    input_tp: f64,
    input_lra: f64,
    input_thresh: f64,
    output_i: f64,
    output_tp: f64,
    output_lra: f64,
    output_thresh: f64,
}
pub(super) fn output_report<F: Fn() -> Result<()>>(
    output: &[f32],
    rate: u32,
    params: &Parameters,
    input: Option<&NormalizationMeasurement>,
    check: F,
) -> Result<Option<Report>> {
    if params.print == Print::None {
        return Ok(None);
    }
    let input = input.ok_or("normalization report input measurement missing")?;
    let channels = input.weights.len();
    let mut meter = crate::owned_loudness::LoudnessMeter::new_with_true_peak(rate, &input.weights)?;
    for block in output.chunks(channels * 4096) {
        check()?;
        let pcm: Vec<f64> = block.iter().map(|&sample| f64::from(sample)).collect();
        meter.push(&pcm)?;
    }
    check()?;
    meter.finish();
    let values = meter.report();
    let peak = meter
        .true_peak_report()
        .ok_or("normalization report peak missing")?;
    Ok(Some(Report {
        input_i: input.integrated_lufs.unwrap_or(f64::NEG_INFINITY),
        input_tp: input.stats.true_peak_dbfs,
        input_lra: input.stats.range_lu,
        input_thresh: input.relative_thresh,
        output_i: values.integrated_lufs.unwrap_or(f64::NEG_INFINITY),
        output_tp: peak.true_peak_dbfs.unwrap_or(f64::NEG_INFINITY),
        output_lra: values.range_lu.unwrap_or(0.),
        output_thresh: meter.relative_gate_lufs(),
    }))
}
pub(super) fn print_report(format: Print, report: Report, target: f64, mode: &str) {
    let offset = if report.output_i.is_finite() {
        target - report.output_i
    } else {
        0.
    };
    if format == Print::Json {
        eprintln!(
            "{}",
            serde_json::json!({
                "input_i":format!("{:.2}", report.input_i), "input_tp":format!("{:.2}", report.input_tp),
                "input_lra":format!("{:.2}", report.input_lra), "input_thresh":format!("{:.2}", report.input_thresh),
                "output_i":format!("{:.2}", report.output_i), "output_tp":format!("{:.2}", report.output_tp),
                "output_lra":format!("{:.2}", report.output_lra), "output_thresh":format!("{:.2}", report.output_thresh),
                "normalization_type": mode, "target_offset":format!("{offset:.2}")
            })
        );
    } else if format == Print::Summary {
        eprintln!(
            "Input Integrated: {:.2} LUFS\nInput True Peak: {:.2} dBTP\nInput LRA: {:.2} LU\nInput Threshold: {:.2} LUFS\nOutput Integrated: {:.2} LUFS\nOutput True Peak: {:.2} dBTP\nOutput LRA: {:.2} LU\nOutput Threshold: {:.2} LUFS\nNormalization Type: {mode}\nTarget Offset: {offset:.2} LU",
            report.input_i,
            report.input_tp,
            report.input_lra,
            report.input_thresh,
            report.output_i,
            report.output_tp,
            report.output_lra,
            report.output_thresh
        );
    }
}
