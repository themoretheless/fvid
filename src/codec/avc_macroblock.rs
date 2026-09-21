//! Progressive 4:2:0 intra macroblock syntax for CAVLC slices.
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
fn neighbour<T: Copy>(grid: &[T], width: usize, x: usize, y: usize) -> (Option<T>, Option<T>) {
    (
        if x > 0 {
            Some(grid[y * width + x - 1])
        } else {
            None
        },
        if y > 0 {
            Some(grid[(y - 1) * width + x])
        } else {
            None
        },
    )
}
fn nc(grid: &[u8], width: usize, x: usize, y: usize) -> i8 {
    let (a, b) = neighbour(grid, width, x, y);
    let a = a.filter(|&v| v != 255);
    let b = b.filter(|&v| v != 255);
    match (a, b) {
        (Some(a), Some(b)) => ((a + b + 1) >> 1) as i8,
        (Some(v), None) | (None, Some(v)) => v as i8,
        _ => 0,
    }
}
pub struct IntraCavlcReader<'a> {
    bits: BitReader<'a>,
    sps: &'a Sps,
    pps: &'a Pps,
    address: u32,
    qp: i32,
    luma_counts: Vec<u8>,
    chroma_counts: [Vec<u8>; 2],
    modes: Vec<u8>,
    finished: bool,
}
impl<'a> IntraCavlcReader<'a> {
    pub fn new(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        max_macroblocks: usize,
    ) -> Result<Self> {
        if header.slice_type != SliceType::I {
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
        if pps.cabac
            || !sps.frame_mbs_only
            || sps.chroma_format != 1
            || sps.separate_colour_plane
            || !matches!(pps.slice_groups, SliceGroups::Single)
        {
            return Err(invalid(
                "intra CAVLC reader requires progressive 4:2:0 I slices without FMO",
            ));
        }
        if header.pps_id != pps.id || pps.sps_id != sps.id {
            return Err(invalid("slice parameter-set mismatch"));
        }
        let count = (sps.width_mbs as usize)
            .checked_mul(sps.height_map_units as usize)
            .ok_or_else(|| invalid("macroblock count overflow"))?;
        if count == 0 || count > max_macroblocks || count > 65536 {
            return Err(invalid("macroblock context budget exceeded"));
        }
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
            address: header.first_mb,
            qp: header.slice_qp,
            luma_counts: grid(count * 16)?,
            chroma_counts: [grid(count * 4)?, grid(count * 4)?],
            modes: grid(count * 16)?,
            finished: false,
        })
    }
    pub fn bit_position(&self) -> usize {
        self.bits.position()
    }
    pub fn read_macroblock(&mut self) -> Result<Option<IntraMacroblock>> {
        if self.finished {
            return Ok(None);
        }
        if !self.bits.more_rbsp_data() {
            self.bits.finish_rbsp()?;
            self.finished = true;
            return Ok(None);
        }
        let mb_type = self.bits.unsigned_golomb()?;
        self.read_body(mb_type)
    }
    /// Parse after the mixed-slice dispatcher has consumed mb_type and mapped
    /// it to the I table. Discard this context on error; the caller cursor only
    /// advances after success.
    pub fn read_embedded(
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
        let mb = self
            .read_body(mb_type)?
            .ok_or_else(|| invalid("missing embedded intra macroblock"))?;
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
        if address >= width * self.sps.height_map_units as usize
            || luma.iter().chain(chroma.iter().flatten()).any(|&v| v > 16)
        {
            return Err(invalid("invalid mixed-slice coefficient counts"));
        }
        let (mx, my) = (address % width, address / width);
        for i in 0..16 {
            let at = (my * 4 + i / 4) * width * 4 + mx * 4 + i % 4;
            self.luma_counts[at] = luma[i];
            // Available inter neighbours contribute DC unless constrained prediction excludes them.
            self.modes[at] = if self.pps.constrained_intra_pred {
                255
            } else {
                2
            };
        }
        for c in 0..2 {
            for i in 0..4 {
                self.chroma_counts[c][(my * 2 + i / 2) * width * 2 + mx * 2 + i % 2] = chroma[c][i];
            }
        }
        Ok(())
    }
    pub fn counts(&self, address: usize) -> Result<([u8; 16], [[u8; 4]; 2])> {
        let width = self.sps.width_mbs as usize;
        if address >= width * self.sps.height_map_units as usize {
            return Err(invalid("AVC count address out of range"));
        }
        let (mx, my) = (address % width, address / width);
        Ok((
            std::array::from_fn(|i| {
                self.luma_counts[(my * 4 + i / 4) * width * 4 + mx * 4 + i % 4]
            }),
            std::array::from_fn(|c| {
                std::array::from_fn(|i| {
                    self.chroma_counts[c][(my * 2 + i / 2) * width * 2 + mx * 2 + i % 2]
                })
            }),
        ))
    }
    fn read_body(&mut self, mb_type: u32) -> Result<Option<IntraMacroblock>> {
        let width = self.sps.width_mbs as usize;
        let height = self.sps.height_map_units as usize;
        let address = self.address as usize;
        if address >= width * height {
            return Err(invalid("too many macroblocks in slice"));
        }
        let mx = address % width;
        let my = address / width;
        let stride = width * 4;
        if mb_type > 25 {
            return Err(invalid("invalid intra macroblock type"));
        }
        let mut mb = IntraMacroblock {
            address: self.address,
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
            for by in 0..4 {
                for bx in 0..4 {
                    let at = (my * 4 + by) * stride + mx * 4 + bx;
                    self.luma_counts[at] = 16;
                    self.modes[at] = 2;
                }
            }
            for grid in &mut self.chroma_counts {
                for by in 0..2 {
                    for bx in 0..2 {
                        grid[(my * 2 + by) * width * 2 + mx * 2 + bx] = 16;
                    }
                }
            }
            self.address += 1;
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
                let x = mx * 4 + bx;
                let y = my * 4 + by;
                let (a, b) = neighbour(&self.modes, stride, x, y);
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
                        self.modes[(y + dy) * stride + x + dx] = value as u8;
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
                    self.modes[(my * 4 + by) * stride + mx * 4 + bx] = 2;
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
            let context = nc(&self.luma_counts, stride, mx * 4, my * 4);
            mb.luma_dc = inverse_scan_4x4(
                &read_residual(&mut self.bits, context, 16)?.coefficients,
                false,
            );
        }
        for block in 0..16 {
            let (bx, by) = luma_block_xy(block)?;
            let x = mx * 4 + bx;
            let y = my * 4 + by;
            let mut count = 0;
            if mb.coded_block_pattern & (1 << (block / 4)) != 0 {
                let context = nc(&self.luma_counts, stride, x, y);
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
                mb.luma_levels[by * 4 + bx] = inverse_scan_4x4(&levels, false);
            }
            self.luma_counts[y * stride + x] = count;
        }
        if let IntraLuma::Blocks8 { levels, .. } = &mut mb.luma {
            for block in levels {
                *block = super::avc_transform8::inverse_scan_8x8(block, false);
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
                let x = mx * 2 + block % 2;
                let y = my * 2 + block / 2;
                let mut count = 0;
                if mb.coded_block_pattern >> 4 == 2 {
                    let context = nc(&self.chroma_counts[component], width * 2, x, y);
                    let r = read_residual(&mut self.bits, context, 15)?;
                    count = r.total_coefficients;
                    let mut levels = [0; 16];
                    levels[1..].copy_from_slice(&r.coefficients[..15]);
                    mb.chroma_ac[component][block] = inverse_scan_4x4(&levels, false);
                }
                self.chroma_counts[component][y * width * 2 + x] = count;
            }
        }
        self.address += 1;
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
        assert_eq!(nc(&[255, 255, 255, 0], 2, 1, 1), 0);
        assert_eq!(nc(&[255, 4, 255, 0], 2, 1, 1), 4);
        assert_eq!(nc(&[255, 4, 7, 0], 2, 1, 1), 6);
    }
}
