//! Frame/field/MBAFF 4:2:0 intra syntax and mixed CAVLC contexts.
//! This yields prediction modes and transform levels, not reconstructed or filtered pictures.
use super::{
    avc::{Pps, SliceGroups, Sps},
    avc_intra::{ChromaMode, Intra4Mode, derive_intra4_mode},
    avc_slice::{SliceHeader, SliceType},
    avc_transform::inverse_scan_4x4,
    bits::BitReader,
    cavlc::read_residual,
};
use crate::{Result, invalid};
#[derive(Debug)]
pub enum IntraLuma {
    Blocks4([Intra4Mode; 16]),
    Blocks8 {
        modes: [Intra4Mode; 4],
        levels: [[i32; 64]; 4],
    },
    Block16(u8),
    Pcm {
        y: [u16; 256],
        cb: [u16; 64],
        cr: [u16; 64],
    },
}
#[derive(Debug)]
pub struct IntraMacroblock {
    pub address: u32,
    pub qp: i32,
    /// Present only for the switching SI macroblock, not ordinary I types in SI slices.
    pub switching_qs: Option<u8>,
    pub luma: IntraLuma,
    pub chroma_mode: ChromaMode,
    pub coded_block_pattern: u8,
    /// Raster order for both 4x4 blocks and coefficients. DC is separate for Block16.
    pub luma_dc: [i32; 16],
    pub luma_levels: [[i32; 16]; 16],
    pub chroma_dc: [[i32; 4]; 2],
    pub chroma_ac: [[[i32; 16]; 4]; 2],
}
/// Luma block scan index -> raster block coordinate within a macroblock.
pub fn luma_block_xy(index: usize) -> Result<(usize, usize)> {
    if index >= 16 {
        return Err(invalid("luma block index out of range"));
    }
    Ok((
        (index & 1) + ((index >> 2) & 1) * 2,
        ((index >> 1) & 1) + (index >> 3) * 2,
    ))
}
pub struct IntraCavlcReader<'a> {
    bits: BitReader<'a>,
    sps: &'a Sps,
    pps: &'a Pps,
    address: u32,
    previous_address: Option<u32>,
    qp: i32,
    luma_counts: Vec<u8>,
    chroma_counts: [Vec<u8>; 2],
    modes: Vec<u8>,
    finished: bool,
    mbaff: bool,
    field_picture: bool,
    pair_fields: Vec<u8>,
    slice_group_map: Vec<u8>,
    si_qs: Option<u8>,
}
impl<'a> IntraCavlcReader<'a> {
    pub fn new(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        if !matches!(header.slice_type, SliceType::I | SliceType::Si) {
            return Err(invalid("intra reader requires an I slice"));
        }
        Self::new_context(header, sps, pps, max_macroblocks)
    }
    /// Shared intra context for mixed P/B slices. Caller owns macroblock dispatch.
    pub fn new_context(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        Self::new_context_impl(header, sps, pps, max_macroblocks, false, false)
    }
    /// Syntax-only intra MBAFF reader. Reconstruction is a separate pipeline.
    pub fn new_mbaff(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        if header.slice_type != SliceType::I {
            return Err(invalid("MBAFF CAVLC reader requires an intra frame slice"));
        }
        Self::new_context_mbaff(header, sps, pps, max_macroblocks)
    }
    /// Pair-address context for an external mixed-slice dispatcher.
    pub fn new_context_mbaff(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        if !matches!(
            header.slice_type,
            SliceType::I | SliceType::P | SliceType::B
        ) || header.field_pic
            || sps.frame_mbs_only
            || !sps.mb_adaptive_frame_field
        {
            return Err(invalid("MBAFF CAVLC reader requires an intra frame slice"));
        }
        Self::new_context_impl(header, sps, pps, max_macroblocks, true, false)
    }
    fn new_context_impl(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
        mbaff: bool,
        allow_fmo: bool,
    ) -> Result<Self> {
        if header.slice_type == SliceType::Si
            && (sps.profile != 88
                || sps.mb_adaptive_frame_field && !header.field_pic
                || sps.bit_depth_luma != 8
                || sps.bit_depth_chroma != 8
                || pps.transform_8x8
                || sps.transform_bypass)
        {
            return Err(crate::unsupported(
                "AVC SI requires non-MBAFF eight-bit Extended profile",
            ));
        }
        if pps.cabac
            || (!mbaff && sps.mb_adaptive_frame_field && !header.field_pic)
            || sps.chroma_format != 1
            || sps.separate_colour_plane
            || (!allow_fmo && !matches!(pps.slice_groups, SliceGroups::Single))
        {
            return Err(invalid(
                "intra CAVLC reader requires admitted 4:2:0 geometry without FMO or CABAC",
            ));
        }
        if header.pps_id != pps.id || pps.sps_id != sps.id {
            return Err(invalid("slice parameter-set mismatch"));
        }
        let count = (sps.width_mbs as usize)
            .checked_mul(sps.height_map_units as usize)
            .and_then(|n| {
                n.checked_mul(if !sps.frame_mbs_only && !header.field_pic {
                    2
                } else {
                    1
                })
            })
            .ok_or_else(|| invalid("macroblock count overflow"))?;
        if count == 0 || count > max_macroblocks || count > 65536 {
            return Err(invalid("macroblock context budget exceeded"));
        }
        let slice_group_map = if matches!(pps.slice_groups, SliceGroups::Single) {
            Vec::new()
        } else {
            let map = super::avc_slice_group_map::map_units(
                &pps.slice_groups,
                sps.width_mbs as usize,
                sps.height_map_units as usize,
                header.slice_group_change_cycle,
                count,
            )?;
            super::avc_slice_group_map::macroblocks(
                &map,
                sps.width_mbs as usize,
                sps.frame_mbs_only,
                header.field_pic,
                mbaff,
                count,
            )?
        };
        let mut bits = BitReader::new(&header.rbsp);
        bits.skip(header.entropy_bit_offset)?;
        let grid = |size| -> Result<Vec<u8>> {
            let mut data = crate::buffer(size)?;
            data.fill(255);
            Ok(data)
        };
        Ok(Self {
            bits,
            sps,
            pps,
            address: header
                .first_mb
                .checked_mul(if mbaff { 2 } else { 1 })
                .ok_or_else(|| invalid("MBAFF slice address overflow"))?,
            previous_address: None,
            qp: header.slice_qp,
            luma_counts: grid(count * 16)?,
            chroma_counts: [grid(count * 4)?, grid(count * 4)?],
            modes: grid(count * 16)?,
            finished: false,
            mbaff,
            field_picture: header.field_pic,
            pair_fields: grid(if mbaff { count / 2 } else { 0 })?,
            slice_group_map,
            si_qs: if header.slice_type == SliceType::Si {
                Some(header.slice_qs.ok_or_else(|| invalid("missing SI QS"))? as u8)
            } else {
                None
            },
        })
    }
    /// Syntax-only FMO I-slice reader. Picture reconstruction remains separate.
    pub fn new_fmo(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        if !matches!(header.slice_type, SliceType::I | SliceType::Si) {
            return Err(invalid("FMO reader requires an I slice"));
        }
        Self::new_context_impl(
            header,
            sps,
            pps,
            max_macroblocks,
            sps.mb_adaptive_frame_field && !sps.frame_mbs_only && !header.field_pic,
            true,
        )
    }
    /// FMO mixed-slice intra context; the external dispatcher supplies mb_type.
    pub fn new_context_fmo(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        if header.field_pic
            || !matches!(
                header.slice_type,
                SliceType::I | SliceType::P | SliceType::B
            )
        {
            return Err(invalid("invalid FMO mixed intra context"));
        }
        Self::new_context_impl(
            header,
            sps,
            pps,
            max_macroblocks,
            sps.mb_adaptive_frame_field && !sps.frame_mbs_only && !header.field_pic,
            true,
        )
    }
    fn advance_address(&mut self) -> Result<()> {
        self.previous_address = Some(self.address);
        self.address = if self.slice_group_map.is_empty() {
            self.address + 1
        } else {
            super::avc_slice_group_map::next_address(&self.slice_group_map, self.address as usize)?
                as u32
        };
        Ok(())
    }
    fn coefficient_context(
        &self,
        component: usize,
        address: usize,
        block: [usize; 2],
    ) -> Result<i8> {
        let width = self.sps.width_mbs as usize;
        let height = self.height_mbs();
        let (grid, side, sub) = if component == 0 {
            (&self.luma_counts, 4, [1, 1])
        } else {
            (&self.chroma_counts[component - 1], 2, [2, 2])
        };
        super::avc_mbaff::cavlc_context(
            address,
            block,
            width,
            height,
            self.mbaff,
            sub,
            |pair| self.pair_field(pair),
            |owner, local| {
                grid.get(owner * side * side + local[1] * side + local[0])
                    .copied()
                    .filter(|count| *count != 255)
            },
        )
    }
    fn mode_neighbours(
        &self,
        address: usize,
        block: [usize; 2],
    ) -> Result<(Option<u8>, Option<u8>)> {
        let mut values = [None; 2];
        let x = (block[0] * 4) as isize;
        let y = (block[1] * 4) as isize;
        for (value, offset) in values.iter_mut().zip([[x - 1, y], [x, y - 1]]) {
            if let Some((owner, local)) = super::avc_mbaff::neighbour_location(
                address,
                offset,
                self.sps.width_mbs as usize,
                self.height_mbs(),
                self.mbaff,
                [1, 1],
                |pair| self.pair_field(pair),
            )? {
                *value = self
                    .modes
                    .get(owner * 16 + local[1] / 4 * 4 + local[0] / 4)
                    .copied();
            }
        }
        Ok((values[0], values[1]))
    }
    fn height_mbs(&self) -> usize {
        self.sps.height_map_units as usize
            * if !self.sps.frame_mbs_only && !self.field_picture {
                2
            } else {
                1
            }
    }
    /// Parsed field mode for an MBAFF pair, or None before its flag is read.
    pub fn pair_field(&self, pair: usize) -> Option<bool> {
        self.pair_fields
            .get(pair)
            .copied()
            .filter(|v| *v != 255)
            .map(|v| v != 0)
    }
    /// Mode of the most recently parsed macroblock (false for progressive).
    pub fn field_decoding(&self) -> bool {
        self.field_picture
            || self.mbaff
                && self
                    .previous_address
                    .is_some_and(|a| self.pair_field(a as usize / 2) == Some(true))
    }
    pub fn bit_position(&self) -> usize {
        self.bits.position()
    }
    pub fn read_macroblock(&mut self) -> Result<Option<IntraMacroblock>> {
        if self.finished {
            return Ok(None);
        }
        if !self.bits.more_rbsp_data() {
            if self.mbaff && self.address % 2 != 0 {
                return Err(invalid("MBAFF slice ends with incomplete macroblock pair"));
            }
            self.bits.finish_rbsp()?;
            self.finished = true;
            return Ok(None);
        }
        if self.mbaff {
            let pair = self.address as usize / 2;
            if self.address % 2 == 0 {
                let slot = self
                    .pair_fields
                    .get_mut(pair)
                    .ok_or_else(|| invalid("MBAFF macroblock outside picture"))?;
                *slot = u8::from(self.bits.bit()?);
            } else if self.pair_field(pair).is_none() {
                return Err(invalid("MBAFF bottom macroblock lacks field flag"));
            }
        }
        let mb_type = self.bits.unsigned_golomb()?;
        let switching = self.si_qs.is_some() && mb_type == 0;
        let mapped = if self.si_qs.is_some() && !switching {
            mb_type
                .checked_sub(1)
                .ok_or_else(|| invalid("invalid SI macroblock type"))?
        } else {
            mb_type
        };
        let mut mb = self.read_body(mapped)?;
        if switching {
            if let Some(mb) = &mut mb {
                mb.switching_qs = self.si_qs;
            }
        }
        Ok(mb)
    }
    /// Parse after the mixed-slice dispatcher has consumed mb_type and mapped
    /// it to the I table (or retained the SI table). Discard this context on error; the caller cursor only
    /// advances after success.
    pub fn read_embedded(
        &mut self,
        bits: &mut BitReader<'a>,
        address: u32,
        qp: i32,
        mb_type: u32,
    ) -> Result<IntraMacroblock> {
        if self.mbaff {
            return Err(invalid(
                "embedded MBAFF mixed-slice syntax is not connected",
            ));
        }
        self.read_embedded_body(bits, address, qp, mb_type)
    }
    /// The dispatcher has consumed the pair flag and mb_type. On error discard
    /// this context; the external bit cursor advances only after success.
    pub fn read_embedded_mbaff(
        &mut self,
        bits: &mut BitReader<'a>,
        address: u32,
        qp: i32,
        mb_type: u32,
        field: bool,
    ) -> Result<IntraMacroblock> {
        self.record_pair_mode(address as usize, field)?;
        self.read_embedded_body(bits, address, qp, mb_type)
    }
    /// Publish the dispatcher-owned mode for coded or skipped macroblocks.
    /// Both members of a pair must use the same mode.
    pub fn record_pair_mode(&mut self, address: usize, field: bool) -> Result<()> {
        if !self.mbaff || address >= self.sps.width_mbs as usize * self.height_mbs() {
            return Err(invalid("invalid MBAFF intra-context address"));
        }
        let slot = &mut self.pair_fields[address / 2];
        if *slot != 255 && *slot != u8::from(field) {
            return Err(invalid("MBAFF intra-context pair mode changed"));
        }
        *slot = u8::from(field);
        Ok(())
    }
    fn read_embedded_body(
        &mut self,
        bits: &mut BitReader<'a>,
        address: u32,
        qp: i32,
        mb_type: u32,
    ) -> Result<IntraMacroblock> {
        super::avc_residual_syntax::update_qp(qp, 0, self.sps.bit_depth_luma)?;
        self.bits = bits.clone();
        self.address = address;
        self.qp = qp;
        // Embedded SI retains its own macroblock table: zero is SI,
        // while positive types map to the ordinary I table one entry earlier.
        let switching = self.si_qs.is_some() && mb_type == 0;
        let mapped = if self.si_qs.is_some() && !switching {
            mb_type - 1
        } else {
            mb_type
        };
        let mut mb = self
            .read_body(mapped)?
            .ok_or_else(|| invalid("missing embedded intra macroblock"))?;
        if switching {
            mb.switching_qs = self.si_qs;
        }
        *bits = self.bits.clone();
        Ok(mb)
    }
    /// Publish counts from an inter/skip block for the next intra neighbour.
    pub fn record_inter(
        &mut self,
        address: usize,
        luma: [u8; 16],
        chroma: [[u8; 4]; 2],
    ) -> Result<()> {
        let width = self.sps.width_mbs as usize;
        if address >= width * self.height_mbs()
            || luma.iter().chain(chroma.iter().flatten()).any(|&v| v > 16)
        {
            return Err(invalid("invalid mixed-slice coefficient counts"));
        }
        self.luma_counts[address * 16..address * 16 + 16].copy_from_slice(&luma);
        self.modes[address * 16..address * 16 + 16].fill(if self.pps.constrained_intra_pred {
            255
        } else {
            2
        });
        for c in 0..2 {
            self.chroma_counts[c][address * 4..address * 4 + 4].copy_from_slice(&chroma[c]);
        }
        Ok(())
    }
    pub fn counts(&self, address: usize) -> Result<([u8; 16], [[u8; 4]; 2])> {
        let width = self.sps.width_mbs as usize;
        if address >= width * self.height_mbs() {
            return Err(invalid("AVC count address out of range"));
        }
        Ok((
            std::array::from_fn(|i| self.luma_counts[address * 16 + i]),
            std::array::from_fn(|c| {
                std::array::from_fn(|i| self.chroma_counts[c][address * 4 + i])
            }),
        ))
    }

