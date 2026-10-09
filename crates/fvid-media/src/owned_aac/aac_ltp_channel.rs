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
#[derive(Clone)]
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
impl LtpChannelCheckpoint {
    pub(crate) fn visit_retained(
        &self,
        footprint: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.history.visit_retained(footprint)?;
        self.synthesis.visit_retained(footprint)
    }
    /// Retained heap payload of a packet-boundary snapshot. Excludes the stack
    /// object, allocator headers, decode temporaries and caller-owned PCM.
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        let mut footprint = super::memory::Footprint::new();
        self.visit_retained(&mut footprint)
            .map_err(|e| invalid(&e))?;
        Ok(footprint.total())
    }
}
impl LtpChannel {
    pub(crate) fn visit_retained(
        &self,
        footprint: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.history.visit_retained(footprint)?;
        self.analysis.visit_retained(footprint)?;
        self.synthesis.visit_retained(footprint)
    }
    /// Retained heap payload, counting shared tables once. This measures
    /// retained storage, not peak decode memory or process RSS.
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        self.retained_payload_bytes_with_checkpoint(None)
    }
    pub fn retained_payload_bytes_with_checkpoint(
        &self,
        checkpoint: Option<&LtpChannelCheckpoint>,
    ) -> Result<usize> {
        let mut footprint = super::memory::Footprint::new();
        self.visit_retained(&mut footprint)
            .map_err(|e| invalid(&e))?;
        if let Some(saved) = checkpoint {
            saved
                .visit_retained(&mut footprint)
                .map_err(|e| invalid(&e))?;
        }
        Ok(footprint.total())
    }

    pub fn new(n: usize) -> Result<Self> {
        Ok(Self {
            n,
            history: LtpHistory::new_float(n)?,
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

#[cfg(test)]
mod retained_tests {
    use super::*;
    #[test]
    fn independent_clones_share_tables_but_count_all_mutable_storage() {
        for n in [960usize, 1024] {
            let state = LtpChannel::new(n).unwrap();
            let clone = state.clone();
            let one = state.retained_payload_bytes().unwrap();
            let mut footprint = super::super::memory::Footprint::new();
            state.visit_retained(&mut footprint).unwrap();
            clone.visit_retained(&mut footprint).unwrap();
            // Four history blocks; 2N analysis input; 3N+N/4 synthesis
            // buffers; separate forward and inverse FFT workspaces.
            let fft = (2 * n - 1).next_power_of_two();
            let mutable = (9 * n + n / 4) * std::mem::size_of::<f64>()
                + 2 * fft * std::mem::size_of::<[f64; 2]>();
            assert_eq!(footprint.total(), one + mutable);
            assert!(footprint.total() < 2 * one);
            let checkpoint = state.checkpoint();
            assert_eq!(
                checkpoint.retained_payload_bytes().unwrap(),
                5 * n * std::mem::size_of::<f64>()
            );
            assert_eq!(
                state
                    .retained_payload_bytes_with_checkpoint(Some(&checkpoint))
                    .unwrap(),
                one + 5 * n * std::mem::size_of::<f64>()
            );
        }
    }
    #[test]
    fn clone_history_is_independent_and_memory_is_stable_after_process_restore_reset() {
        use super::super::aac_bands::BandTables;
        for n in [960usize, 1024] {
            let mut state = LtpChannel::new(n).unwrap();
            let mut clone = state.clone();
            let saved = state.checkpoint();
            let before = state.retained_payload_bytes().unwrap();
            let tables = BandTables::new(24000, n).unwrap();
            let mut residual = vec![0f32; n];
            residual[0] = 1024.;
            let process = |s: &mut LtpChannel| {
                s.process(
                    residual.clone(),
                    None,
                    WindowSequence::OnlyLong,
                    WindowShape::Kbd,
                    tables.long,
                    2,
                    None,
                )
                .unwrap()
            };
            let first = process(&mut state);
            assert_eq!(first, process(&mut clone));
            assert_eq!(before, state.retained_payload_bytes().unwrap());
            state.restore(&saved).unwrap();
            assert_eq!(first, process(&mut state));
            assert_ne!(first, process(&mut clone));
            state.reset();
            assert_eq!(first, process(&mut state));
            assert_eq!(before, state.retained_payload_bytes().unwrap());
        }
    }
}
