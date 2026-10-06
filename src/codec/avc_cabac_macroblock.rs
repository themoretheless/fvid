//! 4:2:0 CABAC intra macroblocks with frame/field coefficient scans.
//! The explicit MBAFF intra reader is separate from progressive P/B dispatch.
use super::{
    avc::{Pps, SliceGroups, Sps},
    avc_cabac::{AvcCabac, ResidualCategory as Cat},
    avc_intra::{ChromaMode, Intra4Mode, derive_intra4_mode},
    avc_macroblock::{IntraLuma, IntraMacroblock, luma_block_xy},
    avc_slice::{SliceHeader, SliceType},
    avc_transform::inverse_scan_4x4,
};
use crate::{Result, invalid};

/// Decoded inter/skip state needed by subsequent embedded intra blocks.
pub struct InterNeighbourContext {
    pub qp: i32,
    pub qp_delta_nonzero: bool,
    pub pattern: u8,
    pub transform8: bool,
    pub luma_coded: [bool; 16],
    pub chroma_dc: [bool; 2],
    pub chroma_ac: [[bool; 4]; 2],
}

pub struct IntraCabacReader<'a> {
    cabac: AvcCabac<'a>,
    sps: &'a Sps,
    pps: &'a Pps,
    address: usize,
    qp: i32,
    previous_delta: bool,
    slice_type: SliceType,
    finished: bool,
    types: Vec<u8>,
    eight: Vec<u8>,
    patterns: Vec<u8>,
    chroma_modes: Vec<u8>,
    dc: [Vec<u8>; 3],
    luma: Vec<u8>,
    chroma: [Vec<u8>; 2],
    modes: Vec<u8>,
    mbaff: bool,
    pair_fields: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macroblock_contexts_preserve_availability_and_component_flags() {
        let grid = [0, 1, 255, 0];
        assert_eq!(neighbours(&grid, 2, 1, 1).unwrap(), [255, 1]);
        assert_eq!(coded_context(&grid, 2, 1, 1, true).unwrap(), 3);
        assert_eq!(coded_context(&grid, 2, 1, 1, false).unwrap(), 2);
        assert_eq!(neighbours(&grid, 2, 0, 0).unwrap(), [255; 2]);
        assert!(neighbours(&grid, 0, 0, 0).is_err());
        assert!(neighbours(&grid, 2, 2, 0).is_err());
    }
    #[test]
    fn address_owned_cells_keep_cross_macroblock_neighbours() {
        let luma: Vec<u8> = (0..64).collect();
        assert_eq!(block_neighbours(&luma, 2, 4, 4, 4).unwrap(), [35, 28]);
        assert_eq!(block_neighbours(&luma, 2, 4, 5, 5).unwrap(), [52, 49]);
        assert_eq!(block_neighbours(&luma, 2, 4, 0, 0).unwrap(), [255; 2]);
        let chroma: Vec<u8> = (0..16).collect();
        assert_eq!(block_neighbours(&chroma, 2, 2, 2, 2).unwrap(), [9, 6]);
        let mut flags = vec![255; 64];
        flags[35] = 1;
        flags[28] = 0;
        assert_eq!(block_coded_context(&flags, 2, 4, 4, 4, true).unwrap(), 1);
        flags[28] = 255;
        assert_eq!(block_coded_context(&flags, 2, 4, 4, 4, true).unwrap(), 3);
        assert_eq!(block_coded_context(&flags, 2, 4, 4, 4, false).unwrap(), 1);
    }
    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    #[test]
    fn real_cabac_p_inter_residual_reconstructs_changed_luma() {
        use super::super::{avc_cabac_inter as inter, avc_cabac_motion::CabacMotionContexts};
        let sps = Sps::parse(&hex("674d400ad91e84000003000400000300c83c489920")).unwrap();
        let pps = Pps::parse(&hex("68eb81b2c8"), &sps).unwrap();
        let header = SliceHeader::parse(&hex("419a39ff5d2e09a431c3d011f0"), &sps, &pps).unwrap();
        let mut reader = IntraCabacReader::new_context(&header, &sps, &pps, 1).unwrap();
        assert!(!inter::skip(reader.arithmetic().unwrap(), SliceType::P, [false; 2]).unwrap());
        let code =
            inter::macroblock_type(reader.arithmetic().unwrap(), SliceType::P, [false; 2]).unwrap();
        assert_eq!(code, 0);
        let mut motion = CabacMotionContexts::new(1, 1, 4096).unwrap();
        let parts = motion
            .read_prediction(
                reader.arithmetic().unwrap(),
                0,
                0,
                SliceType::P,
                code,
                [header.refs_l0, header.refs_l1],
            )
            .unwrap();
        assert_eq!(parts[0].differences, [[0; 2]; 2]);
        let (control, coefficients) = reader.read_inter_residual(&parts).unwrap();
        assert!(reader.is_finished());
        assert!(!control.transform8);
        // Saved stream encodes flat Y=64 followed by Y=66 at QP=20.
        for levels in coefficients.luma4 {
            let residual = super::super::avc_transform::residual_4x4(
                &levels,
                control.qp as u8,
                8,
                &[16; 16],
                None,
            )
            .unwrap();
            let pixels =
                super::super::avc_transform::reconstruct_4x4(&[64; 16], &residual, 8).unwrap();
            assert_eq!(pixels, [66; 16]);
        }
        assert_eq!(coefficients.chroma_dc, [[0; 4]; 2]);
        assert_eq!(coefficients.chroma_ac, [[[0; 16]; 4]; 2]);
    }
    #[test]
    fn embedded_intra_in_real_p_slice_uses_shared_arithmetic_state() {
        use super::super::avc_cabac_inter as inter;
        let sps = Sps::parse(&hex("674d400ad91e84000003000400000300c83c489920")).unwrap();
        let pps = Pps::parse(&hex("68eb81b2c8"), &sps).unwrap();
        let header = SliceHeader::parse(&hex("419a39ff91cfbf81f1f29b"), &sps, &pps).unwrap();
        let mut reader = IntraCabacReader::new_context(&header, &sps, &pps, 1).unwrap();
        assert!(!inter::skip(reader.arithmetic().unwrap(), SliceType::P, [false; 2]).unwrap());
        let code =
            inter::macroblock_type(reader.arithmetic().unwrap(), SliceType::P, [false; 2]).unwrap();
        assert!(code >= 5, "fixture must contain embedded intra: {code}");
        let mb = reader.read_embedded(code - 5).unwrap().unwrap();
        assert!(reader.is_finished());
        assert_eq!(mb.address, 0);
        let mut picture = super::super::avc_picture::IntraPicture {
            coded_width: 16,
            coded_height: 16,
            crop: [0; 4],
            bit_depth: 8,
            y: vec![0; 256],
            cb: vec![0; 64],
            cr: vec![0; 64],
        };
        super::super::avc_picture::reconstruct_macroblock(
            &mut picture,
            &mb,
            &sps,
            &pps,
            &crate::codec::avc_scaling::ScalingMatrices::new(&sps, &pps).unwrap(),
            &mut [0; 16],
        )
        .unwrap();
        assert_eq!(picture.y, vec![235; 256]);
        assert_eq!(picture.cb, vec![128; 64]);
        assert_eq!(picture.cr, vec![128; 64]);
    }
    #[test]
    fn real_p_skip_blocks_publish_context_and_terminate() {
        let sps = Sps::parse(&hex("674d400ad9096c0440000003004000000c83c48992")).unwrap();
        let pps = Pps::parse(&hex("68eb83cb20"), &sps).unwrap();
        let header = SliceHeader::parse(&hex("419a390afffe56"), &sps, &pps).unwrap();
        let mut reader = IntraCabacReader::new_context(&header, &sps, &pps, 4).unwrap();
        for at in 0..4 {
            assert!(
                super::super::avc_cabac_inter::skip(
                    reader.arithmetic().unwrap(),
                    SliceType::P,
                    [false; 2]
                )
                .unwrap()
            );
            reader
                .record_inter(&InterNeighbourContext {
                    qp: header.slice_qp,
                    qp_delta_nonzero: false,
                    pattern: 0,
                    transform8: false,
                    luma_coded: [false; 16],
                    chroma_dc: [false; 2],
                    chroma_ac: [[false; 4]; 2],
                })
                .unwrap();
            assert_eq!(reader.address(), at + 1);
            assert_eq!(reader.is_finished(), at == 3);
        }
        assert_eq!(reader.modes, vec![2; 64]);
        assert_eq!(reader.luma, vec![0; 64]);
        assert!(reader.arithmetic().is_err());
    }
    #[test]
    fn saved_main_profile_picture_and_truncated_payloads() {
        // One 16x16 gray IDR, generated by the benchmark encoder. Runtime decoding
        // and this test need no external executable or library.
        let sps = Sps::parse(&hex("674d400addec044000000300400000030083c489e0")).unwrap();
        let pps = Pps::parse(&hex("68ee0f2c80"), &sps).unwrap();
        let nal = hex("65888404bffef7addf813683");
        let header = SliceHeader::parse(&nal, &sps, &pps).unwrap();
        let picture =
            super::super::avc_picture::decode_intra_picture(&header, &sps, &pps, 4096).unwrap();
        assert_eq!(picture.dimensions(), (16, 16));
        assert_eq!(picture.y, vec![126; 256]);
        assert_eq!(picture.cb, vec![128; 64]);
        assert_eq!(picture.cr, vec![128; 64]);
        for end in 0..nal.len() {
            if let Ok(header) = SliceHeader::parse(&nal[..end], &sps, &pps) {
                assert!(
                    super::super::avc_picture::decode_intra_picture(&header, &sps, &pps, 4096)
                        .is_err(),
                    "accepted truncation {end}"
                );
            }
        }
        assert!(IntraCabacReader::new(&header, &sps, &pps, 0).is_err());
        for index in 1..nal.len() {
            for mask in [1, 16, 128] {
                let mut damaged = nal.clone();
                damaged[index] ^= mask;
                if let Ok(header) = SliceHeader::parse(&damaged, &sps, &pps) {
                    let _ =
                        super::super::avc_picture::decode_intra_picture(&header, &sps, &pps, 4096);
                }
            }
        }
    }
}
fn neighbours(grid: &[u8], width: usize, x: usize, y: usize) -> Result<[u8; 2]> {
    if width == 0 || grid.len() % width != 0 || x >= width {
        return Err(invalid("invalid CABAC macroblock context geometry"));
    }
    let address = y
        .checked_mul(width)
        .and_then(|v| v.checked_add(x))
        .ok_or_else(|| invalid("CABAC macroblock context address overflow"))?;
    let neighbours = super::avc_mbaff::macroblock_neighbours(
        address,
        width,
        grid.len() / width,
        false,
        [1, 1],
        |_| None,
    )?;
    Ok(neighbours.map(|n| n.map_or(255, |owner| grid[owner])))
}
fn coded_context(grid: &[u8], width: usize, x: usize, y: usize, intra: bool) -> Result<u8> {
    let [a, b] = neighbours(grid, width, x, y)?;
    Ok(u8::from(a != 0 && (a != 255 || intra)) + 2 * u8::from(b != 0 && (b != 255 || intra)))
}
// Store component blocks by macroblock address and local raster cell. This
// separates storage ownership from the picture's spatial neighbour geometry.
fn block_index(width_mbs: usize, side: usize, x: usize, y: usize) -> usize {
    (y / side * width_mbs + x / side) * side * side + y % side * side + x % side
}
fn block_neighbours(
    grid: &[u8],
    width_mbs: usize,
    side: usize,
    x: usize,
    y: usize,
) -> Result<[u8; 2]> {
    if width_mbs == 0 || !matches!(side, 2 | 4) || grid.len() % (width_mbs * side * side) != 0 {
        return Err(invalid("invalid CABAC component context geometry"));
    }
    let height_mbs = grid.len() / (width_mbs * side * side);
    let neighbours = super::avc_mbaff::block_neighbours(
        y / side * width_mbs + x / side,
        [x % side, y % side],
        width_mbs,
        height_mbs,
        false,
        [4 / side; 2],
        |_| None,
    )?;
    Ok(neighbours.map(|n| {
        n.map_or(255, |(owner, local)| {
            grid[owner * side * side + local[1] * side + local[0]]
        })
    }))
}
fn block_coded_context(
    grid: &[u8],
    width_mbs: usize,
    side: usize,
    x: usize,
    y: usize,
    intra: bool,
) -> Result<u8> {
    let [a, b] = block_neighbours(grid, width_mbs, side, x, y)?;
    Ok(u8::from(a != 0 && (a != 255 || intra)) + 2 * u8::from(b != 0 && (b != 255 || intra)))
}
impl<'a> IntraCabacReader<'a> {
    pub fn new(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        if header.slice_type != SliceType::I {
            return Err(invalid("CABAC intra reader requires an I slice"));
        }
        Self::new_context(header, sps, pps, max_macroblocks)
    }
    /// Shared arithmetic and neighbour state for an I/P/B slice. Mixed-slice
    /// dispatch must supply decoded inter context through `record_inter`.
    pub fn new_context(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        Self::new_context_impl(header, sps, pps, max_macroblocks, false)
    }
    /// Explicit MBAFF intra syntax reader; mixed P/B dispatch is separate.
    pub fn new_mbaff(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        if header.slice_type != SliceType::I || sps.frame_mbs_only || !sps.mb_adaptive_frame_field {
            return Err(invalid("MBAFF CABAC reader requires an intra frame slice"));
        }
        Self::new_context_impl(header, sps, pps, max_macroblocks, true)
    }
    fn new_context_impl(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
        mbaff: bool,
    ) -> Result<Self> {
        if !matches!(
            header.slice_type,
            SliceType::I | SliceType::P | SliceType::B
        ) || header.field_pic
            || !pps.cabac
            || (!sps.frame_mbs_only && !mbaff)
            || sps.chroma_format != 1
            || sps.separate_colour_plane
            || !matches!(pps.slice_groups, SliceGroups::Single)
        {
            return Err(invalid(
                "CABAC context requires progressive 4:2:0 I/P/B slices without FMO",
            ));
        }
        if header.pps_id != pps.id || pps.sps_id != sps.id {
            return Err(invalid("slice parameter-set mismatch"));
        }
        let count = (sps.width_mbs as usize)
            .checked_mul(sps.height_map_units as usize)
            .and_then(|n| n.checked_mul(if mbaff { 2 } else { 1 }))
            .ok_or_else(|| invalid("macroblock count overflow"))?;
        if count == 0 || count > max_macroblocks || count > 65536 {
            return Err(invalid("macroblock context budget exceeded"));
        }
        let grid = |n| -> Result<Vec<u8>> {
            let mut v = crate::buffer(n)?;
            v.fill(255);
            Ok(v)
        };
        Ok(Self {
            cabac: AvcCabac::new(
                &header.rbsp,
                header.entropy_bit_offset,
                header.slice_type,
                header.cabac_init_idc as u8,
                header.slice_qp,
            )?,
            sps,
            pps,
            address: (header.first_mb as usize)
                .checked_mul(if mbaff { 2 } else { 1 })
                .filter(|a| *a < count)
                .ok_or_else(|| invalid("CABAC first macroblock outside picture"))?,
            qp: header.slice_qp,
            previous_delta: false,
            slice_type: header.slice_type,
            finished: false,
            types: grid(count)?,
            eight: crate::buffer(count)?,
            patterns: grid(count)?,
            chroma_modes: grid(count)?,
            dc: [grid(count)?, grid(count)?, grid(count)?],
            luma: grid(count * 16)?,
            chroma: [grid(count * 4)?, grid(count * 4)?],
            modes: grid(count * 16)?,
            mbaff,
            pair_fields: if mbaff { grid(count / 2)? } else { Vec::new() },
        })
    }
    fn pair_field(&self, pair: usize) -> Option<bool> {
        self.pair_fields
            .get(pair)
            .filter(|v| **v != 255)
            .map(|v| *v != 0)
    }
    fn current_field(&self) -> bool {
        self.mbaff && self.pair_field(self.address / 2) == Some(true)
    }
    pub fn field_decoding(&self) -> bool {
        self.mbaff
            && self
                .address
                .checked_sub(1)
                .and_then(|a| self.pair_field(a / 2))
                == Some(true)
    }
    fn macro_neighbours(&self, grid: &[u8]) -> Result<[u8; 2]> {
        let w = self.sps.width_mbs as usize;
        if !self.mbaff {
            return neighbours(grid, w, self.address % w, self.address / w);
        }
        Ok(super::avc_mbaff::macroblock_neighbours(
            self.address,
            w,
            self.types.len() / w,
            true,
            [1, 1],
            |pair| self.pair_field(pair),
        )?
        .map(|n| n.map_or(255, |owner| grid[owner])))
    }
    fn component_neighbours(
        &self,
        grid: &[u8],
        side: usize,
        x: usize,
        y: usize,
    ) -> Result<[u8; 2]> {
        let w = self.sps.width_mbs as usize;
        if !self.mbaff {
            return block_neighbours(grid, w, side, x, y);
        }
        Ok(super::avc_mbaff::block_neighbours(
            self.address,
            [x % side, y % side],
            w,
            self.types.len() / w,
            true,
            [4 / side; 2],
            |pair| self.pair_field(pair),
        )?
        .map(|n| {
            n.map_or(255, |(owner, local)| {
                grid[owner * side * side + local[1] * side + local[0]]
            })
        }))
    }
    fn macro_coded_context(&self, grid: &[u8], intra: bool) -> Result<u8> {
        if !self.mbaff {
            let w = self.sps.width_mbs as usize;
            return coded_context(grid, w, self.address % w, self.address / w, intra);
        }
        let [a, b] = self.macro_neighbours(grid)?;
        Ok(u8::from(a != 0 && (a != 255 || intra)) + 2 * u8::from(b != 0 && (b != 255 || intra)))
    }
    fn component_coded_context(
        &self,
        grid: &[u8],
        side: usize,
        x: usize,
        y: usize,
        intra: bool,
    ) -> Result<u8> {
        if !self.mbaff {
            return block_coded_context(grid, self.sps.width_mbs as usize, side, x, y, intra);
        }
        let [a, b] = self.component_neighbours(grid, side, x, y)?;
        Ok(u8::from(a != 0 && (a != 255 || intra)) + 2 * u8::from(b != 0 && (b != 255 || intra)))
    }
    pub fn arithmetic(&mut self) -> Result<&mut AvcCabac<'a>> {
        if self.finished {
            return Err(invalid("CABAC slice already finished"));
        }
        Ok(&mut self.cabac)
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }
    pub fn address(&self) -> usize {
        self.address
    }
    pub fn qp(&self) -> i32 {
        self.qp
    }
    /// Publish an inter/skip block's entropy context and consume its end flag.
    /// `luma_coded` is raster 4x4 order; for 8x8 each covered cell receives the
    /// inferred flag. Discard the reader after any error, including termination.
    pub fn record_inter(&mut self, block: &InterNeighbourContext) -> Result<()> {
        if self.finished || self.slice_type == SliceType::I || self.address >= self.types.len() {
            return Err(invalid("invalid CABAC inter context position"));
        }
        super::avc_residual_syntax::update_qp(block.qp, 0, self.sps.bit_depth_luma)?;
        if block.pattern > 47 {
            return Err(invalid("invalid CABAC inter coded block pattern"));
        }
        let w = self.sps.width_mbs as usize;
        let at = self.address;
        let (mx, my) = (at % w, at / w);
        self.types[at] = 254;
        self.patterns[at] = block.pattern;
        self.eight[at] = u8::from(block.transform8);
        self.chroma_modes[at] = 0;
        self.dc[0][at] = 0;
        for c in 0..2 {
            self.dc[c + 1][at] = u8::from(block.chroma_dc[c]);
        }
        for y in 0..4 {
            for x in 0..4 {
                let p = block_index(w, 4, mx * 4 + x, my * 4 + y);
                self.luma[p] = u8::from(block.luma_coded[y * 4 + x]);
                self.modes[p] = if self.pps.constrained_intra_pred {
                    255
                } else {
                    2
                };
            }
        }
        for c in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    self.chroma[c][block_index(w, 2, mx * 2 + x, my * 2 + y)] =
                        u8::from(block.chroma_ac[c][y * 2 + x]);
                }
            }
        }
        self.qp = block.qp;
        self.previous_delta = block.qp_delta_nonzero;
        self.end_mb()
    }
    fn bin(&mut self, index: usize) -> Result<u8> {
        Ok(u8::from(self.cabac.decision(index)?))
    }
    fn end_mb(&mut self) -> Result<()> {
        let terminal =
            super::avc_cabac_inter::end_of_slice_flag(&mut self.cabac, self.address, self.mbaff)?;
        self.address += 1;
        if terminal {
            self.cabac.finish_slice()?;
            self.finished = true;
        }
        Ok(())
    }
    pub fn read_macroblock(&mut self) -> Result<Option<IntraMacroblock>> {
        if self.slice_type != SliceType::I {
            return Err(invalid("mixed CABAC requires macroblock dispatch"));
        }
        if self.finished {
            return Ok(None);
        }
        let w = self.sps.width_mbs as usize;
        let at = self.address;
        if at >= self.types.len() {
            return Err(invalid("too many CABAC macroblocks"));
        }
        if self.mbaff {
            let pair = at / 2;
            if at % 2 == 0 {
                let left = pair % w != 0 && self.pair_field(pair - 1) == Some(true);
                let top = pair.checked_sub(w).and_then(|p| self.pair_field(p)) == Some(true);
                let field =
                    super::avc_cabac_inter::field_decoding_flag(&mut self.cabac, [left, top])?;
                self.pair_fields[pair] = u8::from(field);
            } else if self.pair_field(pair).is_none() {
                return Err(invalid("MBAFF CABAC bottom block lacks pair field flag"));
            }
        }
        let [a, b] = self.macro_neighbours(&self.types)?;
        let inc = usize::from(a != 255 && a != 0) + usize::from(b != 255 && b != 0);
        let mb_type = if self.bin(3 + inc)? == 0 {
            0
        } else if self.cabac.terminate()? {
            25
        } else {
            let luma = self.bin(6)?;
            let chroma = if self.bin(7)? == 0 {
                0
            } else {
                1 + self.bin(8)?
            };
            let mode = 2 * self.bin(9)? + self.bin(10)?;
            1 + mode + 4 * chroma + 12 * luma
        };
        self.read_embedded(mb_type)
    }
    /// Read intra prediction/residual syntax after mb_type has been decoded
    /// and mapped to the I table. Also consumes end_of_slice_flag.
    pub fn read_embedded(&mut self, mb_type: u8) -> Result<Option<IntraMacroblock>> {
        if self.finished {
            return Ok(None);
        }
        let w = self.sps.width_mbs as usize;
        let at = self.address;
        if at >= self.types.len() || mb_type > 25 {
            return Err(invalid("invalid embedded CABAC intra macroblock"));
        }
        let (mx, my) = (at % w, at / w);
        self.types[at] = mb_type;
        let mut mb = IntraMacroblock {
            address: at as u32,
            qp: self.qp,
            luma: IntraLuma::Block16(0),
            chroma_mode: ChromaMode::Dc,
            coded_block_pattern: 0,
            luma_dc: [0; 16],
            luma_levels: [[0; 16]; 16],
            chroma_dc: [[0; 4]; 2],
            chroma_ac: [[[0; 16]; 4]; 2],
        };
        if mb_type == 25 {
            mb.luma = self
                .cabac
                .pcm(self.sps.bit_depth_luma, self.sps.bit_depth_chroma)?;
            mb.qp = 0;
            self.previous_delta = false;
            self.patterns[at] = 47;
            self.chroma_modes[at] = 0;
            for dc in &mut self.dc {
                dc[at] = 1;
            }
            for y in 0..4 {
                for x in 0..4 {
                    let p = block_index(w, 4, mx * 4 + x, my * 4 + y);
                    self.modes[p] = 2;
                    self.luma[p] = 1;
                }
            }
            for plane in &mut self.chroma {
                for y in 0..2 {
                    for x in 0..2 {
                        plane[block_index(w, 2, mx * 2 + x, my * 2 + y)] = 1;
                    }
                }
            }
            self.end_mb()?;
            return Ok(Some(mb));
        }
        if mb_type == 0 {
            let [a, b] = self.macro_neighbours(&self.eight)?;
            let inc = usize::from(a == 1) + usize::from(b == 1);
            let eight = self.pps.transform_8x8 && self.bin(399 + inc)? != 0;
            self.eight[at] = u8::from(eight);
            let mut modes = [Intra4Mode::Dc; 16];
            for block in 0..if eight { 4 } else { 16 } {
                let (bx, by) = if eight {
                    (block % 2 * 2, block / 2 * 2)
                } else {
                    luma_block_xy(block)?
                };
                let (x, y) = (mx * 4 + bx, my * 4 + by);
                let [a, b] = self.component_neighbours(&self.modes, 4, x, y)?;
                let predicted = self.bin(68)? != 0;
                let remainder = if predicted {
                    0
                } else {
                    self.bin(69)? | self.bin(69)? << 1 | self.bin(69)? << 2
                };
                let mode = derive_intra4_mode(
                    Intra4Mode::try_from(a).ok(),
                    Intra4Mode::try_from(b).ok(),
                    predicted,
                    remainder,
                )?;
                for dy in 0..if eight { 2 } else { 1 } {
                    for dx in 0..if eight { 2 } else { 1 } {
                        self.modes[block_index(w, 4, x + dx, y + dy)] = mode as u8;
                    }
                }
                modes[by * 4 + bx] = mode;
            }
            mb.luma = if eight {
                IntraLuma::Blocks8 {
                    modes: [modes[0], modes[2], modes[8], modes[10]],
                    levels: [[0; 64]; 4],
                }
            } else {
                IntraLuma::Blocks4(modes)
            };
        } else {
            mb.luma = IntraLuma::Block16((mb_type - 1) % 4);
            mb.coded_block_pattern = ((mb_type - 1) / 4 % 3) * 16 + (mb_type - 1) / 12 * 15;
            for y in 0..4 {
                for x in 0..4 {
                    self.modes[block_index(w, 4, mx * 4 + x, my * 4 + y)] = 2;
                }
            }
        }
        let [a, b] = self.macro_neighbours(&self.chroma_modes)?;
        let inc = usize::from(a != 255 && a != 0) + usize::from(b != 255 && b != 0);
        let mode = if self.bin(64 + inc)? == 0 {
            0
        } else if self.bin(67)? == 0 {
            1
        } else {
            2 + self.bin(67)?
        };
        self.chroma_modes[at] = mode;
        mb.chroma_mode = ChromaMode::try_from(mode)?;
        if mb_type == 0 {
            mb.coded_block_pattern = self.read_pattern()?;
        }
        self.read_coefficients(&mut mb, mb_type, true)?;
        self.end_mb()?;
        Ok(Some(mb))
    }
    /// CABAC inter CBP, transform size, QP and residuals after motion syntax.
    /// Commits coefficient context and consumes end_of_slice_flag.
    pub fn read_inter_residual(
        &mut self,
        partitions: &[super::avc_inter::Partition],
    ) -> Result<(
        super::avc_residual_syntax::InterResidualControl,
        super::avc_inter_coefficients::InterCoefficients,
    )> {
        if self.finished || self.slice_type == SliceType::I || self.address >= self.types.len() {
            return Err(invalid("invalid CABAC inter residual position"));
        }
        let at = self.address;
        let w = self.sps.width_mbs as usize;
        let (mx, my) = (at % w, at / w);
        let pattern = self.read_pattern()?;
        let [a, b] = self.macro_neighbours(&self.eight)?;
        let eight = pattern & 15 != 0
            && self.pps.transform_8x8
            && super::avc_inter::allows_transform8(partitions, self.sps.direct_8x8_inference)
            && self.bin(399 + usize::from(a == 1) + usize::from(b == 1))? != 0;
        self.eight[at] = u8::from(eight);
        let mut mb = IntraMacroblock {
            address: at as u32,
            qp: self.qp,
            coded_block_pattern: pattern,
            luma: if eight {
                IntraLuma::Blocks8 {
                    modes: [Intra4Mode::Dc; 4],
                    levels: [[0; 64]; 4],
                }
            } else {
                IntraLuma::Blocks4([Intra4Mode::Dc; 16])
            },
            chroma_mode: ChromaMode::Dc,
            luma_dc: [0; 16],
            luma_levels: [[0; 16]; 16],
            chroma_dc: [[0; 4]; 2],
            chroma_ac: [[[0; 16]; 4]; 2],
        };
        self.read_coefficients(&mut mb, 0, false)?;
        self.types[at] = 254;
        self.chroma_modes[at] = 0;
        for y in 0..4 {
            for x in 0..4 {
                self.modes[block_index(w, 4, mx * 4 + x, my * 4 + y)] =
                    if self.pps.constrained_intra_pred {
                        255
                    } else {
                        2
                    };
            }
        }
        let coefficients = super::avc_inter_coefficients::InterCoefficients {
            luma_counts: mb
                .luma_levels
                .map(|block| block.iter().filter(|&&v| v != 0).count() as u8),
            chroma_counts: mb
                .chroma_ac
                .map(|plane| plane.map(|block| block.iter().filter(|&&v| v != 0).count() as u8)),
            luma4: mb.luma_levels,
            luma8: match mb.luma {
                IntraLuma::Blocks8 { levels, .. } => levels,
                _ => [[0; 64]; 4],
            },
            chroma_dc: mb.chroma_dc,
            chroma_ac: mb.chroma_ac,
        };
        self.end_mb()?;
        Ok((
            super::avc_residual_syntax::InterResidualControl {
                pattern,
                transform8: eight,
                qp: mb.qp,
            },
            coefficients,
        ))
    }
    fn read_pattern(&mut self) -> Result<u8> {
        let mut pattern = 0;
        let [a, b] = self.macro_neighbours(&self.patterns)?;
        for block in 0..4 {
            let bx = block % 2;
            let by = block / 2;
            let left = if bx > 0 {
                Some((pattern >> (block - 1)) & 1)
            } else if a != 255 {
                Some((a >> (by * 2 + 1)) & 1)
            } else {
                None
            };
            let top = if by > 0 {
                Some((pattern >> (block - 2)) & 1)
            } else if b != 255 {
                Some((b >> (2 + bx)) & 1)
            } else {
                None
            };
            let inc = usize::from(left == Some(0)) + 2 * usize::from(top == Some(0));
            pattern |= self.bin(73 + inc)? << block;
        }
        let inc = usize::from(a != 255 && a >> 4 != 0) + 2 * usize::from(b != 255 && b >> 4 != 0);
        if self.bin(77 + inc)? != 0 {
            let inc =
                usize::from(a != 255 && a >> 4 == 2) + 2 * usize::from(b != 255 && b >> 4 == 2);
            pattern |= (1 + self.bin(81 + inc)?) << 4;
        }
        Ok(pattern)
    }
    fn read_coefficients(
        &mut self,
        mb: &mut IntraMacroblock,
        mb_type: u8,
        intra: bool,
    ) -> Result<()> {
        let w = self.sps.width_mbs as usize;
        let at = self.address;
        let field = self.current_field();
        let (mx, my) = (at % w, at / w);
        self.patterns[at] = mb.coded_block_pattern;
        let offset = 6 * (i32::from(self.sps.bit_depth_luma) - 8);
        if mb_type != 0 || mb.coded_block_pattern != 0 {
            let mut value = i32::from(self.bin(60 + usize::from(self.previous_delta))?);
            if value != 0 {
                while self.bin(if value == 1 { 62 } else { 63 })? != 0 {
                    value += 1;
                    if value > 52 + offset {
                        return Err(invalid("CABAC mb_qp_delta exceeds range"));
                    }
                }
            }
            let delta = if value % 2 == 0 {
                -value / 2
            } else {
                (value + 1) / 2
            };
            if !(-(26 + offset / 2)..=25 + offset / 2).contains(&delta) {
                return Err(invalid("CABAC mb_qp_delta out of range"));
            }
            self.qp = (self.qp + delta + 52 + 2 * offset) % (52 + offset) - offset;
            self.previous_delta = delta != 0;
        } else {
            self.previous_delta = false;
        }
        mb.qp = self.qp;
        self.dc[0][at] = 0;
        if mb_type != 0 {
            let inc = self.macro_coded_context(&self.dc[0], intra)?;
            let r = self.cabac.residual(Cat::LumaDc, inc, field, false)?;
            self.dc[0][at] = u8::from(r.total_coefficients != 0);
            mb.luma_dc = inverse_scan_4x4(&r.coefficients, field);
        }
        if let IntraLuma::Blocks8 { levels, .. } = &mut mb.luma {
            for block in 0..4 {
                let coded = mb.coded_block_pattern & (1 << block) != 0;
                if coded {
                    levels[block] = super::avc_transform8::inverse_scan_8x8(
                        &self.cabac.residual8(field)?.coefficients,
                        field,
                    );
                }
                for by in 0..2 {
                    for bx in 0..2 {
                        self.luma[block_index(
                            w,
                            4,
                            mx * 4 + block % 2 * 2 + bx,
                            my * 4 + block / 2 * 2 + by,
                        )] = u8::from(coded);
                    }
                }
            }
        } else {
            for block in 0..16 {
                let (bx, by) = luma_block_xy(block)?;
                let (x, y) = (mx * 4 + bx, my * 4 + by);
                let mut coded = 0;
                if mb.coded_block_pattern & (1 << (block / 4)) != 0 {
                    let inc = self.component_coded_context(&self.luma, 4, x, y, intra)?;
                    let r = self.cabac.residual(
                        if mb_type == 0 {
                            Cat::Luma4
                        } else {
                            Cat::LumaAc
                        },
                        inc,
                        field,
                        false,
                    )?;
                    coded = u8::from(r.total_coefficients != 0);
                    let mut levels = r.coefficients;
                    if mb_type != 0 {
                        levels.copy_within(0..15, 1);
                        levels[0] = 0;
                    }
                    mb.luma_levels[by * 4 + bx] = inverse_scan_4x4(&levels, field);
                }
                self.luma[block_index(w, 4, x, y)] = coded;
            }
        }
        for component in 0..2 {
            let inc = self.macro_coded_context(&self.dc[component + 1], intra)?;
            self.dc[component + 1][at] = 0;
            if mb.coded_block_pattern >> 4 != 0 {
                let r = self.cabac.residual(Cat::ChromaDc, inc, field, false)?;
                self.dc[component + 1][at] = u8::from(r.total_coefficients != 0);
                mb.chroma_dc[component].copy_from_slice(&r.coefficients[..4]);
            }
        }
        for component in 0..2 {
            for block in 0..4 {
                let (x, y) = (mx * 2 + block % 2, my * 2 + block / 2);
                let mut coded = 0;
                if mb.coded_block_pattern >> 4 == 2 {
                    let inc =
                        self.component_coded_context(&self.chroma[component], 2, x, y, intra)?;
                    let r = self.cabac.residual(Cat::ChromaAc, inc, field, false)?;
                    coded = u8::from(r.total_coefficients != 0);
                    let mut levels = [0; 16];
                    levels[1..].copy_from_slice(&r.coefficients[..15]);
                    mb.chroma_ac[component][block] = inverse_scan_4x4(&levels, field);
                }
                self.chroma[component][block_index(w, 2, x, y)] = coded;
            }
        }
        Ok(())
    }
}
