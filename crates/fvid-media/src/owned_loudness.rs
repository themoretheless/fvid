//! Owned streaming integrated loudness; no decoder/backend dependencies.
use super::owned_k_weight::KWeighting;
use std::collections::BTreeMap;

pub use fvid_media_info::PcmLoudnessStats as IntegratedLoudness;
/// Explicit channel energy weights avoid guessing layout from channel count.
/// Use 1 for front channels, 1.41 for surrounds, 0 for LFE.
/// Relative gating uses 0.01 LU histogram bins, bounding storage by level range.
pub struct LoudnessMeter {
    true_peak: Option<crate::owned_true_peak::TruePeakMeter>,
    short_ring: Vec<f64>,
    short_position: usize,
    short_sum: f64,
    short_energies: BTreeMap<i32, (u64, f64)>,
    sample_peak: f64,
    filter: KWeighting,
    weights: Vec<f64>,
    ring: Vec<f64>,
    position: usize,
    sum: f64,
    frames: u64,
    hop: usize,
    blocks: u64,
    energies: BTreeMap<i32, (u64, f64)>,
}
impl LoudnessMeter {
    pub fn new(sample_rate: u32, weights: &[f64]) -> Result<Self, String> {
        if sample_rate % 10 != 0
            || weights
                .iter()
                .any(|w| !w.is_finite() || *w < 0.0 || *w > 2.0)
        {
            return Err(
                "loudness requires a rate divisible by ten and channel weights in 0..=2".into(),
            );
        }
        let filter = KWeighting::new(sample_rate, weights.len())?;
        Ok(Self {
            true_peak: None,
            short_ring: vec![0.0; sample_rate as usize * 3],
            short_position: 0,
            short_sum: 0.0,
            short_energies: BTreeMap::new(),
            sample_peak: 0.0,
            filter,
            weights: weights.to_vec(),
            ring: vec![0.0; sample_rate as usize * 2 / 5],
            position: 0,
            sum: 0.0,
            frames: 0,
            hop: sample_rate as usize / 10,
            blocks: 0,
            energies: BTreeMap::new(),
        })
    }
    /// Enable the Annex 2 four-phase estimator for standard PCM rates
    /// from 48 kHz upward (at least 192 kHz reconstructed grid).
    /// The default constructor retains its existing sample-peak-only cost.
    pub fn new_with_true_peak(sample_rate: u32, weights: &[f64]) -> Result<Self, String> {
        if !matches!(
            sample_rate,
            48000 | 88200 | 96000 | 176400 | 192000 | 352800 | 384000
        ) {
            return Err(
                "four-phase loudness true peak requires a qualified standard rate >= 48000 Hz"
                    .into(),
            );
        }
        let mut meter = Self::new(sample_rate, weights)?;
        meter.true_peak = Some(crate::owned_true_peak::TruePeakMeter::new(weights.len())?);
        Ok(meter)
    }
    /// Flush true-peak interpolation history without adding loudness frames.
    pub fn finish(&mut self) {
        if let Some(meter) = &mut self.true_peak {
            meter.finish();
        }
    }
    pub fn true_peak_report(&self) -> Option<crate::owned_true_peak::PeakReport> {
        self.true_peak.as_ref().map(|meter| meter.report())
    }
    pub fn push(&mut self, pcm: &[f64]) -> Result<(), String> {
        if pcm.len() % self.weights.len() != 0
            || pcm.iter().any(|x| !x.is_finite() || x.abs() > 1e100)
        {
            return Err("loudness requires complete finite PCM frames within numeric range".into());
        }
        self.frames
            .checked_add((pcm.len() / self.weights.len()) as u64)
            .ok_or("loudness sample count overflow")?;
        if let Some(meter) = &mut self.true_peak {
            meter.push(pcm)?;
        }
        let mut scratch = [0.0; 64];
        for frame in pcm.chunks_exact(self.weights.len()) {
            for sample in frame {
                self.sample_peak = self.sample_peak.max(sample.abs());
            }
            let samples = &mut scratch[..frame.len()];
            samples.copy_from_slice(frame);
            self.filter.process(samples)?;
            let energy: f64 = samples
                .iter()
                .zip(&self.weights)
                .map(|(x, w)| x * x * w)
                .sum();
            self.sum += energy - self.ring[self.position];
            self.ring[self.position] = energy;
            self.position = (self.position + 1) % self.ring.len();
            self.short_sum += energy - self.short_ring[self.short_position];
            self.short_ring[self.short_position] = energy;
            self.short_position = (self.short_position + 1) % self.short_ring.len();
            self.frames += 1;
            if self.frames >= self.short_ring.len() as u64 && self.frames % self.hop as u64 == 0 {
                let power = self.short_sum.max(0.0) / self.short_ring.len() as f64;
                let level = -0.691 + 10.0 * power.log10();
                if level >= -70.0 {
                    let entry = self
                        .short_energies
                        .entry((level * 100.0).floor() as i32)
                        .or_default();
                    entry.0 += 1;
                    entry.1 += power;
                }
            }
            if self.frames >= self.ring.len() as u64
                && (self.frames - self.ring.len() as u64) % self.hop as u64 == 0
            {
                self.blocks += 1;
                let power = self.sum.max(0.0) / self.ring.len() as f64;
                let level = -0.691 + 10.0 * power.log10();
                if level >= -70.0 {
                    let entry = self
                        .energies
                        .entry((level * 100.0).floor() as i32)
                        .or_default();
                    entry.0 += 1;
                    entry.1 += power;
                }
            }
        }
        Ok(())
    }
    /// Lower and upper gated short-term percentiles used for loudness range.
    pub fn range_bounds(&self) -> Option<(f64, f64)> {
        let short_count: u64 = self.short_energies.values().map(|e| e.0).sum();
        let short_sum: f64 = self.short_energies.values().map(|e| e.1).sum();
        let threshold = if short_count > 0 {
            short_sum / short_count as f64 / 100.0
        } else {
            f64::INFINITY
        };
        let kept: Vec<_> = self
            .short_energies
            .iter()
            .filter(|(_, e)| e.1 / e.0 as f64 >= threshold)
            .collect();
        let count: u64 = kept.iter().map(|(_, e)| e.0).sum();
        let percentile = |fraction: f64| {
            let rank = ((count as f64 * fraction).ceil() as u64).max(1);
            let mut accumulated = 0;
            for (level, e) in &kept {
                accumulated += e.0;
                if accumulated >= rank {
                    return **level as f64 / 100.0;
                }
            }
            0.0
        };
        (count > 0).then(|| (percentile(0.1), percentile(0.95)))
    }
    pub fn report(&self) -> IntegratedLoudness {
        let count: u64 = self.energies.values().map(|e| e.0).sum();
        let total: f64 = self.energies.values().map(|e| e.1).sum();
        let threshold = if count > 0 {
            total / count as f64 / 10.0
        } else {
            f64::INFINITY
        };
        let mut gated_count = 0u64;
        let mut gated_sum = 0.0;
        for (count, sum) in self.energies.values() {
            if sum / (*count as f64) >= threshold {
                gated_count += count;
                gated_sum += sum;
            }
        }
        let range = self.range_bounds();
        IntegratedLoudness {
            range_lu: range.map(|(low, high)| high - low),
            sample_peak_dbfs: (self.sample_peak > 0.0).then(|| 20.0 * self.sample_peak.log10()),
            sample_frames: self.frames,
            measured_blocks: self.blocks,
            integrated_lufs: (gated_count > 0)
                .then(|| -0.691 + 10.0 * (gated_sum / gated_count as f64).log10()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peak_is_unweighted_and_rejected_input_does_not_change_it() {
        let mut meter = LoudnessMeter::new(48000, &[1.0, 0.0]).unwrap();
        assert!(meter.report().sample_peak_dbfs.is_none());
        meter.push(&[0.25, -0.5]).unwrap();
        assert!((meter.report().sample_peak_dbfs.unwrap() + 6.020599913279624).abs() < 1e-12);
        assert!(meter.push(&[1.0, f64::NAN]).is_err());
        assert!((meter.report().sample_peak_dbfs.unwrap() + 6.020599913279624).abs() < 1e-12);
        assert!(meter.report().integrated_lufs.is_none());
        meter.push(&[0.0, 2.0]).unwrap();
        assert!((meter.report().sample_peak_dbfs.unwrap() - 6.020599913279624).abs() < 1e-12);
    }
    #[test]
    fn silence_short_stream_chunking_and_layout_weights() {
        let mut silent = LoudnessMeter::new(48000, &[1.0]).unwrap();
        silent.push(&vec![0.0; 48000]).unwrap();
        assert!(silent.report().integrated_lufs.is_none());
        assert!(silent.report().range_lu.is_none());
        assert_eq!(silent.report().measured_blocks, 7);
        let pcm: Vec<f64> = (0..48000)
            .flat_map(|i| [(i as f64 * 0.13).sin() * 0.1, 0.0])
            .collect();
        let mut whole = LoudnessMeter::new(48000, &[1.0, 0.0]).unwrap();
        whole.push(&pcm).unwrap();
        let mut split = LoudnessMeter::new(48000, &[1.0, 0.0]).unwrap();
        assert!(split.push(&[f64::NAN, 0.0]).is_err());
        for part in pcm.chunks(38) {
            split.push(part).unwrap();
        }
        assert_eq!(
            whole.report().integrated_lufs,
            split.report().integrated_lufs
        );
        let mut lfe = LoudnessMeter::new(48000, &[0.0, 1.0]).unwrap();
        lfe.push(&pcm).unwrap();
        assert!(lfe.report().integrated_lufs.is_none());
    }
}

#[cfg(test)]
mod true_peak_tests {
    use super::*;
    #[test]
    fn optional_true_peak_does_not_change_loudness_and_keeps_lfe_peak() {
        let pcm: Vec<_> = (0..24000)
            .flat_map(|i| {
                let fade = (i.min(23999 - i) as f64 / 64.).min(1.);
                [
                    0.,
                    fade * (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4)
                        .sin(),
                ]
            })
            .collect();
        let mut basic = LoudnessMeter::new(48000, &[1., 0.]).unwrap();
        let mut full = LoudnessMeter::new_with_true_peak(48000, &[1., 0.]).unwrap();
        for samples in pcm.chunks(254) {
            basic.push(samples).unwrap();
            full.push(samples).unwrap();
        }
        full.finish();
        assert!(basic.true_peak_report().is_none());
        let before = full.report();
        let expected = basic.report();
        assert_eq!(before.sample_frames, expected.sample_frames);
        assert_eq!(before.measured_blocks, expected.measured_blocks);
        assert_eq!(before.integrated_lufs, expected.integrated_lufs);
        assert_eq!(before.range_lu, expected.range_lu);
        assert_eq!(before.sample_peak_dbfs, expected.sample_peak_dbfs);
        assert!(before.integrated_lufs.is_none());
        let peak = full.true_peak_report().unwrap();
        assert!(peak.true_peak_dbfs.unwrap().abs() < 0.1);
        assert_eq!(peak.sample_frames, 24000);
        assert!(peak.finished);
        assert!(full.push(&[1., 1.]).is_err());
        assert_eq!(full.report().sample_frames, 24000);
        assert!(LoudnessMeter::new_with_true_peak(44100, &[1.]).is_err());
    }
}
