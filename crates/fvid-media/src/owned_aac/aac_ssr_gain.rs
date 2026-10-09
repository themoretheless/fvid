//! AAC-SSR gain compensation and overlap, ISO/IEC 14496-3:2009 4.6.12.3.1–3.
//! Input is four windowed IMDCT bands, not entropy-coded AAC packets.
use super::{
    Result,
    aac_gain_control::{Adjustment, GainControl},
    aac_synthesis::WindowSequence,
    invalid,
};

fn fragment(adjustments: &[Adjustment], length: usize) -> Result<(f64, [f64; 256])> {
    if adjustments.len() > 7 {
        return Err(invalid("AAC SSR too many gain adjustments"));
    }
    let mut previous = None;
    for adjustment in adjustments {
        let location = usize::from(adjustment.location) * 8;
        if adjustment.level > 15 || location >= length || previous.is_some_and(|v| location <= v) {
            return Err(invalid("AAC SSR invalid gain adjustment location or level"));
        }
        previous = Some(location);
    }
    let initial = adjustments.first().map_or(0, |a| i32::from(a.level) - 4);
    let mut values = [1.0; 256];
    // Segment at zero carries the first transmitted level; the terminal point
    // at length is unity. Interpolation spans eight samples in log2 space.
    let mut segment = 0;
    for (j, value) in values[..length].iter_mut().enumerate() {
        while segment < adjustments.len() && usize::from(adjustments[segment].location) * 8 <= j {
            segment += 1;
        }
        let (location, a) = if segment == 0 {
            (0, initial)
        } else {
            let q = &adjustments[segment - 1];
            (usize::from(q.location) * 8, i32::from(q.level) - 4)
        };
        let b = adjustments
            .get(segment)
            .map_or(0, |q| i32::from(q.level) - 4);
        let offset = (j - location).min(8) as f64;
        *value = 2.0f64.powf((f64::from(a) * (8.0 - offset) + f64::from(b) * offset) / 8.0);
    }
    Ok((2.0f64.powi(initial), values))
}

