//! H.264 fractional-sample motion compensation and weighted prediction (8.4.2.2–3).
//! Reference planes contain reconstructed, deblocked samples before display cropping.
use crate::{Result, invalid};

/// Samples in the largest interpolation window: a 16x16 luma partition plus
/// the 6-tap filter margin (21 x 21).
pub const SCRATCH: usize = 21 * 21;
/// Horizontal sums for every window row of a 16-wide partition.
const SUMS: usize = 16 * 21;
/// Working memory for one macroblock's interpolation, reused across
/// partitions so hot loops never zero it.
pub struct Scratch {
    window: [u16; SCRATCH],
    sums: [i32; SUMS],
}
impl Scratch {
    pub fn new() -> Self {
        Self {
            window: [0; SCRATCH],
            sums: [0; SUMS],
        }
    }
}
impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

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
    /// `new` for a plane this decoder reconstructed itself: every sample was
    /// clipped to the bit depth on the way out, so only geometry is checked
    /// instead of scanning the whole picture again per reference.
    pub fn from_decoded(
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
        debug_assert!(samples.iter().all(|&p| i64::from(p) <= max));
        Ok(Self {
            samples,
            width,
            height,
            stride,
            depth,
        })
    }
    fn at(&self, x: i64, y: i64) -> u16 {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        self.samples[y * self.stride + x]
    }
    fn inside(&self, x: i64, y: i64, cols: usize, rows: usize) -> bool {
        x >= 0
            && y >= 0
            && x + cols as i64 <= self.width as i64
            && y + rows as i64 <= self.height as i64
    }
    /// The `cols` x `rows` window whose top-left is (`x`, `y`) as a sample
    /// slice plus stride: the plane itself when the window lies inside it,
    /// else a copy with edge samples replicated (8.4.2.2.1 coordinate clipping).
    fn window<'w>(
        &'w self,
        x: i64,
        y: i64,
        cols: usize,
        rows: usize,
        copy: &'w mut [u16; SCRATCH],
    ) -> (&'w [u16], usize) {
        if self.inside(x, y, cols, rows) {
            let base = y as usize * self.stride + x as usize;
            return (&self.samples[base..], self.stride);
        }
        let out = &mut copy[..cols * rows];
        for row in 0..rows {
            for col in 0..cols {
                out[row * cols + col] = self.at(x + col as i64, y + row as i64);
            }
        }
        (out, cols)
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
        self.luma_with(origin, motion, width, height, output, &mut Scratch::new())
    }
    /// `luma` with caller-provided working memory, so hot loops skip zeroing it.
    pub fn luma_with(
        &self,
        origin: [i32; 2],
        motion: [i32; 2],
        width: usize,
        height: usize,
        output: &mut [u16],
        scratch: &mut Scratch,
    ) -> Result<()> {
        partition(width, height, output.len())?;
        let x = i64::from(origin[0]) + i64::from(motion[0] >> 2);
        let y = i64::from(origin[1]) + i64::from(motion[1] >> 2);
        // Two samples of margin before and three after each axis cover every tap.
        let Scratch { window, sums } = scratch;
        let (samples, stride) = self.window(x - 2, y - 2, width + 5, height + 5, window);
        let max = (1i32 << self.depth) - 1;
        luma_core(
            samples,
            stride,
            width,
            height,
            [motion[0] & 3, motion[1] & 3],
            max,
            output,
            sums,
        );
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
        self.chroma_with(origin, motion, width, height, output, &mut Scratch::new())
    }
    /// `chroma` with caller-provided working memory.
    pub fn chroma_with(
        &self,
        origin: [i32; 2],
        motion: [i32; 2],
        width: usize,
        height: usize,
        output: &mut [u16],
        scratch: &mut Scratch,
    ) -> Result<()> {
        partition(width, height, output.len())?;
        let x = i64::from(origin[0]) + i64::from(motion[0] >> 3);
        let y = i64::from(origin[1]) + i64::from(motion[1] >> 3);
        let fx = motion[0] & 7;
        let fy = motion[1] & 7;
        let (samples, stride) = self.window(x, y, width + 1, height + 1, &mut scratch.window);
        let w = [
            (8 - fx) * (8 - fy),
            fx * (8 - fy),
            (8 - fx) * fy,
            fx * fy,
        ];
        for row in 0..height {
            let top = &samples[row * stride..][..width + 1];
            let bottom = &samples[(row + 1) * stride..][..width + 1];
            let out = &mut output[row * width..][..width];
            for col in 0..width {
                let value = w[0] * i32::from(top[col])
                    + w[1] * i32::from(top[col + 1])
                    + w[2] * i32::from(bottom[col])
                    + w[3] * i32::from(bottom[col + 1]);
                out[col] = ((value + 32) >> 6) as u16;
            }
        }
        Ok(())
    }
}
/// Quarter-sample luma interpolation over a window (`samples`, `stride`) whose
/// origin is two samples above and left of the partition. Positions that need
/// the centre half-sample `j` first store unrounded horizontal sums for every
/// window row in `sums`, so the vertical pass reads six values per sample
/// instead of recomputing six 6-tap filters.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn luma_core(
    s: &[u16],
    stride: usize,
    width: usize,
    height: usize,
    fraction: [i32; 2],
    max: i32,
    output: &mut [u16],
    sums: &mut [i32; SUMS],
) {
    let clip = |v: i32| v.clamp(0, max) as u16;
    let avg = |a: u16, b: u16| ((u32::from(a) + u32::from(b) + 1) >> 1) as u16;
    let get = |px: usize, py: usize| s[py * stride + px];
    let hsum = |px: usize, py: usize| -> i32 {
        let r = &s[py * stride + px - 2..][..6];
        i32::from(r[0]) - 5 * i32::from(r[1]) + 20 * i32::from(r[2]) + 20 * i32::from(r[3])
            - 5 * i32::from(r[4])
            + i32::from(r[5])
    };
    let vsum = |px: usize, py: usize| -> i32 {
        let base = (py - 2) * stride + px;
        i32::from(s[base]) - 5 * i32::from(s[base + stride]) + 20 * i32::from(s[base + 2 * stride])
            + 20 * i32::from(s[base + 3 * stride])
            - 5 * i32::from(s[base + 4 * stride])
            + i32::from(s[base + 5 * stride])
    };
    let [fx, fy] = fraction;
    if fx == 0 && fy == 0 {
        for row in 0..height {
            output[row * width..][..width].copy_from_slice(&s[(row + 2) * stride + 2..][..width]);
        }
        return;
    }
    let needs_j = (fx == 2 && fy != 0) || (fy == 2 && fx != 0);
    if needs_j {
        for r in 0..height + 5 {
            for c in 0..width {
                sums[r * width + c] = hsum(c + 2, r);
            }
        }
    }
    match (fx, fy) {
        (0, 1) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let h = clip((vsum(px, py) + 16) >> 5);
                    output[row * width + col] = avg(get(px, py), h);
                }
            }
        }
        (0, 2) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    output[row * width + col] = clip((vsum(px, py) + 16) >> 5);
                }
            }
        }
        (0, 3) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let h = clip((vsum(px, py) + 16) >> 5);
                    output[row * width + col] = avg(h, get(px, py + 1));
                }
            }
        }
        (1, 0) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let b = clip((hsum(px, py) + 16) >> 5);
                    output[row * width + col] = avg(get(px, py), b);
                }
            }
        }
        (2, 0) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    output[row * width + col] = clip((hsum(px, py) + 16) >> 5);
                }
            }
        }
        (3, 0) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let b = clip((hsum(px, py) + 16) >> 5);
                    output[row * width + col] = avg(b, get(px + 1, py));
                }
            }
        }
        (1, 1) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let b = clip((hsum(px, py) + 16) >> 5);
                    let h = clip((vsum(px, py) + 16) >> 5);
                    output[row * width + col] = avg(b, h);
                }
            }
        }
        (2, 1) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let b = clip((hsum(px, py) + 16) >> 5);
                    let t = |k: usize| sums[(row + k) * width + col];
                    let j = clip((t(0) - 5 * t(1) + 20 * t(2) + 20 * t(3) - 5 * t(4) + t(5) + 512) >> 10);
                    output[row * width + col] = avg(b, j);
                }
            }
        }
        (3, 1) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let b = clip((hsum(px, py) + 16) >> 5);
                    let m = clip((vsum(px + 1, py) + 16) >> 5);
                    output[row * width + col] = avg(b, m);
                }
            }
        }
        (1, 2) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let h = clip((vsum(px, py) + 16) >> 5);
                    let t = |k: usize| sums[(row + k) * width + col];
                    let j = clip((t(0) - 5 * t(1) + 20 * t(2) + 20 * t(3) - 5 * t(4) + t(5) + 512) >> 10);
                    output[row * width + col] = avg(h, j);
                }
            }
        }
        (2, 2) => {
            for row in 0..height {
                for col in 0..width {
                    let t = |k: usize| sums[(row + k) * width + col];
                    output[row * width + col] = clip((t(0) - 5 * t(1) + 20 * t(2) + 20 * t(3) - 5 * t(4) + t(5) + 512) >> 10);
                }
            }
        }
        (3, 2) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let m = clip((vsum(px + 1, py) + 16) >> 5);
                    let t = |k: usize| sums[(row + k) * width + col];
                    let j = clip((t(0) - 5 * t(1) + 20 * t(2) + 20 * t(3) - 5 * t(4) + t(5) + 512) >> 10);
                    output[row * width + col] = avg(j, m);
                }
            }
        }
        (1, 3) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let h = clip((vsum(px, py) + 16) >> 5);
                    let s_ = clip((hsum(px, py + 1) + 16) >> 5);
                    output[row * width + col] = avg(h, s_);
                }
            }
        }
        (2, 3) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let s_ = clip((hsum(px, py + 1) + 16) >> 5);
                    let t = |k: usize| sums[(row + k) * width + col];
                    let j = clip((t(0) - 5 * t(1) + 20 * t(2) + 20 * t(3) - 5 * t(4) + t(5) + 512) >> 10);
                    output[row * width + col] = avg(j, s_);
                }
            }
        }
        (3, 3) => {
            for row in 0..height {
                for col in 0..width {
                    let (px, py) = (col + 2, row + 2);
                    let m = clip((vsum(px + 1, py) + 16) >> 5);
                    let s_ = clip((hsum(px, py + 1) + 16) >> 5);
                    output[row * width + col] = avg(m, s_);
                }
            }
        }
        _ => unreachable!(),
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
/// `weighted` over a whole partition: parameters are validated once, each
/// sample is still range-checked, and the arithmetic is identical.
pub fn weight_block(
    samples: &mut [u16],
    weight: i16,
    offset: i16,
    denominator: u8,
    depth: u8,
) -> Result<()> {
    let max = maximum(depth)?;
    validate_weight(weight, offset, denominator)?;
    // Interpolation clips every sample to the bit depth, so the range holds.
    debug_assert!(samples.iter().all(|&s| i64::from(s) <= max));
    // Samples are at most 14 bits and weights 8 bits, so i32 cannot overflow.
    let rounding = if denominator == 0 {
        0
    } else {
        1i32 << (denominator - 1)
    };
    let (weight, offset) = (i32::from(weight), i32::from(offset) << (depth - 8));
    let max = max as i32;
    for sample in samples {
        *sample = (((i32::from(*sample) * weight + rounding) >> denominator) + offset)
            .clamp(0, max) as u16;
    }
    Ok(())
}
/// `bipred` over a whole partition, blending `b` into `a` in place.
pub fn bipred_block(
    a: &mut [u16],
    b: &[u16],
    weights: [i16; 2],
    offsets: [i16; 2],
    denominator: u8,
    depth: u8,
) -> Result<()> {
    let max = maximum(depth)?;
    for i in 0..2 {
        validate_weight(weights[i], offsets[i], denominator)?;
    }
    if a.len() != b.len() {
        return Err(invalid("bipred partition sizes differ"));
    }
    debug_assert!(a.iter().chain(b).all(|&s| i64::from(s) <= max));
    // Two 14-bit samples times 8-bit weights stay far inside i32.
    let w = weights.map(i32::from);
    let rounding = 1i32 << denominator;
    let shift = denominator + 1;
    let offset = ((i32::from(offsets[0]) + i32::from(offsets[1])) * (1i32 << (depth - 8)) + 1) >> 1;
    let max = max as i32;
    for (a, b) in a.iter_mut().zip(b) {
        let value = (i32::from(*a) * w[0] + i32::from(*b) * w[1] + rounding) >> shift;
        *a = (value + offset).clamp(0, max) as u16;
    }
    Ok(())
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
