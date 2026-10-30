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
/// The spatial domain uses the LD-MPS analysis phase/window; ordinary
/// LD-SBR retains CLDFB. A spatial channel never runs a PCM synthesis bank.
#[derive(Clone, Debug, PartialEq)]
enum AnalysisBank {
    Cldfb(Analysis),
    Spatial(super::aac_ld_qmf::Analysis<32>),
}
impl AnalysisBank {
    fn process(&mut self, pcm: &[f32]) -> Result<Vec<[Complex; 32]>> {
        match self {
            Self::Cldfb(bank) => bank.process(pcm),
            Self::Spatial(bank) => {
                bank.process(&pcm.iter().map(|&v| f64::from(v)).collect::<Vec<_>>())
            }
        }
    }
    fn bands(&self) -> usize {
        match self {
            Self::Cldfb(bank) => bank.bands(),
            Self::Spatial(_) => 32,
        }
    }
    fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        match self {
            Self::Cldfb(bank) => bank.visit_retained(f),
            Self::Spatial(bank) => bank.visit_retained(f),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
struct Channel {
    analysis: AnalysisBank,
    full_analysis: Option<AnalysisBank>,
    factor: usize,
    tail: [[Complex; 32]; 2],
    previous_kx: u8,
    chirp: Chirp,
    mapping: History,
    assembly: Assembly,
    synthesis: Option<Synthesis>,
}
impl Channel {
    fn new(noise: usize, bands: usize, factor: usize, spatial: bool) -> Result<Self> {
        if spatial && factor != 1 {
            return Err(invalid("spatial LD-SBR downscale is not yet implemented"));
        }
        Ok(Self {
            analysis: if spatial {
                AnalysisBank::Spatial(super::aac_ld_qmf::Analysis::new()?)
            } else {
                AnalysisBank::Cldfb(if factor == 1 {
                    Analysis::default()
                } else {
                    Analysis::new_downscaled(factor)?
                })
            },
            full_analysis: if factor == 1 {
                None
            } else {
                Some(AnalysisBank::Cldfb(Analysis::new_downscaled(1)?))
            },
            factor,
            tail: [[Complex::default(); 32]; 2],
            previous_kx: 0,
            chirp: Chirp::new(noise)?,
            mapping: History::default(),
            assembly: Assembly::default(),
            synthesis: if spatial {
                None
            } else {
                Some(if factor == 1 {
                    Synthesis::new(bands)?
                } else {
                    Synthesis::new_downscaled(bands, factor)?
                })
            },
        })
    }
    fn low(&mut self, pcm: &[f32], full: Option<&[f32]>, kx: u8) -> Result<Vec<[Complex; 32]>> {
        let short = self.analysis.process(pcm)?;
        let full =
            match &mut self.full_analysis {
                Some(bank) => Some(bank.process(
                    full.ok_or_else(|| invalid("missing LD-SBR full analysis companion"))?,
                )?),
                None => None,
            };
        let use_full = usize::from(kx) > self.analysis.bands();
        let mut analysis = if use_full {
            full.as_ref()
                .ok_or_else(|| invalid("missing LD-SBR crossover analysis"))?
                .clone()
        } else {
            short.clone()
        };
        let scale = 32768. * if use_full { 1. } else { self.factor as f64 };
        if analysis.len() < 2 || !(1..=32).contains(&kx) {
            return Err(invalid("invalid low-delay SBR analysis geometry"));
        }
        for row in &mut analysis {
            for value in row {
                value.re *= scale;
                value.im *= scale;
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
        // HF prediction consumes the two LF columns actually emitted before
        // this frame. Using the newly selected bank's cached past columns
        // changes the predecessor signal at a short/full crossover switch.
        self.tail.copy_from_slice(&analysis[analysis.len() - 2..]);
        self.previous_kx = kx;
        Ok(low)
    }
    fn output(&mut self, rows: &[[Complex; 64]], bands: usize) -> Result<Vec<f32>> {
        // The 32-band normative modulation has twice the full-rate coefficient.
        // Restore the same normalized PCM amplitude at both output sample rates.
        let divisor = if self.factor == 1 && bands == 32 {
            65536.
        } else {
            32768.
        };
        let pcm: Vec<f32> = self
            .synthesis
            .as_mut()
            .ok_or_else(|| invalid("spatial LD-SBR has no PCM synthesis bank"))?
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
    spatial: bool,
    factor: usize,
    channels: Vec<Channel>,
    bands: usize,
    format: Option<(u32, u8, usize)>,
    frequency: Option<FrequencyTables>,
}
impl Dsp {
    pub(crate) fn new(double_rate: bool) -> Self {
        Self {
            spatial: false,
            factor: 1,
            channels: Vec::new(),
            bands: if double_rate { 64 } else { 32 },
            format: None,
            frequency: None,
        }
    }
    pub(crate) fn new_spatial(double_rate: bool) -> Self {
        let mut dsp = Self::new(double_rate);
        dsp.spatial = true;
        dsp
    }
    pub(crate) fn new_downscaled(double_rate: bool, factor: usize) -> Self {
        let mut dsp = Self::new(double_rate);
        dsp.factor = factor;
        dsp
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
            if let Some(full) = &c.full_analysis {
                full.visit_retained(f)?;
            }
            if let Some(synthesis) = &c.synthesis {
                synthesis.visit_retained(f)?;
            }
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
        full: Option<&[&[f32]]>,
    ) -> Result<Vec<Vec<f32>>> {
        if self.spatial {
            return Err(invalid("spatial LD-SBR requires QMF output"));
        }
        let (mut trial, rows) = self.prepare_rows(frame, pcm, core_rate, slots, full)?;
        let mut output = Vec::with_capacity(rows.len());
        for (channel, rows) in trial.channels.iter_mut().zip(rows) {
            output.push(channel.output(&rows, trial.bands)?);
        }
        *self = trial;
        Ok(output)
    }
    pub(crate) fn process_qmf(
        &mut self,
        frame: &Frame,
        pcm: &[&[f32]],
        core_rate: u32,
        slots: u8,
    ) -> Result<Vec<Vec<Vec<Complex>>>> {
        if !self.spatial {
            return Err(invalid("CLDFB LD-SBR cannot supply spatial QMF"));
        }
        let (trial, rows) = self.prepare_rows(frame, pcm, core_rate, slots, None)?;
        let output = normalized(&rows, trial.bands)?;
        *self = trial;
        Ok(output)
    }
    fn prepare_rows(
        &self,
        frame: &Frame,
        pcm: &[&[f32]],
        core_rate: u32,
        slots: u8,
        full: Option<&[&[f32]]>,
    ) -> Result<(Self, Vec<Vec<[Complex; 64]>>)> {
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
            || pcm.iter().any(|p| {
                p.len() != 32 / self.factor * usize::from(slots) || p.iter().any(|v| !v.is_finite())
            })
            || data.has_ps_extension()
        {
            return Err(invalid("invalid low-delay SBR synthesis frame"));
        }
        if self.factor != 1
            && full.is_none_or(|p| {
                p.len() != pcm.len() || p.iter().any(|v| v.len() != 32 * usize::from(slots))
            })
        {
            return Err(invalid("invalid LD-SBR full companion geometry"));
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
                trial.channels.push(Channel::new(
                    noise,
                    trial.bands,
                    trial.factor,
                    trial.spatial,
                )?);
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
            let low = c.low(samples, full.map(|p| p[i]), kx)?;
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
            output.push(rows);
        }
        trial.format = Some(format);
        trial.frequency = Some(tables.clone());
        Ok((trial, output))
    }
    pub(crate) fn upsample_lfe(
        &mut self,
        pcm: &[f32],
        slots: u8,
        full: Option<&[f32]>,
    ) -> Result<Vec<f32>> {
        if self.spatial {
            return Err(invalid("spatial LD-SBR requires QMF LFE output"));
        }
        let (mut trial, rows) = self.prepare_lfe(pcm, slots, full)?;
        let output = trial.channels[0].output(&rows, trial.bands)?;
        *self = trial;
        Ok(output)
    }
    pub(crate) fn upsample_lfe_qmf(&mut self, pcm: &[f32], slots: u8) -> Result<Vec<Vec<Complex>>> {
        if !self.spatial {
            return Err(invalid("CLDFB LD-SBR cannot supply spatial LFE QMF"));
        }
        let (trial, rows) = self.prepare_lfe(pcm, slots, None)?;
        let mut output = normalized(&[rows], trial.bands)?;
        *self = trial;
        Ok(output.remove(0))
    }
    fn prepare_lfe(
        &self,
        pcm: &[f32],
        slots: u8,
        full: Option<&[f32]>,
    ) -> Result<(Self, Vec<[Complex; 64]>)> {
        if !matches!(slots, 15 | 16) || pcm.len() != 32 / self.factor * usize::from(slots) {
            return Err(invalid("invalid low-delay SBR LFE frame"));
        }
        let mut trial = self.clone();
        if trial.channels.is_empty() {
            trial
                .channels
                .push(Channel::new(1, trial.bands, trial.factor, trial.spatial)?);
        }
        let low = trial.channels[0].low(pcm, full, 32)?;
        let rows: Vec<_> = low[2..]
            .iter()
            .map(|r| std::array::from_fn(|k| if k < 32 { r[k] } else { Complex::default() }))
            .collect();
        Ok((trial, rows))
    }
}

/// SBR processing uses signed-16-bit signal units. Spatial synthesis owns
/// the only PCM bank, so hand off normalized subbands without a round trip.
fn normalized(rows: &[Vec<[Complex; 64]>], bands: usize) -> Result<Vec<Vec<Vec<Complex>>>> {
    let mut output = Vec::with_capacity(rows.len());
    for channel in rows {
        let mut slots = Vec::with_capacity(channel.len());
        for row in channel {
            let values: Vec<_> = row[..bands]
                .iter()
                .map(|v| Complex {
                    re: v.re / 32768.,
                    im: v.im / 32768.,
                })
                .collect();
            if values
                .iter()
                .any(|v| !v.re.is_finite() || !v.im.is_finite())
            {
                return Err(invalid("nonfinite spatial LD-SBR QMF output"));
            }
            slots.push(values);
        }
        output.push(slots);
    }
    Ok(output)
}

#[cfg(test)]
mod crossover_tests {
    use super::*;
    #[test]
    fn predictor_prefix_is_the_preceding_emitted_signal_in_both_bank_directions() {
        let mut channel = Channel::new(1, 64, 8, false).unwrap();
        let mut previous = None::<Vec<[Complex; 32]>>;
        for (frame, kx) in [4, 5, 4, 5, 4].into_iter().enumerate() {
            let short: Vec<f32> = (0..60)
                .map(|i| ((i + frame * 60) as f32 * 0.13).sin() * 0.2)
                .collect();
            let full: Vec<f32> = (0..480)
                .map(|i| ((i + frame * 480) as f32 * 0.07).cos() * 0.1)
                .collect();
            let rows = channel.low(&short, Some(&full), kx).unwrap();
            if let Some(old) = previous {
                assert_eq!(&rows[..2], &old[old.len() - 2..]);
            } else {
                assert_eq!(rows[..2], [[Complex::default(); 32]; 2]);
            }
            previous = Some(rows);
        }
    }
}
