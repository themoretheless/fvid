//! Owned ELD inverse modulation and four-block normative-window overlap.
use super::{
    Result, aac_eld_downscale_window, aac_eld_windows, aac_imdct::Imdct, invalid, memory::Footprint,
};
use std::sync::Arc;
#[derive(Clone)]
enum Window {
    Fixed(&'static [f64]),
    Adapted(Arc<Vec<f64>>),
}
impl Window {
    fn values(&self) -> &[f64] {
        match self {
            Self::Fixed(v) => v,
            Self::Adapted(v) => v,
        }
    }
    fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        if let Self::Adapted(v) = self
            && f.shared(v)?
        {
            f.vector(v)?;
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct EldHistory {
    overlap: Vec<f64>,
}
impl EldHistory {
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        f.vector(&self.overlap)
    }
}
#[derive(Clone)]
pub struct EldSynthesis {
    transform: Imdct,
    window: Window,
    source_samples: usize,
    complex: Vec<[f64; 2]>,
    inverse: Vec<f64>,
    overlap: EldHistory,
}
impl EldSynthesis {
    pub fn new(n: usize) -> Result<Self> {
        let window: &'static [f64] = match n {
            512 => &aac_eld_windows::WINDOW_512,
            480 => &aac_eld_windows::WINDOW_480,
            _ => return Err(invalid("AAC ELD filterbank requires 480 or 512 samples")),
        };
        Self::with_window(n, n, Window::Fixed(window))
    }
    /// Decode the low-frequency spectrum with a short ELD filterbank. The
    /// nominal source transform still determines spectral level normalization.
    pub fn new_downscaled(source_samples: usize, factor: u8) -> Result<Self> {
        if !matches!(source_samples, 480 | 512)
            || !matches!(factor, 2 | 3 | 4 | 8)
            || !source_samples.is_multiple_of(usize::from(factor))
        {
            return Err(invalid("unsupported AAC ELD downscale factor"));
        }
        let n = source_samples / usize::from(factor);
        let window = Window::Adapted(Arc::new(aac_eld_downscale_window::window(n)?));
        Self::with_window(source_samples, n, window)
    }
    fn with_window(source_samples: usize, n: usize, window: Window) -> Result<Self> {
        let transform = Imdct::new(n)?;
        Ok(Self {
            complex: vec![[0.; 2]; transform.scratch_len()],
            transform,
            window,
            source_samples,
            inverse: vec![0.; 2 * n],
            overlap: EldHistory {
                overlap: vec![0.; 3 * n],
            },
        })
    }
    pub fn checkpoint(&self) -> EldHistory {
        self.overlap.clone()
    }
    pub fn restore(&mut self, s: &EldHistory) -> Result<()> {
        if s.overlap.len() != self.overlap.overlap.len() || s.overlap.iter().any(|v| !v.is_finite())
        {
            return Err(invalid("AAC ELD checkpoint mismatch"));
        }
        self.overlap.overlap.copy_from_slice(&s.overlap);
        Ok(())
    }
    pub fn reset(&mut self) {
        self.overlap.overlap.fill(0.);
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        self.transform.visit_retained(f)?;
        self.window.visit_retained(f)?;
        f.vector(&self.complex)?;
        f.vector(&self.inverse)?;
        self.overlap.visit_retained(f)
    }
    pub fn synthesize_pcm(&mut self, spectrum: &[f32]) -> Result<Vec<f32>> {
        let n = self.inverse.len() / 2;
        if spectrum.len() != self.source_samples || spectrum.iter().any(|v| !v.is_finite()) {
            return Err(invalid("invalid AAC ELD source spectrum"));
        }
        self.transform.inverse_with_scratch(
            &spectrum[..n],
            &mut self.inverse,
            &mut self.complex,
        )?;
        // ELD's negative half-frame modulation is a shifted ordinary IMDCT.
        // Its 1/N scale with PCM /32768 equals our 2/N scale with PCM /65536.
        let mut accumulated = vec![0.; 4 * n];
        for (i, v) in accumulated.iter_mut().enumerate() {
            let (j, sign) = if i < n {
                (i + n, 1.)
            } else if i < 3 * n {
                (i - n, -1.)
            } else {
                (i - 3 * n, 1.)
            };
            *v = self.inverse[j] * sign * self.window.values()[i]
                + if i < 3 * n {
                    self.overlap.overlap[i]
                } else {
                    0.
                };
        }
        let output: Vec<f32> = accumulated[..n]
            .iter()
            .map(|v| (*v * (n as f64 / self.source_samples as f64) / 65536.) as f32)
            .collect();
        if accumulated.iter().any(|v| !v.is_finite()) || output.iter().any(|v| !v.is_finite()) {
            return Err(invalid("AAC ELD synthesis exceeds finite PCM"));
        }
        self.overlap.overlap.copy_from_slice(&accumulated[n..]);
        Ok(output)
    }
}
