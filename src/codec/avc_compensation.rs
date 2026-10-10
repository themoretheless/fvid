//! Frame/field 4:2:0 partition prediction from deblocked reference pictures.
use super::avc_motion::{ReferencePlane, Scratch, bipred_block, weight_block};
use crate::{Result, invalid};

pub struct Reference420<'a> {
    planes: Option<[ReferencePlane<'a>; 3]>,
    depth: u8,
}
#[derive(Clone, Copy, Debug)]
pub struct ComponentWeight {
    pub weight: i16,
    pub offset: i16,
    pub denominator: u8,
}
impl Default for ComponentWeight {
    fn default() -> Self {
        Self {
            weight: 1,
            offset: 0,
            denominator: 0,
        }
    }
}
/// Packed samples occupy width*height (luma) and width*height/4 (chroma).
/// Remaining array entries are zero. No per-partition heap allocation is needed.
pub struct Prediction420 {
    width: usize,
    height: usize,
    pub y: [u16; 256],
    pub cb: [u16; 64],
    pub cr: [u16; 64],
    depth: u8,
}
impl<'a> Reference420<'a> {
    /// Select matching luma/chroma row parity without allocating field planes.
    /// Motion and origin supplied to this view must be in field coordinates.
    pub fn field_view(&self, bottom: bool) -> Result<Self> {
        Ok(Self {
            planes: match &self.planes {
                Some(planes) => Some([
                    planes[0].field_view(bottom)?,
                    planes[1].field_view(bottom)?,
                    planes[2].field_view(bottom)?,
                ]),
                None => None,
            },
            depth: self.depth,
        })
    }
    /// A non-existing DPB slot carries no pixels. It may remain unused in an
    /// active list, but every attempt to sample it is a bitstream error.
    pub(crate) fn unavailable(depth: u8) -> Result<Self> {
        if !(8..=14).contains(&depth) {
            return Err(invalid("invalid unavailable AVC reference depth"));
        }
        Ok(Self {
            planes: None,
            depth,
        })
    }
    /// Plane strides are in samples. Construct once per reference frame, since
    /// validation inspects the sample values in all three planes.
    pub fn new(
        planes: [&'a [u16]; 3],
        width: usize,
        height: usize,
        strides: [usize; 3],
        depth: u8,
    ) -> Result<Self> {
        if width % 2 != 0 || height % 2 != 0 {
            return Err(invalid("AVC 4:2:0 reference dimensions must be even"));
        }
        Ok(Self {
            planes: Some([
                ReferencePlane::new(planes[0], width, height, strides[0], depth)?,
                ReferencePlane::new(planes[1], width / 2, height / 2, strides[1], depth)?,
                ReferencePlane::new(planes[2], width / 2, height / 2, strides[2], depth)?,
            ]),
            depth,
        })
    }
    /// `new` for planes this decoder reconstructed: skips the full-picture
    /// sample scan (see `ReferencePlane::from_decoded`).
    pub fn from_decoded(
        planes: [&'a [u16]; 3],
        width: usize,
        height: usize,
        strides: [usize; 3],
        depth: u8,
    ) -> Result<Self> {
        if width % 2 != 0 || height % 2 != 0 {
            return Err(invalid("AVC 4:2:0 reference dimensions must be even"));
        }
        Ok(Self {
            planes: Some([
                ReferencePlane::from_decoded(planes[0], width, height, strides[0], depth)?,
                ReferencePlane::from_decoded(planes[1], width / 2, height / 2, strides[1], depth)?,
                ReferencePlane::from_decoded(planes[2], width / 2, height / 2, strides[2], depth)?,
            ]),
            depth,
        })
    }
    /// Origin and size are in luma samples, vector in quarter-luma units.
    pub fn predict(
        &self,
        origin: [i32; 2],
        motion: [i32; 2],
        size: [usize; 2],
    ) -> Result<Prediction420> {
        let mut out = Prediction420::empty(self.depth);
        self.predict_into(origin, motion, size, &mut out, &mut Scratch::new())?;
        Ok(out)
    }
    /// `predict` into a caller-owned buffer; only the packed region is written.
    pub fn predict_into(
        &self,
        origin: [i32; 2],
        motion: [i32; 2],
        size: [usize; 2],
        out: &mut Prediction420,
        scratch: &mut Scratch,
    ) -> Result<()> {
        self.predict_into_motion(origin, motion, motion, size, out, scratch)
    }
    /// Separate vectors permit the field-parity chroma adjustment in 4:2:0.
    pub fn predict_into_motion(
        &self,
        origin: [i32; 2],
        motion: [i32; 2],
        chroma_motion: [i32; 2],
        size: [usize; 2],
        out: &mut Prediction420,
        scratch: &mut Scratch,
    ) -> Result<()> {
        let planes = self
            .planes
            .as_ref()
            .ok_or_else(|| invalid("AVC prediction selects non-existing reference picture"))?;
        let [width, height] = size;
        if ![4, 8, 16].contains(&width)
            || ![4, 8, 16].contains(&height)
            || origin.iter().any(|v| v % 2 != 0)
        {
            return Err(invalid("invalid AVC 4:2:0 partition geometry"));
        }
        out.width = width;
        out.height = height;
        out.depth = self.depth;
        let count = width * height;
        planes[0].luma_with(origin, motion, width, height, &mut out.y[..count], scratch)?;
        let chroma_origin = [origin[0] / 2, origin[1] / 2];
        planes[1].chroma_with(
            chroma_origin,
            chroma_motion,
            width / 2,
            height / 2,
            &mut out.cb[..count / 4],
            scratch,
        )?;
        planes[2].chroma_with(
            chroma_origin,
            chroma_motion,
            width / 2,
            height / 2,
            &mut out.cr[..count / 4],
            scratch,
        )?;
        Ok(())
    }
}
impl Prediction420 {
    /// A zero 16x16 buffer for `predict_into`.
    pub fn empty(depth: u8) -> Self {
        Self {
            width: 16,
            height: 16,
            y: [0; 256],
            cb: [0; 64],
            cr: [0; 64],
            depth,
        }
    }
    pub fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    pub fn bit_depth(&self) -> u8 {
        self.depth
    }
    /// Apply explicit uni-prediction weights in Y/Cb/Cr order.
    pub fn weight(mut self, weights: [ComponentWeight; 3]) -> Result<Self> {
        self.weight_in_place(weights)?;
        Ok(self)
    }
    pub fn weight_in_place(&mut self, weights: [ComponentWeight; 3]) -> Result<()> {
        let count = self.width * self.height;
        for (component, samples) in [
            &mut self.y[..count],
            &mut self.cb[..count / 4],
            &mut self.cr[..count / 4],
        ]
        .into_iter()
        .enumerate()
        {
            let w = weights[component];
            weight_block(samples, w.weight, w.offset, w.denominator, self.depth)?;
        }
        Ok(())
    }
    /// Blend raw L0/L1 predictions. Explicit component denominators must match.
    /// Defaults use weight 1, offset 0, denominator 0; implicit weights use 5.
    pub fn blend(mut self, other: Self, weights: [[ComponentWeight; 3]; 2]) -> Result<Self> {
        self.blend_in_place(&other, weights)?;
        Ok(self)
    }
    pub fn blend_in_place(
        &mut self,
        other: &Self,
        weights: [[ComponentWeight; 3]; 2],
    ) -> Result<()> {
        if self.width != other.width || self.height != other.height || self.depth != other.depth {
            return Err(invalid("AVC prediction geometry or bit depth mismatch"));
        }
        let count = self.width * self.height;
        let a = [
            &mut self.y[..count],
            &mut self.cb[..count / 4],
            &mut self.cr[..count / 4],
        ];
        let b = [
            &other.y[..count],
            &other.cb[..count / 4],
            &other.cr[..count / 4],
        ];
        for (component, (a, b)) in a.into_iter().zip(b).enumerate() {
            let w = [weights[0][component], weights[1][component]];
            if w[0].denominator != w[1].denominator {
                return Err(invalid("AVC bipred denominators differ"));
            }
            bipred_block(
                a,
                b,
                [w[0].weight, w[1].weight],
                [w[0].offset, w[1].offset],
                w[0].denominator,
                self.depth,
            )?;
        }
        Ok(())
    }
}

/// Coefficients are raster ordered within blocks and blocks are raster ordered
/// within the macroblock. Chroma AC slot zero is replaced by transformed DC.
pub enum InterLumaResidual<'a> {
    Blocks4(&'a [[i32; 16]; 16]),
    Blocks8(&'a [[i32; 64]; 4]),
}
impl Prediction420 {
    /// Lossless inter residuals bypass scaling and transforms without intra DPCM.
    pub fn reconstruct_inter_bypass(
        mut self,
        luma: InterLumaResidual<'_>,
        chroma_dc: &[[i32; 4]; 2],
        chroma_ac: &[[[i32; 16]; 4]; 2],
    ) -> Result<Self> {
        if self.dimensions() != (16, 16) {
            return Err(invalid("inter bypass requires a full macroblock"));
        }
        let residual = match luma {
            InterLumaResidual::Blocks4(blocks) => {
                super::avc_bypass::blocks4::<256>(blocks, None, 16)?
            }
            InterLumaResidual::Blocks8(blocks) => {
                let mut plane = [0; 256];
                for (block, levels) in blocks.iter().enumerate() {
                    let (x, y) = (block % 2 * 8, block / 2 * 8);
                    for i in 0..64 {
                        plane[(y + i / 8) * 16 + x + i % 8] = levels[i];
                    }
                }
                plane
            }
        };
        self.y = super::avc_transform::reconstruct(&self.y, &residual, self.depth)?;
        for (component, plane) in [&mut self.cb, &mut self.cr].into_iter().enumerate() {
            let residual = super::avc_bypass::blocks4::<64>(
                &chroma_ac[component],
                Some(&chroma_dc[component]),
                8,
            )?;
            *plane = super::avc_transform::reconstruct(plane, &residual, self.depth)?;
        }
        Ok(self)
    }
    /// Primary-SP reconstruction, including requantization for skipped blocks.
    /// Entropy supplies raster luma/AC blocks and scanned 2x2 chroma DC levels.
    pub fn reconstruct_primary_sp(
        self,
        luma: &[[i32; 16]; 16],
        chroma_dc: &[[i32; 4]; 2],
        chroma_ac: &[[[i32; 16]; 4]; 2],
        qp: [u8; 3],
        qs: [u8; 3],
    ) -> Result<Self> {
        self.reconstruct_sp(luma, chroma_dc, chroma_ac, qp, qs, false)
    }
    /// Reconstruct primary or secondary SP from prediction and parsed levels.
    pub fn reconstruct_sp(
        mut self,
        luma: &[[i32; 16]; 16],
        chroma_dc: &[[i32; 4]; 2],
        chroma_ac: &[[[i32; 16]; 4]; 2],
        qp: [u8; 3],
        qs: [u8; 3],
        switching: bool,
    ) -> Result<Self> {
        if self.dimensions() != (16, 16) || self.depth != 8 {
            return Err(invalid("SP requires an eight-bit 16x16 prediction"));
        }
        for index in 0..16 {
            let x = index % 4 * 4;
            let y = index / 4 * 4;
            let p = std::array::from_fn(|i| self.y[(y + i / 4) * 16 + x + i % 4]);
            let block =
                super::avc_transform::switching_luma_4x4(&p, &luma[index], qp[0], qs[0], switching)?;
            for i in 0..16 {
                self.y[(y + i / 4) * 16 + x + i % 4] = block[i];
            }
        }
        for (component, plane) in [&mut self.cb, &mut self.cr].into_iter().enumerate() {
            let v = chroma_dc[component];
            let dc = [v[0], v[2], v[1], v[3]];
            let p = plane
                .as_slice()
                .try_into()
                .map_err(|_| invalid("invalid SP chroma geometry"))?;
            let samples = if switching {
                super::avc_transform::switching_chroma_420(p, &dc, &chroma_ac[component], qs[component + 1])?
            } else {
                super::avc_transform::primary_sp_chroma_420(p, &dc, &chroma_ac[component], qp[component + 1], qs[component + 1])?
            };
            plane.copy_from_slice(&samples);
        }
        Ok(self)
    }
    /// Reconstruct a complete inter macroblock before deblocking. QPs include
    /// the bit-depth offset; the caller derives component QPs from slice QP.
    /// Scaling lists must already have SPS/PPS fallback rules applied.
    pub fn reconstruct_inter(
        mut self,
        luma: InterLumaResidual<'_>,
        chroma_dc: &[[i32; 4]; 2],
        chroma_ac: &[[[i32; 16]; 4]; 2],
        qp: [u8; 3],
        weights4: &[[u8; 16]; 3],
        weights8: &[u8; 64],
    ) -> Result<Self> {
        use super::avc_transform::{chroma_dc_2x2, reconstruct_4x4, residual_4x4};
        use super::avc_transform8::{reconstruct_8x8, residual_8x8};
        if self.dimensions() != (16, 16) {
            return Err(invalid(
                "inter residual reconstruction requires a full macroblock",
            ));
        }
        match luma {
            InterLumaResidual::Blocks4(blocks) => {
                for (index, levels) in blocks.iter().enumerate() {
                    let (x, y) = (index % 4 * 4, index / 4 * 4);
                    let pred = std::array::from_fn(|i| self.y[(y + i / 4) * 16 + x + i % 4]);
                    let residual = residual_4x4(levels, qp[0], self.depth, &weights4[0], None)?;
                    let block = reconstruct_4x4(&pred, &residual, self.depth)?;
                    for row in 0..4 {
                        self.y[(y + row) * 16 + x..(y + row) * 16 + x + 4]
                            .copy_from_slice(&block[row * 4..row * 4 + 4]);
                    }
                }
            }
            InterLumaResidual::Blocks8(blocks) => {
                for (index, levels) in blocks.iter().enumerate() {
                    let (x, y) = (index % 2 * 8, index / 2 * 8);
                    let pred = std::array::from_fn(|i| self.y[(y + i / 8) * 16 + x + i % 8]);
                    let residual = residual_8x8(levels, qp[0], self.depth, weights8)?;
                    let block = reconstruct_8x8(&pred, &residual, self.depth)?;
                    for row in 0..8 {
                        self.y[(y + row) * 16 + x..(y + row) * 16 + x + 8]
                            .copy_from_slice(&block[row * 8..row * 8 + 8]);
                    }
                }
            }
        }
        for (component, plane) in [&mut self.cb, &mut self.cr].into_iter().enumerate() {
            let dc = chroma_dc_2x2(
                &chroma_dc[component],
                qp[component + 1],
                self.depth,
                weights4[component + 1][0],
            )?;
            for index in 0..4 {
                let (x, y) = (index % 2 * 4, index / 2 * 4);
                let pred = std::array::from_fn(|i| plane[(y + i / 4) * 8 + x + i % 4]);
                let residual = residual_4x4(
                    &chroma_ac[component][index],
                    qp[component + 1],
                    self.depth,
                    &weights4[component + 1],
                    Some(dc[index]),
                )?;
                let block = reconstruct_4x4(&pred, &residual, self.depth)?;
                for row in 0..4 {
                    plane[(y + row) * 8 + x..(y + row) * 8 + x + 4]
                        .copy_from_slice(&block[row * 4..row * 4 + 4]);
                }
            }
        }
        Ok(self)
    }
}

/// Assemble non-overlapping motion-compensated partitions into one macroblock.
/// Coverage is tracked at 4x4 luma granularity; chroma follows the same regions.
pub struct MacroblockPrediction {
    picture: Prediction420,
    covered: u16,
}
impl MacroblockPrediction {
    pub fn new(depth: u8) -> Result<Self> {
        if !(8..=14).contains(&depth) {
            return Err(invalid("AVC prediction bit depth out of range"));
        }
        Ok(Self {
            picture: Prediction420 {
                width: 16,
                height: 16,
                y: [0; 256],
                cb: [0; 64],
                cr: [0; 64],
                depth,
            },
            covered: 0,
        })
    }
    /// Origin is relative to this macroblock. Invalid input leaves it unchanged.
    pub fn insert(&mut self, origin: [usize; 2], partition: &Prediction420) -> Result<()> {
        let [x, y] = origin;
        let (w, h) = partition.dimensions();
        if partition.depth != self.picture.depth
            || x > 16 - w
            || y > 16 - h
            || x % 4 != 0
            || y % 4 != 0
        {
            return Err(invalid("AVC partition does not fit macroblock"));
        }
        let mut mask = 0u16;
        for row in y / 4..(y + h) / 4 {
            for col in x / 4..(x + w) / 4 {
                mask |= 1 << (row * 4 + col);
            }
        }
        if self.covered & mask != 0 {
            return Err(invalid("overlapping AVC prediction partitions"));
        }
        // Predictions are clipped to the bit depth when interpolated and weighted.
        debug_assert!({
            let max = (1u16 << self.picture.depth) - 1;
            partition.y[..w * h]
                .iter()
                .chain(&partition.cb[..w * h / 4])
                .chain(&partition.cr[..w * h / 4])
                .all(|&v| v <= max)
        });
        // Rows are 4 to 16 samples; plain loops beat a memmove call per row.
        for row in 0..h {
            let dst = &mut self.picture.y[(y + row) * 16 + x..][..w];
            let src = &partition.y[row * w..][..w];
            for (d, s) in dst.iter_mut().zip(src) {
                *d = *s;
            }
        }
        let (cw, ch) = (w / 2, h / 2);
        for (dst, src) in [
            (&mut self.picture.cb, &partition.cb),
            (&mut self.picture.cr, &partition.cr),
        ] {
            for row in 0..ch {
                let dst = &mut dst[(y / 2 + row) * 8 + x / 2..][..cw];
                let src = &src[row * cw..][..cw];
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = *s;
                }
            }
        }
        self.covered |= mask;
        Ok(())
    }
    pub fn finish(self) -> Result<Prediction420> {
        if self.covered != u16::MAX {
            return Err(invalid("incomplete AVC macroblock prediction"));
        }
        Ok(self.picture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strided_field_views_match_packed_fields_for_fractional_and_edge_prediction() {
        for depth in [8, 10] {
            let max = (1u16 << depth) - 1;
            let make = |width: usize, height: usize, stride: usize, salt: usize| {
                let mut data = vec![max; stride * height];
                for y in 0..height {
                    for x in 0..width {
                        data[y * stride + x] = ((x * 17 + y * 37 + salt) % usize::from(max)) as u16;
                    }
                }
                data
            };
            let y = make(16, 64, 21, 3);
            let cb = make(8, 32, 13, 31);
            let cr = make(8, 32, 11, 67);
            let frame = Reference420::new([&y, &cb, &cr], 16, 64, [21, 13, 11], depth).unwrap();
            for bottom in [false, true] {
                let pack = |data: &[u16], width: usize, height: usize, stride: usize| {
                    (usize::from(bottom)..height)
                        .step_by(2)
                        .flat_map(|row| data[row * stride..row * stride + width].iter().copied())
                        .collect::<Vec<_>>()
                };
                let py = pack(&y, 16, 64, 21);
                let pcb = pack(&cb, 8, 32, 13);
                let pcr = pack(&cr, 8, 32, 11);
                let packed =
                    Reference420::new([&py, &pcb, &pcr], 16, 32, [16, 8, 8], depth).unwrap();
                let view = frame.field_view(bottom).unwrap();
                let integer = view.predict([0, 0], [0, 0], [16, 16]).unwrap();
                assert_eq!(integer.y[0], y[usize::from(bottom) * 21]);
                assert_eq!(integer.cb[0], cb[usize::from(bottom) * 13]);
                assert_eq!(integer.cr[0], cr[usize::from(bottom) * 11]);
                for origin in [[-2, -2], [2, 6], [14, 26]] {
                    for dy in -4..4 {
                        for dx in -4..4 {
                            let a = view.predict(origin, [dx, dy], [8, 8]).unwrap();
                            let b = packed.predict(origin, [dx, dy], [8, 8]).unwrap();
                            assert_eq!(a.y, b.y);
                            assert_eq!(a.cb, b.cb);
                            assert_eq!(a.cr, b.cr);
                        }
                    }
                }
            }
        }
        let tiny = Reference420::new([&[0; 4], &[0], &[0]], 2, 2, [2, 1, 1], 8).unwrap();
        assert!(tiny.field_view(false).is_err());
    }
    #[test]
    fn fractional_luma_chroma_and_bipred() {
        let y: Vec<_> = (0..16)
            .flat_map(|y| (0..16).map(move |x| 100 + 8 * x + 16 * y))
            .collect();
        let cb: Vec<_> = (0..8)
            .flat_map(|y| (0..8).map(move |x| 100 + 16 * x + 32 * y))
            .collect();
        let cr = vec![400; 64];
        let r = Reference420::new([&y, &cb, &cr], 16, 16, [16, 8, 8], 10).unwrap();
        let a = r.predict([4, 4], [1, 2], [4, 4]).unwrap();
        assert_eq!(a.y[0], 206);
        assert_eq!(a.cb[0], 206);
        assert_eq!(a.cr[0], 400);
        let b = r.predict([4, 4], [-1, -2], [4, 4]).unwrap();
        let mixed = a.blend(b, [[ComponentWeight::default(); 3]; 2]).unwrap();
        assert_eq!(mixed.y[0], 196);
        assert_eq!(mixed.cb[0], 196);
        let scaled = mixed
            .weight(
                [ComponentWeight {
                    weight: 1,
                    offset: -10,
                    denominator: 0,
                }; 3],
            )
            .unwrap();
        assert_eq!(scaled.y[0], 156);
        assert_eq!(scaled.cr[0], 360);
        assert_eq!(scaled.y[16], 0);
        assert_eq!(scaled.cb[4], 0);
    }
    #[test]
    fn border_extension_and_invalid_geometry() {
        let y = [10; 256];
        let c = [20; 64];
        let r = Reference420::new([&y, &c, &c], 16, 16, [16, 8, 8], 8).unwrap();
        let p = r.predict([0, 0], [i32::MIN, i32::MAX], [16, 16]).unwrap();
        assert_eq!(p.y, [10; 256]);
        assert_eq!(p.cb, [20; 64]);
        assert!(r.predict([1, 0], [0, 0], [4, 4]).is_err());
        assert!(r.predict([0, 0], [0, 0], [32, 16]).is_err());
        assert!(Reference420::new([&y, &c[..63], &c], 16, 16, [16, 8, 8], 8).is_err());
    }
}

#[cfg(test)]
mod assembly_tests {
    use super::*;
    #[test]
    fn mixed_partitions_preserve_luma_and_chroma_positions() {
        let y = [37; 256];
        let cb = [81; 64];
        let cr = [123; 64];
        let r = Reference420::new([&y, &cb, &cr], 16, 16, [16, 8, 8], 8).unwrap();
        let mut mb = MacroblockPrediction::new(8).unwrap();
        let top = r.predict([0, 0], [0, 0], [16, 8]).unwrap();
        mb.insert([0, 0], &top).unwrap();
        assert!(mb.insert([0, 0], &top).is_err());
        assert!(mb.insert([usize::MAX, 0], &top).is_err());
        for x in [0, 8] {
            let p = r
                .predict([x as i32, 8], [0, 0], [8, 8])
                .unwrap()
                .weight(
                    [ComponentWeight {
                        weight: 1,
                        offset: x as i16 + 1,
                        denominator: 0,
                    }; 3],
                )
                .unwrap();
            mb.insert([x, 8], &p).unwrap();
        }
        let out = mb.finish().unwrap();
        assert_eq!(out.y[0], 37);
        assert_eq!(out.y[128], 38);
        assert_eq!(out.y[136], 46);
        assert_eq!(out.cb[32], 82);
        assert_eq!(out.cb[36], 90);
        assert_eq!(out.cr[36], 132);
        assert!(MacroblockPrediction::new(8).unwrap().finish().is_err());
    }
}

#[cfg(test)]
mod residual_tests {
    use super::*;
    #[test]
    fn inter_residual_dc_placement_clipping_and_zero_identity() {
        let y = [250; 256];
        let cb = [2; 64];
        let cr = [100; 64];
        let reference = Reference420::new([&y, &cb, &cr], 16, 16, [16, 8, 8], 8).unwrap();
        let mut levels = [[0; 16]; 16];
        levels[0][0] = 64;
        levels[15][0] = -64;
        let result = reference
            .predict([0, 0], [0, 0], [16, 16])
            .unwrap()
            .reconstruct_inter(
                InterLumaResidual::Blocks4(&levels),
                &[[-64, 0, 0, 0], [0; 4]],
                &[[[0; 16]; 4]; 2],
                [0; 3],
                &[[16; 16]; 3],
                &[16; 64],
            )
            .unwrap();
        assert_eq!(result.y[0], 255);
        assert_eq!(result.y[12 * 16 + 12], 240);
        assert_eq!(result.y[4], 250);
        assert_eq!(result.cb, [0; 64]);
        assert_eq!(result.cr, [100; 64]);
        let result = reference
            .predict([0, 0], [0, 0], [16, 16])
            .unwrap()
            .reconstruct_inter(
                InterLumaResidual::Blocks8(&[[0; 64]; 4]),
                &[[0; 4]; 2],
                &[[[0; 16]; 4]; 2],
                [51; 3],
                &[[16; 16]; 3],
                &[16; 64],
            )
            .unwrap();
        assert_eq!(result.y, y);
        assert_eq!(result.cb, cb);
        assert_eq!(result.cr, cr);
    }
}

#[cfg(test)]
mod unavailable_tests {
    use super::*;
    #[test]
    fn absent_reference_has_no_planes_and_refuses_sampling_without_writing_output() {
        for depth in [8, 10, 12, 14] {
            let reference = Reference420::unavailable(depth).unwrap();
            assert!(reference.planes.is_none());
            for field in [None, Some(false), Some(true)] {
                let reference = match field {
                    None => Reference420::unavailable(depth).unwrap(),
                    Some(bottom) => reference.field_view(bottom).unwrap(),
                };
                let mut output = Prediction420::empty(depth);
                output.y.fill(123);
                output.cb.fill(45);
                output.cr.fill(67);
                let error = reference
                    .predict_into_motion(
                        [0, 0],
                        [0, 0],
                        [0, 2],
                        [16, 16],
                        &mut output,
                        &mut Scratch::new(),
                    )
                    .err()
                    .unwrap();
                assert!(error.to_string().contains("non-existing reference"));
                assert_eq!(output.y, [123; 256]);
                assert_eq!(output.cb, [45; 64]);
                assert_eq!(output.cr, [67; 64]);
            }
        }
        assert!(Reference420::unavailable(7).is_err());
        assert!(Reference420::unavailable(15).is_err());
    }
}
