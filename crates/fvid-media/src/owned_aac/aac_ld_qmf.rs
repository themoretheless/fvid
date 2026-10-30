//! Owned LD-QMF building block for spatial reconstruction, ISO 23003-2 §7.14.2.
//! This is not the CLDFB used by plain LD-SBR. Spatial bitstream dispatch and
//! upmix are separate requirements; this module alone does not enable LD-MPS.
use super::{Result, aac_ld_qmf_window::WINDOW, aac_sbr_qmf::Complex, invalid};
use std::sync::OnceLock;

fn geometry<const B: usize>() -> Result<()> {
    if !matches!(B, 32 | 64) {
        return Err(invalid("invalid LD-QMF band count"));
    }
    Ok(())
}
fn coefficient<const B: usize>(i: usize) -> f64 {
    if B == 64 {
        WINDOW[i]
    } else {
        (WINDOW[2 * i] + WINDOW[2 * i + 1]) * 0.5
    }
}
fn matrix<const B: usize>(synthesis: bool) -> &'static [Complex] {
    static A32: OnceLock<Vec<Complex>> = OnceLock::new();
    static A64: OnceLock<Vec<Complex>> = OnceLock::new();
    static S32: OnceLock<Vec<Complex>> = OnceLock::new();
    static S64: OnceLock<Vec<Complex>> = OnceLock::new();
    let cache = match (B, synthesis) {
        (32, false) => &A32,
        (64, false) => &A64,
        (32, true) => &S32,
        (64, true) => &S64,
        _ => unreachable!(),
    };
    cache.get_or_init(|| {
        (0..2 * B)
            .flat_map(|n| {
                (0..B).map(move |k| {
                    let shift = if synthesis {
                        -(4.0 * B as f64 - 1.0)
                    } else {
                        2.0 * B as f64 + 1.0
                    };
                    let phase = std::f64::consts::PI * (k as f64 + 0.5) * (2.0 * n as f64 + shift)
                        / (2.0 * B as f64);
                    let gain = if synthesis {
                        1.0 / 64.0
                    } else {
                        64.0 / B as f64
                    };
                    let (im, re) = phase.sin_cos();
                    Complex {
                        re: gain * re,
                        im: gain * im,
                    }
                })
            })
            .collect()
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Analysis<const B: usize> {
    history: Vec<f64>,
}
impl<const B: usize> Analysis<B> {
    pub fn new() -> Result<Self> {
        geometry::<B>()?;
        Ok(Self {
            history: vec![0.; 10 * B],
        })
    }
    pub fn reset(&mut self) {
        self.history.fill(0.);
    }
    pub fn process(&mut self, pcm: &[f64]) -> Result<Vec<[Complex; B]>> {
        if !pcm.len().is_multiple_of(B) || pcm.iter().any(|v| !v.is_finite()) {
            return Err(invalid("invalid LD-QMF PCM block"));
        }
        let mut history = self.history.clone();
        let mut result = Vec::with_capacity(pcm.len() / B);
        let m = matrix::<B>(false);
        for block in pcm.chunks_exact(B) {
            history.copy_within(0..9 * B, B);
            for (dst, src) in history[..B].iter_mut().zip(block.iter().rev()) {
                *dst = *src;
            }
            let folded: Vec<_> = (0..2 * B)
                .map(|n| {
                    (0..5)
                        .map(|p| {
                            let i = n + 2 * B * p;
                            history[i] * coefficient::<B>(i)
                        })
                        .sum::<f64>()
                })
                .collect();
            let slot = std::array::from_fn(|k| {
                let mut v = Complex::default();
                for (n, x) in folded.iter().enumerate() {
                    let c = m[n * B + k];
                    v.re += x * c.re;
                    v.im += x * c.im;
                }
                v
            });
            if slot.iter().any(|v| !v.re.is_finite() || !v.im.is_finite()) {
                return Err(invalid("overflowing LD-QMF analysis"));
            }
            result.push(slot);
        }
        self.history = history;
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Synthesis<const B: usize> {
    history: Vec<f64>,
}
impl<const B: usize> Synthesis<B> {
    pub fn new() -> Result<Self> {
        geometry::<B>()?;
        Ok(Self {
            history: vec![0.; 20 * B],
        })
    }
    pub fn reset(&mut self) {
        self.history.fill(0.);
    }
    pub fn process(&mut self, slots: &[[Complex; B]]) -> Result<Vec<f64>> {
        if slots
            .iter()
            .flatten()
            .any(|v| !v.re.is_finite() || !v.im.is_finite())
        {
            return Err(invalid("invalid LD-QMF subband value"));
        }
        let capacity = slots
            .len()
            .checked_mul(B)
            .ok_or_else(|| invalid("oversized LD-QMF synthesis input"))?;
        let mut history = self.history.clone();
        let mut result = Vec::with_capacity(capacity);
        let m = matrix::<B>(true);
        for slot in slots {
            history.copy_within(0..18 * B, 2 * B);
            for n in 0..2 * B {
                history[n] = slot
                    .iter()
                    .enumerate()
                    .map(|(k, v)| {
                        let c = m[n * B + k];
                        v.re * c.re - v.im * c.im
                    })
                    .sum();
            }
            if history[..2 * B].iter().any(|v| !v.is_finite()) {
                return Err(invalid("overflowing LD-QMF synthesis transform"));
            }
            for k in 0..B {
                let value = (0..5)
                    .map(|p| {
                        history[4 * B * p + k] * coefficient::<B>(2 * B * p + k)
                            + history[4 * B * p + 3 * B + k] * coefficient::<B>(2 * B * p + B + k)
                    })
                    .sum::<f64>();
                if !value.is_finite() {
                    return Err(invalid("overflowing LD-QMF synthesis"));
                }
                result.push(value);
            }
        }
        self.history = history;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/aac-ld-qmf")
    }
    fn doubles(name: &str) -> Vec<f64> {
        std::fs::read(root().join(name))
            .unwrap()
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
            .collect()
    }
    fn qualify<const B: usize>() {
        for name in ["impulse", "tones", "dense"] {
            let key = format!("{B}-{name}");
            let pcm: Vec<_> = std::fs::read(root().join(format!("{key}.input.f32le")))
                .unwrap()
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()) as f64)
                .collect();
            let mut a = Analysis::<B>::new().unwrap();
            let slots = a.process(&pcm).unwrap();
            let expected = doubles(&format!("{key}.analysis.f64le"));
            assert_eq!(expected.len(), slots.len() * B * 2);
            for (v, b) in slots.iter().flatten().zip(expected.chunks_exact(2)) {
                assert!(
                    (v.re - b[0]).abs() < 3e-12 && (v.im - b[1]).abs() < 3e-12,
                    "{key}: {v:?} != {b:?}"
                );
            }
            let mut s = Synthesis::<B>::new().unwrap();
            let output = s.process(&slots).unwrap();
            let expected = doubles(&format!("{key}.output.f64le"));
            assert_eq!(output.len(), expected.len());
            for (i, (x, y)) in output.iter().zip(&expected).enumerate() {
                assert!((x - y).abs() < 3e-12, "{key}/{i}: {x} != {y}");
            }
            if name == "impulse" {
                assert_eq!(
                    output
                        .iter()
                        .enumerate()
                        .max_by(|(_, x), (_, y)| x.abs().total_cmp(&y.abs()))
                        .unwrap()
                        .0,
                    4 * B
                );
            }
            let mut streamed = Analysis::<B>::new().unwrap();
            let mut split = Vec::new();
            for block in pcm.chunks_exact(B) {
                split.extend(streamed.process(block).unwrap());
            }
            assert_eq!(slots, split);
            assert_eq!(a, streamed);
            let mut streamed = Synthesis::<B>::new().unwrap();
            let mut split = Vec::new();
            for block in &slots {
                split.extend(streamed.process(std::slice::from_ref(block)).unwrap());
            }
            assert_eq!(output, split);
            assert_eq!(s, streamed);
            let mut resumed_a = Analysis::<B>::new().unwrap();
            resumed_a.process(&pcm[..7 * B]).unwrap();
            let mut clone_a = resumed_a.clone();
            assert_eq!(
                resumed_a.process(&pcm[7 * B..]).unwrap(),
                clone_a.process(&pcm[7 * B..]).unwrap()
            );
            let mut resumed_s = Synthesis::<B>::new().unwrap();
            resumed_s.process(&slots[..7]).unwrap();
            let mut clone_s = resumed_s.clone();
            assert_eq!(
                resumed_s.process(&slots[7..]).unwrap(),
                clone_s.process(&slots[7..]).unwrap()
            );
            a.reset();
            s.reset();
            assert_eq!(a, Analysis::new().unwrap());
            assert_eq!(s, Synthesis::new().unwrap());
            assert_eq!(a.process(&pcm).unwrap(), slots);
            assert_eq!(s.process(&slots).unwrap(), output);
        }
    }
    fn arbitrary_synthesis<const B: usize>() {
        for name in ["low-real", "high-imag", "complex-dense"] {
            let key = format!("{B}-{name}");
            let bytes = std::fs::read(root().join(format!("{key}.complex.f32le"))).unwrap();
            let slots: Vec<[Complex; B]> = bytes
                .chunks_exact(B * 8)
                .map(|row| {
                    std::array::from_fn(|k| Complex {
                        re: f32::from_le_bytes(row[k * 8..k * 8 + 4].try_into().unwrap()) as f64,
                        im: f32::from_le_bytes(row[k * 8 + 4..k * 8 + 8].try_into().unwrap())
                            as f64,
                    })
                })
                .collect();
            let expected = doubles(&format!("{key}.output.f64le"));
            let mut s = Synthesis::<B>::new().unwrap();
            let output = s.process(&slots).unwrap();
            assert_eq!(output.len(), expected.len());
            for (i, (a, b)) in output.iter().zip(&expected).enumerate() {
                assert!((a - b).abs() < 3e-12, "{key}/{i}: {a} != {b}");
            }
            let mut chunks = Synthesis::<B>::new().unwrap();
            let mut streamed = Vec::new();
            for block in slots.chunks(3) {
                streamed.extend(chunks.process(block).unwrap());
            }
            assert_eq!(streamed, output);
            assert_eq!(s, chunks);
        }
    }
    #[test]
    fn arbitrary_upmix_domain_real_imaginary_and_dense_subbands_match_direct_oracle() {
        arbitrary_synthesis::<32>();
        arbitrary_synthesis::<64>();
    }
    #[test]
    fn direct_convolution_32_and_64_band_traces_keep_streaming_and_checkpoint_state() {
        qualify::<32>();
        qualify::<64>();
    }
    fn rollback<const B: usize>() {
        let mut a = Analysis::<B>::new().unwrap();
        a.process(&vec![0.25; 3 * B]).unwrap();
        let saved = a.clone();
        assert!(a.process(&vec![0.; B + 1]).is_err());
        assert_eq!(a, saved);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            let mut block = vec![bad; 20 * B];
            block[0] = 1.;
            assert!(a.process(&block).is_err());
            assert_eq!(a, saved);
        }
        let mut s = Synthesis::<B>::new().unwrap();
        s.process(&[[Complex { re: 0.3, im: -0.2 }; B]; 3]).unwrap();
        let saved = s.clone();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut slots = vec![[Complex { re: bad, im: bad }; B]; 20];
            slots[0] = [Complex::default(); B];
            assert!(s.process(&slots).is_err());
            assert_eq!(s, saved);
        }
        // Align every subband with its contribution to one output sample,
        // including the sign of each prototype tap. Constant MAX subbands can
        // cancel and are not necessarily an arithmetic overflow.
        let mut slots = vec![[Complex::default(); B]; 11];
        let m = matrix::<B>(true);
        for p in 0..5 {
            for (delay, row, tap) in [(2 * p, 0, 2 * B * p), (2 * p + 1, B, 2 * B * p + B)] {
                let weight = coefficient::<B>(tap);
                for k in 0..B {
                    let c = m[row * B + k];
                    slots[10 - delay][k] = Complex {
                        re: (c.re * weight).signum() * f64::MAX,
                        im: -(c.im * weight).signum() * f64::MAX,
                    };
                }
            }
        }
        assert!(s.process(&slots).is_err());
        assert_eq!(s, saved);
    }
    #[test]
    fn malformed_and_later_overflowing_blocks_preserve_all_history() {
        rollback::<32>();
        rollback::<64>();
        assert!(Analysis::<0>::new().is_err());
        assert!(Synthesis::<128>::new().is_err());
    }
}

impl<const B: usize> Analysis<B> {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        f.vector(&self.history)
    }
}

impl<const B: usize> Synthesis<B> {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        f.vector(&self.history)
    }
}
