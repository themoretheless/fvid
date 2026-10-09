//! Owned AAC-SSR four-band IMDCT/window/gain/IPQF pipeline.
//! Spectrum is in decoded AAC window order: 1024 long or eight 128-line windows.
//! Even one-based PQF bands reverse spectral order before their IMDCT.
use super::{
    Result,
    aac_gain_control::GainControl,
    aac_imdct::Imdct,
    aac_ssr_gain::SsrGainOverlap,
    aac_ssr_ipqf::SsrIpqf,
    aac_synthesis::{WindowSequence, WindowShape, kbd_window},
    invalid,
};
use std::{f64::consts::PI, sync::Arc};

#[derive(Clone)]
pub struct SsrSynthesis {
    long: Imdct,
    short: Imdct,
    kbd_long: Arc<Vec<f64>>,
    kbd_short: Arc<Vec<f64>>,
    scratch: Vec<[f64; 2]>,
    previous_shape: Option<WindowShape>,
    gain: SsrGainOverlap,
    ipqf: SsrIpqf,
}
impl SsrSynthesis {
    pub(crate) fn visit_retained(
        &self,
        footprint: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.long.visit_retained(footprint)?;
        self.short.visit_retained(footprint)?;
        for window in [&self.kbd_long, &self.kbd_short] {
            if footprint.shared(window)? {
                footprint.vector(window)?;
            }
        }
        footprint.vector(&self.scratch)?;
        self.ipqf.visit_retained(footprint)
    }
    pub fn new() -> Result<Self> {
        let long = Imdct::new(256)?;
        let scratch = vec![[0.0; 2]; long.scratch_len()];
        Ok(Self {
            long,
            short: Imdct::new(32)?,
            kbd_long: Arc::new(kbd_window(256, 4.0)),
            kbd_short: Arc::new(kbd_window(32, 6.0)),
            scratch,
            previous_shape: None,
            gain: SsrGainOverlap::new(),
            ipqf: SsrIpqf::new(),
        })
    }
    pub fn reset(&mut self) {
        self.previous_shape = None;
        self.gain.reset();
        self.ipqf.reset();
    }
    pub fn output_samples(sequence: WindowSequence) -> usize {
        4 * SsrGainOverlap::output_rows(sequence)
    }
    fn weight(&self, shape: WindowShape, short: bool, i: usize) -> f64 {
        if shape == WindowShape::Kbd {
            if short {
                self.kbd_short[i]
            } else {
                self.kbd_long[i]
            }
        } else {
            let n = if short { 64 } else { 512 };
            (PI * (i as f64 + 0.5) / n as f64).sin()
        }
    }
    /// Produces the mathematical filterbank signal. AAC PCM normalization is
    /// separate. Invalid spectrum, gain syntax or output geometry is atomic.
    pub fn synthesize(
        &mut self,
        sequence: WindowSequence,
        shape: WindowShape,
        gain: &GainControl,
        spectrum: &[f32],
        output: &mut [f64],
    ) -> Result<()> {
        if spectrum.len() != 1024
            || spectrum.iter().any(|v| !v.is_finite())
            || output.len() != Self::output_samples(sequence)
        {
            return Err(invalid(
                "AAC SSR synthesis invalid spectrum or output geometry",
            ));
        }
        let previous = self.previous_shape.unwrap_or(shape);
        let mut raw = [[0.0; 512]; 4];
        let mut coefficients = [0.0; 256];
        for (band, samples) in raw.iter_mut().enumerate() {
            if sequence == WindowSequence::EightShort {
                for window in 0..8 {
                    let source =
                        &spectrum[128 * window + 32 * band..128 * window + 32 * (band + 1)];
                    for j in 0..32 {
                        coefficients[j] = source[if band % 2 == 1 { 31 - j } else { j }];
                    }
                    let block = &mut samples[64 * window..64 * (window + 1)];
                    self.short.inverse_with_scratch(
                        &coefficients[..32],
                        block,
                        &mut self.scratch[..self.short.scratch_len()],
                    )?;
                    for (i, v) in block.iter_mut().enumerate() {
                        *v *= self.weight(
                            if window == 0 && i < 32 {
                                previous
                            } else {
                                shape
                            },
                            true,
                            i,
                        );
                    }
                }
            } else {
                let source = &spectrum[256 * band..256 * (band + 1)];
                for j in 0..256 {
                    coefficients[j] = source[if band % 2 == 1 { 255 - j } else { j }];
                }
                self.long
                    .inverse_with_scratch(&coefficients, samples, &mut self.scratch)?;
                for (i, v) in samples.iter_mut().enumerate() {
                    let weight = match sequence {
                        WindowSequence::LongStart => match i {
                            0..=255 => self.weight(previous, false, i),
                            256..=367 => 1.0,
                            368..=399 => self.weight(shape, true, i - 336),
                            _ => 0.0,
                        },
                        WindowSequence::LongStop => match i {
                            0..=111 => 0.0,
                            112..=143 => self.weight(previous, true, i - 112),
                            144..=255 => 1.0,
                            _ => self.weight(shape, false, i),
                        },
                        _ => self.weight(if i < 256 { previous } else { shape }, false, i),
                    };
                    *v *= weight;
                }
            }
        }
        let mut rows = [[0.0; 4]; 368];
        let rows = &mut rows[..SsrGainOverlap::output_rows(sequence)];
        let saved = self.gain.clone();
        self.gain.synthesize(sequence, gain, &raw, rows)?;
        if let Err(error) = self.ipqf.synthesize(rows, output) {
            self.gain = saved;
            return Err(error);
        }
        self.previous_shape = Some(shape);
        Ok(())
    }
    pub fn synthesize_pcm(
        &mut self,
        sequence: WindowSequence,
        shape: WindowShape,
        gain: &GainControl,
        spectrum: &[f32],
        output: &mut [f64],
    ) -> Result<()> {
        self.synthesize(sequence, shape, gain, spectrum, output)?;
        for v in output {
            *v /= 65536.0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kbd_windows_match_normative_ssr_table_anchors() {
        let s = SsrSynthesis::new().unwrap();
        for (i, w) in [
            (0, 0.0000875914060105),
            (15, 0.6674601983218376),
            (31, 0.9999999961638728),
        ] {
            // Published decimal tables approximate the analytic I0 definition.
            assert!((s.kbd_short[i] - w).abs() < 1e-9);
        }
        for (i, w) in [
            (0, 0.0005851230124487),
            (128, 0.7110428359000029),
            (143, 0.8173544143867162),
        ] {
            assert!((s.kbd_long[i] - w).abs() < 1e-9);
        }
    }
    // Independent quadratic cosine sum. No FFT/chirp or spectrum reversal helper.
    fn direct(spectrum: &[f32], band: usize, short: bool, window: usize) -> Vec<f64> {
        let n = if short { 32 } else { 256 };
        (0..2 * n)
            .map(|i| {
                (0..n)
                    .map(|k| {
                        let source = if band % 2 == 1 { n - 1 - k } else { k };
                        let offset = if short {
                            128 * window + 32 * band
                        } else {
                            256 * band
                        };
                        f64::from(spectrum[offset + source])
                            * (PI / n as f64 * (i as f64 + 0.5 + n as f64 / 2.0) * (k as f64 + 0.5))
                                .cos()
                    })
                    .sum::<f64>()
                    * 2.0
                    / n as f64
            })
            .collect()
    }
    #[test]
    fn dense_nonzero_bands_and_all_window_transitions_match_direct_transforms() {
        let mut fast = SsrSynthesis::new().unwrap();
        let mut gain = SsrGainOverlap::new();
        let mut ipqf = SsrIpqf::new();
        let mut previous = None;
        for (sequence, shape, windows) in [
            (WindowSequence::OnlyLong, WindowShape::Kbd, 1),
            (WindowSequence::LongStart, WindowShape::Sine, 2),
            (WindowSequence::EightShort, WindowShape::Kbd, 8),
            (WindowSequence::EightShort, WindowShape::Sine, 8),
            (WindowSequence::LongStop, WindowShape::Kbd, 2),
            (WindowSequence::OnlyLong, WindowShape::Sine, 1),
        ] {
            let control = GainControl {
                bands: vec![
                    vec![
                        vec![super::super::aac_gain_control::Adjustment {
                            level: 5,
                            location: 1
                        }];
                        windows
                    ];
                    3
                ],
            };
            let spectrum: Vec<_> = (0..1024)
                .map(|i| ((i as f64 * 0.03).sin() * 31.0) as f32)
                .collect();
            let prior = previous.unwrap_or(shape);
            let long = |shape, i| {
                if shape == WindowShape::Kbd {
                    kbd_window(256, 4.0)[i]
                } else {
                    (PI * (i as f64 + 0.5) / 512.0).sin()
                }
            };
            let small = |shape, i| {
                if shape == WindowShape::Kbd {
                    kbd_window(32, 6.0)[i]
                } else {
                    (PI * (i as f64 + 0.5) / 64.0).sin()
                }
            };
            let mut raw = [[0.0; 512]; 4];
            for band in 0..4 {
                if sequence == WindowSequence::EightShort {
                    for window in 0..8 {
                        for (i, value) in direct(&spectrum, band, true, window)
                            .into_iter()
                            .enumerate()
                        {
                            raw[band][64 * window + i] =
                                value * small(if window == 0 && i < 32 { prior } else { shape }, i);
                        }
                    }
                } else {
                    for (i, value) in direct(&spectrum, band, false, 0).into_iter().enumerate() {
                        let w = match sequence {
                            WindowSequence::LongStart => {
                                if i < 256 {
                                    long(prior, i)
                                } else if i < 368 {
                                    1.0
                                } else if i < 400 {
                                    small(shape, i - 336)
                                } else {
                                    0.0
                                }
                            }
                            WindowSequence::LongStop => {
                                if i < 112 {
                                    0.0
                                } else if i < 144 {
                                    small(prior, i - 112)
                                } else if i < 256 {
                                    1.0
                                } else {
                                    long(shape, i)
                                }
                            }
                            _ => long(if i < 256 { prior } else { shape }, i),
                        };
                        raw[band][i] = value * w;
                    }
                }
            }
            let mut rows = vec![[0.0; 4]; SsrGainOverlap::output_rows(sequence)];
            gain.synthesize(sequence, &control, &raw, &mut rows)
                .unwrap();
            let mut expected = vec![0.0; 4 * rows.len()];
            ipqf.synthesize(&rows, &mut expected).unwrap();
            let mut actual = vec![0.0; expected.len()];
            fast.synthesize(sequence, shape, &control, &spectrum, &mut actual)
                .unwrap();
            for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
                assert!((a - e).abs() < 1e-11, "{sequence:?} sample {i}: {a} vs {e}");
            }
            previous = Some(shape);
        }
    }
    #[test]
    fn failure_and_reset_preserve_all_gain_filter_and_window_history() {
        let empty = GainControl { bands: vec![] };
        let mut state = SsrSynthesis::new().unwrap();
        let spectrum = vec![3.0; 1024];
        let mut pcm = vec![0.0; 1024];
        state
            .synthesize_pcm(
                WindowSequence::OnlyLong,
                WindowShape::Kbd,
                &empty,
                &spectrum,
                &mut pcm,
            )
            .unwrap();
        let mut reference = state.clone();
        pcm.fill(17.0);
        let invalid_gain = GainControl {
            bands: vec![vec![]],
        };
        assert!(
            state
                .synthesize(
                    WindowSequence::OnlyLong,
                    WindowShape::Sine,
                    &invalid_gain,
                    &spectrum,
                    &mut pcm
                )
                .is_err()
        );
        assert!(
            state
                .synthesize(
                    WindowSequence::OnlyLong,
                    WindowShape::Sine,
                    &empty,
                    &[f32::NAN; 1024],
                    &mut pcm
                )
                .is_err()
        );
        assert!(pcm.iter().all(|v| *v == 17.0));
        assert_eq!(state.previous_shape, Some(WindowShape::Kbd));
        state
            .synthesize_pcm(
                WindowSequence::OnlyLong,
                WindowShape::Sine,
                &empty,
                &spectrum,
                &mut pcm,
            )
            .unwrap();
        let mut expected = vec![0.0; 1024];
        reference
            .synthesize_pcm(
                WindowSequence::OnlyLong,
                WindowShape::Sine,
                &empty,
                &spectrum,
                &mut expected,
            )
            .unwrap();
        assert_eq!(pcm, expected);
        state.reset();
        reference = SsrSynthesis::new().unwrap();
        state
            .synthesize_pcm(
                WindowSequence::OnlyLong,
                WindowShape::Kbd,
                &empty,
                &spectrum,
                &mut pcm,
            )
            .unwrap();
        reference
            .synthesize_pcm(
                WindowSequence::OnlyLong,
                WindowShape::Kbd,
                &empty,
                &spectrum,
                &mut expected,
            )
            .unwrap();
        assert_eq!(pcm, expected);
    }
}
