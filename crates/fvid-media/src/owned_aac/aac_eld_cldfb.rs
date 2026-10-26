//! Owned low-delay complex filterbanks, ISO 14496-3 / GOST 53556.4 §6.19.4.
//! One analysis column consumes 32 core samples. Synthesis emits 32 or 64.
use super::{
    Result, aac_eld_cldfb_window::WINDOW, aac_sbr_qmf::Complex, invalid, memory::Footprint,
};
use std::sync::OnceLock;
fn analysis_matrix() -> &'static [[Complex; 64]] {
    static M: OnceLock<Vec<[Complex; 64]>> = OnceLock::new();
    M.get_or_init(|| {
        (0..32)
            .map(|k| {
                std::array::from_fn(|n| {
                    let phase =
                        std::f64::consts::PI * (k as f64 + 0.5) * (2.0 * n as f64 - 96.0) / 64.0;
                    let (im, re) = phase.sin_cos();
                    Complex {
                        re: 2.0 * re,
                        im: 2.0 * im,
                    }
                })
            })
            .collect()
    })
}
fn synthesis64_matrix() -> &'static [[Complex; 64]; 128] {
    static M: OnceLock<Vec<[Complex; 64]>> = OnceLock::new();
    M.get_or_init(|| {
        (0..128)
            .map(|n| {
                std::array::from_fn(|k| {
                    let phase =
                        std::f64::consts::PI * (k as f64 + 0.5) * (2.0 * n as f64 - 63.0) / 128.0;
                    let (im, re) = phase.sin_cos();
                    Complex {
                        re: re / 64.0,
                        im: im / 64.0,
                    }
                })
            })
            .collect()
    })
    .as_slice()
    .try_into()
    .expect("fixed CLDFB matrix dimensions")
}
fn synthesis32_matrix() -> &'static [[Complex; 32]; 64] {
    static M: OnceLock<Vec<[Complex; 32]>> = OnceLock::new();
    M.get_or_init(|| {
        (0..64)
            .map(|n| {
                std::array::from_fn(|k| {
                    let phase =
                        std::f64::consts::PI * (k as f64 + 0.5) * (2.0 * n as f64 - 31.0) / 64.0;
                    let (im, re) = phase.sin_cos();
                    Complex {
                        re: re / 32.0,
                        im: im / 32.0,
                    }
                })
            })
            .collect()
    })
    .as_slice()
    .try_into()
    .expect("fixed CLDFB matrix dimensions")
}
fn window32(i: usize) -> f64 {
    0.5 * (WINDOW[2 * i] + WINDOW[2 * i + 1])
}
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    history: Vec<f64>,
}
impl Default for Analysis {
    fn default() -> Self {
        Self {
            history: vec![0.0; 320],
        }
    }
}
impl Analysis {
    pub fn reset(&mut self) {
        self.history.fill(0.0);
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        f.vector(&self.history)
    }
    pub fn process(&mut self, pcm: &[f32]) -> Result<Vec<[Complex; 32]>> {
        if !pcm.len().is_multiple_of(32) || pcm.iter().any(|v| !v.is_finite()) {
            return Err(invalid("invalid ELD CLDFB analysis input"));
        }
        let mut history = self.history.clone();
        let mut output = Vec::with_capacity(pcm.len() / 32);
        for block in pcm.chunks_exact(32) {
            history.copy_within(0..288, 32);
            for (dst, &src) in history[..32].iter_mut().zip(block.iter().rev()) {
                *dst = f64::from(src);
            }
            let u: [f64; 64] = std::array::from_fn(|n| {
                (0..5)
                    .map(|j| {
                        let i = n + 64 * j;
                        history[i] * window32(i)
                    })
                    .sum()
            });
            let row = std::array::from_fn(|k| {
                let mut v = Complex::default();
                for (n, &x) in u.iter().enumerate() {
                    v.re += x * analysis_matrix()[k][n].re;
                    v.im += x * analysis_matrix()[k][n].im;
                }
                v
            });
            if row.iter().any(|v| !v.re.is_finite() || !v.im.is_finite()) {
                return Err(invalid("overflowing ELD CLDFB analysis"));
            }
            output.push(row);
        }
        self.history = history;
        Ok(output)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Synthesis {
    bands: usize,
    history: Vec<f64>,
}
impl Synthesis {
    pub fn new(bands: usize) -> Result<Self> {
        if !matches!(bands, 32 | 64) {
            return Err(invalid("ELD CLDFB synthesis requires 32 or 64 bands"));
        }
        Ok(Self {
            bands,
            history: vec![0.0; 20 * bands],
        })
    }
    pub fn reset(&mut self) {
        self.history.fill(0.0);
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        f.vector(&self.history)
    }
    pub fn process(&mut self, rows: &[[Complex; 64]]) -> Result<Vec<f64>> {
        if rows
            .iter()
            .flatten()
            .any(|v| !v.re.is_finite() || !v.im.is_finite())
        {
            return Err(invalid("invalid ELD CLDFB synthesis input"));
        }
        let capacity = rows
            .len()
            .checked_mul(self.bands)
            .ok_or_else(|| invalid("ELD CLDFB synthesis size overflow"))?;
        let mut history = self.history.clone();
        let mut output = Vec::with_capacity(capacity);
        let b = self.bands;
        for row in rows {
            history.copy_within(0..18 * b, 2 * b);
            let good = if b == 64 {
                fvid_cpu::sbr_qmf_synthesis_slot(
                    row,
                    synthesis64_matrix(),
                    (&mut history[..128]).try_into().unwrap(),
                )
            } else {
                fvid_cpu::sbr_qmf_synthesis32_slot(
                    (&row[..32]).try_into().unwrap(),
                    synthesis32_matrix(),
                    (&mut history[..64]).try_into().unwrap(),
                )
            };
            if !good {
                return Err(invalid("overflowing ELD CLDFB synthesis transform"));
            }
            for k in 0..b {
                let mut value = 0.0;
                for p in 0..5 {
                    let w0 = 2 * b * p + k;
                    let w1 = w0 + b;
                    let c0 = if b == 64 { WINDOW[w0] } else { window32(w0) };
                    let c1 = if b == 64 { WINDOW[w1] } else { window32(w1) };
                    value += history[4 * b * p + k] * c0 + history[4 * b * p + 3 * b + k] * c1;
                }
                if !value.is_finite() {
                    return Err(invalid("overflowing ELD CLDFB synthesis PCM"));
                }
                output.push(value);
            }
        }
        self.history = history;
        Ok(output)
    }
}
