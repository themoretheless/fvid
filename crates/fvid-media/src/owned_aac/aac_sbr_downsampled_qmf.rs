//! Owned 32-band downsampled complex SBR synthesis QMF.
use super::{Result, aac_sbr_qmf::Complex, aac_sbr_qmf_window::WINDOW, invalid};
use std::sync::OnceLock;
fn matrix() -> &'static [[Complex; 32]; 64] {
    static MATRIX: OnceLock<[[Complex; 32]; 64]> = OnceLock::new();
    MATRIX.get_or_init(|| {
        std::array::from_fn(|n| {
            std::array::from_fn(|k| {
                let phase =
                    std::f64::consts::PI * (k as f64 + 0.5) * (2.0 * n as f64 - 127.5) / 64.0;
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
    history: [f64; 640],
}
impl Default for Synthesis {
    fn default() -> Self {
        Self {
            history: [0.0; 640],
        }
    }
}
impl Synthesis {
    pub fn reset(&mut self) {
        self.history.fill(0.0);
    }
    /// Return 32 chronological PCM samples per slot. Retained history commits
    /// only when the whole call succeeds, including finite arithmetic checks.
    pub fn process(&mut self, slots: &[[Complex; 32]]) -> Result<Vec<f64>> {
        if slots
            .iter()
            .flatten()
            .any(|x| !x.re.is_finite() || !x.im.is_finite())
        {
            return Err(invalid("invalid SBR synthesis subband value"));
        }
        let capacity = slots
            .len()
            .checked_mul(32)
            .ok_or_else(|| invalid("oversized SBR synthesis input"))?;
        let mut history = self.history;
        let mut result = Vec::with_capacity(capacity);
        let transform = matrix();
        for slot in slots {
            history.copy_within(0..576, 64);
            for (n, value) in history[..64].iter_mut().enumerate() {
                *value = slot
                    .iter()
                    .zip(&transform[n])
                    .map(|(x, m)| x.re * m.re - x.im * m.im)
                    .sum();
                if !value.is_finite() {
                    return Err(invalid("overflowing SBR synthesis transform"));
                }
            }
            for k in 0..32 {
                let mut sample = 0.0;
                for p in 0..5 {
                    sample += history[128 * p + k] * WINDOW[128 * p + 2 * k];
                    sample += history[128 * p + 96 + k] * WINDOW[128 * p + 64 + 2 * k];
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
    fn fixture(name: &str) -> (Vec<[Complex; 32]>, Vec<f64>) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let bytes =
            std::fs::read(root.join(format!("aac-sbr-synthesis32-{name}.complex-f64le"))).unwrap();
        let slots = bytes
            .chunks_exact(32 * 16)
            .map(|b| {
                std::array::from_fn(|k| Complex {
                    re: f64::from_le_bytes(b[k * 16..k * 16 + 8].try_into().unwrap()),
                    im: f64::from_le_bytes(b[k * 16 + 8..k * 16 + 16].try_into().unwrap()),
                })
            })
            .collect();
        let bytes =
            std::fs::read(root.join(format!("aac-sbr-synthesis32-{name}.pcm-f64le"))).unwrap();
        let pcm = bytes
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        (slots, pcm)
    }
    #[test]
    fn all_independent_traces_and_full_rate_decimation_agree() {
        for name in ["low-real", "high-imag", "dense", "analysis-bypass"] {
            let (slots, expected) = fixture(name);
            let mut state = Synthesis::default();
            let actual = state.process(&slots).unwrap();
            assert_eq!(actual.len(), 22 * 32);
            assert_eq!(actual.len(), expected.len());
            for (i, (&a, &b)) in actual.iter().zip(&expected).enumerate() {
                assert!((a - b).abs() < 3e-13, "{name}:{i}: {a}!={b}");
            }
            let wide: Vec<_> = slots
                .iter()
                .map(|x| std::array::from_fn(|k| if k < 32 { x[k] } else { Complex::default() }))
                .collect();
            let full = super::super::aac_sbr_synthesis_qmf::Synthesis::default()
                .process(&wide)
                .unwrap();
            assert_eq!(actual, full.into_iter().step_by(2).collect::<Vec<_>>());
            let mut streamed = Synthesis::default();
            let mut split = Vec::new();
            for slot in &slots {
                split.extend(streamed.process(std::slice::from_ref(slot)).unwrap());
            }
            assert_eq!(actual, split);
            assert_eq!(state, streamed);
        }
    }
    #[test]
    fn composed_analysis_checkpoint_reset_and_invalid_call_rollback() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let bytes = std::fs::read(root.join("aac-sbr-qmf-tones.f32le")).unwrap();
        let pcm: Vec<_> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let slots = super::super::aac_sbr_qmf::Analysis::default()
            .process(&pcm)
            .unwrap();
        let actual = Synthesis::default().process(&slots).unwrap();
        let (_, expected) = fixture("analysis-bypass");
        for (&a, &b) in actual.iter().zip(&expected) {
            assert!((a - b).abs() < 3e-13);
        }
        let mut state = Synthesis::default();
        state.process(&slots[..9]).unwrap();
        let mut copy = state.clone();
        assert_eq!(
            state.process(&slots[9..]).unwrap(),
            copy.process(&slots[9..]).unwrap()
        );
        let saved = state.clone();
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut bad = slots[..2].to_vec();
            bad[1][31].re = value;
            assert!(state.process(&bad).is_err());
            assert_eq!(state, saved);
        }
        state.reset();
        assert_eq!(state, Synthesis::default());
        assert!(
            state
                .process(&[[Complex::default(); 32]; 10])
                .unwrap()
                .iter()
                .all(|x| *x == 0.0)
        );
        assert_eq!(
            state
                .process(&[[Complex::default(); 32]; 30])
                .unwrap()
                .len(),
            960
        );
        assert_eq!(
            state
                .process(&[[Complex::default(); 32]; 32])
                .unwrap()
                .len(),
            1024
        );
    }
}
