//! Owned LD spectral prediction, TNS and PCM synthesis. Packet dispatch is separate.
use super::{
    Result,
    aac_ld_history::LdLtpHistory,
    aac_ld_ltp::LdLtpData,
    aac_ld_synthesis::{LdSynthesis, LdSynthesisHistory, LdWindowShape},
    aac_ltp_analysis::apply_long_prediction,
    aac_tns::TnsData,
    invalid,
    memory::Footprint,
};

#[derive(Clone)]
pub struct LdChannel {
    n: usize,
    factor: u8,
    short: Option<LdSynthesis>,
    history: LdLtpHistory,
    synthesis: LdSynthesis,
    previous: LdWindowShape,
    estimate: Vec<f64>,
}
#[derive(Clone)]
pub struct LdChannelCheckpoint {
    n: usize,
    factor: u8,
    short: Option<LdSynthesisHistory>,
    history: LdLtpHistory,
    synthesis: LdSynthesisHistory,
    previous: LdWindowShape,
}
impl LdChannelCheckpoint {
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        self.history.visit_retained(f)?;
        self.synthesis.visit_retained(f)?;
        if let Some(short) = &self.short {
            short.visit_retained(f)?;
        }
        Ok(())
    }
}
impl LdChannel {
    pub fn new(n: usize) -> Result<Self> {
        Self::new_downscaled(n, 1)
    }
    /// Keep source-clock prediction; synthesize low coefficients directly at
    /// the target clock. Full nominal history is required by future LD-LTP.
    pub fn new_downscaled(n: usize, factor: u8) -> Result<Self> {
        if !matches!(n, 480 | 512)
            || !matches!(factor, 1 | 2 | 3 | 4 | 8)
            || !n.is_multiple_of(usize::from(factor))
        {
            return Err(invalid("unsupported AAC LD output geometry"));
        }
        Ok(Self {
            n,
            factor,
            short: if factor == 1 {
                None
            } else {
                Some(LdSynthesis::new(n / usize::from(factor))?)
            },
            history: LdLtpHistory::new(n)?,
            synthesis: LdSynthesis::new(n)?,
            previous: LdWindowShape::Sine,
            estimate: vec![0.; 2 * n],
        })
    }
    pub fn checkpoint(&self) -> LdChannelCheckpoint {
        LdChannelCheckpoint {
            n: self.n,
            factor: self.factor,
            short: self.short.as_ref().map(LdSynthesis::checkpoint),
            history: self.history.clone(),
            synthesis: self.synthesis.checkpoint(),
            previous: self.previous,
        }
    }
    pub fn restore(&mut self, saved: &LdChannelCheckpoint) -> Result<()> {
        if self.n != saved.n
            || self.factor != saved.factor
            || self.short.is_some() != saved.short.is_some()
        {
            return Err(invalid("AAC LD channel checkpoint geometry mismatch"));
        }
        // Private checkpoint fields can only originate from a valid channel.
        self.history.restore(&saved.history)?;
        self.synthesis.restore(&saved.synthesis)?;
        if let (Some(short), Some(state)) = (&mut self.short, &saved.short) {
            short.restore(state)?;
        }
        self.previous = saved.previous;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.history.reset();
        self.synthesis.reset();
        if let Some(short) = &mut self.short {
            short.reset();
        }
        self.previous = LdWindowShape::Sine;
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        self.history.visit_retained(f)?;
        self.synthesis.visit_retained(f)?;
        if let Some(short) = &self.short {
            short.visit_retained(f)?;
        }
        f.vector(&self.estimate)
    }
    pub fn retained_bytes(&self, checkpoint: Option<&LdChannelCheckpoint>) -> Result<usize> {
        let mut f = Footprint::new();
        self.visit_retained(&mut f).map_err(|e| invalid(&e))?;
        if let Some(saved) = checkpoint {
            saved.visit_retained(&mut f).map_err(|e| invalid(&e))?;
        }
        Ok(f.total())
    }
    /// No PCM/window/lag history is advanced here. Coupling may be applied
    /// to the residual before this stage or to the prepared spectrum afterwards.
    pub fn prepare_spectrum(
        &mut self,
        mut residual: Vec<f32>,
        data: Option<&LdLtpData>,
        shape: LdWindowShape,
        offsets: &[usize],
        tns_max_band: usize,
        tns: Option<&TnsData>,
    ) -> Result<Vec<f32>> {
        if residual.len() != self.n
            || residual.iter().any(|v| !v.is_finite())
            || offsets.first() != Some(&0)
            || offsets.last() != Some(&self.n)
            || offsets.windows(2).any(|p| p[0] >= p[1])
            || tns_max_band >= offsets.len()
        {
            return Err(invalid("invalid AAC LD channel spectral geometry"));
        }
        if let Some(data) = data {
            if data.used.len() >= offsets.len() {
                return Err(invalid("AAC LD LTP bands exceed geometry"));
            }
            self.history.estimate(data, &mut self.estimate)?;
            let mut predicted = vec![0.; self.n];
            self.synthesis
                .analyze(&self.estimate, self.previous, shape, &mut predicted)?;
            if let Some(tns) = tns {
                predicted = tns.analyze_owned(predicted, offsets, tns_max_band)?;
            }
            apply_long_prediction(&mut residual, &predicted, offsets, &data.used)?;
        }
        if let Some(tns) = tns {
            residual = tns.filter_owned(residual, offsets, tns_max_band)?;
        }
        Ok(residual)
    }
    /// Pass the same predictor and shape used during preparation. Commit both
    /// PCM and lag only when synthesis and history validation succeed.
    pub fn synthesize_spectrum(
        &mut self,
        spectrum: &[f32],
        data: Option<&LdLtpData>,
        shape: LdWindowShape,
    ) -> Result<Vec<f64>> {
        let mut next_synthesis = self.synthesis.clone();
        let mut next_history = self.history.clone();
        let mut next_short = self.short.clone();
        let mut pcm = vec![0.; self.n];
        next_synthesis.synthesize_raw(spectrum, shape, &mut pcm)?;
        next_history.update_raw(&pcm, next_synthesis.checkpoint().overlap_raw(), data)?;
        if let Some(short) = &mut next_short {
            let count = self.n / usize::from(self.factor);
            let mut reduced = vec![0.; count];
            short.synthesize_raw(&spectrum[..count], shape, &mut reduced)?;
            for v in &mut reduced {
                *v *= count as f64 / self.n as f64;
            }
            pcm = reduced;
        }
        for v in &mut pcm {
            *v /= 65536.;
        }
        self.short = next_short;
        self.synthesis = next_synthesis;
        self.history = next_history;
        self.previous = shape;
        Ok(pcm)
    }
    pub fn process(
        &mut self,
        residual: Vec<f32>,
        data: Option<&LdLtpData>,
        shape: LdWindowShape,
        offsets: &[usize],
        tns_max_band: usize,
        tns: Option<&TnsData>,
    ) -> Result<Vec<f64>> {
        let spectrum = self.prepare_spectrum(residual, data, shape, offsets, tns_max_band, tns)?;
        self.synthesize_spectrum(&spectrum, data, shape)
    }
}

