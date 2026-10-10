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
    history: LdLtpHistory,
    synthesis: LdSynthesis,
    previous: LdWindowShape,
    estimate: Vec<f64>,
}
#[derive(Clone)]
pub struct LdChannelCheckpoint {
    n: usize,
    history: LdLtpHistory,
    synthesis: LdSynthesisHistory,
    previous: LdWindowShape,
}
impl LdChannelCheckpoint {
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        self.history.visit_retained(f)?;
        self.synthesis.visit_retained(f)
    }
}
impl LdChannel {
    pub fn new(n: usize) -> Result<Self> {
        Ok(Self {
            n,
            history: LdLtpHistory::new(n)?,
            synthesis: LdSynthesis::new(n)?,
            previous: LdWindowShape::Sine,
            estimate: vec![0.; 2 * n],
        })
    }
    pub fn checkpoint(&self) -> LdChannelCheckpoint {
        LdChannelCheckpoint {
            n: self.n,
            history: self.history.clone(),
            synthesis: self.synthesis.checkpoint(),
            previous: self.previous,
        }
    }
    pub fn restore(&mut self, saved: &LdChannelCheckpoint) -> Result<()> {
        if self.n != saved.n {
            return Err(invalid("AAC LD channel checkpoint geometry mismatch"));
        }
        // Private checkpoint fields can only originate from a valid channel.
        self.history.restore(&saved.history)?;
        self.synthesis.restore(&saved.synthesis)?;
        self.previous = saved.previous;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.history.reset();
        self.synthesis.reset();
        self.previous = LdWindowShape::Sine;
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        self.history.visit_retained(f)?;
        self.synthesis.visit_retained(f)?;
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
        let mut pcm = vec![0.; self.n];
        next_synthesis.synthesize_raw(spectrum, shape, &mut pcm)?;
        next_history.update_raw(&pcm, next_synthesis.checkpoint().overlap_raw(), data)?;
        for v in &mut pcm {
            *v /= 65536.;
        }
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
