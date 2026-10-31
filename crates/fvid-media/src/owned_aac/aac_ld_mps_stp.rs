//! LD-MPS STP in normalized full-complex LD-QMF units.
//!
//! Preserve the ISO reference's defined active-bin weighting: wet bins below12
//! are merged before energy analysis; bins12..15 use BP[bin-7], then unity.
//! Never evaluate its invalid negative BP indices in the zeroed wet interval.
use super::aac_ld_mps_matrix_apply::Samples;
use super::aac_ld_mps_stp_energy::EnergyTracker;
use super::{Result, aac_sbr_qmf::Complex, invalid, memory::Footprint};
const BP: [f64; 25] = [
    0., 0.0005, 0.0092, 0.0587, 0.2580, 0.7392, 0.9791, 0.9993, 1., 1., 1., 1., 0.9999, 0.9984,
    0.9908, 0.9639, 0.8952, 0.7711, 0.6127, 0.4609, 0.3391, 0.2493, 0.1848, 0.1387, 0.1053,
];
const GF: [f64; 25] = [
    0., 0., 0., 0., 0., 0., 1e-8, 8.1e-7, 3.61e-6, 8.41e-6, 1.6e-5, 2.704e-5, 3.969e-5, 5.625e-5,
    7.396e-5, 9.801e-5, 0.00012321, 0.00015625, 0.00019881, 0.00024964, 0.00032041, 0.00041209,
    0.00053824, 0.00070756, 0.00094249,
];
const FLAGS: [&[i8]; 8] = [
    &[0, 1, 2, -1, 3, 4],
    &[0, 3, 1, 4, 2, -1],
    &[0, 2, 1, 3, -1, -1],
    &[0, 4, 2, 1, 5, 3, -1, -1],
    &[0, 4, 2, 1, 5, 3, -1, -1],
    &[0, 2, -1, 1, 3, -1, -1, -1],
    &[-1, 2, 0, -1, 3, 1, -1, -1],
    &[0, 1],
];
fn groups(tree: u8) -> &'static [&'static [usize]] {
    match tree {
        0 => &[&[0, 1, 2, 4, 5]],
        1 => &[&[0, 2, 4, 1, 3]],
        2 => &[&[0, 1], &[2, 3]],
        3 | 4 => &[&[0, 2, 1], &[3, 5, 4]],
        5 => &[&[0, 1], &[3, 4]],
        6 => &[&[2, 1], &[5, 4]],
        7 => &[&[0, 1]],
        _ => unreachable!(),
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    tree: u8,
    bands: usize,
    energy: EnergyTracker,
}
impl Decoder {
    pub fn new(tree: u8, bands: usize) -> Result<Self> {
        if tree > 7 || !matches!(bands, 32 | 64 | 128) {
            return Err(invalid("invalid LD-MPS STP geometry"));
        }
        let inputs = groups(tree);
        let map: Vec<_> = (0..FLAGS[tree as usize].len())
            .map(|row| inputs.iter().position(|g| g.contains(&row)))
            .collect();
        Ok(Self {
            tree,
            bands,
            energy: EnergyTracker::new(inputs.len(), &map)?,
        })
    }
    pub fn reset(&mut self) {
        self.energy.reset();
    }
    /// dry/wet retain matrix-native channel order. None disables output shaping,
    /// but still advances energy and gain histories. All mutations are atomic.
    pub fn apply(
        &mut self,
        dry: &mut Samples,
        wet: &mut Samples,
        flags: Option<&[bool]>,
    ) -> Result<()> {
        let map = FLAGS[self.tree as usize];
        let count = map.iter().copied().max().unwrap() as usize + 1;
        let valid = |s: &Samples| {
            s.len() == dry.len()
                && s.iter().all(|slot| {
                    slot.len() == map.len()
                        && slot.iter().all(|row| {
                            row.len() == self.bands
                                && row.iter().all(|v| v.re.is_finite() && v.im.is_finite())
                        })
                })
        };
        if dry.is_empty()
            || dry.len() > 32
            || !valid(dry)
            || !valid(wet)
            || flags.is_some_and(|f| f.len() != count)
        {
            return Err(invalid("invalid LD-MPS STP samples or flags"));
        }
        let mut next = self.clone();
        let mut d = dry.clone();
        let mut w = wet.clone();
        for t in 0..d.len() {
            for row in 0..map.len() {
                for b in 0..12 {
                    d[t][row][b].re += w[t][row][b].re;
                    d[t][row][b].im += w[t][row][b].im;
                    w[t][row][b] = Complex::default();
                }
            }
            let dry_energy: Vec<_> = groups(self.tree)
                .iter()
                .map(|g| {
                    (1..25)
                        .map(|b| {
                            let mut z = Complex::default();
                            for &row in g.iter() {
                                z.re += d[t][row][b].re;
                                z.im += d[t][row][b].im;
                            }
                            let re = z.re * BP[b] * GF[b];
                            let im = z.im * BP[b] * GF[b];
                            re * re + im * im
                        })
                        .sum()
                })
                .collect();
            let wet_energy: Vec<_> = (0..map.len())
                .map(|row| {
                    (1..25)
                        .map(|b| {
                            let z = w[t][row][b];
                            (z.re * z.re + z.im * z.im) * BP[b] * BP[b] * GF[b] * GF[b]
                        })
                        .sum()
                })
                .collect();
            let gains = next.energy.step(&dry_energy, &wet_energy)?;
            for (row, &flag) in map.iter().enumerate() {
                if flag >= 0 && flags.is_some_and(|f| f[flag as usize]) {
                    for b in 12..self.bands {
                        let gain = gains[row] * if b < 16 { BP[b - 7] } else { 1. };
                        w[t][row][b].re *= gain;
                        w[t][row][b].im *= gain;
                    }
                }
            }
        }
        if d.iter()
            .chain(&w)
            .flatten()
            .flatten()
            .any(|v| !v.re.is_finite() || !v.im.is_finite())
        {
            return Err(invalid("overflowing LD-MPS STP output"));
        }
        *self = next;
        *dry = d;
        *wet = w;
        Ok(())
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        self.energy.visit_retained(f)
    }
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        self.energy.retained_payload_bytes()
    }
}
