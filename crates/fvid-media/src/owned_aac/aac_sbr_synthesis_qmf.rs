//! Owned 64-band complex SBR synthesis QMF.
use super::{Result, aac_sbr_qmf::Complex, aac_sbr_qmf_window::WINDOW, invalid};
use std::sync::OnceLock;
fn matrix() -> &'static [[Complex; 64]; 128] {
    static MATRIX: OnceLock<[[Complex; 64]; 128]> = OnceLock::new();
    MATRIX.get_or_init(|| {
        std::array::from_fn(|n| {
            std::array::from_fn(|k| {
                let phase =
                    std::f64::consts::PI * (k as f64 + 0.5) * (2.0 * n as f64 - 255.0) / 128.0;
                let (im, re) = phase.sin_cos();
                Complex {
                    re: re / 64.0,
                    im: im / 64.0,
                }
            })
        })
    })
}
#[derive(Clone, Debug, PartialEq)]
pub struct Synthesis {
    history: [f64; 1280],
}
impl Default for Synthesis {
    fn default() -> Self {
        Self {
            history: [0.0; 1280],
        }
    }
}
impl Synthesis {
    pub fn reset(&mut self) {
        self.history.fill(0.0);
    }
    /// Return 64 chronological PCM samples per slot. Retained history commits
    /// only when the whole call succeeds, including finite arithmetic checks.
    pub fn process(&mut self, slots: &[[Complex; 64]]) -> Result<Vec<f64>> {
        if slots
            .iter()
            .flatten()
            .any(|x| !x.re.is_finite() || !x.im.is_finite())
        {
            return Err(invalid("invalid SBR synthesis subband value"));
        }
        let capacity = slots
            .len()
            .checked_mul(64)
            .ok_or_else(|| invalid("oversized SBR synthesis input"))?;
        let mut history = self.history;
        let mut result = Vec::with_capacity(capacity);
        let transform = matrix();
        for slot in slots {
            history.copy_within(0..1152, 128);
            for (n, value) in history[..128].iter_mut().enumerate() {
                *value = slot
                    .iter()
                    .zip(&transform[n])
                    .map(|(x, m)| x.re * m.re - x.im * m.im)
                    .sum();
                if !value.is_finite() {
                    return Err(invalid("overflowing SBR synthesis transform"));
                }
            }
            for k in 0..64 {
                let mut sample = 0.0;
                for p in 0..5 {
                    sample += history[256 * p + k] * WINDOW[128 * p + k];
                    sample += history[256 * p + 192 + k] * WINDOW[128 * p + 64 + k];
                }
                if !sample.is_finite() {
                    return Err(invalid("overflowing SBR synthesis PCM"));
                }
                result.push(sample);
            }
        }
        self.history = history;
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> (Vec<[Complex; 64]>, Vec<f64>) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let raw =
            std::fs::read(root.join(format!("aac-sbr-synthesis-{name}.complex-f64le"))).unwrap();
        let slots = raw
            .chunks_exact(64 * 16)
            .map(|b| {
                std::array::from_fn(|k| Complex {
                    re: f64::from_le_bytes(b[k * 16..k * 16 + 8].try_into().unwrap()),
                    im: f64::from_le_bytes(b[k * 16 + 8..k * 16 + 16].try_into().unwrap()),
                })
            })
            .collect();
        let raw = std::fs::read(root.join(format!("aac-sbr-synthesis-{name}.pcm-f64le"))).unwrap();
        let pcm = raw
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        (slots, pcm)
    }
    #[test]
    fn independent_direct_convolution_matches_real_imaginary_dense_and_analysis_inputs() {
        for name in ["low-real", "high-imag", "dense", "analysis-bypass"] {
            let (slots, expected) = fixture(name);
            let mut state = Synthesis::default();
            let actual = state.process(&slots).unwrap();
            assert_eq!(actual.len(), expected.len());
            assert_eq!(actual.len(), 22 * 64);
            for (i, (&a, &b)) in actual.iter().zip(&expected).enumerate() {
                assert!((a - b).abs() < 3e-13, "{name} sample {i}: {a} != {b}");
            }
            let mut streamed = Synthesis::default();
            let mut split = Vec::new();
            for slot in &slots {
                split.extend(streamed.process(std::slice::from_ref(slot)).unwrap());
            }
            assert_eq!(actual, split);
            assert_eq!(state, streamed);
            let mut state = Synthesis::default();
            state.process(&slots[..9]).unwrap();
            let mut copy = state.clone();
            assert_eq!(
                state.process(&slots[9..]).unwrap(),
                copy.process(&slots[9..]).unwrap()
            );
            state.reset();
            assert_eq!(state, Synthesis::default());
        }
    }
    #[test]
    fn invalid_later_slot_rolls_back_the_entire_call_and_reset_discards_tail() {
        let (slots, _) = fixture("dense");
        let mut state = Synthesis::default();
        state.process(&slots[..4]).unwrap();
        let saved = state.clone();
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut bad = slots[4..6].to_vec();
            bad[1][63].im = value;
            assert!(state.process(&bad).is_err());
            assert_eq!(state, saved);
        }
        assert_eq!(state.process(&[]).unwrap(), Vec::<f64>::new());
        assert_eq!(state, saved);
        state.reset();
        assert!(
            state
                .process(&[[Complex::default(); 64]; 10])
                .unwrap()
                .iter()
                .all(|x| *x == 0.0)
        );
        assert_eq!(
            state
                .process(&[[Complex::default(); 64]; 30])
                .unwrap()
                .len(),
            1920
        );
        assert_eq!(
            state
                .process(&[[Complex::default(); 64]; 32])
                .unwrap()
                .len(),
            2048
        );
    }
    #[test]
    fn finite_arithmetic_overflow_is_transactional_after_a_valid_slot() {
        let (slots, _) = fixture("dense");
        let mut state = Synthesis::default();
        state.process(&slots[..3]).unwrap();
        let saved = state.clone();
        let extreme = std::array::from_fn(|k| {
            let phase = std::f64::consts::PI * (k as f64 + 0.5) * (128.0 - 255.0) / 128.0;
            let (sin, cos) = phase.sin_cos();
            Complex {
                re: f64::MAX * cos.signum(),
                im: -f64::MAX * sin.signum(),
            }
        });
        let error = state.process(&[slots[3], extreme]).unwrap_err();
        assert!(error.to_string().contains("overflowing"));
        assert_eq!(state, saved);
    }
    #[test]
    fn owned_analysis_and_synthesis_compose_against_independent_pcm_trace() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let raw = std::fs::read(root.join("aac-sbr-qmf-tones.f32le")).unwrap();
        let pcm: Vec<_> = raw
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let low = super::super::aac_sbr_qmf::Analysis::default()
            .process(&pcm)
            .unwrap();
        let slots: Vec<_> = low
            .iter()
            .map(|l| std::array::from_fn(|k| if k < 32 { l[k] } else { Complex::default() }))
            .collect();
        let actual = Synthesis::default().process(&slots).unwrap();
        let (_, expected) = fixture("analysis-bypass");
        assert_eq!(actual.len(), expected.len());
        for (&a, &b) in actual.iter().zip(&expected) {
            assert!((a - b).abs() < 3e-13);
        }
    }
}
