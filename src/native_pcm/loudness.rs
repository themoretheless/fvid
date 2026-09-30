//! Owned streaming integrated loudness; no decoder/backend dependencies.
use super::KWeighting;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct IntegratedLoudness {
    pub sample_frames: u64,
    pub measured_blocks: u64,
    /// None for silence or streams shorter than a complete 400 ms window.
    pub integrated_lufs: Option<f64>,
}
/// Explicit channel energy weights avoid guessing layout from channel count.
/// Use 1 for front channels, 1.41 for surrounds, 0 for LFE.
/// Relative gating uses 0.01 LU histogram bins, bounding storage by level range.
pub struct LoudnessMeter {
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
    pub fn push(&mut self, pcm: &[f64]) -> Result<(), String> {
        if pcm.len() % self.weights.len() != 0
            || pcm.iter().any(|x| !x.is_finite() || x.abs() > 1e100)
        {
            return Err("loudness requires complete finite PCM frames within numeric range".into());
        }
        self.frames
            .checked_add((pcm.len() / self.weights.len()) as u64)
            .ok_or("loudness sample count overflow")?;
        let mut scratch = [0.0; 64];
        for frame in pcm.chunks_exact(self.weights.len()) {
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
            self.frames += 1;
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
        IntegratedLoudness {
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
    fn silence_short_stream_chunking_and_layout_weights() {
        let mut silent = LoudnessMeter::new(48000, &[1.0]).unwrap();
        silent.push(&vec![0.0; 48000]).unwrap();
        assert!(silent.report().integrated_lufs.is_none());
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
mod oracle {
    use super::*;
    #[test]
    #[ignore = "requires FVID_REFERENCE_FFMPEG"]
    fn integrated_loudness_matches_independent_meter() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        for rate in [44100, 48000, 96000] {
            let pcm: Vec<f64> = (0..rate * 5)
                .map(|i| {
                    let gain = if i < rate {
                        0.0
                    } else if i < rate * 3 {
                        0.1
                    } else {
                        0.02
                    };
                    gain * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / rate as f64).sin()
                })
                .collect();
            let mut meter = LoudnessMeter::new(rate, &[1.0]).unwrap();
            for part in pcm.chunks(337) {
                meter.push(part).unwrap();
            }
            let mut child = Command::new(std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap())
                .args([
                    "-hide_banner",
                    "-nostats",
                    "-f",
                    "f64le",
                    "-ar",
                    &rate.to_string(),
                    "-ac",
                    "1",
                    "-i",
                    "pipe:0",
                    "-af",
                    "ebur128",
                    "-f",
                    "null",
                    "-",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut input = child.stdin.take().unwrap();
            let writer = std::thread::spawn(move || {
                for sample in pcm {
                    input.write_all(&sample.to_le_bytes()).unwrap();
                }
            });
            let result = child.wait_with_output().unwrap();
            writer.join().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let log = String::from_utf8(result.stderr).unwrap();
            let expected: f64 = log
                .lines()
                .rev()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("I:")
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap();
            let actual = meter.report().integrated_lufs.unwrap();
            assert!(
                (actual - expected).abs() < 0.11,
                "{rate}: {actual} != {expected}"
            );
        }
    }
}
