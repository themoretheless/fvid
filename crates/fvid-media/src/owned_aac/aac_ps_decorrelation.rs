//! Owned PS decorrelation and transient suppression, GOST R 53556.8 6.4.5/A.3.
//! Allpass recurrences are derived directly from the normative Z transfer
//! function. No external DSP/codec implementation is used.
use super::{Result, aac_ps_mapping::Bands, aac_sbr_qmf::Complex, invalid};
use std::f64::consts::PI;
const A: [f64; 3] = [0.65143905753106, 0.56471812200776, 0.48954165955695];
const Q: [f64; 3] = [0.43, 0.75, 0.347];
const D: [usize; 3] = [3, 4, 5];
const PEAK_DECAY: f64 = 0.76592833836465;
const CENTERS_20: [f64; 10] = [
    -3. / 8.,
    -1. / 8.,
    1. / 8.,
    3. / 8.,
    5. / 8.,
    7. / 8.,
    5. / 4.,
    7. / 4.,
    9. / 4.,
    11. / 4.,
];
const CENTERS_34: [f64; 32] = [
    1. / 12.,
    3. / 12.,
    5. / 12.,
    7. / 12.,
    9. / 12.,
    11. / 12.,
    13. / 12.,
    15. / 12.,
    17. / 12.,
    -5. / 12.,
    -3. / 12.,
    -1. / 12.,
    17. / 8.,
    19. / 8.,
    5. / 8.,
    7. / 8.,
    9. / 8.,
    11. / 8.,
    13. / 8.,
    15. / 8.,
    9. / 4.,
    11. / 4.,
    13. / 4.,
    7. / 4.,
    17. / 4.,
    11. / 4.,
    13. / 4.,
    15. / 4.,
    17. / 4.,
    19. / 4.,
    21. / 4.,
    15. / 4.,
];
fn limits(bands: Bands) -> (usize, usize, usize) {
    match bands {
        Bands::Twenty => (10, 30, 42),
        Bands::ThirtyFour => (32, 50, 62),
    }
}
fn rotate(q: Complex, x: Complex) -> Complex {
    Complex {
        re: q.re * x.re - q.im * x.im,
        im: q.re * x.im + q.im * x.re,
    }
}
fn phase(q: f64, center: f64) -> Complex {
    let (im, re) = (-PI * q * center).sin_cos();
    Complex { re, im }
}
fn finite(c: Complex) -> bool {
    c.re.is_finite() && c.im.is_finite()
}
fn scale(x: Complex, a: f64) -> Complex {
    Complex {
        re: x.re * a,
        im: x.im * a,
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AllpassCoefficients {
    pub center: f64,
    pub decay: f64,
    pub initial_phase: Complex,
    pub link_phases: [Complex; 3],
    pub feedback: [f64; 3],
    pub delays: [usize; 3],
}
pub fn coefficients(bands: Bands, k: usize) -> Result<Option<AllpassCoefficients>> {
    if k >= bands.bindings().len() {
        return Err(invalid("PS decorrelator subband exceeds configuration"));
    }
    let (cutoff, allpass, _) = limits(bands);
    if k >= allpass {
        return Ok(None);
    }
    let center = match bands {
        Bands::Twenty => CENTERS_20.get(k).copied().unwrap_or(k as f64 + 0.5 - 7.),
        Bands::ThirtyFour => CENTERS_34.get(k).copied().unwrap_or(k as f64 + 0.5 - 27.),
    };
    let decay = if k < cutoff {
        1.
    } else {
        (1. - 0.05 * (k - cutoff) as f64).max(0.)
    };
    Ok(Some(AllpassCoefficients {
        center,
        decay,
        initial_phase: phase(0.39, center),
        link_phases: Q.map(|q| phase(q, center)),
        feedback: A.map(|a| a * decay),
        delays: D,
    }))
}
#[derive(Clone, Debug, PartialEq)]
struct Delay {
    samples: Vec<Complex>,
    cursor: usize,
}
impl Delay {
    fn new(length: usize) -> Self {
        Self {
            samples: vec![Complex::default(); length],
            cursor: 0,
        }
    }
    fn previous(&self) -> Complex {
        self.samples[self.cursor]
    }
    fn push(&mut self, x: Complex) -> Complex {
        let old = self.previous();
        self.samples[self.cursor] = x;
        self.cursor = (self.cursor + 1) % self.samples.len();
        old
    }
    fn clear(&mut self) {
        self.samples.fill(Complex::default());
        self.cursor = 0;
    }
}
#[derive(Clone, Debug, PartialEq)]
struct Link {
    q: Complex,
    a: f64,
    input: Delay,
    output: Delay,
}
impl Link {
    fn step(&mut self, x: Complex) -> Result<Complex> {
        // y[n] = Q*x[n-d] - a*x[n] + a*Q*y[n-d]. The output
        // history is the raw allpass output, before transient attenuation.
        let past_x = rotate(self.q, self.input.push(x));
        let past_y = rotate(self.q, self.output.previous());
        let y = Complex {
            re: past_x.re - self.a * x.re + self.a * past_y.re,
            im: past_x.im - self.a * x.im + self.a * past_y.im,
        };
        if !finite(y) {
            return Err(invalid("PS allpass output is not representable"));
        }
        self.output.push(y);
        Ok(y)
    }
    fn clear(&mut self) {
        self.input.clear();
        self.output.clear();
    }
}
#[derive(Clone, Debug, PartialEq)]
enum Filter {
    Delay(Delay),
    Allpass {
        initial: Delay,
        phase: Complex,
        links: [Link; 3],
    },
}
impl Filter {
    fn new(bands: Bands, k: usize) -> Self {
        match coefficients(bands, k).expect("bounded decorrelation subband") {
            Some(c) => Self::Allpass {
                initial: Delay::new(2),
                phase: c.initial_phase,
                links: std::array::from_fn(|m| Link {
                    q: c.link_phases[m],
                    a: c.feedback[m],
                    input: Delay::new(c.delays[m]),
                    output: Delay::new(c.delays[m]),
                }),
            },
            None => Self::Delay(Delay::new(if k < limits(bands).2 { 14 } else { 1 })),
        }
    }
    fn step(&mut self, x: Complex) -> Result<Complex> {
        match self {
            Self::Delay(d) => Ok(d.push(x)),
            Self::Allpass {
                initial,
                phase,
                links,
            } => {
                let mut x = rotate(*phase, initial.push(x));
                if !finite(x) {
                    return Err(invalid("PS fractional delay output is not representable"));
                }
                for link in links {
                    x = link.step(x)?;
                }
                Ok(x)
            }
        }
    }
    fn clear(&mut self) {
        match self {
            Self::Delay(d) => d.clear(),
            Self::Allpass { initial, links, .. } => {
                initial.clear();
                for link in links {
                    link.clear();
                }
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameControls {
    /// Full state reset if the preceding frame had no PS element (A.3).
    pub previous_ps_present: bool,
    /// Exclusive upper generated QMF channel, kx+M, in 0..=64. At a frame
    /// boundary filter histories above it are cleared, not the current input.
    pub qmf_limit: usize,
}
impl Default for FrameControls {
    fn default() -> Self {
        Self {
            previous_ps_present: true,
            qmf_limit: 64,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    bands: Bands,
    filters: Vec<Filter>,
    peak: Vec<f64>,
    smooth: Vec<f64>,
    difference: Vec<f64>,
}
impl Default for State {
    fn default() -> Self {
        Self::new(Bands::Twenty)
    }
}
impl State {
    pub fn new(bands: Bands) -> Self {
        Self {
            bands,
            filters: (0..bands.bindings().len())
                .map(|k| Filter::new(bands, k))
                .collect(),
            peak: vec![0.; bands.count()],
            smooth: vec![0.; bands.count()],
            difference: vec![0.; bands.count()],
        }
    }
    pub fn bands(&self) -> Bands {
        self.bands
    }
    pub fn reset(&mut self, bands: Bands) {
        *self = Self::new(bands);
    }
    pub fn peak(&self) -> &[f64] {
        &self.peak
    }
    pub fn smoothed_power(&self) -> &[f64] {
        &self.smooth
    }
    pub fn smoothed_difference(&self) -> &[f64] {
        &self.difference
    }
    /// Continuous chunks with no frame-boundary reset. Grid changes reset all
    /// decorrelator state (6.4.6.1), independently of the hybrid input history.
    pub fn process(&mut self, bands: Bands, input: &[Vec<Complex>]) -> Result<Frame> {
        self.process_frame(bands, FrameControls::default(), input)
    }
    /// Frame-boundary controls and every sample are one transaction. Empty
    /// calls have no state effects; malformed controls/input always fail.
    pub fn process_frame(
        &mut self,
        bands: Bands,
        controls: FrameControls,
        input: &[Vec<Complex>],
    ) -> Result<Frame> {
        if controls.qmf_limit > 64 {
            return Err(invalid("PS generated QMF limit exceeds 64 channels"));
        }
        if input
            .iter()
            .any(|slot| slot.len() != bands.bindings().len())
        {
            return Err(invalid(
                "PS decorrelator input width differs from configuration",
            ));
        }
        if input.iter().flatten().any(|&c| !finite(c)) {
            return Err(invalid("PS decorrelator input must be finite"));
        }
        if input.is_empty() {
            return Ok(Frame {
                bands: self.bands,
                bands_changed: false,
                output: vec![],
                unattenuated: vec![],
                power: vec![],
                gains: vec![],
            });
        }
        let changed = self.bands != bands;
        let mut trial = if changed || !controls.previous_ps_present {
            Self::new(bands)
        } else {
            self.clone()
        };
        for (filter, binding) in trial.filters.iter_mut().zip(bands.bindings()) {
            if usize::from(binding.qmf) >= controls.qmf_limit {
                filter.clear();
            }
        }
        let mut frame = Frame {
            bands,
            bands_changed: changed,
            output: Vec::with_capacity(input.len()),
            unattenuated: Vec::with_capacity(input.len()),
            power: Vec::with_capacity(input.len()),
            gains: Vec::with_capacity(input.len()),
        };
        for slot in input {
            let mut power = vec![0.; bands.count()];
            for (&c, binding) in slot.iter().zip(bands.bindings()) {
                power[usize::from(binding.parameter)] += c.re * c.re + c.im * c.im;
            }
            if power.iter().any(|p| !p.is_finite()) {
                return Err(invalid("PS decorrelation energy is not representable"));
            }
            let mut gains = vec![1.; bands.count()];
            for (b, &p) in power.iter().enumerate() {
                trial.peak[b] = p.max(PEAK_DECAY * trial.peak[b]);
                trial.smooth[b] = 0.25 * p + 0.75 * trial.smooth[b];
                trial.difference[b] = 0.25 * (trial.peak[b] - p) + 0.75 * trial.difference[b];
                if !trial.smooth[b].is_finite() || !trial.difference[b].is_finite() {
                    return Err(invalid("PS transient state is not representable"));
                }
                // Compare/divide without forming 1.5*difference: that product
                // may overflow although the gain is representable in [0,1].
                if trial.difference[b] > trial.smooth[b] / 1.5 {
                    gains[b] = (trial.smooth[b] / trial.difference[b]) / 1.5;
                }
            }
            let mut raw = Vec::with_capacity(slot.len());
            let mut attenuated = Vec::with_capacity(slot.len());
            for ((filter, &x), binding) in trial.filters.iter_mut().zip(slot).zip(bands.bindings())
            {
                let y = filter.step(x)?;
                let output = scale(y, gains[usize::from(binding.parameter)]);
                if !finite(output) {
                    return Err(invalid("PS decorrelated output is not representable"));
                }
                raw.push(y);
                attenuated.push(output);
            }
            frame.power.push(power);
            frame.gains.push(gains);
            frame.unattenuated.push(raw);
            frame.output.push(attenuated);
        }
        *self = trial;
        Ok(frame)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub bands: Bands,
    pub bands_changed: bool,
    pub output: Vec<Vec<Complex>>,
    /// Raw filter outputs before per-parameter-band transient suppression.
    pub unattenuated: Vec<Vec<Complex>>,
    pub power: Vec<Vec<f64>>,
    pub gains: Vec<Vec<f64>>,
}
