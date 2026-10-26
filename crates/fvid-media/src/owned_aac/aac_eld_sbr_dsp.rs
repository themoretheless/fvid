//! Owned CLDFB -> low-delay HF generation/adjustment -> normalized PCM.
use super::{
    Result,
    aac_eld_cldfb::{Analysis, Synthesis},
    aac_sbr_assembly::Assembly,
    aac_sbr_bands::FrequencyTables,
    aac_sbr_chirp::Chirp,
    aac_sbr_energy, aac_sbr_gain, aac_sbr_hf,
    aac_sbr_history::Frame,
    aac_sbr_limiter,
    aac_sbr_mapping::History,
    aac_sbr_qmf::Complex,
    aac_sbr_timing::Timing,
    invalid,
    memory::Footprint,
};
#[derive(Clone, Debug, PartialEq)]
struct Channel {
    analysis: Analysis,
    tail: [[Complex; 32]; 2],
    previous_kx: u8,
    chirp: Chirp,
    mapping: History,
    assembly: Assembly,
    synthesis: Synthesis,
}
impl Channel {
    fn new(noise: usize, bands: usize) -> Result<Self> {
        Ok(Self {
            analysis: Analysis::default(),
            tail: [[Complex::default(); 32]; 2],
            previous_kx: 0,
            chirp: Chirp::new(noise)?,
            mapping: History::default(),
            assembly: Assembly::default(),
            synthesis: Synthesis::new(bands)?,
        })
    }
    fn low(&mut self, pcm: &[f32], kx: u8) -> Result<Vec<[Complex; 32]>> {
        let mut analysis = self.analysis.process(pcm)?;
        if analysis.len() < 2 || !(1..=32).contains(&kx) {
            return Err(invalid("invalid low-delay SBR analysis geometry"));
        }
        for row in &mut analysis {
            for value in row {
                value.re *= 32768.;
                value.im *= 32768.;
            }
        }
        let mut low = Vec::with_capacity(analysis.len() + 2);
        for (rows, cutoff) in [
            (&self.tail[..], self.previous_kx),
            (analysis.as_slice(), kx),
        ] {
            for row in rows {
                let mut v = [Complex::default(); 32];
                v[..usize::from(cutoff)].copy_from_slice(&row[..usize::from(cutoff)]);
                low.push(v);
            }
        }
        self.tail.copy_from_slice(&analysis[analysis.len() - 2..]);
        self.previous_kx = kx;
        Ok(low)
    }
    fn output(&mut self, rows: &[[Complex; 64]], bands: usize) -> Result<Vec<f32>> {
        // The 32-band normative modulation has twice the full-rate coefficient.
        // Restore the same normalized PCM amplitude at both output sample rates.
        let divisor = if bands == 32 { 65536. } else { 32768. };
        let pcm: Vec<f32> = self
            .synthesis
            .process(rows)?
            .into_iter()
            .map(|v| (v / divisor) as f32)
            .collect();
        if pcm.iter().any(|v| !v.is_finite()) {
            return Err(invalid("low-delay SBR output exceeds finite PCM"));
        }
        Ok(pcm)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Dsp {
    channels: Vec<Channel>,
    bands: usize,
    format: Option<(u32, u8, usize)>,
    frequency: Option<FrequencyTables>,
}
impl Dsp {
    pub(crate) fn new(double_rate: bool) -> Self {
        Self {
            channels: Vec::new(),
            bands: if double_rate { 64 } else { 32 },
            format: None,
            frequency: None,
        }
    }
    pub(crate) fn reset(&mut self) {
        self.channels.clear();
        self.format = None;
        self.frequency = None;
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        f.vector(&self.channels)?;
        for c in &self.channels {
            c.analysis.visit_retained(f)?;
            c.synthesis.visit_retained(f)?;
            c.chirp.visit_retained(f)?;
        }
        if let Some(t) = &self.frequency {
            for v in [&t.master, &t.high, &t.low, &t.noise] {
                f.vector(v)?;
            }
        }
        Ok(())
    }
    pub(crate) fn process(
        &mut self,
        frame: &Frame,
        pcm: &[&[f32]],
        core_rate: u32,
        slots: u8,
    ) -> Result<Vec<Vec<f32>>> {
        let rate = core_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("low-delay SBR frequency overflow"))?;
        let header = &frame.syntax.header;
        let data = &frame.syntax.data;
        let tables = &data.frequency;
        if !matches!(slots, 15 | 16)
            || !matches!(pcm.len(), 1 | 2)
            || data.channels.len() != pcm.len()
            || frame.parameters.channels.len() != pcm.len()
            || FrequencyTables::from_header(header, rate)? != *tables
            || pcm
                .iter()
                .any(|p| p.len() != 32 * usize::from(slots) || p.iter().any(|v| !v.is_finite()))
            || data.has_ps_extension()
        {
            return Err(invalid("invalid low-delay SBR synthesis frame"));
        }
        let mut trial = self.clone();
        let format = (core_rate, slots, pcm.len());
        if trial.format.is_some_and(|f| f != format) && !frame.syntax.format_reset {
            return Err(invalid("low-delay SBR format changed without reset"));
        }
        if frame.syntax.format_reset {
            trial.reset();
        }
        if trial.frequency.as_ref().is_some_and(|t| t != tables) && !frame.syntax.header_reset {
            return Err(invalid(
                "low-delay SBR frequency changed without header reset",
            ));
        }
        let noise = tables.noise.len() - 1;
        if trial.channels.is_empty() {
            for _ in pcm {
                trial.channels.push(Channel::new(noise, trial.bands)?);
            }
        } else if frame.syntax.header_reset {
            for c in &mut trial.channels {
                c.chirp = Chirp::new(noise)?;
            }
        }
        let kx = tables.high[0];
        let stop = *tables.high.last().unwrap();
        let patches = aac_sbr_hf::patches(&tables.master, kx, rate)?;
        let borders = aac_sbr_limiter::borders(&tables.low, &patches, header.limiter_bands)?;
        let mut output = Vec::with_capacity(pcm.len());
        for (i, samples) in pcm.iter().enumerate() {
            let c = &mut trial.channels[i];
            let syntax = &data.channels[i];
            let parameters = &frame.parameters.channels[i];
            let grid = syntax.grid.time_grid(slots)?;
            let low = c.low(samples, kx)?;
            let bw = c.chirp.update(&syntax.inverse_filter)?;
            let high = aac_sbr_hf::generate_with_timing(
                &low,
                slots,
                &grid,
                tables.master[0],
                &patches,
                &tables.noise,
                bw,
                Timing::LowDelay,
            )?;
            let energy = aac_sbr_energy::estimate_with_timing(
                &high,
                slots,
                &grid,
                &syntax.grid.high_resolution,
                tables,
                header.interpolate_frequency,
                Timing::LowDelay,
            )?;
            let mapped = c.mapping.map(
                &syntax.grid,
                slots,
                tables,
                &parameters.envelope,
                &parameters.noise,
                &syntax.harmonics,
                &energy,
            )?;
            let mut levels = Vec::with_capacity(mapped.bands.len());
            for (bands, &suppress) in mapped.bands.iter().zip(&mapped.suppress_noise) {
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
            let mut rows = c.assembly.process_with_timing(
                &high,
                slots,
                &grid,
                kx,
                &levels,
                &mapped.suppress_noise,
                header.smoothing_mode,
                frame.syntax.header_reset,
                Timing::LowDelay,
            )?;
            for (row, source) in rows.iter_mut().zip(&low[2..]) {
                row[..usize::from(kx)].copy_from_slice(&source[..usize::from(kx)]);
                row[usize::from(stop)..].fill(Complex::default());
            }
            output.push(c.output(&rows, trial.bands)?);
        }
        trial.format = Some(format);
        trial.frequency = Some(tables.clone());
        *self = trial;
        Ok(output)
    }
    pub(crate) fn upsample_lfe(&mut self, pcm: &[f32], slots: u8) -> Result<Vec<f32>> {
        if !matches!(slots, 15 | 16) || pcm.len() != 32 * usize::from(slots) {
            return Err(invalid("invalid low-delay SBR LFE frame"));
        }
        let mut trial = self.clone();
        if trial.channels.is_empty() {
            trial.channels.push(Channel::new(1, trial.bands)?);
        }
        let low = trial.channels[0].low(pcm, 32)?;
        let rows: Vec<_> = low[2..]
            .iter()
            .map(|r| std::array::from_fn(|k| if k < 32 { r[k] } else { Complex::default() }))
            .collect();
        let output = trial.channels[0].output(&rows, trial.bands)?;
        *self = trial;
        Ok(output)
    }
}
