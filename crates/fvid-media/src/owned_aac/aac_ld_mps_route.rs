//! LD-MPS W-signal routing after M1/decorrelation, before the two M2 planes.
//! Residual QMF samples must already be decoded; this module has no codec state.
use super::{
    Result, aac_ld_mps_matrix_apply::Samples, aac_ld_mps_prepare::qmf_map, aac_sbr_qmf::Complex,
    invalid, unsupported,
};
#[derive(Clone, Debug, PartialEq)]
pub struct Routed {
    pub dry: Samples,
    pub wet: Samples,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Router {
    direct: usize,
    auxiliary_boxes: Vec<usize>,
    cutoffs: Vec<u8>,
    map: Vec<u8>,
    ttt_modes: Vec<u8>,
}
impl Router {
    /// Residual cutoffs are indexed by OTT box, followed by the TTT box (if any).
    /// TTT modes contain one mode per parameter band for trees 2/3/4; other trees
    /// require an empty list. Tree2 reserves the dormant sixth W column.
    pub fn new(
        tree: u8,
        parameter_bands: u8,
        qmf_bands: usize,
        cutoffs: Vec<u8>,
        ttt_modes: Vec<u8>,
    ) -> Result<Self> {
        let (direct, aux, boxes, lfe): (usize, &[usize], usize, Option<usize>) = match tree {
            0 => (1, &[0, 1, 3, 2], 5, Some(4)),
            1 => (1, &[0, 1, 3, 4], 5, Some(2)),
            2 => (3, &[1, 2, 0], 4, Some(0)),
            3 | 4 => (3, &[1, 2, 0, 3, 4], 6, Some(0)),
            5 | 6 => (6, &[0, 1], 2, None),
            7 => (1, &[0], 1, None),
            _ => return Err(invalid("invalid LD-MPS W routing tree")),
        };
        let map = qmf_map(parameter_bands, qmf_bands)?;
        if cutoffs.len() != boxes
            || cutoffs.iter().any(|&x| x > parameter_bands)
            || if matches!(tree, 2..=4) {
                ttt_modes.len() != usize::from(parameter_bands) || ttt_modes.iter().any(|&m| m > 5)
            } else {
                !ttt_modes.is_empty()
            }
        {
            return Err(invalid("invalid LD-MPS W routing configuration"));
        }
        if lfe.is_some_and(|b| cutoffs[b] != 0) {
            return Err(unsupported(
                "LD-MPS LFE residual routing is not yet qualified",
            ));
        }
        Ok(Self {
            direct,
            auxiliary_boxes: aux.to_vec(),
            cutoffs,
            map,
            ttt_modes,
        })
    }
    pub fn channels(&self) -> usize {
        self.direct + self.auxiliary_boxes.len()
    }
    /// All sample planes use [slot][channel][QMF]. Each optional residual box is
    /// [slot][QMF]. Active W residual regions require supplied decoded samples.
    /// TTT residual input belongs to M1 and is not a W auxiliary replacement.
    /// Input planes are borrowed and are never modified, including on errors.
    pub fn process(
        &self,
        virtuals: &[Vec<Vec<Complex>>],
        decorrelated: &[Vec<Vec<Complex>>],
        residuals: &[Option<Vec<Vec<Complex>>>],
    ) -> Result<Routed> {
        let slots = virtuals.len();
        let bins = self.map.len();
        let channels = self.channels();
        let finite = |z: &Complex| z.re.is_finite() && z.im.is_finite();
        let valid = |x: &[Vec<Vec<Complex>>]| {
            x.len() == slots
                && x.iter().all(|s| {
                    s.len() == channels && s.iter().all(|c| c.len() == bins && c.iter().all(finite))
                })
        };
        if slots > 32
            || !valid(virtuals)
            || !valid(decorrelated)
            || residuals.len() != self.cutoffs.len()
            || residuals.iter().enumerate().any(|(b, r)| match r {
                Some(r) => {
                    r.len() != slots || r.iter().any(|s| s.len() != bins || !s.iter().all(finite))
                }
                None => slots != 0 && self.auxiliary_boxes.contains(&b) && self.cutoffs[b] != 0,
            })
        {
            return Err(invalid("invalid or missing LD-MPS W routing samples"));
        }
        let mut dry = vec![vec![vec![Complex::default(); bins]; channels]; slots];
        let mut wet = dry.clone();
        for t in 0..slots {
            dry[t][..self.direct].clone_from_slice(&virtuals[t][..self.direct]);
            for (j, &b) in self.auxiliary_boxes.iter().enumerate() {
                let channel = self.direct + j;
                for (q, &band) in self.map.iter().enumerate() {
                    if band < self.cutoffs[b] {
                        dry[t][channel][q] = residuals[b].as_ref().unwrap()[t][q];
                    } else {
                        wet[t][channel][q] = decorrelated[t][channel][q];
                    }
                }
            }
            if !self.ttt_modes.is_empty() {
                for (q, &band) in self.map.iter().enumerate() {
                    if self.ttt_modes[usize::from(band)] == 0 {
                        // Combine original decorrelator outputs before the OTT residual masks.
                        // A masked OTT wet channel still contributes to this TTT basis.
                        let a = decorrelated[t][3][q];
                        let b = decorrelated[t][4][q];
                        let c = decorrelated[t][5][q];
                        let value = Complex {
                            re: a.re + b.re + std::f64::consts::FRAC_1_SQRT_2 * c.re,
                            im: a.im + b.im + std::f64::consts::FRAC_1_SQRT_2 * c.im,
                        };
                        if !finite(&value) {
                            return Err(invalid("overflowing LD-MPS TTT W mixture"));
                        }
                        wet[t][5][q] = value;
                    }
                }
            }
        }
        Ok(Routed { dry, wet })
    }
}
