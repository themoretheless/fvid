//! Owned 32-band MPEG polyphase synthesis, ISO/IEC 11172-3 Annex A/B.
//! The persistent delay line belongs to one channel; navigation clears it.
use crate::{Result, invalid};
#[path = "mpeg_synthesis_window.rs"]
mod window;
#[derive(Clone)]
pub struct Synthesis {
    delay: [f64; 1024],
    matrix: [[f64; 32]; 64],
}
impl Default for Synthesis {
    fn default() -> Self {
        let matrix = std::array::from_fn(|i| {
            std::array::from_fn(|k| {
                (std::f64::consts::PI * (16 + i) as f64 * (2 * k + 1) as f64 / 64.).cos()
            })
        });
        Self {
            delay: [0.; 1024],
            matrix,
        }
    }
}
impl Synthesis {
    pub fn reset(&mut self) {
        self.delay.fill(0.);
    }
    pub fn push(&mut self, subbands: &[f64; 32]) -> Result<[f64; 32]> {
        if subbands.iter().any(|v| !v.is_finite()) {
            return Err(invalid("non-finite MPEG subband sample"));
        }
        let mut next = [0.; 64];
        for (i, value) in next.iter_mut().enumerate() {
            *value = self.matrix[i]
                .iter()
                .zip(subbands)
                .map(|(a, b)| a * b)
                .sum();
        }
        if next.iter().any(|v| !v.is_finite()) {
            return Err(invalid("MPEG synthesis matrix overflow"));
        }
        // Validate before mutating history; failed input cannot poison later frames.
        self.delay.copy_within(..960, 64);
        self.delay[..64].copy_from_slice(&next);
        let output = std::array::from_fn(|j| {
            (0..8)
                .map(|q| {
                    self.delay[q * 128 + j] * f64::from(window::WINDOW[q * 64 + j])
                        + self.delay[q * 128 + 96 + j] * f64::from(window::WINDOW[q * 64 + 32 + j])
                })
                .sum::<f64>()
                / 65536.
        });
        Ok(output)
    }
}
