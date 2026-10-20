//! Owned Layer III alias reduction, IMDCT/window/overlap and polyphase synthesis.
//! Input is requantized/reordered spectral data, not a compressed MP3 frame.
use crate::{Result, invalid, mpeg_synthesis::Synthesis};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Window {
    Normal = 0,
    Start = 1,
    Short = 2,
    Stop = 3,
}
#[derive(Clone)]
pub struct Hybrid {
    overlap: [[f64; 18]; 32],
    synthesis: Synthesis,
    long: [[f64; 18]; 36],
    short: [[f64; 6]; 12],
    windows: [[f64; 36]; 4],
    alias: [(f64, f64); 8],
}
impl Default for Hybrid {
    fn default() -> Self {
        let long = std::array::from_fn(|t| {
            std::array::from_fn(|k| {
                (std::f64::consts::PI * (2 * t + 19) as f64 * (2 * k + 1) as f64 / 72.).cos()
            })
        });
        let short = std::array::from_fn(|t| {
            std::array::from_fn(|k| {
                (std::f64::consts::PI * (2 * t + 7) as f64 * (2 * k + 1) as f64 / 24.).cos()
            })
        });
        let sine = |n: usize, t: usize| (std::f64::consts::PI * (t as f64 + 0.5) / n as f64).sin();
        let windows = std::array::from_fn(|kind| {
            std::array::from_fn(|t| match kind {
                0 => sine(36, t),
                1 => match t {
                    0..=17 => sine(36, t),
                    18..=23 => 1.,
                    24..=29 => sine(12, t - 18),
                    _ => 0.,
                },
                2 => {
                    if t < 12 {
                        sine(12, t)
                    } else {
                        0.
                    }
                }
                _ => match t {
                    0..=5 => 0.,
                    6..=11 => sine(12, t - 6),
                    12..=17 => 1.,
                    _ => sine(36, t),
                },
            })
        });
        let alias = [
            -0.6f64, -0.535, -0.33, -0.185, -0.095, -0.041, -0.0142, -0.0037,
        ]
        .map(|c| {
            let cs = 1. / (1. + c * c).sqrt();
            (cs, c * cs)
        });
        Self {
            overlap: [[0.; 18]; 32],
            synthesis: Synthesis::default(),
            long,
            short,
            windows,
            alias,
        }
    }
}
impl Hybrid {
    pub fn reset(&mut self) {
        self.overlap = [[0.; 18]; 32];
        self.synthesis.reset();
    }
    /// `mixed_long_subbands` is 0, 2 (standard mixed) or 4 (8kHz extension).
    /// Long bands have 18 consecutive coefficients. Short bands use
    /// `input[18*band + 3*frequency + window]` after spectral reordering.
    /// One channel's granule yields 576 consecutive time-domain PCM samples.
    pub fn process(
        &mut self,
        input: &[f64; 576],
        window: Window,
        mixed_long_subbands: u8,
    ) -> Result<[f64; 576]> {
        if !matches!(mixed_long_subbands, 0 | 2 | 4) {
            return Err(invalid("invalid MP3 mixed long-band count"));
        }
        if input.iter().any(|v| !v.is_finite()) {
            return Err(invalid("non-finite MP3 spectral input"));
        }
        let mixed = usize::from(mixed_long_subbands);
        let mut spectrum = *input;
        let alias_bands = if window == Window::Short { mixed } else { 32 };
        for band in 1..alias_bands {
            for (i, &(cs, ca)) in self.alias.iter().enumerate() {
                let left = 18 * band - 1 - i;
                let right = 18 * band + i;
                let a = spectrum[left];
                let b = spectrum[right];
                spectrum[left] = a * cs - b * ca;
                spectrum[right] = b * cs + a * ca;
            }
        }
        if spectrum.iter().any(|v| !v.is_finite()) {
            return Err(invalid("MP3 alias reconstruction overflow"));
        }
        let mut overlap = self.overlap;
        let mut synthesis = self.synthesis.clone();
        let mut slots = [[0.; 32]; 18];
        for band in 0..32 {
            let kind = if band < mixed { Window::Normal } else { window };
            let mut block = [0.; 36];
            let coefficients = &spectrum[band * 18..band * 18 + 18];
            if kind == Window::Short {
                for w in 0..3 {
                    for t in 0..12 {
                        let value = (0..6)
                            .map(|k| coefficients[3 * k + w] * self.short[t][k])
                            .sum::<f64>();
                        block[6 + 6 * w + t] += value * self.windows[2][t];
                    }
                }
            } else {
                for (t, out) in block.iter_mut().enumerate() {
                    *out = coefficients
                        .iter()
                        .zip(self.long[t])
                        .map(|(v, c)| v * c)
                        .sum::<f64>()
                        * self.windows[kind as usize][t];
                }
            }
            if block.iter().any(|v| !v.is_finite()) {
                return Err(invalid("MP3 IMDCT overflow"));
            }
            for t in 0..18 {
                let value = block[t] + overlap[band][t];
                if !value.is_finite() {
                    return Err(invalid("MP3 overlap overflow"));
                }
                slots[t][band] = if t % 2 == 1 && band % 2 == 1 {
                    -value
                } else {
                    value
                };
                overlap[band][t] = block[t + 18];
            }
        }
        let mut pcm = [0.; 576];
        for (time, slot) in slots.iter().enumerate() {
            let values = synthesis.push(slot)?;
            if values.iter().any(|v| !v.is_finite()) {
                return Err(invalid("MP3 synthesis output overflow"));
            }
            pcm[time * 32..time * 32 + 32].copy_from_slice(&values);
        }
        self.overlap = overlap;
        self.synthesis = synthesis;
        Ok(pcm)
    }
}