#[cfg(test)]
mod downscaled_tests {
    use super::*;

    #[test]
    fn short_output_preserves_nominal_prediction_and_odd_reused_lags() {
        for n in [480, 512] {
            for factor in [2, 3, 4, 8] {
                if n % factor as usize != 0 {
                    continue;
                }
                let mut full = LdChannel::new(n).unwrap();
                let mut short = LdChannel::new_downscaled(n, factor).unwrap();
                for frame in 0..8 {
                    let shape = if frame % 2 == 0 {
                        LdWindowShape::Sine
                    } else {
                        LdWindowShape::LowOverlap
                    };
                    let data = LdLtpData {
                        lag_update: match frame {
                            0 => Some(1023),
                            4 => Some(13),
                            _ => None,
                        },
                        coefficient_index: frame as u8,
                        used: vec![true],
                    };
                    let residual = (0..n)
                        .map(|i| ((i + frame) as f32 * 0.03).sin())
                        .collect::<Vec<_>>();
                    let a = full
                        .prepare_spectrum(residual.clone(), Some(&data), shape, &[0, n], 1, None)
                        .unwrap();
                    let b = short
                        .prepare_spectrum(residual, Some(&data), shape, &[0, n], 1, None)
                        .unwrap();
                    assert_eq!(a, b);
                    full.synthesize_spectrum(&a, Some(&data), shape).unwrap();
                    assert_eq!(
                        short
                            .synthesize_spectrum(&b, Some(&data), shape)
                            .unwrap()
                            .len(),
                        n / factor as usize
                    );
                    assert_eq!(full.history, short.history);
                }
            }
        }
    }

    #[test]
    fn rejected_restore_and_synthesis_preserve_both_banks() {
        let mut channel = LdChannel::new_downscaled(480, 3).unwrap();
        channel
            .synthesize_spectrum(&vec![1.; 480], None, LdWindowShape::Sine)
            .unwrap();
        let mut control = channel.clone();
        assert!(
            channel
                .restore(&LdChannel::new_downscaled(480, 2).unwrap().checkpoint())
                .is_err()
        );
        assert!(
            channel
                .synthesize_spectrum(&vec![f32::NAN; 480], None, LdWindowShape::Sine)
                .is_err()
        );
        let spectrum = vec![0.5; 480];
        assert_eq!(
            channel
                .synthesize_spectrum(&spectrum, None, LdWindowShape::Sine)
                .unwrap(),
            control
                .synthesize_spectrum(&spectrum, None, LdWindowShape::Sine)
                .unwrap()
        );
        assert_eq!(channel.history, control.history);
    }
}
