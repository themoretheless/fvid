//! Owned PS low-frequency FIR primitives (GOST R 53556.8, 6.4.3, tables 36–38).
//!
//! Outputs are in raw modulation index order, before the 20-band folding and
//! hybrid-band routing. The causal bank contributes six QMF slots of delay;
//! stream-level QMF startup compensation belongs to its integrating owner.
use super::{Result, aac_sbr_qmf::Complex, invalid};
use std::f64::consts::TAU;

pub const DELAY: usize = 6;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prototype {
    TwentyEight,
    TwentyTwo,
    ThirtyFourTwelve,
    ThirtyFourEight,
    ThirtyFourFour,
}
impl Prototype {
    pub fn subbands(self) -> usize {
        match self {
            Self::TwentyTwo => 2,
            Self::ThirtyFourFour => 4,
            Self::ThirtyFourTwelve => 12,
            _ => 8,
        }
    }
    fn half(self) -> [f64; 7] {
        match self {
            Self::TwentyEight => [
                0.00746082949812,
                0.02270420949825,
                0.04546865930473,
                0.07266113929591,
                0.09885108575264,
                0.11793710567217,
                0.125,
            ],
            Self::TwentyTwo => [
                0.,
                0.01899487526049,
                0.,
                -0.07293139167538,
                0.,
                0.30596630545168,
                0.5,
            ],
            Self::ThirtyFourTwelve => [
                0.04081179924692,
                0.03812810994926,
                0.05144908135699,
                0.06399831151592,
                0.07428313801106,
                0.08100347892914,
                0.08333333333333,
            ],
            Self::ThirtyFourEight => [
                0.01565675600122,
                0.03752716391991,
                0.05417891378782,
                0.08417044116767,
                0.10307344158036,
                0.12222452249753,
                0.125,
            ],
            Self::ThirtyFourFour => [
                -0.05908211155639,
                -0.04871498374946,
                0.,
                0.07778723915851,
                0.16486303567403,
                0.23279856662996,
                0.25,
            ],
        }
    }
    pub fn coefficients(self, subband: usize) -> Result<[Complex; 13]> {
        if subband >= self.subbands() {
            return Err(invalid("PS hybrid raw subband exceeds prototype"));
        }
        let half = self.half();
        Ok(std::array::from_fn(|tap| {
            let g = half[tap.min(12 - tap)];
            let position = (tap as f64) - 6.;
            if self == Self::TwentyTwo {
                // Q=2, Type B: cos(pi*q*(tap-6)) is exactly +1 or -1.
                let sign = if (subband * tap) % 2 == 0 { 1. } else { -1. };
                Complex {
                    re: g * sign,
                    im: 0.,
                }
            } else {
                let angle = TAU * (subband as f64 + 0.5) * position / self.subbands() as f64;
                let (sin, cos) = angle.sin_cos();
                Complex {
                    re: g * cos,
                    im: g * sin,
                }
            }
        }))
    }
}
fn finite(value: Complex) -> bool {
    value.re.is_finite() && value.im.is_finite()
}

/// One QMF channel's raw subband filter. Changing the prototype explicitly
/// requires a new bank; checkpoint clones preserve every causal history sample.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    prototype: Prototype,
    history: [Complex; 13],
    coefficients: Vec<[Complex; 13]>,
}
impl Analysis {
    pub fn new(prototype: Prototype) -> Self {
        Self {
            prototype,
            history: [Complex::default(); 13],
            coefficients: (0..prototype.subbands())
                .map(|q| {
                    prototype
                        .coefficients(q)
                        .expect("bounded prototype subband")
                })
                .collect(),
        }
    }
    pub fn prototype(&self) -> Prototype {
        self.prototype
    }
    pub fn reset(&mut self) {
        self.history.fill(Complex::default());
    }
    /// Chronological QMF samples -> chronological raw subband slots.
    /// Invalid or unrepresentable input/output leaves all stream history intact.
    pub fn process(&mut self, input: &[Complex]) -> Result<Vec<Vec<Complex>>> {
        if input.iter().any(|&value| !finite(value)) {
            return Err(invalid("PS hybrid input must be finite"));
        }
        let mut history = self.history;
        let mut output = Vec::with_capacity(input.len());
        for &sample in input {
            history.copy_within(0..12, 1);
            history[0] = sample;
            let mut slot = Vec::with_capacity(self.coefficients.len());
            for taps in &self.coefficients {
                let mut value = Complex::default();
                for (&x, &g) in history.iter().zip(taps) {
                    value.re += x.re * g.re - x.im * g.im;
                    value.im += x.re * g.im + x.im * g.re;
                }
                if !finite(value) {
                    return Err(invalid("PS hybrid output is not representable"));
                }
                slot.push(value);
            }
            output.push(slot);
        }
        self.history = history;
        Ok(output)
    }
}
/// Raw inverse hybrid operation: add all split bands for one QMF channel.
/// No normalization or second FIR; tables retain their published precision.
pub fn synthesize(prototype: Prototype, input: &[Vec<Complex>]) -> Result<Vec<Complex>> {
    input
        .iter()
        .map(|slot| {
            if slot.len() != prototype.subbands() {
                return Err(invalid(
                    "PS hybrid raw synthesis width differs from prototype",
                ));
            }
            let mut sum = Complex::default();
            for &value in slot {
                if !finite(value) {
                    return Err(invalid("PS hybrid synthesis input must be finite"));
                }
                sum.re += value.re;
                sum.im += value.im;
            }
            if !finite(sum) {
                return Err(invalid("PS hybrid synthesis output is not representable"));
            }
            Ok(sum)
        })
        .collect()
}
