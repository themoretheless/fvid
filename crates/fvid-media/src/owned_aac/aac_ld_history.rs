//! LD-LTP time history, including the extra N/2 prediction delay (4.6.7.3).
use super::{Result, aac_ld_ltp::LdLtpData, aac_ltp_history::GAINS, invalid};

#[derive(Clone, Debug, PartialEq)]
pub struct LdLtpHistory {
    frame_samples: usize,
    previous_lag: u16,
    // x_rec(-n-1023 .. -1), followed separately by x_rec(0 .. n-1).
    // More than two PCM frames are needed for lag 1023 at n=480.
    pcm: Vec<f64>,
    overlap: Vec<f64>,
}
impl LdLtpHistory {
    pub fn new(frame_samples: usize) -> Result<Self> {
        if !matches!(frame_samples, 480 | 512) {
            return Err(invalid("invalid AAC LD LTP history geometry"));
        }
        Ok(Self {
            frame_samples,
            previous_lag: 0,
            pcm: vec![0.; frame_samples + 1023],
            overlap: vec![0.; frame_samples],
        })
    }
    pub fn previous_lag(&self) -> u16 {
        self.previous_lag
    }
    /// Predict x_est(i) = gain * x_rec(i - n - lag), i=0..2n.
    /// This operation does not advance PCM or commit a transmitted lag.
    pub fn estimate(&self, data: &LdLtpData, output: &mut [f64]) -> Result<()> {
        let resolved = data.resolve(self.previous_lag, self.frame_samples as u16)?;
        if output.len() != 2 * self.frame_samples {
            return Err(invalid("AAC LD LTP estimate size mismatch"));
        }
        let gain = GAINS[usize::from(resolved.coefficient_index)];
        let start = 1023 - usize::from(resolved.lag);
        let sample = |i: usize| {
            let index = start + i;
            if index < self.pcm.len() {
                self.pcm[index]
            } else {
                self.overlap[index - self.pcm.len()]
            }
        };
        if (0..output.len()).any(|i| !(sample(i) * gain).is_finite()) {
            return Err(invalid("AAC LD LTP estimate overflow"));
        }
        for (i, out) in output.iter_mut().enumerate() {
            *out = sample(i) * gain;
        }
        Ok(())
    }
    /// Commit a successfully synthesized raw frame and optional predictor.
    /// Absent LTP retains the last lag while still advancing reconstructed PCM.
    pub fn update_raw(
        &mut self,
        pcm: &[f64],
        overlap: &[f64],
        data: Option<&LdLtpData>,
    ) -> Result<()> {
        let n = self.frame_samples;
        if pcm.len() != n || overlap.len() != n || pcm.iter().chain(overlap).any(|v| !v.is_finite())
        {
            return Err(invalid("invalid AAC LD LTP reconstructed samples"));
        }
        let lag = match data {
            Some(data) => data.resolve(self.previous_lag, n as u16)?.lag,
            None => self.previous_lag,
        };
        self.pcm.copy_within(n.., 0);
        self.pcm[1023..].copy_from_slice(pcm);
        self.overlap.copy_from_slice(overlap);
        self.previous_lag = lag;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.pcm.fill(0.);
        self.overlap.fill(0.);
        self.previous_lag = 0;
    }
    /// A cloned history is a checkpoint. Restore never reallocates.
    pub fn restore(&mut self, saved: &Self) -> Result<()> {
        if saved.frame_samples != self.frame_samples
            || saved.previous_lag > 1023
            || saved
                .pcm
                .iter()
                .chain(&saved.overlap)
                .any(|v| !v.is_finite())
        {
            return Err(invalid("invalid AAC LD LTP checkpoint"));
        }
        self.pcm.copy_from_slice(&saved.pcm);
        self.overlap.copy_from_slice(&saved.overlap);
        self.previous_lag = saved.previous_lag;
        Ok(())
    }
    pub(crate) fn visit_retained(
        &self,
        footprint: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        footprint.vector(&self.pcm)?;
        footprint.vector(&self.overlap)
    }
    pub fn retained_bytes(&self) -> Result<usize> {
        let mut footprint = super::memory::Footprint::new();
        self.visit_retained(&mut footprint)
            .map_err(|e| invalid(&e))?;
        Ok(footprint.total())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn data(lag: Option<u16>, coefficient_index: u8) -> LdLtpData {
        LdLtpData {
            lag_update: lag,
            coefficient_index,
            used: vec![true; 37],
        }
    }
    #[test]
    fn every_ld_lag_and_gain_match_absolute_pcm_timeline() {
        for n in [480, 512] {
            let mut history = LdLtpHistory::new(n).unwrap();
            let mut timeline = Vec::new();
            let mut overlap = vec![0.; n];
            let budget = history.retained_bytes().unwrap();
            for frame in 0..6 {
                let pcm: Vec<_> = (0..n)
                    .map(|i| ((frame * n + i) as f64 * 0.031).sin() * 20000.)
                    .collect();
                overlap = (0..n)
                    .map(|i| ((frame * n + i) as f64 * 0.019).cos() * 13000.)
                    .collect();
                history.update_raw(&pcm, &overlap, None).unwrap();
                timeline.extend(pcm);
                // Initial frames must read zero before the start of PCM,
                // including the 63 extra historical samples needed at n=480.
                for lag in [0, 479, 480, 511, 960, 1023] {
                    let mut actual = vec![0.; 2 * n];
                    history.estimate(&data(Some(lag), 0), &mut actual).unwrap();
                    for (i, value) in actual.iter().enumerate() {
                        let relative = i as isize - n as isize - isize::try_from(lag).unwrap();
                        let absolute = timeline.len() as isize + relative;
                        let sample = if relative >= 0 {
                            overlap[relative as usize]
                        } else if absolute >= 0 {
                            timeline[absolute as usize]
                        } else {
                            0.
                        };
                        assert_eq!(*value, sample * GAINS[0]);
                    }
                }
            }
            let saved = history.clone();
            let mut actual = vec![0.; 2 * n];
            for lag in 0..=1023 {
                for (coefficient, gain) in GAINS.iter().enumerate() {
                    history
                        .estimate(&data(Some(lag), coefficient as u8), &mut actual)
                        .unwrap();
                    for (i, value) in actual.iter().enumerate() {
                        let relative = i as isize - n as isize - isize::try_from(lag).unwrap();
                        let expected = if relative < 0 {
                            timeline[(timeline.len() as isize + relative) as usize]
                        } else {
                            overlap[relative as usize]
                        };
                        assert_eq!(
                            *value,
                            expected * gain,
                            "n={n} lag={lag} coef={coefficient} i={i}"
                        );
                    }
                }
            }
            assert_eq!(history, saved);
            assert_eq!(history.retained_bytes().unwrap(), budget);
        }
    }
    #[test]
    fn commit_absence_checkpoint_reset_and_errors_are_transactional() {
        let mut h = LdLtpHistory::new(480).unwrap();
        let mut out = vec![1.; 960];
        h.estimate(&data(None, 0), &mut out).unwrap();
        assert!(out.iter().all(|v| *v == 0.));
        h.update_raw(&vec![11.; 480], &vec![23.; 480], Some(&data(Some(1023), 1)))
            .unwrap();
        h.update_raw(&vec![31.; 480], &vec![41.; 480], None)
            .unwrap();
        assert_eq!(h.previous_lag(), 1023);
        let saved = h.clone();
        let mut invalid_pcm = vec![0.; 480];
        invalid_pcm[479] = f64::NAN;
        assert!(
            h.update_raw(&invalid_pcm, &vec![0.; 480], Some(&data(Some(4), 0)))
                .is_err()
        );
        assert!(
            h.update_raw(&vec![0.; 480], &vec![0.; 480], Some(&data(Some(1024), 0)))
                .is_err()
        );
        assert!(h.restore(&LdLtpHistory::new(512).unwrap()).is_err());
        assert_eq!(h, saved);
        out.fill(17.);
        assert!(h.estimate(&data(Some(1024), 0), &mut out).is_err());
        assert!(out.iter().all(|v| *v == 17.));
        h.reset();
        h.restore(&saved).unwrap();
        assert_eq!(h, saved);
        h.reset();
        h.estimate(&data(None, 0), &mut out).unwrap();
        assert!(out.iter().all(|v| *v == 0.));
        assert_eq!(h.previous_lag(), 0);
        h.update_raw(&vec![f64::MAX; 480], &vec![f64::MAX; 480], None)
            .unwrap();
        out.fill(17.);
        assert!(h.estimate(&data(Some(0), 7), &mut out).is_err());
        assert!(out.iter().all(|v| *v == 17.));
    }
    #[test]
    fn retained_accounting_includes_spare_capacity() {
        let mut h = LdLtpHistory::new(512).unwrap();
        h.pcm.reserve(517);
        h.overlap.reserve(119);
        assert_eq!(
            h.retained_bytes().unwrap(),
            (h.pcm.capacity() + h.overlap.capacity()) * std::mem::size_of::<f64>()
        );
    }
}
