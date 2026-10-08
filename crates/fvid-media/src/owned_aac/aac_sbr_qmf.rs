//! Owned 32-band SBR analysis QMF, normative complex modulation.
use super::{Result, aac_sbr_qmf_window::WINDOW, invalid};
use std::sync::OnceLock;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}
fn modulation() -> &'static [[Complex; 64]; 32] {
    static MATRIX: OnceLock<[[Complex; 64]; 32]> = OnceLock::new();
    MATRIX.get_or_init(|| {
        std::array::from_fn(|k| {
            std::array::from_fn(|n| {
                let angle = std::f64::consts::PI * (k as f64 + 0.5) * (2.0 * n as f64 - 0.5) / 64.0;
                let (im, re) = angle.sin_cos();
                Complex {
                    re: 2.0 * re,
                    im: 2.0 * im,
                }
            })
        })
    })
}
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    history: [f64; 320],
}
impl Default for Analysis {
    fn default() -> Self {
        Self {
            history: [0.0; 320],
        }
    }
}
impl Analysis {
    pub fn reset(&mut self) {
        self.history.fill(0.0);
    }
    /// PCM is chronological. One slot consumes 32 core samples; complete
    /// 960/1024-sample frames therefore produce 30/32 complex subband slots.
    /// Validate the whole call before changing retained history.
    pub fn process(&mut self, pcm: &[f32]) -> Result<Vec<[Complex; 32]>> {
        if pcm.len() % 32 != 0 || pcm.iter().any(|x| !x.is_finite()) {
            return Err(invalid("invalid SBR QMF PCM block"));
        }
        let matrix = modulation();
        let mut result = Vec::with_capacity(pcm.len() / 32);
        for block in pcm.chunks_exact(32) {
            self.history.copy_within(0..288, 32);
            for (dst, &src) in self.history[..32].iter_mut().zip(block.iter().rev()) {
                *dst = f64::from(src);
            }
            let u: [f64; 64] = std::array::from_fn(|n| {
                (0..5)
                    .map(|j| {
                        let i = n + 64 * j;
                        self.history[i] * WINDOW[2 * i]
                    })
                    .sum()
            });
            result.push(std::array::from_fn(|k| {
                let mut value = Complex::default();
                for (n, &input) in u.iter().enumerate() {
                    value.re += input * matrix[k][n].re;
                    value.im += input * matrix[k][n].im;
                }
                value
            }));
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_time_convolution_oracles_cover_impulses_dense_signal_and_tones() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for name in ["impulse-first", "impulse-boundary", "dense", "tones"] {
            let raw = std::fs::read(root.join(format!("aac-sbr-qmf-{name}.f32le"))).unwrap();
            let pcm: Vec<_> = raw
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let expected =
                std::fs::read(root.join(format!("aac-sbr-qmf-{name}.complex-f64le"))).unwrap();
            let mut state = Analysis::default();
            let actual = state.process(&pcm).unwrap();
            assert_eq!(actual.len(), 22);
            for (&v, bytes) in actual.iter().flatten().zip(expected.chunks_exact(16)) {
                let re = f64::from_le_bytes(bytes[..8].try_into().unwrap());
                let im = f64::from_le_bytes(bytes[8..].try_into().unwrap());
                assert!(
                    (v.re - re).abs() < 2e-12 && (v.im - im).abs() < 2e-12,
                    "{name}: {v:?} vs {re},{im}"
                );
            }
            assert_eq!(expected.len(), actual.len() * 32 * 16);
            let mut streamed = Analysis::default();
            let mut split = Vec::new();
            for block in pcm.chunks_exact(32) {
                split.extend(streamed.process(block).unwrap());
            }
            assert_eq!(actual, split);
            assert_eq!(state, streamed);
        }
    }
    #[test]
    fn frame_geometry_checkpoint_reset_and_failed_calls_preserve_history() {
        let mut state = Analysis::default();
        assert_eq!(state.process(&[0.0; 960]).unwrap().len(), 30);
        assert_eq!(state.process(&[0.0; 1024]).unwrap().len(), 32);
        state.process(&[0.25; 320]).unwrap();
        let checkpoint = state.clone();
        for invalid in [vec![0.0; 31], vec![f32::NAN; 32], vec![f32::INFINITY; 64]] {
            assert!(state.process(&invalid).is_err());
            assert_eq!(state, checkpoint);
        }
        let mut restored = checkpoint.clone();
        assert_eq!(
            state.process(&[0.5; 64]).unwrap(),
            restored.process(&[0.5; 64]).unwrap()
        );
        state.reset();
        assert_eq!(state, Analysis::default());
        assert!(
            state
                .process(&[0.0; 320])
                .unwrap()
                .iter()
                .flatten()
                .all(|x| *x == Complex::default())
        );
    }
}
