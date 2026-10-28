//! Owned low-delay complex filterbanks, ISO 14496-3 / GOST 53556.4 §6.19.4.
//! Legacy columns consume 32 core samples and emit 32 or 64. Explicit
//! centered adaptation supports 4/8/16/32 analysis and 4/8/16/32/64 synthesis.
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
    bands: usize,
    adapted: bool,
}
impl Default for Analysis {
    fn default() -> Self {
        Self {
            history: vec![0.0; 320],
            bands: 32,
            adapted: false,
        }
    }
}
impl Analysis {
    /// Explicit centered, downscaled analysis. Legacy analysis remains the default.
    pub fn new_downscaled(factor: usize) -> Result<Self> {
        if !matches!(factor, 1 | 2 | 4 | 8) {
            return Err(invalid("ELD CLDFB analysis requires factor 1, 2, 4 or 8"));
        }
        let bands = 32 / factor;
        Ok(Self {
            history: vec![0.0; 10 * bands],
            bands,
            adapted: true,
        })
    }
    pub fn bands(&self) -> usize {
        self.bands
    }

    pub fn reset(&mut self) {
        self.history.fill(0.0);
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        f.vector(&self.history)
    }
    pub fn process(&mut self, pcm: &[f32]) -> Result<Vec<[Complex; 32]>> {
        if !pcm.len().is_multiple_of(self.bands) || pcm.iter().any(|v| !v.is_finite()) {
            return Err(invalid("invalid ELD CLDFB analysis input"));
        }
        let mut history = self.history.clone();
        let b = self.bands;
        let matrix = if self.adapted {
            Some(adapted_analysis_matrix(b))
        } else {
            None
        };
        let mut output = Vec::with_capacity(pcm.len() / b);
        for block in pcm.chunks_exact(b) {
            history.copy_within(0..9 * b, b);
            for (dst, &src) in history[..b].iter_mut().zip(block.iter().rev()) {
                *dst = f64::from(src);
            }
            let u: [f64; 64] = std::array::from_fn(|n| {
                if n >= 2 * b {
                    return 0.0;
                }
                (0..5)
                    .map(|j| {
                        let i = n + 2 * b * j;
                        history[i]
                            * if self.adapted {
                                centered_window(b, i)
                            } else {
                                window32(i)
                            }
                    })
                    .sum()
            });
            let row = std::array::from_fn(|k| {
                let mut v = Complex::default();
                if k >= b {
                    return v;
                }
                let weights = matrix.as_ref().map_or(&analysis_matrix()[k], |m| &m[k]);
                for (n, &x) in u.iter().enumerate() {
                    v.re += x * weights[n].re;
                    v.im += x * weights[n].im;
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
    adapted: bool,
    history: Vec<f64>,
}
impl Synthesis {
    /// Adapted synthesis uses the nominal 64-band normalization at every size.
    pub fn new_downscaled(original_bands: usize, factor: usize) -> Result<Self> {
        if !matches!(original_bands, 32 | 64) || !matches!(factor, 1 | 2 | 4 | 8) {
            return Err(invalid("invalid downscaled ELD CLDFB synthesis geometry"));
        }
        let bands = original_bands / factor;
        Ok(Self {
            bands,
            adapted: true,
            history: vec![0.0; 20 * bands],
        })
    }
    pub fn bands(&self) -> usize {
        self.bands
    }

    pub fn new(bands: usize) -> Result<Self> {
        if !matches!(bands, 32 | 64) {
            return Err(invalid("ELD CLDFB synthesis requires 32 or 64 bands"));
        }
        Ok(Self {
            bands,
            adapted: false,
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
        let matrix = if self.adapted {
            Some(adapted_synthesis_matrix(b))
        } else {
            None
        };
        for row in rows {
            history.copy_within(0..18 * b, 2 * b);
            let good = if let Some(matrix) = &matrix {
                for (dst, weights) in history[..2 * b].iter_mut().zip(matrix.iter()) {
                    *dst = row[..b]
                        .iter()
                        .zip(weights)
                        .map(|(v, w)| v.re * w.re - v.im * w.im)
                        .sum();
                }
                history[..2 * b].iter().all(|v| v.is_finite())
            } else if b == 64 {
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
                    let c0 = if self.adapted {
                        centered_window(b, w0)
                    } else if b == 64 {
                        WINDOW[w0]
                    } else {
                        window32(w0)
                    };
                    let c1 = if self.adapted {
                        centered_window(b, w1)
                    } else if b == 64 {
                        WINDOW[w1]
                    } else {
                        window32(w1)
                    };
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

// Centered prototype sampling. At 64 bands there is no decimation.
fn centered_window(b: usize, i: usize) -> f64 {
    if b == 64 {
        return WINDOW[i];
    }
    let stride = 64 / b;
    let at = stride * i + (stride - 1) / 2;
    0.5 * (WINDOW[at] + WINDOW[at + 1])
}
fn adapted_analysis_matrix(b: usize) -> &'static Vec<[Complex; 64]> {
    static M: [OnceLock<Vec<[Complex; 64]>>; 4] = [const { OnceLock::new() }; 4];
    M[b.ilog2() as usize - 2].get_or_init(|| {
        (0..b)
            .map(|k| {
                std::array::from_fn(|n| {
                    if n >= 2 * b {
                        return Complex::default();
                    }
                    let phase = std::f64::consts::PI
                        * (k as f64 + 0.5)
                        * (2.0 * n as f64 - (3 * b - 1) as f64)
                        / (2 * b) as f64;
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
fn adapted_synthesis_matrix(b: usize) -> &'static Vec<Vec<Complex>> {
    static M: [OnceLock<Vec<Vec<Complex>>>; 5] = [const { OnceLock::new() }; 5];
    M[b.ilog2() as usize - 2].get_or_init(|| {
        (0..2 * b)
            .map(|n| {
                (0..b)
                    .map(|k| {
                        let phase = std::f64::consts::PI
                            * (k as f64 + 0.5)
                            * (2.0 * n as f64 - (b - 1) as f64)
                            / (2 * b) as f64;
                        let (im, re) = phase.sin_cos();
                        Complex {
                            re: re / 64.0,
                            im: im / 64.0,
                        }
                    })
                    .collect()
            })
            .collect()
    })
}
