//! Owned Annex I SPS extension syntax; active reconstruction tools are separate.
use super::bits::BitReader;
use crate::{Result, invalid};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpsExtension {
    pub inter_view_disparity: [bool; 2],
    pub inter_view_mv_scaling: [bool; 2],
    pub disparity_sub_block_log2: u8,
    pub inter_view_residual_prediction: bool,
    pub depth_reference: bool,
    pub view_synthesis_prediction: bool,
    pub depth_based_partition: bool,
    pub texture_motion: bool,
    pub texture_sub_block_log2: u8,
    pub intra_contour: bool,
    pub intra_dc_wedge: bool,
    pub coding_tree_partition: bool,
    pub inter_dc_only: bool,
    pub skip_intra: bool,
}
struct Reader<'a> {
    bits: BitReader<'a>,
    end: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: u8) -> Result<u32> {
        if self
            .bits
            .position()
            .checked_add(usize::from(n))
            .is_none_or(|p| p > self.end)
        {
            return Err(invalid("truncated HEVC 3D SPS extension"));
        }
        self.bits.read(n)
    }
    fn flag(&mut self) -> Result<bool> {
        Ok(self.take(1)? != 0)
    }
    fn sub_block(&mut self, cb: [u8; 2]) -> Result<u8> {
        let mut zeros = 0;
        while !self.flag()? {
            zeros += 1;
            if zeros > 31 {
                return Err(invalid("oversized HEVC 3D sub-block code"));
            }
        }
        let value = ((1u64 << zeros) - 1) + u64::from(self.take(zeros)?);
        if value < u64::from(cb[0] - 3) || value > u64::from(cb[1] - 3) {
            return Err(invalid(
                "HEVC 3D sub-block size exceeds coding-block bounds",
            ));
        }
        Ok(value as u8 + 3)
    }
}
impl SpsExtension {
    pub fn active_for_depth(&self, depth: bool) -> bool {
        if depth {
            self.inter_view_disparity[1]
                || self.inter_view_mv_scaling[1]
                || self.texture_motion
                || self.intra_contour
                || self.intra_dc_wedge
                || self.coding_tree_partition
                || self.inter_dc_only
                || self.skip_intra
        } else {
            self.inter_view_disparity[0]
                || self.inter_view_mv_scaling[0]
                || self.inter_view_residual_prediction
                || self.depth_reference
                || self.view_synthesis_prediction
                || self.depth_based_partition
        }
    }
    pub fn active(&self) -> bool {
        self.active_for_depth(false) || self.active_for_depth(true)
    }
    /// end_bit excludes RBSP stop/alignment, but may include later extensions.
    /// Failure leaves the caller's cursor unchanged.
    pub fn read(bits: &mut BitReader<'_>, end_bit: usize, coding_blocks: [u8; 2]) -> Result<Self> {
        if !(3..=6).contains(&coding_blocks[0])
            || coding_blocks[1] < coding_blocks[0]
            || coding_blocks[1] > 6
        {
            return Err(invalid("invalid HEVC 3D coding-block geometry"));
        }
        if end_bit < bits.position() || end_bit > bits.position().saturating_add(bits.remaining()) {
            return Err(invalid("invalid HEVC 3D SPS syntax boundary"));
        }
        let mut r = Reader {
            bits: bits.clone(),
            end: end_bit,
        };
        let disparity0 = r.flag()?;
        let mv0 = r.flag()?;
        let disparity_sub_block_log2 = r.sub_block(coding_blocks)?;
        let inter_view_residual_prediction = r.flag()?;
        let depth_reference = r.flag()?;
        let view_synthesis_prediction = r.flag()?;
        let depth_based_partition = r.flag()?;
        let disparity1 = r.flag()?;
        let mv1 = r.flag()?;
        let texture_motion = r.flag()?;
        let texture_sub_block_log2 = r.sub_block(coding_blocks)?;
        let intra_contour = r.flag()?;
        let intra_dc_wedge = r.flag()?;
        let coding_tree_partition = r.flag()?;
        let inter_dc_only = r.flag()?;
        let skip_intra = r.flag()?;
        *bits = r.bits;
        Ok(Self {
            inter_view_disparity: [disparity0, disparity1],
            inter_view_mv_scaling: [mv0, mv1],
            disparity_sub_block_log2,
            inter_view_residual_prediction,
            depth_reference,
            view_synthesis_prediction,
            depth_based_partition,
            texture_motion,
            texture_sub_block_log2,
            intra_contour,
            intra_dc_wedge,
            coding_tree_partition,
            inter_dc_only,
            skip_intra,
        })
    }
}
pub(crate) fn payload_end(bytes: &[u8]) -> Result<usize> {
    let (last, byte) = bytes
        .iter()
        .enumerate()
        .rev()
        .find(|(_, v)| **v != 0)
        .ok_or_else(|| invalid("missing HEVC SPS RBSP stop bit"))?;
    Ok(last * 8 + 7 - byte.trailing_zeros() as usize)
}
