//! Owned LD-MPS Schur all-pass decorrelation and energy ducking.
//! Input/output QMF samples use normalized PCM units (full scale 1, not 32768).
//! This component is independent of spatial bitstream parsing and PCM routing.
use super::{
    Result, aac_ld_mps_decor_facts as facts, aac_ld_mps_prepare::qmf_map, aac_sbr_qmf::Complex,
    invalid, unsupported,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Real,
    Complex,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Design {
    pub delay: usize,
    pub denominator: Vec<Complex>,
    pub numerator: Vec<Complex>,
}
fn mul(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re * b.re - a.im * b.im,
        im: a.re * b.im + a.im * b.re,
    }
}
fn add(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re + b.re,
        im: a.im + b.im,
    }
}
fn conj(a: Complex) -> Complex {
    Complex {
        re: a.re,
        im: -a.im,
    }
}
fn finite(x: Complex) -> bool {
    x.re.is_finite() && x.im.is_finite()
}
/// Reflection-to-polynomial Schur recursion, then conjugate-reversed numerator.
/// The two filter modes are explicit implementation choices, not stream flags.
pub fn design(bands: usize, seed: u8, configuration: u8, mode: Mode, bin: usize) -> Result<Design> {
    if bands == 128 {
        return Err(unsupported(
            "LD-MPS 128-band decorrelation is not yet qualified",
        ));
    }
    if !matches!(bands, 32 | 64) || seed >= 10 || configuration > 2 || bin >= bands {
        return Err(invalid("invalid LD-MPS decorrelator design"));
    }
    let boundary = match configuration {
        0 => [14, 23],
        1 => [49, 64],
        2 => [14, 64],
        _ => unreachable!(),
    };
    let (reflection, delay): (&[f64], usize) = if bin < boundary[0] {
        (&facts::REFLECTION_15[usize::from(seed)], 7)
    } else if bin < boundary[1] {
        (&facts::REFLECTION_6[usize::from(seed)], 2)
    } else {
        (&facts::REFLECTION_3[usize::from(seed)], 1)
    };
    let mut a = vec![Complex { re: 1.0, im: 0.0 }];
    for (degree, &value) in reflection.iter().enumerate() {
        let k = if mode == Mode::Complex {
            let (s, c) = (bin as f64 * 0.5 * facts::PHASE[usize::from(seed)][degree]).sin_cos();
            Complex {
                re: value * c,
                im: value * s,
            }
        } else {
            Complex { re: value, im: 0.0 }
        };
        let mut next = vec![Complex { re: 1.0, im: 0.0 }];
        for j in 1..a.len() {
            next.push(add(a[j], mul(k, conj(a[a.len() - j]))));
        }
        next.push(k);
        a = next;
    }
    let b = a.iter().rev().copied().map(conj).collect();
    Ok(Design {
        delay,
        denominator: a,
        numerator: b,
    })
}
#[derive(Clone, Debug, PartialEq)]
struct Band {
    design: Design,
    delay: Vec<Complex>,
    delay_at: usize,
    x: Vec<Complex>,
    y: Vec<Complex>,
    at: usize,
}
impl Band {
    fn new(design: Design) -> Self {
        let n = design.denominator.len() - 1;
        Self {
            delay: vec![Complex::default(); design.delay],
            design,
            delay_at: 0,
            x: vec![Complex::default(); n],
            y: vec![Complex::default(); n],
            at: 0,
        }
    }
    fn reset(&mut self) {
        self.delay.fill(Complex::default());
        self.delay_at = 0;
        self.x.fill(Complex::default());
        self.y.fill(Complex::default());
        self.at = 0;
    }
    fn step(&mut self, input: Complex) -> Result<Complex> {
        let x = self.delay[self.delay_at];
        self.delay[self.delay_at] = input;
        self.delay_at = (self.delay_at + 1) % self.delay.len();
        let mut y = mul(self.design.numerator[0], x);
        let n = self.x.len();
        // Direct-form convolution uses independent input/output rings; no
        // reference transposed-filter state or external DSP implementation.
        for lag in 1..=n {
            let i = (self.at + n - lag) % n;
            let feed = mul(self.design.numerator[lag], self.x[i]);
            let back = mul(self.design.denominator[lag], self.y[i]);
            y = add(
                y,
                Complex {
                    re: feed.re - back.re,
                    im: feed.im - back.im,
                },
            );
        }
        if !finite(y) {
            return Err(invalid("overflowing LD-MPS all-pass output"));
        }
        self.x[self.at] = x;
        self.y[self.at] = y;
        self.at = (self.at + 1) % n;
        Ok(y)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decorrelator {
    bands: Vec<Band>,
    map: Vec<u8>,
    direct_energy: [f64; 23],
    diffuse_energy: [f64; 23],
}
impl Decorrelator {
    pub fn new(bands: usize, seed: u8, configuration: u8, mode: Mode) -> Result<Self> {
        design(bands, seed, configuration, mode, 0)?;
        let designs = (0..bands)
            .map(|bin| design(bands, seed, configuration, mode, bin))
            .collect::<Result<Vec<_>>>()?;
        // Also validate an empty/invalid bin geometry before allocating state.
        if designs.is_empty() {
            return Err(invalid("empty LD-MPS decorrelator geometry"));
        }
        Ok(Self {
            bands: designs.into_iter().map(Band::new).collect(),
            map: qmf_map(23, bands)?,
            direct_energy: [0.0; 23],
            diffuse_energy: [0.0; 23],
        })
    }
    pub fn reset(&mut self) {
        for b in &mut self.bands {
            b.reset();
        }
        self.direct_energy.fill(0.0);
        self.diffuse_energy.fill(0.0);
    }
    /// One bounded LD frame/chunk, up to 32 slots. Empty chunks are no-ops.
    /// All filters, delay lines and ducking histories commit atomically.
    pub fn process(&mut self, input: &[Vec<Complex>]) -> Result<Vec<Vec<Complex>>> {
        if input.len() > 32
            || input
                .iter()
                .any(|r| r.len() != self.bands.len() || r.iter().any(|&x| !finite(x)))
        {
            return Err(invalid("invalid LD-MPS decorrelator QMF frame"));
        }
        let mut next = self.clone();
        let mut output = Vec::with_capacity(input.len());
        for row in input {
            let mut raw = next
                .bands
                .iter_mut()
                .zip(row)
                .map(|(b, &x)| b.step(x))
                .collect::<Result<Vec<_>>>()?;
            let (mut direct, mut diffuse) = ([0.0; 23], [0.0; 23]);
            for (bin, (&x, &y)) in row.iter().zip(&raw).enumerate() {
                let p = usize::from(next.map[bin]);
                direct[p] += x.re * x.re + x.im * x.im;
                diffuse[p] += y.re * y.re + y.im * y.im;
            }
            if direct.iter().chain(&diffuse).any(|x| !x.is_finite()) {
                return Err(invalid("overflowing LD-MPS decorrelator energy"));
            }
            let mut gain = [1.0; 23];
            for p in 0..23 {
                next.direct_energy[p] = 0.8 * next.direct_energy[p] + 0.2 * direct[p];
                next.diffuse_energy[p] = 0.8 * next.diffuse_energy[p] + 0.2 * diffuse[p];
                let (d, r) = (next.direct_energy[p], next.diffuse_energy[p]);
                if !d.is_finite() || !r.is_finite() {
                    return Err(invalid("overflowing LD-MPS smoothed decorrelation energy"));
                }
                if r > 1.5 * d {
                    gain[p] = (1.5 * d / (r + 1e-9)).sqrt();
                } else if d > 1.5 * r {
                    gain[p] = (d / (1.5 * r + 1e-9)).sqrt().min(2.0);
                }
            }
            for (bin, y) in raw.iter_mut().enumerate() {
                let g = gain[usize::from(next.map[bin])];
                y.re *= g;
                y.im *= g;
            }
            if raw.iter().any(|&x| !finite(x)) {
                return Err(invalid("overflowing LD-MPS ducked output"));
            }
            output.push(raw);
        }
        *self = next;
        Ok(output)
    }
}

impl Decorrelator {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        f.vector(&self.bands)?;
        f.vector(&self.map)?;
        for band in &self.bands {
            f.vector(&band.design.denominator)?;
            f.vector(&band.design.numerator)?;
            f.vector(&band.delay)?;
            f.vector(&band.x)?;
            f.vector(&band.y)?;
        }
        Ok(())
    }
}