    fn read_body(&mut self, mb_type: u32) -> Result<Option<IntraMacroblock>> {
        let width = self.sps.width_mbs as usize;
        let height = self.height_mbs();
        let address = self.address as usize;
        let field = self.field_picture || self.mbaff && self.pair_field(address / 2) == Some(true);
        if address >= width * height {
            return Err(invalid("too many macroblocks in slice"));
        }
        if mb_type > 25 {
            return Err(invalid("invalid intra macroblock type"));
        }
        let mut mb = IntraMacroblock {
            address: self.address,
            qp: self.qp,
            switching_qs: None,
            luma: IntraLuma::Block16(0),
            chroma_mode: ChromaMode::Dc,
            coded_block_pattern: 0,
            luma_dc: [0; 16],
            luma_levels: [[0; 16]; 16],
            chroma_dc: [[0; 4]; 2],
            chroma_ac: [[[0; 16]; 4]; 2],
        };
        if mb_type == 25 {
            while !self.bits.position().is_multiple_of(8) {
                if self.bits.bit()? {
                    return Err(invalid("nonzero PCM alignment bit"));
                }
            }
            let mut y = [0; 256];
            let mut cb = [0; 64];
            let mut cr = [0; 64];
            for value in &mut y {
                *value = self.bits.read(self.sps.bit_depth_luma)? as u16;
            }
            for value in cb.iter_mut().chain(cr.iter_mut()) {
                *value = self.bits.read(self.sps.bit_depth_chroma)? as u16;
            }
            mb.luma = IntraLuma::Pcm { y, cb, cr };
            mb.qp = 0;
            self.luma_counts[address * 16..address * 16 + 16].fill(16);
            self.modes[address * 16..address * 16 + 16].fill(2);
            for grid in &mut self.chroma_counts {
                grid[address * 4..address * 4 + 4].fill(16);
            }
            self.advance_address()?;
            return Ok(Some(mb));
        }
        if mb_type == 0 {
            let eight = self.pps.transform_8x8 && self.bits.bit()?;
            let mut modes = [Intra4Mode::Dc; 16];
            for block in 0..if eight { 4 } else { 16 } {
                let (bx, by) = if eight {
                    (block % 2 * 2, block / 2 * 2)
                } else {
                    luma_block_xy(block)?
                };
                let (a, b) = self.mode_neighbours(address, [bx, by])?;
                let mode = |v: Option<u8>| v.and_then(|v| Intra4Mode::try_from(v).ok());
                let predicted = self.bits.bit()?;
                let rem = if predicted {
                    0
                } else {
                    self.bits.read(3)? as u8
                };
                let value = derive_intra4_mode(mode(a), mode(b), predicted, rem)?;
                for dy in 0..if eight { 2 } else { 1 } {
                    for dx in 0..if eight { 2 } else { 1 } {
                        self.modes[address * 16 + (by + dy) * 4 + bx + dx] = value as u8;
                    }
                }
                modes[by * 4 + bx] = value;
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
            mb.luma = IntraLuma::Block16(((mb_type - 1) % 4) as u8);
            mb.coded_block_pattern =
                ((((mb_type - 1) / 4) % 3) * 16 + ((mb_type - 1) / 12) * 15) as u8;
            for by in 0..4 {
                for bx in 0..4 {
                    self.modes[address * 16 + by * 4 + bx] = 2;
                }
            }
        }
        let chroma = self.bits.unsigned_golomb()?;
        mb.chroma_mode = ChromaMode::try_from(
            u8::try_from(chroma).map_err(|_| invalid("chroma mode overflow"))?,
        )?;
        if mb_type == 0 {
            let code = self.bits.unsigned_golomb()?;
            mb.coded_block_pattern =
                super::avc_residual_syntax::coded_block_pattern(code, true, 1)?;
        }
        if mb.coded_block_pattern != 0 || mb_type != 0 {
            let delta = self.bits.signed_golomb()?;
            self.qp =
                super::avc_residual_syntax::update_qp(self.qp, delta, self.sps.bit_depth_luma)?;
        }
        mb.qp = self.qp;
        if mb_type != 0 {
            let context = self.coefficient_context(0, address, [0, 0])?;
            mb.luma_dc = inverse_scan_4x4(
                &read_residual(&mut self.bits, context, 16)?.coefficients,
                field,
            );
        }
        for block in 0..16 {
            let (bx, by) = luma_block_xy(block)?;
            let mut count = 0;
            if mb.coded_block_pattern & (1 << (block / 4)) != 0 {
                let context = self.coefficient_context(0, address, [bx, by])?;
                let r = read_residual(&mut self.bits, context, if mb_type == 0 { 16 } else { 15 })?;
                count = r.total_coefficients;
                if let IntraLuma::Blocks8 { levels, .. } = &mut mb.luma {
                    for i in 0..16 {
                        levels[block / 4][4 * i + block % 4] = r.coefficients[i];
                    }
                }
                let mut levels = r.coefficients;
                if mb_type != 0 {
                    levels.copy_within(0..15, 1);
                    levels[0] = 0;
                }
                mb.luma_levels[by * 4 + bx] = inverse_scan_4x4(&levels, field);
            }
            self.luma_counts[address * 16 + by * 4 + bx] = count;
        }
        if let IntraLuma::Blocks8 { levels, .. } = &mut mb.luma {
            for block in levels {
                *block = super::avc_transform8::inverse_scan_8x8(block, field);
            }
        }
        if mb.coded_block_pattern >> 4 != 0 {
            for component in 0..2 {
                mb.chroma_dc[component]
                    .copy_from_slice(&read_residual(&mut self.bits, -1, 4)?.coefficients[..4]);
            }
        }
        for component in 0..2 {
            for block in 0..4 {
                let mut count = 0;
                if mb.coded_block_pattern >> 4 == 2 {
                    let context =
                        self.coefficient_context(component + 1, address, [block % 2, block / 2])?;
                    let r = read_residual(&mut self.bits, context, 15)?;
                    count = r.total_coefficients;
                    let mut levels = [0; 16];
                    levels[1..].copy_from_slice(&r.coefficients[..15]);
                    mb.chroma_ac[component][block] = inverse_scan_4x4(&levels, field);
                }
                self.chroma_counts[component][address * 4 + block] = count;
            }
        }
        self.advance_address()?;
        Ok(Some(mb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Sps, Pps, SliceHeader) {
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let nal: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let sps = Sps::parse(&nal).unwrap();
        let pps = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &sps).unwrap();
        // Saved IDR header, then mb_type=3 (I16 DC), chroma DC, delta QP=0,
        // zero luma DC residual, rbsp trailing bits. This is a single-MB slice.
        let header = SliceHeader::parse(&[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80], &sps, &pps).unwrap();
        (sps, pps, header)
    }
    #[test]
    fn decodes_zero_residual_intra16_and_consumes_trailing_bits() {
        let (sps, pps, header) = fixture();
        let mut r = IntraCavlcReader::new(&header, &sps, &pps, 4096).unwrap();
        let mb = r.read_macroblock().unwrap().unwrap();
        assert_eq!((mb.address, mb.qp, mb.coded_block_pattern), (0, 25, 0));
        assert!(matches!(mb.luma, IntraLuma::Block16(2)));
        assert_eq!(mb.luma_dc, [0; 16]);
        assert_eq!(mb.luma_levels, [[0; 16]; 16]);
        assert!(r.read_macroblock().unwrap().is_none());
        assert!(r.read_macroblock().unwrap().is_none());
        assert_eq!(r.bit_position(), header.rbsp.len() * 8);
    }
    #[test]
    fn context_budget_and_entropy_mode_are_explicit() {
        let (sps, mut pps, header) = fixture();
        assert!(IntraCavlcReader::new(&header, &sps, &pps, 1).is_err());
        pps.cabac = true;
        assert!(IntraCavlcReader::new(&header, &sps, &pps, 4096).is_err());
    }
    #[test]
    fn mixed_mbaff_context_checks_modes_and_preserves_external_cursor_on_error() {
        let (mut sps, pps, mut header) = fixture();
        sps.frame_mbs_only = false;
        sps.mb_adaptive_frame_field = true;
        for slice in [SliceType::P, SliceType::B] {
            header.slice_type = slice;
            assert!(IntraCavlcReader::new_mbaff(&header, &sps, &pps, 4096).is_err());
            let mut reader =
                IntraCavlcReader::new_context_mbaff(&header, &sps, &pps, 4096).unwrap();
            reader.record_pair_mode(0, true).unwrap();
            reader.record_inter(0, [0; 16], [[0; 4]; 2]).unwrap();
            assert!(reader.record_pair_mode(1, false).is_err());
            let mut bits = BitReader::new(&[]);
            assert!(
                reader
                    .read_embedded_mbaff(&mut bits, 1, 26, 0, true)
                    .is_err()
            );
            assert_eq!(bits.position(), 0);
            let mut reader =
                IntraCavlcReader::new_context_mbaff(&header, &sps, &pps, 4096).unwrap();
            assert!(
                reader
                    .read_embedded_mbaff(&mut bits, u32::MAX, 26, 0, true)
                    .is_err()
            );
            assert_eq!(bits.position(), 0);
            assert!(reader.record_pair_mode(usize::MAX, true).is_err());
        }
        header.field_pic = true;
        assert!(IntraCavlcReader::new_context_mbaff(&header, &sps, &pps, 4096).is_err());
    }
    #[test]
    fn inter_neighbour_participates_in_intra_mode_prediction() {
        let (sps, mut pps, header) = fixture();
        for constrained in [false, true] {
            pps.constrained_intra_pred = constrained;
            let mut reader = IntraCavlcReader::new_context(&header, &sps, &pps, 4096).unwrap();
            reader.record_inter(0, [0; 16], [[0; 4]; 2]).unwrap();
            let top = Intra4Mode::try_from(reader.modes[0]).ok();
            let mode = derive_intra4_mode(Some(Intra4Mode::Vertical), top, true, 0).unwrap();
            assert_eq!(
                mode,
                if constrained {
                    Intra4Mode::Dc
                } else {
                    Intra4Mode::Vertical
                }
            );
        }
    }
    #[test]
    fn block_order_and_context_availability() {
        let expected = [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 0),
            (3, 0),
            (2, 1),
            (3, 1),
            (0, 2),
            (1, 2),
            (0, 3),
            (1, 3),
            (2, 2),
            (3, 2),
            (2, 3),
            (3, 3),
        ];
        for (i, p) in expected.into_iter().enumerate() {
            assert_eq!(luma_block_xy(i).unwrap(), p);
        }
        assert!(luma_block_xy(16).is_err());
        for (grid, expected) in [
            ([255, 255, 255, 0], 0),
            ([255, 4, 255, 0], 4),
            ([255, 4, 7, 0], 6),
        ] {
            assert_eq!(
                super::super::avc_mbaff::cavlc_context(
                    0,
                    [1, 1],
                    1,
                    1,
                    false,
                    [2, 2],
                    |_| None,
                    |_, local| Some(grid[local[1] * 2 + local[0]]).filter(|n| *n != 255)
                )
                .unwrap(),
                expected
            );
        }
    }
}
