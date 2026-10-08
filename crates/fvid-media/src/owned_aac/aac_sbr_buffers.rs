//! Owned non-scalable SBR frame delay and previous/current synthesis routing.
//! ISO SBR overview: tHFGen=8, tHFAdj=2, RATE=2. Rows passed to
//! HF generation start at XLow(0); adjusted HF rows start at tHFAdj.
use super::{Result, aac_sbr_grid::TimeGrid, aac_sbr_qmf::Complex, invalid};

const ZERO32: [Complex; 32] = [Complex { re: 0.0, im: 0.0 }; 32];
const ZERO64: [Complex; 64] = [Complex { re: 0.0, im: 0.0 }; 64];
fn finite<const N: usize>(rows: &[[Complex; N]]) -> bool {
    rows.iter()
        .flatten()
        .all(|x| x.re.is_finite() && x.im.is_finite())
}
fn frame_len(slots: u8) -> Result<usize> {
    if !matches!(slots, 15 | 16) {
        return Err(invalid("invalid SBR buffer frame size"));
    }
    Ok(2 * usize::from(slots))
}
/// Eight raw analysis columns are retained. Masking happens when constructing
/// XLow, using the previous crossover for history and current crossover for W.
/// A geometry/header reset must not discard the previous frame's low tail.
#[derive(Clone, Debug, PartialEq)]
pub struct LowDelay {
    tail: [[Complex; 32]; 8],
    previous_kx: u8,
}
impl Default for LowDelay {
    fn default() -> Self {
        Self {
            tail: [ZERO32; 8],
            previous_kx: 0,
        }
    }
}
impl LowDelay {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn process(
        &mut self,
        analysis: &[[Complex; 32]],
        slots: u8,
        kx: u8,
    ) -> Result<Vec<[Complex; 32]>> {
        let n = frame_len(slots)?;
        if analysis.len() != n || !(1..=32).contains(&kx) || !finite(analysis) {
            return Err(invalid("invalid SBR low delay inputs"));
        }
        let mut low = Vec::with_capacity(n + 8);
        for (rows, cutoff) in [(&self.tail[..], self.previous_kx), (analysis, kx)] {
            for row in rows {
                let mut masked = ZERO32;
                masked[..usize::from(cutoff)].copy_from_slice(&row[..usize::from(cutoff)]);
                low.push(masked);
            }
        }
        self.tail.copy_from_slice(&analysis[n - 8..]);
        self.previous_kx = kx;
        Ok(low)
    }
}
/// Retains adjusted HF beyond the nominal frame. Synthesis selects the old
/// crossover/end for the previous overhang, even when the new header changes
/// geometry. `adjusted` uses the Assembly output origin (tHFAdj).
#[derive(Clone, Debug, PartialEq)]
pub struct SynthesisRows {
    tail: [[Complex; 64]; 6],
    previous_kx: u8,
    previous_end: u8,
    overhang: usize,
}
impl Default for SynthesisRows {
    fn default() -> Self {
        Self {
            tail: [ZERO64; 6],
            previous_kx: 0,
            previous_end: 0,
            overhang: 0,
        }
    }
}
impl SynthesisRows {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn process(
        &mut self,
        low: &[[Complex; 32]],
        adjusted: &[[Complex; 64]],
        slots: u8,
        grid: &TimeGrid,
        kx: u8,
        end: u8,
    ) -> Result<Vec<[Complex; 64]>> {
        let n = frame_len(slots)?;
        if low.len() != n + 8
            || adjusted.len() != n + 6
            || !(1..=32).contains(&kx)
            || end <= kx
            || end > 64
            || grid.envelope.len() < 2
            || grid.envelope.len() > 6
            || grid.envelope[0] > 3
            || grid.envelope.windows(2).any(|x| x[0] >= x[1])
            || usize::from(*grid.envelope.last().unwrap()) < usize::from(slots)
            || usize::from(*grid.envelope.last().unwrap()) > usize::from(slots) + 3
            || !finite(low)
            || !finite(adjusted)
        {
            return Err(invalid("invalid SBR synthesis row inputs"));
        }
        let mut result = vec![ZERO64; n];
        for l in 0..n {
            let (cutoff, stop, high) = if l < self.overhang {
                (
                    usize::from(self.previous_kx),
                    usize::from(self.previous_end),
                    &self.tail[l],
                )
            } else {
                (usize::from(kx), usize::from(end), &adjusted[l])
            };
            result[l][..cutoff].copy_from_slice(&low[l + 2][..cutoff]);
            result[l][cutoff..stop].copy_from_slice(&high[cutoff..stop]);
        }
        self.tail.copy_from_slice(&adjusted[n..]);
        self.previous_kx = kx;
        self.previous_end = end;
        self.overhang = 2 * usize::from(*grid.envelope.last().unwrap()) - n;
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tagged<const N: usize>(frame: usize, n: usize) -> Vec<[Complex; N]> {
        (0..n)
            .map(|l| {
                std::array::from_fn(|k| Complex {
                    re: (frame * 10000 + l * 100 + k + 1) as f64,
                    im: -((frame * 10000 + l * 100 + k + 1) as f64),
                })
            })
            .collect()
    }
    fn grid(slots: u8, excess: u8) -> TimeGrid {
        TimeGrid {
            envelope: vec![0, slots + excess],
            noise: vec![0, slots + excess],
        }
    }
    #[test]
    fn low_delay_uses_previous_crossover_and_exact_eight_column_history() {
        for slots in [15, 16] {
            for first in [1, 7, 16, 32] {
                for second in [1, 7, 16, 32] {
                    let n = 2 * usize::from(slots);
                    let a = tagged::<32>(1, n);
                    let b = tagged::<32>(2, n);
                    let mut state = LowDelay::default();
                    let x = state.process(&a, slots, first).unwrap();
                    assert_eq!(&x[..8], &[ZERO32; 8]);
                    let checkpoint = state.clone();
                    let y = state.process(&b, slots, second).unwrap();
                    assert_eq!(y, checkpoint.clone().process(&b, slots, second).unwrap());
                    for l in 0..n + 8 {
                        for k in 0..32 {
                            let expected = if l < 8 {
                                if k < usize::from(first) {
                                    a[n - 8 + l][k]
                                } else {
                                    Complex::default()
                                }
                            } else if k < usize::from(second) {
                                b[l - 8][k]
                            } else {
                                Complex::default()
                            };
                            assert_eq!(y[l][k], expected);
                        }
                    }
                    state.reset();
                    assert_eq!(state.process(&a, slots, first).unwrap(), x);
                }
            }
        }
    }
    #[test]
    fn synthesis_routes_previous_overhang_with_old_geometry_and_two_column_offset() {
        for slots in [15, 16] {
            for excess in 0..=3 {
                let n = 2 * usize::from(slots);
                let low = tagged::<32>(1, n + 8);
                let a = tagged::<64>(2, n + 6);
                let b = tagged::<64>(3, n + 6);
                for (old_kx, old_end, kx, end) in [(7, 23, 16, 64), (32, 64, 1, 9), (1, 9, 32, 64)]
                {
                    let mut state = SynthesisRows::default();
                    let first = state
                        .process(&low, &a, slots, &grid(slots, excess), old_kx, old_end)
                        .unwrap();
                    let checkpoint = state.clone();
                    let output = state
                        .process(&low, &b, slots, &grid(slots, 0), kx, end)
                        .unwrap();
                    assert_eq!(
                        output,
                        checkpoint
                            .clone()
                            .process(&low, &b, slots, &grid(slots, 0), kx, end)
                            .unwrap()
                    );
                    for l in 0..n {
                        for k in 0..64 {
                            let previous = l < 2 * usize::from(excess);
                            let cut = usize::from(if previous { old_kx } else { kx });
                            let stop = usize::from(if previous { old_end } else { end });
                            let expected = if k < cut {
                                low[l + 2][k]
                            } else if k < stop {
                                if previous { a[n + l][k] } else { b[l][k] }
                            } else {
                                Complex::default()
                            };
                            assert_eq!(output[l][k], expected);
                        }
                    }
                    state.reset();
                    assert_eq!(
                        state
                            .process(&low, &a, slots, &grid(slots, excess), old_kx, old_end)
                            .unwrap(),
                        first
                    );
                }
            }
        }
    }
    #[test]
    fn composed_delay_and_routing_keep_six_column_latency_across_geometry_changes() {
        for slots in [15, 16] {
            let n = 2 * usize::from(slots);
            let mut low_state = LowDelay::default();
            let mut output_state = SynthesisRows::default();
            let mut prior_raw = vec![ZERO32; n];
            let mut prior_high = vec![ZERO64; n + 6];
            let mut prior_kx = 0;
            let mut prior_end = 0;
            let mut prior_excess = 0;
            for (frame, (kx, end, excess)) in [(7, 20, 3), (16, 40, 1), (32, 64, 0), (1, 9, 2)]
                .into_iter()
                .enumerate()
            {
                let raw = tagged::<32>(frame + 1, n);
                let high = tagged::<64>(frame + 10, n + 6);
                let low = low_state.process(&raw, slots, kx).unwrap();
                let output = output_state
                    .process(&low, &high, slots, &grid(slots, excess), kx, end)
                    .unwrap();
                for l in 0..n {
                    for k in 0..64 {
                        let previous = l < 2 * usize::from(prior_excess);
                        let cut = usize::from(if previous { prior_kx } else { kx });
                        let stop = usize::from(if previous { prior_end } else { end });
                        let expected = if k < cut {
                            // tHFGen-tHFAdj = six columns. The retained portion
                            // is masked by its own frame's crossover, not kx.
                            if l < 6 {
                                if k < usize::from(prior_kx) {
                                    prior_raw[n - 6 + l][k]
                                } else {
                                    Complex::default()
                                }
                            } else {
                                raw[l - 6][k]
                            }
                        } else if k < stop {
                            if previous {
                                prior_high[n + l][k]
                            } else {
                                high[l][k]
                            }
                        } else {
                            Complex::default()
                        };
                        assert_eq!(output[l][k], expected, "frame={frame} l={l} k={k}");
                    }
                }
                prior_raw = raw;
                prior_high = high;
                prior_kx = kx;
                prior_end = end;
                prior_excess = excess;
            }
        }
    }
    #[test]
    fn late_invalid_inputs_preserve_both_histories() {
        let mut low_state = LowDelay::default();
        let mut output_state = SynthesisRows::default();
        let a = tagged::<32>(1, 32);
        let low = low_state.process(&a, 16, 12).unwrap();
        let high = tagged::<64>(2, 38);
        output_state
            .process(&low, &high, 16, &grid(16, 3), 12, 40)
            .unwrap();
        let old_low = low_state.clone();
        let old_output = output_state.clone();
        let mut broken = a.clone();
        broken[31][31].im = f64::NAN;
        assert!(low_state.process(&broken, 16, 12).is_err());
        assert_eq!(low_state, old_low);
        let mut broken = high.clone();
        broken[37][63].re = f64::INFINITY;
        assert!(
            output_state
                .process(&low, &broken, 16, &grid(16, 3), 12, 40)
                .is_err()
        );
        assert_eq!(output_state, old_output);
        for slots in [0, 14, 17, 255] {
            assert!(low_state.process(&a, slots, 12).is_err());
        }
        for excess in [4, 255] {
            let bad = TimeGrid {
                envelope: vec![0, excess],
                noise: vec![0, excess],
            };
            assert!(output_state.process(&low, &high, 16, &bad, 12, 40).is_err());
        }
        assert_eq!(output_state, old_output);
    }
}
