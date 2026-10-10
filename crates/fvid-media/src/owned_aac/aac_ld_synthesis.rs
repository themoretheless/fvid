//! Owned AAC-LD filterbank. ASC admission and LD packet syntax remain separate.
use super::{Result, aac_imdct::Imdct, invalid, memory::Footprint};
use std::{f64::consts::PI, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LdWindowShape {
    Sine,
    LowOverlap,
}
impl LdWindowShape {
    fn index(self) -> usize {
        usize::from(self == Self::LowOverlap)
    }
}
#[derive(Clone)]
pub struct LdSynthesisHistory {
    previous: LdWindowShape,
    overlap: Vec<f64>,
}
#[derive(Clone)]
pub struct LdSynthesis {
    transform: Imdct,
    windows: [Arc<Vec<f64>>; 2],
    complex: Vec<[f64; 2]>,
    scratch: Vec<f64>,
    history: LdSynthesisHistory,
}
impl LdSynthesis {
    pub fn new(n: usize) -> Result<Self> {
        if !matches!(n, 480 | 512) {
            return Err(invalid("AAC LD filterbank requires 480 or 512 samples"));
        }
        let width = 2 * n;
        let sine = (0..width)
            .map(|i| (PI * (i as f64 + 0.5) / width as f64).sin())
            .collect();
        // ISO14496-3 4.6.17.2.3: low-overlap support on [3N/16,13N/16).
        let low = (0..width)
            .map(|i| {
                if i < 3 * width / 16 || i >= 13 * width / 16 {
                    0.
                } else if i < 5 * width / 16 {
                    (PI * (i as f64 - 3. * width as f64 / 16. + 0.5) / (width as f64 / 4.)).sin()
                } else if i < 11 * width / 16 {
                    1.
                } else {
                    (PI * (i as f64 - 9. * width as f64 / 16. + 0.5) / (width as f64 / 4.)).sin()
                }
            })
            .collect();
        let transform = Imdct::new(n)?;
        let complex = vec![[0.; 2]; transform.scratch_len()];
        Ok(Self {
            transform,
            windows: [Arc::new(sine), Arc::new(low)],
            complex,
            scratch: vec![0.; width],
            history: LdSynthesisHistory {
                previous: LdWindowShape::Sine,
                overlap: vec![0.; n],
            },
        })
    }
    pub fn checkpoint(&self) -> LdSynthesisHistory {
        self.history.clone()
    }
    pub fn restore(&mut self, state: &LdSynthesisHistory) -> Result<()> {
        if state.overlap.len() != self.history.overlap.len()
            || state.overlap.iter().any(|v| !v.is_finite())
        {
            return Err(invalid("AAC LD synthesis checkpoint mismatch"));
        }
        self.history.overlap.copy_from_slice(&state.overlap);
        self.history.previous = state.previous;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.history.overlap.fill(0.);
        self.history.previous = LdWindowShape::Sine;
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        self.transform.visit_retained(f)?;
        for window in &self.windows {
            if f.shared(window)? {
                f.vector(window)?;
            }
        }
        f.vector(&self.complex)?;
        f.vector(&self.scratch)?;
        f.vector(&self.history.overlap)
    }
    pub fn retained_bytes(&self) -> std::result::Result<usize, String> {
        let mut f = Footprint::new();
        self.visit_retained(&mut f)?;
        Ok(f.total())
    }
    /// No per-frame allocation. Output and overlap are unchanged on failure.
    pub fn synthesize_raw(
        &mut self,
        spectrum: &[f32],
        shape: LdWindowShape,
        out: &mut [f64],
    ) -> Result<()> {
        let n = self.history.overlap.len();
        if out.len() != n {
            return Err(invalid("AAC LD synthesis output size mismatch"));
        }
        self.transform
            .inverse_with_scratch(spectrum, &mut self.scratch, &mut self.complex)?;
        let before = &self.windows[self.history.previous.index()];
        let after = &self.windows[shape.index()];
        for i in 0..n {
            self.scratch[i] = self.history.overlap[i] + self.scratch[i] * before[i];
            self.scratch[n + i] *= after[n + i];
        }
        if self.scratch.iter().any(|x| !x.is_finite()) {
            return Err(invalid("AAC LD synthesis overflow"));
        }
        out.copy_from_slice(&self.scratch[..n]);
        self.history.overlap.copy_from_slice(&self.scratch[n..]);
        self.history.previous = shape;
        Ok(())
    }
    pub fn synthesize_pcm(
        &mut self,
        spectrum: &[f32],
        shape: LdWindowShape,
        out: &mut [f64],
    ) -> Result<()> {
        self.synthesize_raw(spectrum, shape, out)?;
        for x in out {
            *x /= 65536.;
        }
        Ok(())
    }
    /// Windowed forward MDCT for LD-LTP. Does not advance PCM/window history.
    pub fn analyze(
        &mut self,
        input: &[f64],
        previous: LdWindowShape,
        current: LdWindowShape,
        out: &mut [f64],
    ) -> Result<()> {
        let n = self.history.overlap.len();
        if input.len() != 2 * n || out.len() != n || input.iter().any(|x| !x.is_finite()) {
            return Err(invalid("invalid AAC LD analysis samples"));
        }
        for i in 0..2 * n {
            self.scratch[i] = input[i]
                * self.windows[if i < n {
                    previous.index()
                } else {
                    current.index()
                }][i];
        }
        self.transform
            .forward_with_scratch(&self.scratch, out, &mut self.complex)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_overlap_is_symmetric_power_complementary_and_has_exact_support() {
        for n in [480, 512] {
            let s = LdSynthesis::new(n).unwrap();
            let w = &s.windows[1];
            for i in 0..2 * n {
                assert!((w[i] - w[2 * n - 1 - i]).abs() < 1e-14);
                assert_eq!(w[i] == 0., i < 3 * n / 8 || i >= 13 * n / 8);
                if (5 * n / 8..11 * n / 8).contains(&i) {
                    assert_eq!(w[i], 1.);
                }
            }
            for i in 0..n {
                assert!((w[i] * w[i] + w[n + i] * w[n + i] - 1.).abs() < 1e-14);
            }
        }
    }
    #[test]
    fn checkpoint_shape_size_and_invalid_inputs_are_transactional() {
        let mut s = LdSynthesis::new(480).unwrap();
        let mut out = vec![0.; 480];
        s.synthesize_raw(&vec![1.; 480], LdWindowShape::LowOverlap, &mut out)
            .unwrap();
        let saved = s.checkpoint();
        let mut control = s.clone();
        let mut bad = vec![0.; 480];
        bad[13] = f32::NAN;
        out.fill(99.);
        assert!(
            s.synthesize_raw(&bad, LdWindowShape::Sine, &mut out)
                .is_err()
        );
        assert_eq!(out, vec![99.; 480]);
        assert!(
            s.restore(&LdSynthesis::new(512).unwrap().checkpoint())
                .is_err()
        );
        let mut x = vec![0.; 480];
        let mut y = vec![0.; 480];
        s.synthesize_pcm(&vec![0.; 480], LdWindowShape::Sine, &mut x)
            .unwrap();
        control
            .synthesize_pcm(&vec![0.; 480], LdWindowShape::Sine, &mut y)
            .unwrap();
        assert_eq!(x, y);
        s.restore(&saved).unwrap();
        s.synthesize_pcm(&vec![0.; 480], LdWindowShape::Sine, &mut x)
            .unwrap();
        assert_eq!(x, y);
    }
}
