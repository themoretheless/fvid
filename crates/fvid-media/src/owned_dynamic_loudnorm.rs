//! Owned offline dynamic loudness control and linked lookahead peak limiting.
//! No external filters, codecs or subprocesses participate in this pipeline.
#![forbid(unsafe_code)]
use crate::owned_k_weight::KWeighting;
use crate::owned_loudness::LoudnessMeter;
type Result<T> = std::result::Result<T, String>;
const RATE: usize = 192000;
const HOP: usize = RATE / 10;

/// The entire channel frame shares one envelope: channel ratios and signs are
/// retained. A backwards attack constraint anticipates peaks by up to 10 ms;
/// forward exponential release has a 100 ms time constant. No samples are
/// delayed, inserted or removed, including at the beginning/end of the file.
pub(crate) fn limit<F: FnMut() -> Result<()>>(
    pcm: &mut [f32],
    channels: usize,
    ceiling: f64,
    correction: f64,
    mut check: F,
) -> Result<()> {
    if !(1..=64).contains(&channels)
        || pcm.len() % channels != 0
        || !ceiling.is_finite()
        || ceiling <= 0.
        || !correction.is_finite()
        || correction <= 0.
        || pcm
            .iter()
            .any(|&sample| !sample.is_finite() || !(f64::from(sample) * correction).is_finite())
    {
        return Err("invalid dynamic limiter PCM or geometry".into());
    }
    check()?;
    let frames = pcm.len() / channels;
    let mut allowed = Vec::new();
    allowed
        .try_reserve_exact(frames)
        .map_err(|_| "cannot allocate limiter envelope")?;
    allowed.resize(frames, 1.);
    let attack_step = 1. / (RATE as f64 * 0.01);
    let release = 1. - (-1. / (RATE as f64 * 0.1)).exp();
    let mut future = 1f64;
    for (i, frame) in pcm.chunks_exact(channels).enumerate().rev() {
        if i % 4096 == 0 {
            check()?;
        }
        let peak = frame
            .iter()
            .map(|&sample| f64::from(sample).abs() * correction)
            .fold(0f64, f64::max);
        let required = if peak <= ceiling { 1. } else { ceiling / peak };
        future = required.min((future + attack_step).min(1.));
        allowed[i] = future;
    }
    let mut envelope = allowed.first().copied().unwrap_or(1.);
    for (i, (frame, allowed)) in pcm.chunks_exact_mut(channels).zip(allowed).enumerate() {
        if i % 4096 == 0 {
            check()?;
        }
        envelope = allowed.min(envelope + (1. - envelope) * release);
        let gain = envelope * correction;
        for sample in frame {
            *sample = (f64::from(*sample) * gain) as f32;
        }
    }
    Ok(())
}

