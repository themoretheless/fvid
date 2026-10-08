//! Owned two-position PS phase history on the common parameter grid.
//! Configuration changes reset smoothing; zero new envelopes do not invent
//! parameter positions. This produces only new endpoints, not retained real
//! coefficient remapping or the hybrid/decorrelator/PCM stages.
use super::{
    Result,
    aac_ps_mapping::{Bands, Frame as Parameters},
    aac_ps_mixing::{self, ComplexMatrix},
    invalid,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    bands: Bands,
    initialized: bool,
    ipd: [Vec<i16>; 2],
    opd: [Vec<i16>; 2],
}
impl Default for State {
    fn default() -> Self {
        Self::new(Bands::Twenty)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub bands: Bands,
    pub bands_changed: bool,
    pub endpoints: Vec<Vec<ComplexMatrix>>,
}
impl Frame {
    /// Route an endpoint to a hybrid subband, applying the starred complex
    /// coefficient conjugation. Audio itself is never conjugated here.
    pub fn hybrid_endpoint(&self, envelope: usize, subband: usize) -> Result<ComplexMatrix> {
        let binding = self.bands.binding(subband)?;
        let values = self
            .endpoints
            .get(envelope)
            .ok_or_else(|| invalid("PS phase envelope index exceeds frame"))?;
        if values.len() != self.bands.count() {
            return Err(invalid("invalid PS phase endpoint dimensions"));
        }
        let mut m = values[usize::from(binding.parameter)];
        if m.coefficients()
            .iter()
            .any(|v| !v.re.is_finite() || !v.im.is_finite())
        {
            return Err(invalid("invalid PS phase endpoint coefficients"));
        }

        if binding.conjugate {
            m.h11.im = -m.h11.im;
            m.h12.im = -m.h12.im;
            m.h21.im = -m.h21.im;
            m.h22.im = -m.h22.im
        }
        Ok(m)
    }
}
impl State {
    pub fn new(bands: Bands) -> Self {
        Self {
            bands,
            initialized: false,
            ipd: std::array::from_fn(|_| vec![0; bands.count()]),
            opd: std::array::from_fn(|_| vec![0; bands.count()]),
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default()
    }
    pub fn bands(&self) -> Bands {
        self.bands
    }
    /// Ordered oldest/previous parameter positions; both zero at startup.
    pub fn ipd(&self) -> [&[i16]; 2] {
        [&self.ipd[0], &self.ipd[1]]
    }
    pub fn opd(&self) -> [&[i16]; 2] {
        [&self.opd[0], &self.opd[1]]
    }
    /// Before the first accepted frame there are no physical phase positions:
    /// a parser may already have selected a new grid in an unready header.
    /// Start at zero on the FIRST ready grid, then require continuity of the
    /// previous common band field on all later accepted frames.
    pub fn process(&mut self, parameters: &Parameters) -> Result<Frame> {
        if !parameters.initialized {
            return Err(invalid("PS phase history requires initialized parameters"));
        }
        if !self.initialized && parameters.envelopes.is_empty() {
            return Err(invalid("PS phase history startup requires new envelopes"));
        }
        if (self.initialized && parameters.previous_bands != self.bands)
            || parameters.borders.len() != parameters.envelopes.len()
            || parameters.envelopes.len() > 4
            || parameters.borders.iter().any(|&n| n >= 32)
            || parameters.borders.windows(2).any(|b| b[0] >= b[1])
            || parameters
                .envelopes
                .iter()
                .any(|v| v.bands != parameters.bands || v.phase_enabled != parameters.phase_enabled)
        {
            return Err(invalid("invalid PS phase history frame controls"));
        }
        let changed = self.bands != parameters.bands;
        let mut trial = if changed {
            Self::new(parameters.bands)
        } else {
            self.clone()
        };
        let mut endpoints = Vec::with_capacity(parameters.envelopes.len());
        for envelope in &parameters.envelopes {
            envelope.validate()?;
            let real = aac_ps_mixing::envelope(envelope)?;
            let mut complex = Vec::with_capacity(trial.bands.count());
            for (b, real) in real.into_iter().enumerate() {
                let (ipd, opd) = if envelope.phase_enabled {
                    (
                        [trial.ipd[0][b], trial.ipd[1][b], envelope.ipd[b]],
                        [trial.opd[0][b], trial.opd[1][b], envelope.opd[b]],
                    )
                } else {
                    ([0; 3], [0; 3])
                };
                complex.push(aac_ps_mixing::phase_matrix(real, ipd, opd, false)?);
            }
            // Disabled phase has a DEFAULT zero parameter at each new
            // position; it does not erase the preceding position wholesale.
            trial.ipd.rotate_left(1);
            trial.opd.rotate_left(1);
            trial.ipd[1].clone_from(&envelope.ipd);
            trial.opd[1].clone_from(&envelope.opd);
            endpoints.push(complex);
        }
        let frame = Frame {
            bands: trial.bands,
            bands_changed: changed,
            endpoints,
        };
        trial.initialized = true;
        *self = trial;
        Ok(frame)
    }
}
