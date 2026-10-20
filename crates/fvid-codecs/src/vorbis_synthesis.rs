//! Owned Vorbis window and synthesis primitives.
use crate::{Result, invalid};

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
    output.fill(0.);
    output[left_end..right_start].fill(1.);
    let slope = |x: f64| (std::f64::consts::FRAC_PI_2 * x.sin().powi(2)).sin() as f32;
    for (i, sample) in output
        .iter_mut()
        .enumerate()
        .take(left_end)
        .skip(left_start)
    {
        let phase = (i - left_start) as f64 + 0.5;
        *sample = slope(phase / (left_end - left_start) as f64 * std::f64::consts::FRAC_PI_2);
    }
    for (i, sample) in output
        .iter_mut()
        .enumerate()
        .take(right_end)
        .skip(right_start)
    {
        let phase = (right_end - i) as f64 - 0.5;
        *sample = slope(phase / (right_end - right_start) as f64 * std::f64::consts::FRAC_PI_2);
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
        if !frame.len().is_power_of_two() || !(64..=8192).contains(&frame.len()) {
            return Err(invalid("invalid Vorbis overlap block size"));
        }
        let mut next = Vec::new();
        next.try_reserve_exact(frame.len())
            .map_err(|_| invalid("cannot allocate Vorbis overlap history"))?;
        next.extend_from_slice(frame);
        let mut output = Vec::new();
        if !self.previous.is_empty() {
            let hop = (self.previous.len() + frame.len()) / 4;
            output
                .try_reserve_exact(hop)
                .map_err(|_| invalid("cannot allocate Vorbis overlap output"))?;
            for t in 0..hop {
                let previous = self
                    .previous
                    .get(self.previous.len() / 2 + t)
                    .copied()
                    .unwrap_or(0.);
                let at = frame.len() as isize / 2 + t as isize - hop as isize;
                let current = if at < 0 {
                    0.
                } else {
                    frame.get(at as usize).copied().unwrap_or(0.)
                };
                output.push(previous + current);
            }
        }
        self.previous = next;
        Ok(output)
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