/// Process resampled double PCM on a 192 kHz clock. Local three-second energy
/// windows drive a range-compression curve; a 21-point Gaussian smooths its
/// 100 ms gain anchors. Whole-file measurement corrects the resulting gain
/// before the limiter, subject to the peak ceiling. The owned FIR includes
/// reconstruction tails when checking final true peak.
pub(crate) fn process<F: FnMut() -> Result<()>>(
    data: &[u8],
    weights: &[f64],
    global: Option<f64>,
    target: f64,
    range: f64,
    peak_db: f64,
    offset: f64,
    mut check: F,
) -> Result<Vec<f32>> {
    let channels = weights.len();
    if !(1..=64).contains(&channels) || data.len() % (8 * channels) != 0 {
        return Err("invalid dynamic loudnorm frame geometry".into());
    }
    check()?;
    let frames = data.len() / (8 * channels);
    let blocks = frames.div_ceil(HOP);
    let mut powers = Vec::new();
    powers
        .try_reserve_exact(blocks + 1)
        .map_err(|_| "cannot allocate loudnorm energy anchors")?;
    powers.push(0.);
    let mut filter = KWeighting::new(RATE as u32, channels)?;
    for block in data.chunks(HOP * channels * 8) {
        check()?;
        let mut pcm: Vec<f64> = block
            .chunks_exact(8)
            .map(|sample| f64::from_le_bytes(sample.try_into().unwrap()))
            .collect();
        filter.process(&mut pcm)?;
        let energy: f64 = pcm
            .chunks_exact(channels)
            .map(|frame| {
                frame
                    .iter()
                    .zip(weights)
                    .map(|(value, weight)| value * value * weight)
                    .sum::<f64>()
            })
            .sum();
        let sum = powers.last().unwrap() + energy;
        if !sum.is_finite() {
            return Err("non-finite dynamic loudness energy".into());
        }
        powers.push(sum);
    }
    drop(filter);
    let global = global.unwrap_or(-70.);
    let mut anchors = Vec::new();
    anchors
        .try_reserve_exact(blocks)
        .map_err(|_| "cannot allocate loudnorm gain anchors")?;
    let mut previous = 1.;
    for block in 0..blocks {
        let first = block.saturating_sub(15);
        let end = (block + 15).min(blocks).max(first + 1);
        let count = (end * HOP).min(frames) - first * HOP;
        let power = (powers[end] - powers[first]).max(0.) / count as f64;
        let local = -0.691 + 10. * power.log10();
        if local > -70. {
            let deviation = local - global;
            let gain_db = target - local + deviation.clamp(-range / 2., range / 2.);
            previous = 10f64.powf((gain_db + offset) / 20.);
        }
        anchors.push(previous);
    }
    drop(powers);
    let gaussian: Vec<f64> = (-10..=10)
        .map(|i| (-0.5 * (i as f64 / 3.5).powi(2)).exp())
        .collect();
    let total: f64 = gaussian.iter().sum();
    let mut smooth = Vec::new();
    smooth
        .try_reserve_exact(blocks)
        .map_err(|_| "cannot allocate smoothed loudnorm gains")?;
    for i in 0..blocks {
        let gain = gaussian
            .iter()
            .enumerate()
            .map(|(tap, weight)| {
                let index = (i as i128 + tap as i128 - 10).clamp(0, blocks as i128 - 1) as usize;
                weight * anchors[index]
            })
            .sum::<f64>()
            / total;
        smooth.push(gain);
    }
    drop(anchors);
    let mut output = Vec::new();
    output
        .try_reserve_exact(data.len() / 8)
        .map_err(|_| "cannot allocate dynamic loudnorm output")?;
    for (i, frame) in data.chunks_exact(8 * channels).enumerate() {
        if i % 4096 == 0 {
            check()?;
        }
        let anchor = i / HOP;
        let fraction = (i % HOP) as f64 / HOP as f64;
        let gain =
            smooth[anchor] + fraction * (smooth[(anchor + 1).min(blocks - 1)] - smooth[anchor]);
        for sample in frame.chunks_exact(8) {
            let value = (f64::from_le_bytes(sample.try_into().unwrap()) * gain) as f32;
            if !value.is_finite() {
                return Err("non-finite dynamic loudnorm output".into());
            }
            output.push(value);
        }
    }
    drop(smooth);
    let mut meter = LoudnessMeter::new(RATE as u32, weights)?;
    for block in output.chunks(4096 * channels) {
        check()?;
        let pcm: Vec<f64> = block.iter().map(|&sample| f64::from(sample)).collect();
        meter.push(&pcm)?;
    }
    let correction = meter
        .histogram_integrated_lufs()
        .map(|level| 10f64.powf((target + offset - level) / 20.))
        .unwrap_or(1.);
    drop(meter);
    let ceiling = 10f64.powf(peak_db / 20.);
    limit(&mut output, channels, ceiling, correction, &mut check)?;
    qualify_peak(&mut output, channels, ceiling, &mut check)?;
    // Limiting can reduce integrated loudness even when extra headroom exists
    // between isolated peaks. Two bounded feedback passes recover that headroom
    // rather than incorrectly treating every limiter loss as an infeasible I.
    for _ in 0..2 {
        let mut meter = LoudnessMeter::new(RATE as u32, weights)?;
        for block in output.chunks(4096 * channels) {
            check()?;
            let pcm: Vec<f64> = block.iter().map(|&sample| f64::from(sample)).collect();
            meter.push(&pcm)?;
        }
        let level = meter.histogram_integrated_lufs();
        drop(meter);
        let Some(level) = level else {
            break;
        };
        let error = target + offset - level;
        if error.abs() <= 0.075 {
            break;
        }
        limit(
            &mut output,
            channels,
            ceiling,
            10f64.powf(error / 20.),
            &mut check,
        )?;
        qualify_peak(&mut output, channels, ceiling, &mut check)?;
    }
    check()?;
    Ok(output)
}
fn qualify_peak<F: FnMut() -> Result<()>>(
    output: &mut [f32],
    channels: usize,
    ceiling: f64,
    check: &mut F,
) -> Result<()> {
    let mut peak = crate::owned_true_peak::TruePeakMeter::new(channels)?;
    for block in output.chunks(4096 * channels) {
        check()?;
        let pcm: Vec<f64> = block.iter().map(|&sample| f64::from(sample)).collect();
        peak.push(&pcm)?;
    }
    peak.finish();
    let measured = peak.report().true_peak;
    if measured > ceiling {
        let reduction = ceiling / measured;
        for block in output.chunks_mut(4096 * channels) {
            check()?;
            for sample in block {
                *sample = (f64::from(*sample) * reduction) as f32;
            }
        }
    }
    check()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dynamic_normalization_reduces_range_and_keeps_linked_stereo_ratios() {
        let mut source = Vec::new();
        let mut before = LoudnessMeter::new(RATE as u32, &[1., 1.]).unwrap();
        for second in 0..12 {
            let gain = if second < 4 {
                0.003
            } else if second < 8 {
                0.3
            } else {
                0.03
            };
            let pcm: Vec<f64> = (0..RATE)
                .flat_map(|i| {
                    let value =
                        gain * (std::f64::consts::TAU * 1000. * i as f64 / RATE as f64).sin();
                    [value, -value / 2.]
                })
                .collect();
            before.push(&pcm).unwrap();
            source.extend(pcm.iter().flat_map(|value| value.to_le_bytes()));
        }
        let input = before.report();
        let global = before.histogram_integrated_lufs();
        drop(before);
        let output = process(&source, &[1., 1.], global, -16., 11., -1.5, 0., || Ok(())).unwrap();
        assert_eq!(output.len(), 12 * RATE * 2);
        let mut after = LoudnessMeter::new_with_true_peak(RATE as u32, &[1., 1.]).unwrap();
        for block in output.chunks(8192) {
            for frame in block.chunks_exact(2) {
                assert_eq!(frame[1], -frame[0] / 2.);
            }
            let pcm: Vec<f64> = block.iter().map(|&value| f64::from(value)).collect();
            after.push(&pcm).unwrap();
        }
        after.finish();
        let report = after.report();
        assert!(
            (report.integrated_lufs.unwrap() + 16.).abs() < 0.3,
            "{report:?}"
        );
        assert!(report.range_lu.unwrap() <= 12.0, "{report:?}");
        assert!(
            report.range_lu.unwrap() < input.range_lu.unwrap() - 3.,
            "{input:?} -> {report:?}"
        );
        assert!(after.true_peak_report().unwrap().true_peak_dbfs.unwrap() <= -1.5 + 1e-5);
    }
    #[test]
    fn limiter_anticipates_peaks_links_channels_and_flushes_both_edges() {
        let mut pcm = vec![0.1; RATE / 2 * 2];
        for frame in pcm.chunks_exact_mut(2) {
            frame[1] = -frame[0] / 2.;
        }
        for index in [0, RATE / 4, RATE / 2 - 1] {
            pcm[index * 2] = 4.;
            pcm[index * 2 + 1] = -2.;
        }
        let count = pcm.len();
        limit(&mut pcm, 2, 0.5, 2., || Ok(())).unwrap();
        assert_eq!(pcm.len(), count);
        for frame in pcm.chunks_exact(2) {
            assert!(frame[0].is_finite() && frame[0] <= 0.5 + 1e-7);
            assert_eq!(frame[1], -frame[0] / 2.);
        }
        for index in [0, RATE / 4, RATE / 2 - 1] {
            assert_eq!(pcm[index * 2], 0.5);
        }
        let center = RATE / 4;
        assert!(pcm[(center - 1000) * 2] < pcm[(center - 2000) * 2]);
        assert!(pcm[(center + 3000) * 2] > pcm[(center + 1000) * 2]);
    }
    #[test]
    fn malformed_limiter_input_is_rejected_before_changes() {
        for (channels, ceiling, correction) in
            [(0, 0.5, 1.), (3, 0.5, 1.), (2, 0., 1.), (2, 0.5, f64::NAN)]
        {
            let mut pcm = [0.25, -0.5];
            assert!(limit(&mut pcm, channels, ceiling, correction, || Ok(())).is_err());
            assert_eq!(pcm, [0.25, -0.5]);
        }
        let mut pcm = [0.25, f32::INFINITY];
        assert!(limit(&mut pcm, 2, 0.5, 1., || Ok(())).is_err());
        assert_eq!(pcm[0], 0.25);
    }
}
