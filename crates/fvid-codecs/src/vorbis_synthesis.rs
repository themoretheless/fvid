//! Owned Vorbis window and synthesis primitives.
use crate::{Result, invalid};
use std::sync::OnceLock;

fn window_slope(span: usize) -> &'static [f32] {
    static SLOPES: OnceLock<[Vec<f32>; 8]> = OnceLock::new();
    let idx = (span.trailing_zeros() as usize) - 5;
    &SLOPES.get_or_init(|| {
        std::array::from_fn(|k| {
            let len = 32usize << k;
            (0..len)
                .map(|i| {
                    let x = (i as f64 + 0.5) / len as f64 * std::f64::consts::FRAC_PI_2;
                    (std::f64::consts::FRAC_PI_2 * x.sin().powi(2)).sin() as f32
                })
                .collect()
        })
    })[idx]
}

/// Fill the Vorbis I window, including hybrid short/long transitions.
/// All geometry is checked before changing caller-owned output.
pub fn window(
    short: usize,
    long: bool,
    previous_long: bool,
    next_long: bool,
    output: &mut [f32],
) -> Result<()> {
    let n = output.len();
    if !short.is_power_of_two()
        || !(64..=8192).contains(&short)
        || !n.is_power_of_two()
        || !(64..=8192).contains(&n)
        || n < short
        || (!long && n != short)
    {
        return Err(invalid("invalid Vorbis window geometry"));
    }
    let (left_start, left_end) = if long && !previous_long {
        (n / 4 - short / 4, n / 4 + short / 4)
    } else {
        (0, n / 2)
    };
    let (right_start, right_end) = if long && !next_long {
        (3 * n / 4 - short / 4, 3 * n / 4 + short / 4)
    } else {
        (n / 2, n)
    };
    output[..left_start].fill(0.);
    output[left_end..right_start].fill(1.);
    output[right_end..].fill(0.);
    let left_ramp = window_slope(left_end - left_start);
    output[left_start..left_end].copy_from_slice(left_ramp);
    let right_ramp = window_slope(right_end - right_start);
    for (dst, &src) in output[right_start..right_end]
        .iter_mut()
        .zip(right_ramp.iter().rev())
    {
        *dst = src;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_are_complementary_across_every_legal_block_size_pair() {
        for short_exp in 6..=13 {
            for long_exp in short_exp..=13 {
                let short = 1usize << short_exp;
                let large = 1usize << long_exp;
                for previous in [short, large] {
                    for current in [short, large] {
                        let mut a = vec![0.; previous];
                        let mut b = vec![0.; current];
                        window(short, previous == large, true, current == large, &mut a).unwrap();
                        window(short, current == large, previous == large, true, &mut b).unwrap();
                        let hop = (previous + current) / 4;
                        for t in 0..hop {
                            let va = a.get(previous / 2 + t).copied().unwrap_or(0.);
                            let index = current as isize / 2 + t as isize - hop as isize;
                            let vb = if index >= 0 {
                                b.get(index as usize).copied().unwrap_or(0.)
                            } else {
                                0.
                            };
                            assert!(
                                (va * va + vb * vb - 1.).abs() < 0.0000003,
                                "short={short} previous={previous} current={current} t={t}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn transition_windows_have_correct_zero_plateau_and_symmetry() {
        let mut output = vec![0.; 1024];
        window(128, true, false, false, &mut output).unwrap();
        assert!(output[..224].iter().all(|&v| v == 0.));
        assert!(output[288..736].iter().all(|&v| v == 1.));
        assert!(output[800..].iter().all(|&v| v == 0.));
        for (i, &value) in output.iter().enumerate() {
            assert_eq!(value, output[1023 - i]);
            assert!((0. ..=1.).contains(&value));
        }
        let mut invalid = [42.; 100];
        assert!(window(128, true, true, true, &mut invalid).is_err());
        assert_eq!(invalid, [42.; 100]);
    }
}

/// One channel's windowed time-domain overlap state. The first frame primes it.
#[derive(Default)]
pub struct Overlap {
    previous: Vec<f32>,
}
impl Overlap {
    pub fn reset(&mut self) {
        self.previous.clear();
    }
    pub fn push(&mut self, frame: &[f32]) -> Result<Vec<f32>> {
        let mut output = Vec::new();
        self.push_into(frame, &mut output)?;
        Ok(output)
    }

    pub fn push_into(&mut self, frame: &[f32], output: &mut Vec<f32>) -> Result<()> {
        if !frame.len().is_power_of_two() || !(64..=8192).contains(&frame.len()) {
            return Err(invalid("invalid Vorbis overlap block size"));
        }
        output.clear();
        if !self.previous.is_empty() {
            let hop = (self.previous.len() + frame.len()) / 4;
            if output.capacity() < hop {
                output
                    .try_reserve_exact(hop - output.len())
                    .map_err(|_| invalid("cannot allocate Vorbis overlap output"))?;
            }
            let prev_half = self.previous.len() / 2;
            let frame_half = frame.len() as isize / 2;
            for t in 0..hop {
                let previous = self.previous.get(prev_half + t).copied().unwrap_or(0.);
                let at = frame_half + t as isize - hop as isize;
                let current = if at < 0 {
                    0.
                } else {
                    frame.get(at as usize).copied().unwrap_or(0.)
                };
                output.push(previous + current);
            }
        }
        if self.previous.capacity() < frame.len() {
            self.previous
                .try_reserve_exact(frame.len() - self.previous.len())
                .map_err(|_| invalid("cannot allocate Vorbis overlap history"))?;
        }
        self.previous.clear();
        self.previous.extend_from_slice(frame);
        Ok(())
    }
}
#[cfg(test)]
mod overlap_tests {
    use super::*;
    #[test]
    fn overlap_primes_and_returns_the_correct_short_long_spans() {
        for (previous, current) in [(64, 64), (64, 256), (256, 64), (256, 256)] {
            let mut overlap = Overlap::default();
            assert!(overlap.push(&vec![1.; previous]).unwrap().is_empty());
            let output = overlap.push(&vec![2.; current]).unwrap();
            assert_eq!(output.len(), (previous + current) / 4);
            let expected = match (previous, current) {
                (64, 256) => [vec![3.; 32], vec![2.; 48]].concat(),
                (256, 64) => [vec![1.; 48], vec![3.; 32]].concat(),
                _ => vec![3.; output.len()],
            };
            assert_eq!(output, expected);
            overlap.reset();
            assert!(overlap.push(&vec![2.; current]).unwrap().is_empty());
        }
    }
    #[test]
    fn invalid_overlap_frame_does_not_replace_history() {
        let mut overlap = Overlap::default();
        overlap.push(&[1.; 64]).unwrap();
        assert!(overlap.push(&[99.; 63]).is_err());
        assert_eq!(overlap.push(&[2.; 64]).unwrap(), vec![3.; 32]);
    }
}
