//! Owned PS common mapping, retained real matrices, phase and temporal control.
//! This accepts already reconstructed native parameters and hybrid signals;
//! hybrid analysis, decorrelation and stereo QMF synthesis remain separate.
use super::{
    Result,
    aac_ps_history::Parameters,
    aac_ps_interpolation,
    aac_ps_mapping::{self, Bands},
    aac_ps_mixing::{self, ComplexMatrix, Matrix},
    aac_ps_phase_history,
    aac_sbr_qmf::Complex,
    invalid,
};
fn real_boundary(values: &[Matrix]) -> Result<Vec<ComplexMatrix>> {
    values
        .iter()
        .map(|&m| aac_ps_mixing::phase_matrix(m, [0; 3], [0; 3], false))
        .collect()
}
fn remap(values: &[Matrix], bands: Bands) -> Result<Vec<Matrix>> {
    let mut columns = Vec::with_capacity(4);
    for column in 0..4 {
        let source: Vec<_> = values.iter().map(|m| m.coefficients()[column]).collect();
        columns.push(aac_ps_mapping::map_coefficients(&source, bands)?)
    }
    Ok((0..bands.count())
        .map(|b| Matrix {
            h11: columns[0][b],
            h12: columns[1][b],
            h21: columns[2][b],
            h22: columns[3][b],
        })
        .collect())
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub bands_changed: bool,
    pub temporal: aac_ps_interpolation::Frame,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HybridFrame {
    pub bands: Bands,
    pub bands_changed: bool,
    /// Left/right chronological slots of 71 or 91 hybrid samples.
    pub channels: [Vec<Vec<Complex>>; 2],
}
impl Frame {
    pub fn hybrid_matrix(&self, slot: usize, subband: usize) -> Result<ComplexMatrix> {
        let binding = self.temporal.bands.binding(subband)?;
        let row = self
            .temporal
            .coefficients
            .get(slot)
            .ok_or_else(|| invalid("PS matrix slot index exceeds frame"))?;
        if row.len() != self.temporal.bands.count() {
            return Err(invalid("invalid PS temporal matrix dimensions"));
        }
        let mut m = row[usize::from(binding.parameter)];
        if m.coefficients()
            .iter()
            .any(|c| !c.re.is_finite() || !c.im.is_finite())
        {
            return Err(invalid("invalid PS temporal matrix coefficients"));
        }
        if binding.conjugate {
            m.h11.im = -m.h11.im;
            m.h12.im = -m.h12.im;
            m.h21.im = -m.h21.im;
            m.h22.im = -m.h22.im
        }
        Ok(m)
    }
    pub fn mix(&self, mono: &[Vec<Complex>], decorrelated: &[Vec<Complex>]) -> Result<HybridFrame> {
        let slots = self.temporal.coefficients.len();
        let count = self.temporal.bands.bindings().len();
        if !matches!(slots, 24 | 30 | 32)
            || mono.len() != slots
            || decorrelated.len() != slots
            || mono.iter().chain(decorrelated).any(|v| v.len() != count)
        {
            return Err(invalid("invalid PS hybrid mixing signal dimensions"));
        }
        let mut channels = std::array::from_fn(|_| Vec::with_capacity(slots));
        for n in 0..slots {
            let mut output: [Vec<Complex>; 2] = std::array::from_fn(|_| Vec::with_capacity(count));
            for k in 0..count {
                let sample = self
                    .hybrid_matrix(n, k)?
                    .apply(mono[n][k], decorrelated[n][k])?;
                for c in 0..2 {
                    output[c].push(sample[c])
                }
            }
            for (c, row) in output.into_iter().enumerate() {
                channels[c].push(row)
            }
        }
        Ok(HybridFrame {
            bands: self.temporal.bands,
            bands_changed: self.bands_changed,
            channels,
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    mapping: aac_ps_mapping::State,
    phase: aac_ps_phase_history::State,
    temporal: aac_ps_interpolation::State,
    real: Vec<Matrix>,
    slots: Option<u8>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            mapping: Default::default(),
            phase: Default::default(),
            temporal: Default::default(),
            real: vec![Matrix::default(); 20],
            slots: None,
        }
    }
}
impl State {
    pub fn reset(&mut self) {
        *self = Self::default()
    }
    pub fn bands(&self) -> Bands {
        self.temporal.bands()
    }
    pub fn retained_real(&self) -> &[Matrix] {
        &self.real
    }
    pub fn retained_complex(&self) -> &[ComplexMatrix] {
        self.temporal.previous()
    }
    pub fn phase(&self) -> &aac_ps_phase_history::State {
        &self.phase
    }
    pub fn process(&mut self, parameters: &Parameters, slots: u8) -> Result<Frame> {
        if !parameters.initialized {
            return Err(invalid(
                "PS matrix controller requires initialized parameters",
            ));
        }
        if !matches!(slots, 24 | 30 | 32) || parameters.borders.iter().any(|&n| n >= slots) {
            return Err(invalid("invalid PS matrix controller slot geometry"));
        }
        if self.slots.is_some_and(|previous| previous != slots) {
            return Err(invalid(
                "PS matrix controller slot count changed without reset",
            ));
        }
        let mut trial = self.clone();
        let common = trial.mapping.process(parameters)?;
        let phase = trial.phase.process(&common)?;
        let changed = trial.temporal.bands() != common.bands;
        if changed {
            // Map REAL h_ij, not already rotated complex H or quantized IID.
            trial.real = remap(&trial.real, common.bands)?;
            trial
                .temporal
                .replace_previous(common.bands, &real_boundary(&trial.real)?)?;
        }
        if common.envelopes.is_empty() && !common.phase_enabled {
            // Normative retained-frame policy: phase disabled -> unrotated
            // h_ij; phase enabled -> actual previous-frame complex H_ij.
            trial
                .temporal
                .replace_previous(common.bands, &real_boundary(&trial.real)?)?;
        }
        let temporal = trial
            .temporal
            .process(slots, &common.borders, &phase.endpoints)?;
        if let Some(last) = common.envelopes.last() {
            trial.real = aac_ps_mixing::envelope(last)?
        }
        trial.slots = Some(slots);
        *self = trial;
        Ok(Frame {
            bands_changed: changed,
            temporal,
        })
    }
    /// Atomic parameter-state AND hybrid-signal mixing. A late nonfinite
    /// signal/coefficient or geometry error cannot consume phase history.
    pub fn process_and_mix(
        &mut self,
        parameters: &Parameters,
        slots: u8,
        mono: &[Vec<Complex>],
        decorrelated: &[Vec<Complex>],
    ) -> Result<HybridFrame> {
        let mut trial = self.clone();
        let output = trial.process(parameters, slots)?.mix(mono, decorrelated)?;
        *self = trial;
        Ok(output)
    }
}
