//! Owned PS common-band mapping, before dequantization and complex mixing.
//! GOST R 53556.8-2013 6.4.6.1 and tables 44-49. Integer averages truncate
//! toward zero (ANSI C), including negative IID. Phase uses the lower portion
//! of the same maps and explicit upper-band zeros. Real h_ij transitions use
//! floating averages; they must never pass through integer parameter mapping.
use super::{
    Result,
    aac_ps_data::{IccMode, IidMode},
    aac_ps_dequant,
    aac_ps_history::{Indices as NativeIndices, Parameters},
    aac_ps_mapping_tables::*,
    invalid,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bands {
    #[default]
    Twenty,
    ThirtyFour,
}
impl Bands {
    pub fn count(self) -> usize {
        match self {
            Self::Twenty => 20,
            Self::ThirtyFour => 34,
        }
    }
    pub fn phase_bands(self) -> usize {
        match self {
            Self::Twenty => 11,
            Self::ThirtyFour => 17,
        }
    }
    pub fn bindings(self) -> &'static [Binding] {
        match self {
            Self::Twenty => HYBRID_20,
            Self::ThirtyFour => HYBRID_34,
        }
    }
    pub fn binding(self, hybrid_subband: usize) -> Result<Binding> {
        self.bindings()
            .get(hybrid_subband)
            .copied()
            .ok_or_else(|| invalid("PS hybrid subband index exceeds configuration"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub qmf: u8,
    pub parameter: u8,
    /// Conjugate the complex stereo mixing coefficients at this subband,
    /// not the incoming QMF audio. These are the starred table 48/49 entries.
    pub conjugate: bool,
}

fn integer_rows(values: &[i16], rows: &[(&[(u8, u8)], u8)]) -> Vec<i16> {
    rows.iter()
        .map(|&(terms, denominator)| {
            let sum = terms
                .iter()
                .map(|&(index, weight)| i32::from(values[usize::from(index)]) * i32::from(weight))
                .sum::<i32>();
            (sum / i32::from(denominator)) as i16
        })
        .collect()
}

/// Full integer parameter grids, 10/20/34 -> common 20/34. This primitive
/// accepts the full i16 domain; the parameter owner validates quantizer grids.
/// Convex normative weights fit i32 even at signed i16 endpoints.
pub fn map_indices(values: &[i16], target: Bands) -> Result<Vec<i16>> {
    match (values.len(), target) {
        (10, _) => {
            let twenty: Vec<i16> = values.iter().flat_map(|&v| [v, v]).collect();
            Ok(if target == Bands::Twenty {
                twenty
            } else {
                integer_rows(&twenty, UP)
            })
        }
        (20, Bands::Twenty) | (34, Bands::ThirtyFour) => Ok(values.to_vec()),
        (20, Bands::ThirtyFour) => Ok(integer_rows(values, UP)),
        (34, Bands::Twenty) => Ok(integer_rows(values, DOWN)),
        _ => Err(invalid("invalid PS integer mapping source dimensions")),
    }
}

/// Native phase grids 5/11/17, with index zero outside transmitted phase
/// bands. Follow IID mapping without replacing it with circular interpolation.
/// The later phase smoothing stage operates on complex values separately.
pub fn map_phase(values: &[i16], target: Bands) -> Result<Vec<i16>> {
    let native = match values.len() {
        5 => 10,
        11 => 20,
        17 => 34,
        _ => return Err(invalid("invalid PS phase mapping source dimensions")),
    };
    if values.iter().any(|v| !(0..=7).contains(v)) {
        return Err(invalid("PS phase index exceeds quantization grid"));
    }
    let mut padded = vec![0; native];
    padded[..values.len()].copy_from_slice(values);
    let mut mapped = map_indices(&padded, target)?;
    mapped[target.phase_bands()..].fill(0);
    Ok(mapped)
}

/// Transition a real h_11/h_12/h_21/h_22 vector between common band grids.
/// No integer rounding and no phase rotation here: on a configuration change
/// the caller resets phase smoothing/decorrelator and rebuilds complex H_ij.
pub fn map_coefficients(values: &[f64], target: Bands) -> Result<Vec<f64>> {
    if !matches!(values.len(), 20 | 34) || values.iter().any(|v| !v.is_finite()) {
        return Err(invalid("invalid PS coefficient mapping source"));
    }
    let rows = match (values.len(), target) {
        (20, Bands::Twenty) | (34, Bands::ThirtyFour) => return Ok(values.to_vec()),
        (20, Bands::ThirtyFour) => UP,
        (34, Bands::Twenty) => DOWN,
        _ => unreachable!(),
    };
    let output: Vec<f64> = rows
        .iter()
        .map(|&(terms, denominator)| {
            terms
                .iter()
                .map(|&(index, weight)| {
                    values[usize::from(index)] * (f64::from(weight) / f64::from(denominator))
                })
                .sum()
        })
        .collect();
    if output.iter().any(|v| !v.is_finite()) {
        return Err(invalid("unrepresentable PS mapped coefficient"));
    }
    Ok(output)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indices {
    pub bands: Bands,
    /// Native modes retain quantization/mixing semantics; `bands` gives the
    /// common vector geometry, which may differ from those native modes.
    pub iid_mode: IidMode,
    pub icc_mode: IccMode,
    pub iid_enabled: bool,
    pub icc_enabled: bool,
    pub phase_enabled: bool,
    pub iid: Vec<i16>,
    pub icc: Vec<i16>,
    pub ipd: Vec<i16>,
    pub opd: Vec<i16>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Levels {
    pub bands: Bands,
    pub iid_db: Vec<f64>,
    pub coherence: Vec<f64>,
    pub ipd_radians: Vec<f64>,
    pub opd_radians: Vec<f64>,
}
impl Indices {
    pub fn validate(&self) -> Result<()> {
        let count = self.bands.count();
        if [&self.iid, &self.icc, &self.ipd, &self.opd]
            .iter()
            .any(|v| v.len() != count)
            || (!self.iid_enabled && self.iid.iter().any(|&v| v != 0))
            || (!self.icc_enabled && self.icc.iter().any(|&v| v != 0))
            || (self.phase_enabled && !self.iid_enabled)
            || (!self.phase_enabled && self.ipd.iter().chain(&self.opd).any(|&v| v != 0))
            || self.ipd[self.bands.phase_bands()..]
                .iter()
                .chain(&self.opd[self.bands.phase_bands()..])
                .any(|&v| v != 0)
        {
            return Err(invalid(
                "invalid PS common parameter dimensions or defaults",
            ));
        }
        for &v in &self.iid {
            aac_ps_dequant::iid_db(self.iid_mode, v)?;
        }
        for &v in &self.icc {
            aac_ps_dequant::coherence(v)?;
        }
        for &v in self.ipd.iter().chain(&self.opd) {
            aac_ps_dequant::phase_radians(v)?;
        }
        Ok(())
    }
    pub fn dequantize(&self) -> Result<Levels> {
        self.validate()?;
        Ok(Levels {
            bands: self.bands,
            iid_db: self
                .iid
                .iter()
                .map(|&v| aac_ps_dequant::iid_db(self.iid_mode, v))
                .collect::<Result<_>>()?,
            coherence: self
                .icc
                .iter()
                .map(|&v| aac_ps_dequant::coherence(v))
                .collect::<Result<_>>()?,
            ipd_radians: self
                .ipd
                .iter()
                .map(|&v| aac_ps_dequant::phase_radians(v))
                .collect::<Result<_>>()?,
            opd_radians: self
                .opd
                .iter()
                .map(|&v| aac_ps_dequant::phase_radians(v))
                .collect::<Result<_>>()?,
        })
    }
}

pub fn map_parameters(native: &NativeIndices, target: Bands) -> Result<Indices> {
    native.validate()?;
    let mapped = Indices {
        bands: target,
        iid_mode: native.iid_mode,
        icc_mode: native.icc_mode,
        iid_enabled: native.iid_enabled,
        icc_enabled: native.icc_enabled,
        phase_enabled: native.phase_enabled,
        iid: map_indices(&native.iid, target)?,
        icc: map_indices(&native.icc, target)?,
        ipd: map_phase(&native.ipd, target)?,
        opd: map_phase(&native.opd, target)?,
    };
    mapped.validate()?;
    Ok(mapped)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub previous_bands: Bands,
    pub bands: Bands,
    pub initialized: bool,
    pub phase_enabled: bool,
    pub borders: Vec<u8>,
    /// Only newly transmitted envelopes. Zero envelopes retain the previous
    /// *real mixing coefficient* vectors: map_coefficients handles any band
    /// change before phase states are reset/rebuilt. Do not invent envelopes
    /// or remap/dequantize old integer indices as new mixing coefficients.
    pub envelopes: Vec<Indices>,
}
impl Frame {
    pub fn bands_changed(&self) -> bool {
        self.previous_bands != self.bands
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    bands: Bands,
}
impl State {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn bands(&self) -> Bands {
        self.bands
    }
    /// Disabled tools count as 20 bands. When both are disabled the previous
    /// common grid is retained, regardless of stale native mode fields.
    pub fn process(&mut self, parameters: &Parameters) -> Result<Frame> {
        let header = &parameters.header;
        if parameters.envelopes.len() != parameters.borders.len()
            || parameters.envelopes.len() > 4
            || parameters.borders.iter().any(|&b| b >= 32)
            || parameters.borders.windows(2).any(|b| b[0] >= b[1])
            || header.iid.is_some_and(|m| m != parameters.iid_mode)
            || header.icc.is_some_and(|m| m != parameters.icc_mode)
            || (parameters.phase_enabled && (!header.extension || header.iid.is_none()))
        {
            return Err(invalid("invalid PS common mapping frame controls"));
        }
        parameters.retained.validate()?;
        if parameters
            .envelopes
            .last()
            .is_some_and(|v| v != &parameters.retained)
        {
            return Err(invalid("inconsistent PS retained last envelope"));
        }
        let bands = if header.iid.is_none() && header.icc.is_none() {
            self.bands
        } else if header.iid.is_some_and(|m| m.bands() == 34)
            || header.icc.is_some_and(|m| m.bands() == 34)
        {
            Bands::ThirtyFour
        } else {
            Bands::Twenty
        };
        let mut envelopes = Vec::with_capacity(parameters.envelopes.len());
        for native in &parameters.envelopes {
            if native.iid_mode != parameters.iid_mode
                || native.icc_mode != parameters.icc_mode
                || native.iid_enabled != header.iid.is_some()
                || native.icc_enabled != header.icc.is_some()
                || native.phase_enabled != parameters.phase_enabled
            {
                return Err(invalid("inconsistent PS native envelope configuration"));
            }
            envelopes.push(map_parameters(native, bands)?);
        }
        let result = Frame {
            previous_bands: self.bands,
            bands,
            initialized: parameters.initialized,
            phase_enabled: parameters.phase_enabled,
            borders: parameters.borders.clone(),
            envelopes,
        };
        self.bands = bands;
        Ok(result)
    }
}