#[derive(Clone)]
pub struct SsrGainOverlap {
    previous_fragment: [[f64; 256]; 3],
    overlap: [[f64; 256]; 4],
}
impl Default for SsrGainOverlap {
    fn default() -> Self {
        Self::new()
    }
}
impl SsrGainOverlap {
    pub fn new() -> Self {
        Self {
            previous_fragment: [[1.0; 256]; 3],
            overlap: [[0.0; 256]; 4],
        }
    }
    pub fn reset(&mut self) {
        self.previous_fragment.fill([1.0; 256]);
        self.overlap.fill([0.0; 256]);
    }
    pub fn output_rows(sequence: WindowSequence) -> usize {
        match sequence {
            WindowSequence::LongStart => 368,
            WindowSequence::LongStop => 144,
            _ => 256,
        }
    }
    /// Short input contains eight consecutive 64-sample IMDCT windows per band.
    /// Output rows are simultaneous quarter-rate band samples for the IPQF.
    /// Windowing precedes this operation. Validation is atomic; scratch is bounded.
    pub fn synthesize(
        &mut self,
        sequence: WindowSequence,
        gain: &GainControl,
        input: &[[f64; 512]; 4],
        output: &mut [[f64; 4]],
    ) -> Result<()> {
        if output.len() != Self::output_rows(sequence)
            || gain.bands.len() > 3
            || input.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(invalid("AAC SSR gain overlap invalid geometry or input"));
        }
        let lengths: &[usize] = match sequence {
            WindowSequence::OnlyLong => &[256],
            WindowSequence::LongStart => &[112, 32],
            WindowSequence::EightShort => &[32; 8],
            WindowSequence::LongStop => &[112, 256],
        };
        let mut compensated = *input;
        let mut next_fragment = self.previous_fragment;
        for band in 1..4 {
            let rows = gain.bands.get(band - 1);
            if rows.is_some_and(|r| r.len() != lengths.len()) {
                return Err(invalid("AAC SSR gain window count mismatch"));
            }
            let mut curves = [[1.0; 256]; 8];
            let mut initial = [1.0; 8];
            for (window, &length) in lengths.iter().enumerate() {
                (initial[window], curves[window]) =
                    fragment(rows.map_or(&[], |r| r[window].as_slice()), length)?;
            }
            for (j, sample) in compensated[band].iter_mut().enumerate() {
                let modification = match sequence {
                    WindowSequence::OnlyLong => {
                        if j < 256 {
                            initial[0] * self.previous_fragment[band - 1][j]
                        } else {
                            curves[0][j - 256]
                        }
                    }
                    WindowSequence::LongStart => match j {
                        0..=255 => initial[0] * initial[1] * self.previous_fragment[band - 1][j],
                        256..=367 => initial[1] * curves[0][j - 256],
                        368..=399 => curves[1][j - 368],
                        _ => 1.0,
                    },
                    WindowSequence::EightShort => {
                        let window = j / 64;
                        let position = j % 64;
                        if position < 32 {
                            initial[window]
                                * if window == 0 {
                                    self.previous_fragment[band - 1][position]
                                } else {
                                    curves[window - 1][position]
                                }
                        } else {
                            curves[window][position - 32]
                        }
                    }
                    WindowSequence::LongStop => match j {
                        0..=111 => 1.0,
                        112..=143 => {
                            initial[0] * initial[1] * self.previous_fragment[band - 1][j - 112]
                        }
                        144..=255 => initial[1] * curves[0][j - 144],
                        _ => curves[1][j - 256],
                    },
                };
                *sample /= modification;
            }
            next_fragment[band - 1] = curves[lengths.len() - 1];
        }
        let mut next_overlap = [[0.0; 256]; 4];
        for band in 0..4 {
            let current = &compensated[band];
            match sequence {
                WindowSequence::OnlyLong | WindowSequence::LongStart => {
                    for j in 0..256 {
                        output[j][band] = self.overlap[band][j] + current[j];
                    }
                    if sequence == WindowSequence::OnlyLong {
                        next_overlap[band].copy_from_slice(&current[256..]);
                    } else {
                        for j in 0..112 {
                            output[256 + j][band] = current[256 + j];
                        }
                        next_overlap[band][..32].copy_from_slice(&current[368..400]);
                    }
                }
                WindowSequence::EightShort => {
                    for window in 0..8 {
                        for j in 0..32 {
                            output[32 * window + j][band] = current[64 * window + j]
                                + if window == 0 {
                                    self.overlap[band][j]
                                } else {
                                    current[64 * (window - 1) + 32 + j]
                                };
                        }
                    }
                    next_overlap[band][..32].copy_from_slice(&current[480..512]);
                }
                WindowSequence::LongStop => {
                    for j in 0..32 {
                        output[j][band] = self.overlap[band][j] + current[112 + j];
                    }
                    for j in 0..112 {
                        output[32 + j][band] = current[144 + j];
                    }
                    next_overlap[band].copy_from_slice(&current[256..]);
                }
            }
        }
        self.previous_fragment = next_fragment;
        self.overlap = next_overlap;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_gain_survives_long_start_short_stop_and_long_history() {
        let mut state = SsrGainOverlap::new();
        let input = [[1.0; 512]; 4];
        for (sequence, windows, first, boundary) in [
            (WindowSequence::LongStart, 2, 0.25, (256, 0.25)),
            (WindowSequence::EightShort, 8, 0.75, (32, 0.75)),
            (WindowSequence::LongStop, 2, 0.625, (32, 0.25)),
            (WindowSequence::OnlyLong, 1, 0.75, (8, 1.5)),
        ] {
            let gain = GainControl {
                bands: vec![
                    vec![
                        vec![Adjustment {
                            level: 5,
                            location: 0
                        }];
                        windows
                    ];
                    3
                ],
            };
            let mut output = vec![[0.0; 4]; SsrGainOverlap::output_rows(sequence)];
            state
                .synthesize(sequence, &gain, &input, &mut output)
                .unwrap();
            for band in 1..4 {
                assert_eq!(output[0][band], first, "{sequence:?}");
                assert_eq!(output[boundary.0][band], boundary.1, "{sequence:?}");
            }
        }
    }
    #[test]
    fn active_levels_interpolate_geometrically_with_strict_locations() {
        let (initial, curve) = fragment(
            &[
                Adjustment {
                    level: 6,
                    location: 0,
                },
                Adjustment {
                    level: 4,
                    location: 2,
                },
            ],
            32,
        )
        .unwrap();
        assert_eq!(initial, 4.0);
        for j in 0..8 {
            assert!((curve[j] - 2.0f64.powf(2.0 - (j as f64) / 4.0)).abs() < 1e-14);
        }
        assert!(curve[8..32].iter().all(|v| *v == 1.0));
        for bad in [
            vec![Adjustment {
                level: 4,
                location: 14,
            }],
            vec![Adjustment {
                level: 16,
                location: 0,
            }],
            vec![
                Adjustment {
                    level: 4,
                    location: 1
                };
                2
            ],
        ] {
            assert!(fragment(&bad, 112).is_err());
        }
    }
    #[test]
    fn all_window_transitions_emit_correct_regions_and_remember_active_gain() {
        let mut state = SsrGainOverlap::new();
        let input = std::array::from_fn(|b| std::array::from_fn(|j| (1000 * b + j + 1) as f64));
        // A level of +1 at zero falls to unity over the first eight samples.
        let active = GainControl {
            bands: vec![vec![vec![Adjustment {
                level: 5,
                location: 0,
            }]]],
        };
        let mut long = vec![[0.0; 4]; 256];
        state
            .synthesize(WindowSequence::OnlyLong, &active, &input, &mut long)
            .unwrap();
        assert_eq!(long[0][0], 1.0);
        assert_eq!(long[0][1], 1001.0 / 2.0);
        let empty = GainControl { bands: vec![] };
        state
            .synthesize(
                WindowSequence::LongStart,
                &empty,
                &input,
                &mut vec![[0.0; 4]; 368],
            )
            .unwrap();
        let mut short = vec![[0.0; 4]; 256];
        state
            .synthesize(WindowSequence::EightShort, &empty, &input, &mut short)
            .unwrap();
        assert_eq!(short[0][0], 369.0 + 1.0);
        assert_eq!(short[32][0], 33.0 + 65.0);
        let mut stop = vec![[0.0; 4]; 144];
        state
            .synthesize(WindowSequence::LongStop, &empty, &input, &mut stop)
            .unwrap();
        assert_eq!(stop[0][0], 481.0 + 113.0);
        assert_eq!(stop[32][0], 145.0);
        state
            .synthesize(WindowSequence::OnlyLong, &empty, &input, &mut long)
            .unwrap();
        assert_eq!(long[0][0], 257.0 + 1.0);
    }
    #[test]
    fn malformed_gain_preserves_history_and_output_and_reset_discards_it() {
        let mut state = SsrGainOverlap::new();
        let empty = GainControl { bands: vec![] };
        let input = [[1.0; 512]; 4];
        state
            .synthesize(
                WindowSequence::OnlyLong,
                &empty,
                &input,
                &mut [[0.0; 4]; 256],
            )
            .unwrap();
        let mut reference = state.clone();
        let mut output = [[17.0; 4]; 256];
        let bad = GainControl {
            bands: vec![
                vec![vec![Adjustment {
                    level: 5,
                    location: 0,
                }]],
                vec![vec![Adjustment {
                    level: 4,
                    location: 32,
                }]],
            ],
        };
        assert!(
            state
                .synthesize(WindowSequence::OnlyLong, &bad, &input, &mut output)
                .is_err()
        );
        assert_eq!(output, [[17.0; 4]; 256]);
        state
            .synthesize(WindowSequence::OnlyLong, &empty, &input, &mut output)
            .unwrap();
        let mut expected = [[0.0; 4]; 256];
        reference
            .synthesize(WindowSequence::OnlyLong, &empty, &input, &mut expected)
            .unwrap();
        assert_eq!(output, expected);
        state.reset();
        state
            .synthesize(WindowSequence::OnlyLong, &empty, &input, &mut output)
            .unwrap();
        assert_eq!(output, [[1.0; 4]; 256]);
    }
}
