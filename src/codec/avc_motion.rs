//! H.264 fractional-sample motion compensation and weighted prediction (8.4.2.2–3).
//! Reference planes contain reconstructed, deblocked samples before display cropping.
use crate::{Result, invalid};
const TAPS: [i64; 6] = [1, -5, 20, 20, -5, 1];

pub struct ReferencePlane<'a> {
    samples: &'a [u16],
    width: usize,
    height: usize,
    stride: usize,
    depth: u8,
}
impl<'a> ReferencePlane<'a> {
    pub fn new(
        samples: &'a [u16],
        width: usize,
        height: usize,
        stride: usize,
        depth: u8,
    ) -> Result<Self> {
        let max = maximum(depth)?;
        if width == 0
            || height == 0
            || stride < width
            || width > i32::MAX as usize
            || height > i32::MAX as usize
        {
            return Err(invalid("invalid reference plane geometry"));
        }
        let needed = (height - 1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(width))
            .ok_or_else(|| invalid("reference plane size overflow"))?;
        if samples.len() < needed {
            return Err(invalid("truncated reference plane"));
        }
        if (0..height).any(|y| {
            samples[y * stride..y * stride + width]
                .iter()
                .any(|&p| i64::from(p) > max)
        }) {
            return Err(invalid("reference sample exceeds bit depth"));
        }
        Ok(Self {
            samples,
            width,
            height,
            stride,
            depth,
        })
    }
    fn at(&self, x: i64, y: i64) -> i64 {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        i64::from(self.samples[y * self.stride + x])
    }
    fn clip(&self, v: i64) -> u16 {
        v.clamp(0, (1i64 << self.depth) - 1) as u16
    }
    fn horizontal(&self, x: i64, y: i64) -> i64 {
        TAPS.iter()
            .enumerate()
            .map(|(i, &tap)| tap * self.at(x + i as i64 - 2, y))
            .sum()
    }
    fn vertical(&self, x: i64, y: i64) -> i64 {
        TAPS.iter()
            .enumerate()
            .map(|(i, &tap)| tap * self.at(x, y + i as i64 - 2))
            .sum()
    }
    fn quarter(&self, x: i64, y: i64, fx: i32, fy: i32) -> u16 {
        let b = || self.clip((self.horizontal(x, y) + 16) >> 5);
        let h = || self.clip((self.vertical(x, y) + 16) >> 5);
        let m = || self.clip((self.vertical(x + 1, y) + 16) >> 5);
        let s = || self.clip((self.horizontal(x, y + 1) + 16) >> 5);
        let j = || {
            // Do not clip or round the horizontal intermediate values here.
            let sum: i64 = TAPS
                .iter()
                .enumerate()
                .map(|(i, &tap)| tap * self.horizontal(x, y + i as i64 - 2))
                .sum();
            self.clip((sum + 512) >> 10)
        };
        let avg = |a: u16, b: u16| ((u32::from(a) + u32::from(b) + 1) >> 1) as u16;
        match (fx, fy) {
            (0, 0) => self.at(x, y) as u16,
            (0, 1) => avg(self.at(x, y) as u16, h()),
            (0, 2) => h(),
            (0, 3) => avg(h(), self.at(x, y + 1) as u16),
            (1, 0) => avg(self.at(x, y) as u16, b()),
            (2, 0) => b(),
            (3, 0) => avg(b(), self.at(x + 1, y) as u16),
            (1, 1) => avg(b(), h()),
            (2, 1) => avg(b(), j()),
            (3, 1) => avg(b(), m()),
            (1, 2) => avg(h(), j()),
            (2, 2) => j(),
            (3, 2) => avg(j(), m()),
            (1, 3) => avg(h(), s()),
            (2, 3) => avg(j(), s()),
            (3, 3) => avg(m(), s()),
            _ => unreachable!(),
        }
    }
    /// Origin is in integer luma samples; motion is in signed quarter-sample units.
    /// Writes a packed partition of at most 16x16 samples without allocations.
    pub fn luma(
        &self,
        origin: [i32; 2],
        motion: [i32; 2],
        width: usize,
        height: usize,
        output: &mut [u16],
    ) -> Result<()> {
        partition(width, height, output.len())?;
        let x = i64::from(origin[0]) + i64::from(motion[0] >> 2);
        let y = i64::from(origin[1]) + i64::from(motion[1] >> 2);
        for row in 0..height {
            for col in 0..width {
                output[row * width + col] =
                    self.quarter(x + col as i64, y + row as i64, motion[0] & 3, motion[1] & 3);
            }
        }
        Ok(())
    }
    /// Origin is in integer chroma samples; motion is in signed eighth-sample units.
    /// For progressive 4:2:0, pass the same numeric vector as luma.
    pub fn chroma(
        &self,
        origin: [i32; 2],
        motion: [i32; 2],
        width: usize,
        height: usize,
        output: &mut [u16],
    ) -> Result<()> {
        partition(width, height, output.len())?;
        let x = i64::from(origin[0]) + i64::from(motion[0] >> 3);
        let y = i64::from(origin[1]) + i64::from(motion[1] >> 3);
        let fx = i64::from(motion[0] & 7);
        let fy = i64::from(motion[1] & 7);
        for row in 0..height {
            for col in 0..width {
                let (x, y) = (x + col as i64, y + row as i64);
                let value = (8 - fx) * (8 - fy) * self.at(x, y)
                    + fx * (8 - fy) * self.at(x + 1, y)
                    + (8 - fx) * fy * self.at(x, y + 1)
                    + fx * fy * self.at(x + 1, y + 1);
                output[row * width + col] = ((value + 32) >> 6) as u16;
            }
        }
        Ok(())
    }
}
fn partition(width: usize, height: usize, length: usize) -> Result<()> {
    if width == 0 || height == 0 || width > 16 || height > 16 || length != width * height {
        Err(invalid("invalid motion-compensation partition shape"))
    } else {
        Ok(())
    }
}
fn maximum(depth: u8) -> Result<i64> {
    if !(8..=14).contains(&depth) {
        Err(invalid("invalid prediction bit depth"))
    } else {
        Ok((1i64 << depth) - 1)
    }
}
fn validate_weight(weight: i16, offset: i16, denominator: u8) -> Result<()> {
    if !(-128..=128).contains(&weight) || !(-128..=127).contains(&offset) || denominator > 7 {
        Err(invalid("weighted prediction parameter out of range"))
    } else {
        Ok(())
    }
}
/// Offsets are in the 8-bit units from pred_weight_table; apply bit-depth scaling here.
pub fn weighted(sample: u16, weight: i16, offset: i16, denominator: u8, depth: u8) -> Result<u16> {
    let max = maximum(depth)?;
    validate_weight(weight, offset, denominator)?;
    if i64::from(sample) > max {
        return Err(invalid("prediction sample exceeds bit depth"));
    }
    let rounding = if denominator == 0 {
        0
    } else {
        1i64 << (denominator - 1)
    };
    Ok(
        (((i64::from(sample) * i64::from(weight) + rounding) >> denominator)
            + (i64::from(offset) << (depth - 8)))
            .clamp(0, max) as u16,
    )
}
/// Default bi-prediction is weights=[1,1], offsets=[0,0], denominator=0.
/// Implicit weighting supplies derived weights, zero offsets and denominator=5.
pub fn bipred(
    samples: [u16; 2],
    weights: [i16; 2],
    offsets: [i16; 2],
    denominator: u8,
    depth: u8,
) -> Result<u16> {
    let max = maximum(depth)?;
    for i in 0..2 {
        validate_weight(weights[i], offsets[i], denominator)?;
        if i64::from(samples[i]) > max {
            return Err(invalid("prediction sample exceeds bit depth"));
        }
    }
    let value = (i64::from(samples[0]) * i64::from(weights[0])
        + i64::from(samples[1]) * i64::from(weights[1])
        + (1i64 << denominator))
        >> (denominator + 1);
    let offset = ((i64::from(offsets[0]) + i64::from(offsets[1])) * (1i64 << (depth - 8)) + 1) >> 1;
    Ok((value + offset).clamp(0, max) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn affine_field_all_fractional_positions_and_signed_vectors() {
        let data: Vec<_> = (0..16)
            .flat_map(|y| (0..16).map(move |x| (100 + 8 * x + 16 * y) as u16))
            .collect();
        let plane = ReferencePlane::new(&data, 16, 16, 16, 10).unwrap();
        for my in -7..=7 {
            for mx in -7..=7 {
                let mut luma = [0; 4];
                plane.luma([6, 6], [mx, my], 2, 2, &mut luma).unwrap();
                let mut chroma = [0; 4];
                plane.chroma([6, 6], [mx, my], 2, 2, &mut chroma).unwrap();
                for y in 0..2 {
                    for x in 0..2 {
                        assert_eq!(
                            i32::from(luma[y * 2 + x]),
                            100 + 8 * (6 + x as i32) + 16 * (6 + y as i32) + 2 * mx + 4 * my
                        );
                        assert_eq!(
                            i32::from(chroma[y * 2 + x]),
                            100 + 8 * (6 + x as i32) + 16 * (6 + y as i32) + mx + 2 * my
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn edge_extension_stride_and_extreme_motion_vectors() {
        let plane = ReferencePlane::new(&[10, 20, 999, 30, 40], 2, 2, 3, 8).unwrap();
        for (vector, expected) in [([i32::MIN; 2], 10), ([i32::MAX; 2], 40)] {
            let mut out = [0; 4];
            plane.luma([0; 2], vector, 2, 2, &mut out).unwrap();
            assert_eq!(out, [expected; 4]);
            plane.chroma([0; 2], vector, 2, 2, &mut out).unwrap();
            assert_eq!(out, [expected; 4]);
        }
        let mut out = [0];
        plane.chroma([0; 2], [-1, -1], 1, 1, &mut out).unwrap();
        assert_eq!(out, [10]);
        assert!(plane.luma([0; 2], [0; 2], 17, 1, &mut [0; 17]).is_err());
        assert!(ReferencePlane::new(&[0; 3], 2, 2, 2, 8).is_err());
        assert!(ReferencePlane::new(&[256], 1, 1, 1, 8).is_err());
    }
    #[test]
    fn half_sample_impulses_and_diagonal_no_intermediate_clipping() {
        let mut data = [0; 64];
        data[3 * 8 + 3] = 255;
        let plane = ReferencePlane::new(&data, 8, 8, 8, 8).unwrap();
        let mut out = [0];
        let expected = [
            [255, 207, 159, 80],
            [207, 159, 130, 80],
            [159, 130, 100, 50],
            [80, 80, 50, 0],
        ];
        for y in 0..4 {
            for x in 0..4 {
                plane.luma([3, 3], [x, y], 1, 1, &mut out).unwrap();
                assert_eq!(out[0], expected[y as usize][x as usize]);
            }
        }
        plane.luma([3, 3], [2, 0], 1, 1, &mut out).unwrap();
        assert_eq!(out, [159]);
        plane.luma([3, 3], [2, 2], 1, 1, &mut out).unwrap();
        assert_eq!(out, [100]);
        // Negative tap on each axis becomes positive in the 2D product.
        plane.luma([4, 4], [2, 2], 1, 1, &mut out).unwrap();
        assert_eq!(out, [6]);
        plane.luma([4, 3], [2, 0], 1, 1, &mut out).unwrap();
        assert_eq!(out, [0]);
    }
    #[test]
    fn weights_offsets_rounding_and_clipping() {
        assert_eq!(weighted(100, 1, 0, 0, 8).unwrap(), 100);
        assert_eq!(weighted(101, 3, -10, 1, 8).unwrap(), 142);
        assert_eq!(weighted(100, 1, -2, 0, 10).unwrap(), 92);
        assert_eq!(weighted(255, 128, 127, 0, 8).unwrap(), 255);
        assert_eq!(weighted(255, -128, -128, 0, 8).unwrap(), 0);
        assert_eq!(bipred([100, 101], [1, 1], [0, 0], 0, 8).unwrap(), 101);
        assert_eq!(bipred([100, 200], [48, 16], [0, 0], 5, 8).unwrap(), 125);
        assert_eq!(bipred([100, 200], [1, 1], [-1, 0], 0, 10).unwrap(), 148);
        assert!(weighted(0, 1, 0, 8, 8).is_err());
        assert!(bipred([256, 0], [1, 1], [0, 0], 0, 8).is_err());
    }
}
