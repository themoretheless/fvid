//! Owned LD-MPS broadband guided envelope shaping in normalized QMF units.
//! Envelope analysis precedes shaping; all histories advance on disabled slots.
use super::aac_ld_mps_matrix_apply::Samples;
use super::{Result, aac_sbr_qmf::Complex, invalid, memory::Footprint};
#[derive(Clone, Debug, PartialEq)]
struct Energy {
    parts: [f64; 9],
    frame: f64,
    norm: f64,
}
impl Default for Energy {
    fn default() -> Self {
        Self {
            parts: [0.; 9],
            frame: 0.,
            norm: 1.,
        }
    }
}
impl Energy {
    fn envelope(&mut self, signal: &[Vec<Complex>]) -> Result<Vec<f64>> {
        let alpha = (-64f64 / (0.4 * 44100.)).exp();
        let beta = (-64f64 / (0.04 * 44100.)).exp();
        let mut result = Vec::with_capacity(signal.len());
        for row in signal {
            let mut energy = [0.; 9];
            for (band, v) in row.iter().enumerate() {
                let group = match band {
                    0..=8 => band,
                    9..=10 => 9,
                    11..=13 => 10,
                    14..=17 => 11,
                    18..=22 => 12,
                    23..=34 => 13,
                    _ => 14,
                };
                if (10..=18).contains(&group) {
                    energy[group - 10] += v.re * v.re + v.im * v.im;
                }
            }
            let mean = energy.iter().sum::<f64>() / 9.;
            self.frame = alpha * self.frame + (1. - alpha) * mean;
            let mut weighted = 0.;
            for (p, value) in self.parts.iter_mut().zip(energy) {
                *p = alpha * (*p) + (1. - alpha) * value;
                weighted += value * self.frame / (*p + 1e-9);
            }
            self.norm = beta * self.norm + (1. - beta) * weighted;
            let value = (weighted / (self.norm + 1e-9)).sqrt();
            if !value.is_finite()
                || !self.frame.is_finite()
                || !self.norm.is_finite()
                || self.parts.iter().any(|v| !v.is_finite())
            {
                return Err(invalid("overflowing LD-MPS GES envelope history"));
            }
            result.push(value);
        }
        Ok(result)
    }
}
const CHANNELS: [&[i8]; 8] = [
    &[0, 1, 2, -1, 3, 4],
    &[0, 3, 1, 4, 2, -1],
    &[0, 3, 1, 4, 2, -1],
    &[0, 5, 3, 1, 6, 4, 2, -1],
    &[0, 5, 3, 1, 6, 4, 2, -1],
    &[0, 2, -1, 1, 3, -1, -1, -1],
    &[-1, 2, 0, -1, 3, 1, -1, -1],
    &[0, 1],
];
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    tree: u8,
    bands: usize,
    inputs: usize,
    downmix: Vec<Energy>,
    output: Vec<Energy>,
}
impl Decoder {
    pub fn new(tree: u8, bands: usize) -> Result<Self> {
        if tree > 7 || !matches!(bands, 32 | 64 | 128) {
            return Err(invalid("invalid LD-MPS GES geometry"));
        }
        let inputs = [1, 1, 2, 2, 2, 6, 6, 1][usize::from(tree)];
        Ok(Self {
            tree,
            bands,
            inputs,
            downmix: vec![Energy::default(); inputs.min(2)],
            output: vec![Energy::default(); CHANNELS[usize::from(tree)].len()],
        })
    }
    pub fn reset(&mut self) {
        self.downmix.fill(Energy::default());
        self.output.fill(Energy::default());
    }
    /// Input is canonical downmix; dry/wet retain the matrix builder's native rows.
    /// None means disabled signalling, with envelope histories still advancing.
    /// A failure preserves both decoder state and caller-owned dry samples.
    pub fn apply(
        &mut self,
        downmix: &[Vec<Vec<Complex>>],
        dry: &mut Samples,
        wet: &[Vec<Vec<Complex>>],
        envelopes: Option<&[Option<Vec<f64>>]>,
    ) -> Result<()> {
        let map = CHANNELS[usize::from(self.tree)];
        let slots = dry.len();
        let valid = |rows: &[Vec<Vec<Complex>>], channels| {
            rows.len() == slots
                && rows.iter().all(|r| {
                    r.len() == channels
                        && r.iter().all(|c| {
                            c.len() == self.bands
                                && c.iter().all(|v| v.re.is_finite() && v.im.is_finite())
                        })
                })
        };
        let shape_channels = map.iter().copied().max().unwrap() as usize + 1;
        if slots == 0
            || slots > 32
            || !valid(downmix, self.inputs)
            || !valid(dry, map.len())
            || !valid(wet, map.len())
            || envelopes.is_some_and(|e| {
                e.len() != shape_channels
                    || e.iter().flatten().any(|row| {
                        row.len() != slots || row.iter().any(|v| !v.is_finite() || *v <= 0.)
                    })
            })
        {
            return Err(invalid("invalid LD-MPS GES frame or envelope"));
        }
        let mut next = self.clone();
        let mut shaped = dry.clone();
        let mut input_env = Vec::new();
        for i in 0..next.downmix.len() {
            let source = if self.tree == 6 { i + 4 } else { i };
            let signal: Vec<_> = downmix.iter().map(|s| s[source].clone()).collect();
            input_env.push(next.downmix[i].envelope(&signal)?);
        }
        for (row, &channel) in map.iter().enumerate() {
            if channel < 0 {
                continue;
            }
            let ch = channel as usize;
            let signal: Vec<Vec<Complex>> = (0..slots)
                .map(|t| {
                    dry[t][row]
                        .iter()
                        .zip(&wet[t][row])
                        .map(|(a, b)| Complex {
                            re: a.re + b.re,
                            im: a.im + b.im,
                        })
                        .collect()
                })
                .collect();
            let env = next.output[row].envelope(&signal)?;
            let Some(target) = envelopes.and_then(|e| e[ch].as_ref()) else {
                continue;
            };
            for t in 0..slots {
                let reference = match self.tree {
                    0 | 1 | 7 => input_env[0][t],
                    2..=4 => match ch {
                        0 | 3 | 5 => input_env[0][t],
                        1 | 4 | 6 => input_env[1][t],
                        2 => (input_env[0][t] + input_env[1][t]) * 0.5,
                        _ => unreachable!(),
                    },
                    5 | 6 => input_env[ch % 2][t],
                    _ => unreachable!(),
                };
                let requested = target[t] * reference / (env[t] + 1e-9);
                let energy = |v: &[Complex]| {
                    v[6..]
                        .iter()
                        .map(|v| v.re * v.re + v.im * v.im)
                        .sum::<f64>()
                };
                let ratio = (energy(&wet[t][row]) / (energy(&dry[t][row]) + 1e-9)).sqrt();
                let gain = (requested + ratio * (requested - 1.))
                    .max(0.)
                    .clamp(0.25, 4.);
                if !requested.is_finite() || !ratio.is_finite() {
                    return Err(invalid("overflowing LD-MPS GES shaping gain"));
                }
                for v in &mut shaped[t][row][6..] {
                    v.re *= gain;
                    v.im *= gain;
                    if !v.re.is_finite() || !v.im.is_finite() {
                        return Err(invalid("overflowing LD-MPS GES output"));
                    }
                }
            }
        }
        *self = next;
        *dry = shaped;
        Ok(())
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        f.vector(&self.downmix)?;
        f.vector(&self.output)?;
        Ok(())
    }
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        let mut f = Footprint::new();
        self.visit_retained(&mut f).map_err(|e| invalid(&e))?;
        Ok(f.total())
    }
}
