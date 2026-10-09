//! Owned ordinary LTP analysis filterbank; decoder admission remains separate.
use super::{
    Result,
    aac_imdct::Imdct,
    aac_synthesis::{WindowSequence, WindowShape, kbd_window},
    invalid, unsupported,
};
use std::f64::consts::PI;
pub struct LtpAnalysis {
    n: usize,
    long: [Vec<f64>; 2],
    short: [Vec<f64>; 2],
    transform: Imdct,
    windowed: Vec<f64>,
    scratch: Vec<[f64; 2]>,
}
impl LtpAnalysis {
    pub fn new(n: usize) -> Result<Self> {
        if !matches!(n, 960 | 1024) {
            return Err(invalid("invalid AAC LTP analysis geometry"));
        }
        let sine = |size: usize| {
            (0..2 * size)
                .map(|i| (PI * (i as f64 + 0.5) / (2 * size) as f64).sin())
                .collect()
        };
        let transform = Imdct::new(n)?;
        let scratch = vec![[0.0; 2]; transform.scratch_len()];
        Ok(Self {
            n,
            long: [sine(n), kbd_window(n, 4.0)],
            short: [sine(n / 8), kbd_window(n / 8, 6.0)],
            transform,
            windowed: vec![0.0; 2 * n],
            scratch,
        })
    }
    /// No state advancement or per-call allocation. Raw cosine-sum normalization.
    pub fn analyze(
        &mut self,
        input: &[f64],
        sequence: WindowSequence,
        previous: WindowShape,
        current: WindowShape,
        output: &mut [f64],
    ) -> Result<()> {
        let n = self.n;
        if input.len() != 2 * n || output.len() != n || input.iter().any(|x| !x.is_finite()) {
            return Err(invalid("invalid AAC LTP analysis samples"));
        }
        if sequence == WindowSequence::EightShort {
            return Err(unsupported(
                "AAC short-window LTP analysis is not integrated",
            ));
        }
        let shape = |s| usize::from(s == WindowShape::Kbd);
        let (before, after) = (shape(previous), shape(current));
        let small = n / 8;
        let flat = (n - small) / 2;
        for (i, value) in input.iter().enumerate() {
            let weight = match sequence {
                WindowSequence::LongStart if i >= n => {
                    let j = i - n;
                    if j < flat {
                        1.0
                    } else if j < flat + small {
                        self.short[after][small + j - flat]
                    } else {
                        0.0
                    }
                }
                WindowSequence::LongStop if i < n => {
                    if i < flat {
                        0.0
                    } else if i < flat + small {
                        self.short[before][i - flat]
                    } else {
                        1.0
                    }
                }
                _ => {
                    if i < n {
                        self.long[before][i]
                    } else {
                        self.long[after][i]
                    }
                }
            };
            self.windowed[i] = value * weight;
        }
        self.transform
            .forward_with_scratch(&self.windowed, output, &mut self.scratch)
    }
}
