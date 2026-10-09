//! Owned LTP channel processing in the existing raw synthesis-unit convention.
//! Production profile dispatch and normative PCM qualification remain separate.
use super::{
    Result,
    aac_ltp_analysis::LtpAnalysis,
    aac_ltp_history::LtpHistory,
    aac_ltp_syntax::LtpData,
    aac_synthesis::{LongSineSynthesis, SynthesisHistory, WindowSequence, WindowShape},
    aac_tns::TnsData,
    invalid,
};
pub struct LtpChannel {
    n: usize,
    history: LtpHistory,
    analysis: LtpAnalysis,
    synthesis: LongSineSynthesis,
    previous: WindowShape,
}
#[derive(Clone)]
pub struct LtpChannelCheckpoint {
    n: usize,
    history: LtpHistory,
    synthesis: SynthesisHistory,
    previous: WindowShape,
}
impl LtpChannel {
    pub fn new(n: usize) -> Result<Self> {
        Ok(Self {
            n,
            history: LtpHistory::new(n)?,
            analysis: LtpAnalysis::new(n)?,
            synthesis: LongSineSynthesis::new(n)?,
            previous: WindowShape::Sine,
        })
    }
    pub fn checkpoint(&self) -> LtpChannelCheckpoint {
        LtpChannelCheckpoint {
            n: self.n,
            history: self.history.clone(),
            synthesis: self.synthesis.history(),
            previous: self.previous,
        }
    }
    pub fn restore(&mut self, saved: &LtpChannelCheckpoint) -> Result<()> {
        if self.n != saved.n {
            return Err(invalid("AAC LTP channel checkpoint geometry mismatch"));
        }
        self.synthesis.restore_history(&saved.synthesis)?;
        self.history.restore(&saved.history)?;
        self.previous = saved.previous;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.history.reset();
        self.synthesis.reset();
        self.previous = WindowShape::Sine;
    }
    /// Process a reconstructed residual. Advance both histories only after all
    /// stages succeed. Returned PCM uses the existing owned 1/65536 convention.
    pub fn process(
        &mut self,
        mut residual: Vec<f32>,
        data: Option<&LtpData>,
        sequence: WindowSequence,
        shape: WindowShape,
        offsets: &[usize],
        tns_max_band: usize,
        tns: Option<&TnsData>,
    ) -> Result<Vec<f64>> {
        if residual.len() != self.n {
            return Err(invalid("invalid AAC LTP channel residual geometry"));
        }
        if let Some(data) = data {
            self.analysis.predict_long(
                &self.history,
                data,
                sequence,
                self.previous,
                shape,
                offsets,
                tns_max_band,
                tns,
                &mut residual,
            )?;
        }
        if let Some(tns) = tns {
            residual = tns.filter_owned(residual, offsets, tns_max_band)?;
        }
        // Initial transactional adapter: clone state, never expose partial advance.
        let mut next_synthesis = self.synthesis.clone();
        let mut next_history = self.history.clone();
        let mut pcm = vec![0.; self.n];
        next_synthesis.synthesize_shaped(sequence, shape, &residual, &mut pcm)?;
        let saved = next_synthesis.history();
        next_history.update_raw(&pcm, saved.overlap_raw())?;
        for value in &mut pcm {
            *value /= 65536.;
        }
        self.synthesis = next_synthesis;
        self.history = next_history;
        self.previous = shape;
        Ok(pcm)
    }
}
