//! Bounded, unfiltered HEVC plane reconstruction with decoded-sample availability.
use super::hevc_intra::References;
use crate::{Result, invalid};
pub struct Plane {
    width: usize,
    height: usize,
    depth: u8,
    samples: Vec<u16>,
    ready: Vec<bool>,
}
impl Plane {
    /// Apply single-slice, single-tile SAO after reconstruction/deblocking.
    /// Neighbours always come from the unchanged input plane. Extra workspace
    /// is two bytes per sample and is accounted for by the picture decoder.
    pub fn apply_sao(&mut self, log2_ctu: u8, parameters: &[super::hevc_sao::Sao]) -> Result<()> {
        if !(3..=6).contains(&log2_ctu) || !self.complete() {
            return Err(invalid("invalid SAO plane state"));
        }
        let side = 1usize << log2_ctu;
        let columns = self.width.div_ceil(side);
        let rows = self.height.div_ceil(side);
        if columns.checked_mul(rows) != Some(parameters.len()) {
            return Err(invalid("invalid SAO parameter grid"));
        }
        let mut output = Vec::with_capacity(self.samples.len());
        for y in 0..self.height {
            for x in 0..self.width {
                let sao = parameters[(y / side) * columns + x / side];
                let neighbours = if let Some(positions) = sao.neighbours()? {
                    let mut pair = [0; 2];
                    let mut valid = true;
                    for (i, [dx, dy]) in positions.into_iter().enumerate() {
                        let xx = x.checked_add_signed(dx as isize);
                        let yy = y.checked_add_signed(dy as isize);
                        if let (Some(xx), Some(yy)) = (xx, yy) {
                            if xx < self.width && yy < self.height {
                                pair[i] = self.samples[yy * self.width + xx];
                            } else {
                                valid = false;
                            }
                        } else {
                            valid = false;
                        }
                    }
                    valid.then_some(pair)
                } else {
                    None
                };
                output.push(sao.apply(self.samples[y * self.width + x], neighbours, self.depth)?);
            }
        }
        self.samples = output;
        Ok(())
    }
    pub fn new(width: usize, height: usize, depth: u8, budget: usize) -> Result<Self> {
        let count = width
            .checked_mul(height)
            .filter(|&n| n > 0 && n <= budget / 3)
            .ok_or_else(|| invalid("HEVC plane exceeds memory budget"))?;
        if !(8..=10).contains(&depth) {
            return Err(invalid("unsupported HEVC plane depth"));
        }
        Ok(Self {
            width,
            height,
            depth,
            samples: vec![0; count],
            ready: vec![false; count],
        })
    }
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }
    pub fn complete(&self) -> bool {
        self.ready.iter().all(|v| *v)
    }
    /// `available` applies slice/tile/constrained-intra restrictions in addition
    /// to the plane's bounds and already-reconstructed sample checks.
    pub fn reconstruct_intra(
        &mut self,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        chroma: bool,
        strong: bool,
        residual: &[i32],
        available: impl Fn(usize, usize) -> bool,
    ) -> Result<()> {
        if !(2..=5).contains(&log) {
            return Err(invalid("invalid HEVC plane block size"));
        }
        let n = 1usize << log;
        let [x, y] = origin;
        if x.checked_add(n).is_none_or(|end| end > self.width)
            || y.checked_add(n).is_none_or(|end| end > self.height)
            || x % n != 0
            || y % n != 0
            || residual.len() != n * n
        {
            return Err(invalid("invalid HEVC plane block geometry"));
        }
        for yy in y..y + n {
            if self.ready[yy * self.width + x..yy * self.width + x + n]
                .iter()
                .any(|v| *v)
            {
                return Err(invalid("HEVC block overlaps reconstructed samples"));
            }
        }
        let sample = |xx: Option<usize>, yy: Option<usize>| {
            let (xx, yy) = (xx?, yy?);
            if xx >= self.width
                || yy >= self.height
                || !self.ready[yy * self.width + xx]
                || !available(xx, yy)
            {
                None
            } else {
                Some(self.samples[yy * self.width + xx])
            }
        };
        let top: Vec<_> = (0..2 * n)
            .map(|i| sample(x.checked_add(i), y.checked_sub(1)))
            .collect();
        let left: Vec<_> = (0..2 * n)
            .map(|i| sample(x.checked_sub(1), y.checked_add(i)))
            .collect();
        let references = References::new(
            log,
            self.depth,
            sample(x.checked_sub(1), y.checked_sub(1)),
            &top,
            &left,
        )?;
        let prediction = references.predict(mode, chroma, strong)?;
        let max = (1i64 << self.depth) - 1;
        for yy in 0..n {
            for xx in 0..n {
                let i = yy * n + xx;
                let target = (y + yy) * self.width + x + xx;
                self.samples[target] =
                    (i64::from(prediction[i]) + i64::from(residual[i])).clamp(0, max) as u16;
                self.ready[target] = true;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sao_uses_original_neighbours_and_commits_only_valid_output() {
        use super::super::hevc_sao::Sao;
        let mut p = Plane::new(4, 4, 8, 48).unwrap();
        p.samples = [100, 101, 100, 99].repeat(4);
        p.ready.fill(true);
        p.apply_sao(
            3,
            &[Sao::Edge {
                class: 0,
                offsets: [7, 3, -3, -7],
            }],
        )
        .unwrap();
        assert_eq!(p.samples, [100, 94, 100, 99].repeat(4));
        let saved = p.samples.clone();
        assert!(
            p.apply_sao(
                3,
                &[Sao::Band {
                    position: 32,
                    offsets: [0; 4]
                }]
            )
            .is_err()
        );
        assert_eq!(p.samples, saved);
        assert!(p.apply_sao(3, &[]).is_err());
    }
    #[test]
    fn published_blocks_feed_prediction_but_future_samples_do_not() {
        let mut p = Plane::new(8, 4, 8, 96).unwrap();
        p.reconstruct_intra([0, 0], 2, 1, false, false, &[12; 16], |_, _| true)
            .unwrap();
        p.reconstruct_intra([4, 0], 2, 10, false, false, &[0; 16], |_, _| true)
            .unwrap();
        assert!(p.complete());
        assert_eq!(p.samples(), [140; 32]);
        let saved = p.samples().to_vec();
        assert!(
            p.reconstruct_intra([0, 0], 2, 1, false, false, &[0; 16], |_, _| true)
                .is_err()
        );
        assert_eq!(p.samples(), saved);
        assert!(Plane::new(8, 4, 8, 95).is_err());
    }
    #[test]
    fn unavailable_boundaries_and_extreme_residuals_are_safe() {
        let mut p = Plane::new(8, 4, 10, 96).unwrap();
        p.reconstruct_intra([0, 0], 2, 1, true, false, &[i32::MAX; 16], |_, _| true)
            .unwrap();
        p.reconstruct_intra([4, 0], 2, 10, true, false, &[0; 16], |_, _| false)
            .unwrap();
        for row in p.samples().chunks_exact(8) {
            assert_eq!(&row[..4], &[1023; 4]);
            assert_eq!(&row[4..], &[512; 4]);
        }
    }
}
