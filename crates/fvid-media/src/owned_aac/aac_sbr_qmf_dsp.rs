//! Owned SBR preparation and envelope adjustment, before PCM synthesis.
//! Extended payloads are opaque here: the enclosing decoder must dispatch them.
use super::{
    Result, aac_sbr_assembly::Assembly, aac_sbr_buffers::SynthesisRows, aac_sbr_gain, aac_sbr_hf,
    aac_sbr_history::Frame, aac_sbr_limiter, aac_sbr_prepare::Preparation, aac_sbr_qmf::Complex,
};
#[derive(Clone, Debug, Default, PartialEq)]
struct Channel {
    assembly: Assembly,
    rows: SynthesisRows,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dsp {
    preparation: Preparation,
    channels: Vec<Channel>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QmfFrame {
    pub rows: Vec<Vec<[Complex; 64]>>,
    pub format_reset: bool,
    pub qmf_limit: usize,
}
impl Dsp {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn visit_retained(
        &self,
        footprint: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.preparation.visit_retained(footprint)?;
        footprint.vector(&self.channels)
    }
    pub fn process_upsampling(
        &mut self,
        pcm: &[&[f32]],
        rate: u32,
        slots: u8,
    ) -> Result<Vec<Vec<[Complex; 64]>>> {
        let mut trial = self.clone();
        let rows = trial.preparation.upsample_rows(pcm, rate, slots)?;
        if trial.channels.is_empty() {
            trial.channels.resize_with(rows.len(), Default::default);
        }
        for state in &mut trial.channels {
            state.rows.reset();
        }
        *self = trial;
        Ok(rows)
    }
    /// Normalized core PCM to 16-bit-unit QMF rows. No synthesis history is advanced.
    pub fn process(
        &mut self,
        frame: &Frame,
        pcm: &[&[f32]],
        rate: u32,
        slots: u8,
    ) -> Result<QmfFrame> {
        // First received header may follow delay-only frames of the same format.
        // Initializing SBR syntax must not discard their QMF analysis/synthesis.
        let mut frame = frame.clone();
        if frame.syntax.format_reset && self.preparation.matches_format(rate, slots, pcm.len()) {
            frame.syntax.format_reset = false;
        }
        let mut trial = self.clone();
        if frame.syntax.format_reset {
            trial.reset();
        }
        let prepared = trial.preparation.process(&frame, pcm, rate, slots)?;
        if trial.channels.is_empty() {
            for _ in &prepared {
                trial.channels.push(Channel {
                    assembly: Assembly::default(),
                    rows: SynthesisRows::default(),
                });
            }
        }
        let tables = &frame.syntax.data.frequency;
        let header = &frame.syntax.header;
        let kx = tables.high[0];
        let end = *tables.high.last().unwrap();
        let patches = aac_sbr_hf::patches(&tables.master, kx, rate)?;
        let borders = aac_sbr_limiter::borders(&tables.low, &patches, header.limiter_bands)?;
        let mut output = Vec::with_capacity(prepared.len());
        for (index, prepared) in prepared.into_iter().enumerate() {
            let state = &mut trial.channels[index];
            let grid = frame.syntax.data.channels[index].grid.time_grid(slots)?;
            let mut levels = Vec::with_capacity(prepared.mapped.bands.len());
            for (bands, &suppress) in prepared
                .mapped
                .bands
                .iter()
                .zip(&prepared.mapped.suppress_noise)
            {
                let initial = aac_sbr_gain::calculate(bands, suppress)?;
                levels.push(aac_sbr_gain::limit(
                    bands,
                    &initial,
                    kx,
                    &borders,
                    header.limiter_gains,
                    suppress,
                )?);
            }
            let adjusted = state.assembly.process(
                &prepared.high,
                slots,
                &grid,
                kx,
                &levels,
                &prepared.mapped.suppress_noise,
                header.smoothing_mode,
                frame.syntax.header_reset,
            )?;
            let rows = state
                .rows
                .process(&prepared.low, &adjusted, slots, &grid, kx, end)?;
            output.push(rows);
        }
        *self = trial;
        Ok(QmfFrame {
            rows: output,
            format_reset: frame.syntax.format_reset,
            qmf_limit: usize::from(end),
        })
    }
}
