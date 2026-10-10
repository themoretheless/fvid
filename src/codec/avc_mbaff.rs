//! Macroblock sample addressing for progressive and MBAFF pictures.
//! H.264 6.4.1. Entropy, neighbour availability and deblocking are separate.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleLayout {
    pub origin: [usize; 2],
    pub row_step: usize,
    pub size: [usize; 2],
}
/// Address one macroblock in a component plane. `subsampling` contains the
/// horizontal/vertical luma-to-component factors (1 or 2).
/// MBAFF addresses enumerate the top/bottom macroblocks of each pair first.
pub fn layout(
    address: usize,
    width_mbs: usize,
    height_mbs: usize,
    mbaff: bool,
    field: bool,
    subsampling: [usize; 2],
) -> Result<SampleLayout> {
    if width_mbs == 0
        || height_mbs == 0
        || subsampling.iter().any(|s| !matches!(s, 1 | 2))
        || (mbaff && height_mbs % 2 != 0)
        || (field && !mbaff)
    {
        return Err(invalid("invalid AVC macroblock sample geometry"));
    }
    let count = width_mbs
        .checked_mul(height_mbs)
        .ok_or_else(|| invalid("AVC macroblock sample geometry overflow"))?;
    if address >= count {
        return Err(invalid("AVC macroblock address outside picture"));
    }
    // Validate complete component extents before deriving any address.
    width_mbs
        .checked_mul(16)
        .ok_or_else(|| invalid("AVC sample width overflow"))?;
    height_mbs
        .checked_mul(16)
        .ok_or_else(|| invalid("AVC sample height overflow"))?;
    let size = [16 / subsampling[0], 16 / subsampling[1]];
    let (x, y, row_step) = if mbaff {
        let pair = address / 2;
        let parity = address % 2;
        (
            pair % width_mbs * size[0],
            pair / width_mbs * size[1] * 2 + parity * if field { 1 } else { size[1] },
            if field { 2 } else { 1 },
        )
    } else {
        (
            address % width_mbs * size[0],
            address / width_mbs * size[1],
            1,
        )
    };
    Ok(SampleLayout {
        origin: [x, y],
        row_step,
        size,
    })
}
/// Resolve a component sample to its owner and local macroblock coordinates.
/// Unknown pair modes remain unavailable; availability across slices must be
/// checked separately by the prediction caller.
pub fn owner(
    sample: [usize; 2],
    width_mbs: usize,
    height_mbs: usize,
    mbaff: bool,
    subsampling: [usize; 2],
    pair_field: impl FnOnce(usize) -> Option<bool>,
) -> Result<Option<(usize, [usize; 2])>> {
    let geometry = layout(0, width_mbs, height_mbs, mbaff, false, subsampling)?;
    let [bw, bh] = geometry.size;
    let [x, y] = sample;
    if x >= width_mbs * bw || y >= height_mbs * bh {
        return Ok(None);
    }
    if !mbaff {
        return Ok(Some((y / bh * width_mbs + x / bw, [x % bw, y % bh])));
    }
    let pair = y / (2 * bh) * width_mbs + x / bw;
    let Some(field) = pair_field(pair) else {
        return Ok(None);
    };
    let within = y % (2 * bh);
    let (parity, local_y) = if field {
        (within % 2, within / 2)
    } else {
        (within / bh, within % bh)
    };
    Ok(Some((pair * 2 + parity, [x % bw, local_y])))
}
/// Geometry for one deblocking line. Ownership supplies the p-side QP and
/// slice identity; it does not decide whether the boundary may be filtered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeblockLine {
    pub q: [usize; 2],
    pub at: usize,
    pub step: usize,
    pub p_owner: (usize, [usize; 2]),
}
/// Locate q0 and p0 in frame storage. `field_filter` selects a two-row
/// perpendicular stride for a horizontal mixed boundary of a frame block.
/// Its extra edge uses offset 1; q coordinates still follow the block layout.
/// A field block already uses two-row filtering, regardless of this override.
pub fn deblock_line(
    address: usize,
    width_mbs: usize,
    height_mbs: usize,
    mbaff: bool,
    subsampling: [usize; 2],
    vertical: bool,
    offset: usize,
    line: usize,
    field_filter: bool,
    mut pair_field: impl FnMut(usize) -> Option<bool>,
) -> Result<Option<DeblockLine>> {
    let geometry = layout(address, width_mbs, height_mbs, mbaff, false, subsampling)?;
    if offset >= geometry.size[usize::from(!vertical)]
        || line >= geometry.size[usize::from(vertical)]
        || (field_filter && (!mbaff || vertical))
    {
        return Err(invalid("invalid AVC deblocking line geometry"));
    }
    let field = if mbaff {
        let Some(field) = pair_field(address / 2) else {
            return Ok(None);
        };
        field
    } else {
        false
    };
    let block = layout(address, width_mbs, height_mbs, mbaff, field, subsampling)?;
    let q = [
        block.origin[0] + if vertical { offset } else { line },
        block.origin[1] + if vertical { line } else { offset } * block.row_step,
    ];
    let row_step = if field || field_filter { 2 } else { 1 };
    let p = if vertical {
        let Some(x) = q[0].checked_sub(1) else {
            return Ok(None);
        };
        [x, q[1]]
    } else {
        let Some(y) = q[1].checked_sub(row_step) else {
            return Ok(None);
        };
        [q[0], y]
    };
    let Some(p_owner) = owner(p, width_mbs, height_mbs, mbaff, subsampling, pair_field)? else {
        return Ok(None);
    };
    let width = width_mbs * block.size[0];
    let step = if vertical {
        1
    } else {
        width
            .checked_mul(row_step)
            .ok_or_else(|| invalid("AVC deblocking stride overflow"))?
    };
    let at = q[1]
        .checked_mul(width)
        .and_then(|v| v.checked_add(q[0]))
        .ok_or_else(|| invalid("AVC deblocking position overflow"))?;
    Ok(Some(DeblockLine {
        q,
        at,
        step,
        p_owner,
    }))
}
/// Locate a neighbour expressed in the current macroblock's component sample
/// coordinates. A field block's vertical offset advances by two picture rows.
/// Ownership is geometric: callers must still check slice and decoded-block
/// availability before using a returned neighbour for prediction or CAVLC nC.
pub fn neighbour_location(
    address: usize,
    offset: [isize; 2],
    width_mbs: usize,
    height_mbs: usize,
    mbaff: bool,
    subsampling: [usize; 2],
    mut pair_field: impl FnMut(usize) -> Option<bool>,
) -> Result<Option<(usize, [usize; 2])>> {
    layout(address, width_mbs, height_mbs, mbaff, false, subsampling)?;
    let field = if mbaff {
        let Some(field) = pair_field(address / 2) else {
            return Ok(None);
        };
        field
    } else {
        false
    };
    let block = layout(address, width_mbs, height_mbs, mbaff, field, subsampling)?;
    let x = block.origin[0] as i128 + offset[0] as i128;
    let y = block.origin[1] as i128 + offset[1] as i128 * block.row_step as i128;
    let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
        return Ok(None);
    };
    owner(
        [x, y],
        width_mbs,
        height_mbs,
        mbaff,
        subsampling,
        pair_field,
    )
}
/// Resolve A/B/C/D motion neighbours (left, top, top-right, top-left) in local
/// luma sample coordinates. Returned locations are address-owned 4x4 cells.
/// This does not convert reference indices or vectors between frame/field units.
pub fn motion_neighbours(
    address: usize,
    origin: [usize; 2],
    size: [usize; 2],
    width_mbs: usize,
    height_mbs: usize,
    mbaff: bool,
    mut pair_field: impl FnMut(usize) -> Option<bool>,
) -> Result<[Option<(usize, [usize; 2])>; 4]> {
    if size.iter().any(|s| !matches!(s, 4 | 8 | 16))
        || origin.iter().any(|o| o % 4 != 0)
        || (0..2).any(|i| origin[i].checked_add(size[i]).is_none_or(|n| n > 16))
    {
        return Err(invalid("invalid AVC motion neighbour partition"));
    }
    let [x, y] = origin.map(|v| v as isize);
    let mut neighbours = [None; 4];
    for (slot, offset) in neighbours.iter_mut().zip([
        [x - 1, y],
        [x, y - 1],
        [x + size[0] as isize, y - 1],
        [x - 1, y - 1],
    ]) {
        *slot = neighbour_location(
            address,
            offset,
            width_mbs,
            height_mbs,
            mbaff,
            [1, 1],
            &mut pair_field,
        )?
        .map(|(owner, local)| (owner, [local[0] / 4, local[1] / 4]));
    }
    Ok(neighbours)
}
/// Locate the macroblocks at the current component block's left/top origin.
/// Unknown modes and picture boundaries remain unavailable. Context-specific
/// availability (slice, skip or constrained prediction) belongs to the caller.
pub fn macroblock_neighbours(
    address: usize,
    width_mbs: usize,
    height_mbs: usize,
    mbaff: bool,
    subsampling: [usize; 2],
    mut pair_field: impl FnMut(usize) -> Option<bool>,
) -> Result<[Option<usize>; 2]> {
    let mut neighbours = [None; 2];
    for (slot, offset) in neighbours.iter_mut().zip([[-1, 0], [0, -1]]) {
        *slot = neighbour_location(
            address,
            offset,
            width_mbs,
            height_mbs,
            mbaff,
            subsampling,
            &mut pair_field,
        )?
        .map(|(owner, _)| owner);
    }
    Ok(neighbours)
}
/// Resolve left/top 4x4 cells to macroblock-address/local-cell ownership.
/// Geometry is shared by CAVLC counts and CABAC coded/mode contexts. Callers
/// must still enforce slice/decoded availability before reading stored state.
pub fn block_neighbours(
    address: usize,
    block: [usize; 2],
    width_mbs: usize,
    height_mbs: usize,
    mbaff: bool,
    subsampling: [usize; 2],
    mut pair_field: impl FnMut(usize) -> Option<bool>,
) -> Result<[Option<(usize, [usize; 2])>; 2]> {
    let size = layout(address, width_mbs, height_mbs, mbaff, false, subsampling)?.size;
    if block[0] >= size[0] / 4 || block[1] >= size[1] / 4 {
        return Err(invalid("AVC neighbour block outside macroblock"));
    }
    let [x, y] = block.map(|v| (v * 4) as isize);
    let mut neighbours = [None; 2];
    for (slot, offset) in neighbours.iter_mut().zip([[x - 1, y], [x, y - 1]]) {
        *slot = neighbour_location(
            address,
            offset,
            width_mbs,
            height_mbs,
            mbaff,
            subsampling,
            &mut pair_field,
        )?
        .map(|(owner, local)| (owner, [local[0] / 4, local[1] / 4]));
    }
    Ok(neighbours)
}
/// Derive CAVLC nC for a 4x4 component block from available neighbours.
/// `total_coefficients` must return None for blocks not decoded in this slice.
pub fn cavlc_context(
    address: usize,
    block: [usize; 2],
    width_mbs: usize,
    height_mbs: usize,
    mbaff: bool,
    subsampling: [usize; 2],
    mut pair_field: impl FnMut(usize) -> Option<bool>,
    mut total_coefficients: impl FnMut(usize, [usize; 2]) -> Option<u8>,
) -> Result<i8> {
    let mut counts = [None; 2];
    let neighbours = block_neighbours(
        address,
        block,
        width_mbs,
        height_mbs,
        mbaff,
        subsampling,
        &mut pair_field,
    )?;
    for (slot, neighbour) in counts.iter_mut().zip(neighbours) {
        if let Some((owner, local)) = neighbour {
            *slot = total_coefficients(owner, local);
            if slot.is_some_and(|count| count > 16) {
                return Err(invalid(
                    "AVC neighbour coefficient count exceeds block size",
                ));
            }
        }
    }
    Ok(match counts {
        [Some(a), Some(b)] => ((a + b + 1) / 2) as i8,
        [Some(a), None] | [None, Some(a)] => a as i8,
        [None, None] => 0,
    })
}
/// Scatter a reconstructed component block into frame storage. Validate the
/// complete footprint before writing, so invalid geometry cannot partly mutate
/// the destination. Field layouts use every other row.
pub fn write_samples(
    plane: &mut [u16],
    stride: usize,
    block: SampleLayout,
    samples: &[u16],
) -> Result<()> {
    let [width, height] = block.size;
    if stride == 0
        || width == 0
        || height == 0
        || !matches!(block.row_step, 1 | 2)
        || plane.len() % stride != 0
    {
        return Err(invalid("invalid AVC reconstructed block geometry"));
    }
    let count = width
        .checked_mul(height)
        .ok_or_else(|| invalid("AVC reconstructed block size overflow"))?;
    if samples.len() != count {
        return Err(invalid("AVC reconstructed block sample count mismatch"));
    }
    let right = block.origin[0]
        .checked_add(width)
        .ok_or_else(|| invalid("AVC reconstructed block horizontal overflow"))?;
    let bottom = (height - 1)
        .checked_mul(block.row_step)
        .and_then(|n| block.origin[1].checked_add(n))
        .ok_or_else(|| invalid("AVC reconstructed block vertical overflow"))?;
    if right > stride || bottom >= plane.len() / stride {
        return Err(invalid("AVC reconstructed block outside destination plane"));
    }
    for row in 0..height {
        let start = (block.origin[1] + row * block.row_step) * stride + block.origin[0];
        plane[start..start + width].copy_from_slice(&samples[row * width..row * width + width]);
    }
    Ok(())
}
/// Gather intra prediction edges in frame storage; vertical offsets follow
/// the current macroblock's frame/field row step. Availability is supplied by
/// the caller's decoded-block/slice map and checked for every edge sample.
pub fn prediction_edges<const N: usize>(
    plane: &[u16],
    stride: usize,
    origin: [usize; 2],
    row_step: usize,
    mut available: impl FnMut([usize; 2]) -> bool,
) -> Result<(Option<[u16; N]>, Option<[u16; N]>, Option<u16>)> {
    if !matches!(N, 4 | 8 | 16)
        || stride == 0
        || plane.len() % stride != 0
        || !matches!(row_step, 1 | 2)
        || origin[0].checked_add(N).is_none_or(|right| right > stride)
        || (N - 1)
            .checked_mul(row_step)
            .and_then(|n| origin[1].checked_add(n))
            .is_none_or(|bottom| bottom >= plane.len() / stride)
    {
        return Err(invalid("invalid AVC intra prediction edge geometry"));
    }
    let mut sample = |dx: isize, dy: isize| -> Option<u16> {
        let x = origin[0].checked_add_signed(dx)?;
        let y = origin[1].checked_add_signed(dy.checked_mul(row_step as isize)?)?;
        if x >= stride || y >= plane.len() / stride || !available([x, y]) {
            return None;
        }
        Some(plane[y * stride + x])
    };
    let mut top = [0; N];
    let mut left = [0; N];
    let mut top_known = true;
    let mut left_known = true;
    for i in 0..N {
        if let Some(value) = sample(i as isize, -1) {
            top[i] = value;
        } else {
            top_known = false;
        }
        if let Some(value) = sample(-1, i as isize) {
            left[i] = value;
        } else {
            left_known = false;
        }
    }
    Ok((
        top_known.then_some(top),
        left_known.then_some(left),
        sample(-1, -1),
    ))
}
/// Slice-local reconstruction readiness for 4:2:0, indexed by macroblock.
/// A bit identifies one completed 4x4 block in its component plane.
pub struct Readiness420 {
    width: usize,
    height: usize,
    mbaff: bool,
    pair_fields: Vec<u8>,
    // Chroma availability uses bits0..3; bit15 of Cb marks an SI macroblock.
    // Keeping the tag here preserves the existing bounded storage layout.
    blocks: Vec<[u16; 3]>,
}
impl Readiness420 {
    pub fn new(width: usize, height: usize, mbaff: bool, budget: usize) -> Result<Self> {
        layout(0, width, height, mbaff, false, [1, 1])?;
        let count = width
            .checked_mul(height)
            .ok_or_else(|| invalid("AVC readiness size overflow"))?;
        let pairs = if mbaff { count / 2 } else { 0 };
        let storage = count
            .checked_mul(std::mem::size_of::<[u16; 3]>())
            .and_then(|n| n.checked_add(pairs))
            .ok_or_else(|| invalid("AVC readiness budget overflow"))?;
        if count > 65536 || storage > budget {
            return Err(invalid("AVC readiness exceeds budget"));
        }
        let mut blocks = Vec::new();
        blocks
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate AVC readiness"))?;
        blocks.resize(count, [0; 3]);
        let mut pair_fields = crate::buffer(pairs)?;
        pair_fields.fill(255);
        Ok(Self {
            width,
            height,
            mbaff,
            pair_fields,
            blocks,
        })
    }
    /// Validate a complete MBAFF publication without changing availability.
    pub fn check_mbaff_complete(
        &self,
        address: usize,
        geometry: [usize; 2],
        field: bool,
    ) -> Result<()> {
        if !self.mbaff || geometry != [self.width, self.height] {
            return Err(invalid("MBAFF readiness geometry mismatch"));
        }
        layout(address, self.width, self.height, true, field, [1, 1])?;
        let mode = self.pair_fields[address / 2];
        if mode != 255 && mode != u8::from(field) {
            return Err(invalid("AVC readiness pair mode changed"));
        }
        if self.blocks[address] != [0; 3] {
            return Err(invalid(
                "MBAFF macroblock already has reconstructed samples",
            ));
        }
        Ok(())
    }
    /// Mark all three components together after complete sample publication.
    pub fn publish_mbaff_complete(
        &mut self,
        address: usize,
        geometry: [usize; 2],
        field: bool,
    ) -> Result<()> {
        self.publish_mbaff_complete_kind(address, geometry, field, false)
    }
    /// Publish an intra macroblock while retaining SI identity for constrained
    /// intra prediction. Ordinary intra and inter publications have kind1.
    pub fn publish_mbaff_complete_kind(
        &mut self,
        address: usize,
        geometry: [usize; 2],
        field: bool,
        switching: bool,
    ) -> Result<()> {
        self.check_mbaff_complete(address, geometry, field)?;
        self.pair_fields[address / 2] = u8::from(field);
        self.blocks[address] = [u16::MAX, 15 | if switching { 1 << 15 } else { 0 }, 15];
        Ok(())
    }
    /// Publish only after component samples have been successfully written.
    pub fn publish(
        &mut self,
        address: usize,
        component: usize,
        rect: [usize; 4],
        field: bool,
    ) -> Result<()> {
        if component > 2 {
            return Err(invalid("AVC readiness component outside range"));
        }
        let sub = if component == 0 { [1, 1] } else { [2, 2] };
        let geometry = layout(address, self.width, self.height, self.mbaff, field, sub)?;
        let [x, y, w, h] = rect;
        let side = geometry.size[0] / 4;
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|n| n > side)
            || y.checked_add(h).is_none_or(|n| n > side)
        {
            return Err(invalid("AVC readiness rectangle outside macroblock"));
        }
        if self.mbaff
            && self.pair_fields[address / 2] != 255
            && self.pair_fields[address / 2] != u8::from(field)
        {
            return Err(invalid("AVC readiness pair mode changed"));
        }
        let mut mask = 0;
        for row in y..y + h {
            for col in x..x + w {
                mask |= 1u16 << (row * side + col);
            }
        }
        if self.mbaff {
            self.pair_fields[address / 2] = u8::from(field);
        }
        self.blocks[address][component] |= mask;
        Ok(())
    }
    pub fn available(&self, component: usize, sample: [usize; 2]) -> Result<bool> {
        Ok(self.available_kind(component, sample)? != 0)
    }
    /// Zero means unavailable, one ordinary reconstructed samples, two SI.
    /// Physical sample ownership includes the pair's frame/field row layout.
    pub fn available_kind(&self, component: usize, sample: [usize; 2]) -> Result<u8> {
        if component > 2 {
            return Err(invalid("AVC readiness component outside range"));
        }
        let sub = if component == 0 { [1, 1] } else { [2, 2] };
        let Some((address, local)) =
            owner(sample, self.width, self.height, self.mbaff, sub, |pair| {
                self.pair_fields
                    .get(pair)
                    .copied()
                    .filter(|v| *v != 255)
                    .map(|v| v != 0)
            })?
        else {
            return Ok(0);
        };
        let side = 4 / sub[0];
        let bit = local[1] / 4 * side + local[0] / 4;
        Ok(if self.blocks[address][component] & (1u16 << bit) == 0 {
            0
        } else if self.blocks[address][1] & (1 << 15) != 0 {
            2
        } else {
            1
        })
    }
    pub fn reset_slice(&mut self) {
        self.blocks.fill([0; 3]);
        self.pair_fields.fill(255);
    }
}
/// MBAFF slice-data field flag state (H.264 7.3.4). The caller supplies
/// the CAVLC bit or CABAC context decision only when syntax requires it.
#[derive(Default)]
pub struct PairMode {
    known: Option<(usize, bool)>,
}
impl PairMode {
    pub fn read(
        &mut self,
        address: usize,
        previous_skipped: bool,
        read_flag: impl FnOnce() -> Result<bool>,
    ) -> Result<bool> {
        let pair = address / 2;
        if address % 2 == 0 || previous_skipped {
            let field = read_flag()?;
            self.known = Some((pair, field));
            Ok(field)
        } else {
            self.known
                .filter(|(known, _)| *known == pair)
                .map(|(_, field)| field)
                .ok_or_else(|| invalid("AVC MBAFF bottom macroblock lacks pair field flag"))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn motion_partition_neighbours_resolve_all_four_mixed_locations() {
        // Right field top: left crosses into the lower frame block at y=16.
        assert_eq!(
            motion_neighbours(2, [0, 8], [8, 8], 2, 2, true, |pair| Some(pair == 1)).unwrap(),
            [
                Some((1, [3, 0])),
                Some((2, [0, 1])),
                Some((2, [2, 1])),
                Some((0, [3, 3])),
            ]
        );
        // A frame top under field pairs selects opposite-parity top samples.
        assert_eq!(
            motion_neighbours(4, [0, 0], [16, 16], 2, 4, true, |pair| Some(pair < 2)).unwrap(),
            [None, Some((1, [0, 3])), Some((3, [0, 3])), None,]
        );
        assert_eq!(
            motion_neighbours(0, [0, 0], [16, 16], 2, 2, false, |_| None).unwrap(),
            [None; 4]
        );
        assert_eq!(
            motion_neighbours(2, [0, 8], [8, 8], 2, 2, true, |_| None).unwrap(),
            [None; 4]
        );
        assert!(motion_neighbours(0, [12, 0], [8, 16], 2, 2, true, |_| Some(false)).is_err());
    }
    #[test]
    fn macroblock_context_origins_follow_pair_modes() {
        for sub in [[1, 1], [2, 1], [2, 2]] {
            assert_eq!(
                macroblock_neighbours(3, 2, 4, true, sub, |pair| Some(pair == 1)).unwrap(),
                [Some(0), None]
            );
            assert_eq!(
                macroblock_neighbours(2, 2, 4, true, sub, |pair| Some(pair == 1)).unwrap(),
                [Some(0), None]
            );
            assert_eq!(
                macroblock_neighbours(4, 2, 4, true, sub, |pair| Some(pair == 0)).unwrap(),
                [None, Some(1)]
            );
            assert_eq!(
                macroblock_neighbours(5, 2, 4, true, sub, |pair| Some(pair == 2)).unwrap(),
                [None, Some(1)]
            );
            assert_eq!(
                macroblock_neighbours(3, 2, 2, false, sub, |_| None).unwrap(),
                [Some(2), Some(1)]
            );
        }
        assert_eq!(
            macroblock_neighbours(2, 2, 2, true, [1, 1], |_| None).unwrap(),
            [None; 2]
        );
        assert!(macroblock_neighbours(4, 2, 2, true, [1, 1], |_| Some(false)).is_err());
    }
    #[test]
    fn component_context_cells_follow_mixed_frame_field_ownership() {
        assert_eq!(
            block_neighbours(2, [0, 2], 2, 2, true, [1, 1], |pair| Some(pair == 1)).unwrap(),
            [Some((1, [3, 0])), Some((2, [0, 1]))]
        );
        assert_eq!(
            block_neighbours(2, [0, 1], 2, 2, true, [2, 2], |pair| Some(pair == 1)).unwrap(),
            [Some((1, [1, 0])), Some((2, [0, 0]))]
        );
        assert_eq!(
            block_neighbours(4, [0, 0], 2, 4, true, [1, 1], |pair| Some(pair == 0)).unwrap(),
            [None, Some((1, [0, 3]))]
        );
        assert_eq!(
            block_neighbours(2, [0, 2], 2, 2, true, [1, 1], |pair| if pair == 1 {
                Some(true)
            } else {
                None
            })
            .unwrap(),
            [None, Some((2, [0, 1]))]
        );
        assert!(block_neighbours(2, [2, 0], 2, 2, true, [2, 2], |_| Some(false)).is_err());
    }
    #[test]
    fn deblocking_lines_resolve_mixed_pair_qp_owners() {
        for sub in [[1, 1], [2, 1], [2, 2]] {
            let bw = 16 / sub[0];
            let bh = 16 / sub[1];
            // Right field top crosses from the upper to lower frame block.
            let v = deblock_line(2, 2, 2, true, sub, true, 0, bh / 2, false, |pair| {
                Some(pair == 1)
            })
            .unwrap()
            .unwrap();
            assert_eq!(v.q, [bw, bh]);
            assert_eq!(v.at, bh * (2 * bw) + bw);
            assert_eq!(v.step, 1);
            assert_eq!(v.p_owner, (1, [bw - 1, 0]));
            // A frame top below a field pair has two horizontal boundaries.
            for parity in 0..2 {
                let h = deblock_line(4, 2, 4, true, sub, false, parity, 0, true, |pair| {
                    Some(pair == 0)
                })
                .unwrap()
                .unwrap();
                assert_eq!(h.q, [0, 2 * bh + parity]);
                assert_eq!(h.step, 4 * bw);
                assert_eq!(h.p_owner, (parity, [0, bh - 1]));
            }
            let progressive = deblock_line(2, 2, 2, false, sub, false, 0, 3, false, |_| {
                panic!("progressive geometry does not need pair modes")
            })
            .unwrap()
            .unwrap();
            assert_eq!(progressive.q, [3, bh]);
            assert_eq!(progressive.step, 2 * bw);
            assert_eq!(progressive.p_owner, (0, [3, bh - 1]));
        }
        assert!(
            deblock_line(0, 2, 2, true, [1, 1], false, 0, 0, false, |_| Some(false))
                .unwrap()
                .is_none()
        );
        assert!(
            deblock_line(2, 2, 2, true, [1, 1], true, 0, 0, false, |_| None)
                .unwrap()
                .is_none()
        );
        assert!(deblock_line(2, 2, 2, true, [1, 1], true, 16, 0, false, |_| Some(false)).is_err());
        assert!(deblock_line(2, 2, 2, true, [1, 1], true, 0, 0, true, |_| Some(false)).is_err());
    }
    #[test]
    fn mixed_frame_field_pairs_cover_each_component_exactly_once() {
        for sub in [[1, 1], [2, 1], [2, 2]] {
            let (width, height) = (48 / sub[0], 64 / sub[1]);
            let mut coverage = vec![0u8; width * height];
            for address in 0..12 {
                let block = layout(address, 3, 4, true, address / 2 % 2 == 1, sub).unwrap();
                for y in 0..block.size[1] {
                    for x in 0..block.size[0] {
                        let index =
                            (block.origin[1] + y * block.row_step) * width + block.origin[0] + x;
                        coverage[index] += 1;
                        assert_eq!(
                            owner(
                                [block.origin[0] + x, block.origin[1] + y * block.row_step],
                                3,
                                4,
                                true,
                                sub,
                                |pair| Some(pair % 2 == 1)
                            )
                            .unwrap(),
                            Some((address, [x, y]))
                        );
                    }
                }
            }
            assert!(coverage.iter().all(|v| *v == 1));
        }
        assert_eq!(layout(7, 3, 4, true, true, [1, 1]).unwrap().origin, [0, 33]);
        assert_eq!(
            layout(7, 3, 4, true, false, [1, 1]).unwrap().origin,
            [0, 48]
        );
        assert_eq!(
            layout(7, 3, 4, false, false, [1, 1]).unwrap().origin,
            [16, 32]
        );
    }
    #[test]
    fn inverse_address_preserves_boundaries_and_unavailable_pair_modes() {
        assert_eq!(owner([0, 0], 3, 4, true, [1, 1], |_| None).unwrap(), None);
        for sample in [[48, 0], [0, 64], [usize::MAX, 0]] {
            assert_eq!(
                owner(sample, 3, 4, true, [1, 1], |_| Some(false)).unwrap(),
                None
            );
        }
        assert_eq!(
            owner([17, 35], 3, 4, false, [1, 1], |_| panic!("not MBAFF")).unwrap(),
            Some((7, [1, 3]))
        );
        assert_eq!(
            owner([9, 17], 3, 4, true, [2, 2], |_| Some(true)).unwrap(),
            Some((9, [1, 0]))
        );
    }
    #[test]
    fn field_flag_is_read_once_per_pair_or_after_a_skipped_top() {
        let mut state = PairMode::default();
        assert!(state.read(0, false, || Ok(true)).unwrap());
        assert!(
            state
                .read(1, false, || panic!("inferred bottom flag"))
                .unwrap()
        );
        assert!(!state.read(3, true, || Ok(false)).unwrap());
        assert!(
            state
                .read(5, false, || panic!("missing top is invalid"))
                .is_err()
        );
        assert!(!state.read(4, false, || Ok(false)).unwrap());
        assert!(
            state
                .read(6, false, || Err(invalid("truncated flag")))
                .is_err()
        );
        assert!(
            !state
                .read(5, false, || panic!("prior pair survives error"))
                .unwrap()
        );
    }
    #[test]
    fn neighbours_cross_frame_field_pairs_with_component_row_stride() {
        let modes = |pair| Some(pair == 1);
        assert_eq!(
            neighbour_location(2, [-1, 8], 2, 2, true, [1, 1], modes).unwrap(),
            Some((1, [15, 0]))
        );
        assert_eq!(
            neighbour_location(3, [-1, 7], 2, 2, true, [1, 1], modes).unwrap(),
            Some((0, [15, 15]))
        );
        assert_eq!(
            neighbour_location(1, [16, 0], 2, 2, true, [1, 1], modes).unwrap(),
            Some((2, [0, 8]))
        );
        assert_eq!(
            neighbour_location(2, [-1, 4], 2, 2, true, [2, 2], modes).unwrap(),
            Some((1, [7, 0]))
        );
        assert_eq!(
            neighbour_location(3, [0, -1], 2, 2, true, [1, 1], modes).unwrap(),
            None
        );
        assert_eq!(
            neighbour_location(2, [-1, 0], 2, 2, true, [1, 1], |_| None).unwrap(),
            None
        );
        assert_eq!(
            neighbour_location(0, [isize::MIN, 0], 2, 2, true, [1, 1], modes).unwrap(),
            None
        );
    }
    #[test]
    fn cavlc_context_uses_available_frame_field_neighbours() {
        let mode = |pair| Some(pair == 1);
        let value = cavlc_context(2, [0, 2], 2, 2, true, [1, 1], mode, |owner, block| {
            match (owner, block) {
                (1, [3, 0]) => Some(7),
                (2, [0, 1]) => Some(4),
                _ => None,
            }
        })
        .unwrap();
        assert_eq!(value, 6);
        assert_eq!(
            cavlc_context(2, [0, 2], 2, 2, true, [1, 1], mode, |owner, _| {
                if owner == 1 { Some(7) } else { None }
            })
            .unwrap(),
            7
        );
        assert_eq!(
            cavlc_context(0, [0, 0], 2, 2, true, [1, 1], mode, |_, _| None).unwrap(),
            0
        );
        assert!(cavlc_context(2, [0, 2], 2, 2, true, [1, 1], mode, |_, _| Some(17)).is_err());
        assert!(cavlc_context(2, [2, 0], 2, 2, true, [2, 2], mode, |_, _| None).is_err());
    }
    #[test]
    fn reconstructed_field_blocks_interleave_without_touching_the_other_field() {
        let mut plane = vec![0u16; 16 * 32];
        let upper = layout(0, 1, 2, true, true, [1, 1]).unwrap();
        let lower = layout(1, 1, 2, true, true, [1, 1]).unwrap();
        write_samples(&mut plane, 16, upper, &[37; 256]).unwrap();
        for (row, values) in plane.chunks_exact(16).enumerate() {
            assert!(
                values
                    .iter()
                    .all(|value| *value == if row % 2 == 0 { 37 } else { 0 })
            );
        }
        write_samples(&mut plane, 16, lower, &[211; 256]).unwrap();
        for (row, values) in plane.chunks_exact(16).enumerate() {
            assert!(
                values
                    .iter()
                    .all(|value| *value == if row % 2 == 0 { 37 } else { 211 })
            );
        }
        let saved = plane.clone();
        for invalid in [
            SampleLayout {
                origin: [1, 0],
                ..upper
            },
            SampleLayout {
                origin: [0, 2],
                ..upper
            },
            SampleLayout {
                row_step: usize::MAX,
                ..upper
            },
        ] {
            assert!(write_samples(&mut plane, 16, invalid, &[99; 256]).is_err());
            assert_eq!(plane, saved);
        }
        assert!(write_samples(&mut plane, 16, upper, &[0; 255]).is_err());
        assert_eq!(plane, saved);
    }
    #[test]
    fn prediction_edges_follow_field_rows_and_sample_availability() {
        let plane: Vec<u16> = (0..32)
            .flat_map(|y| (0..32).map(move |x| y * 100 + x))
            .collect();
        assert_eq!(
            prediction_edges::<4>(&plane, 32, [16, 16], 2, |_| true).unwrap(),
            (
                Some([1416, 1417, 1418, 1419]),
                Some([1615, 1815, 2015, 2215]),
                Some(1415)
            )
        );
        let (top, left, corner) =
            prediction_edges::<4>(&plane, 32, [16, 16], 2, |point| point != [17, 14]).unwrap();
        assert!(top.is_none());
        assert!(left.is_some());
        assert_eq!(corner, Some(1415));
        assert_eq!(
            prediction_edges::<4>(&plane, 32, [0, 0], 2, |_| true).unwrap(),
            (None, None, None)
        );
        assert!(prediction_edges::<16>(&plane, 32, [0, 4], 2, |_| true).is_err());
    }
    #[test]
    fn si_readiness_tags_follow_physical_parity_and_reset_without_extra_storage() {
        let mut ready = Readiness420::new(1, 2, true, 13).unwrap();
        ready.publish_mbaff_complete_kind(0, [1, 2], true, true).unwrap();
        ready.publish_mbaff_complete(1, [1, 2], true).unwrap();
        for component in 0..3 {
            let height = if component == 0 { 32 } else { 16 };
            for y in 0..height {
                assert_eq!(ready.available_kind(component, [0, y]).unwrap(),
                    if y % 2 == 0 { 2 } else { 1 });
                assert!(ready.available(component, [0, y]).unwrap());
            }
        }
        assert!(ready.publish_mbaff_complete_kind(0, [1, 2], true, false).is_err());
        assert_eq!(ready.available_kind(0, [0, 0]).unwrap(), 2);
        ready.reset_slice();
        for component in 0..3 {
            assert_eq!(ready.available_kind(component, [0, 0]).unwrap(), 0);
        }
        ready.publish_mbaff_complete_kind(0, [1, 2], false, true).unwrap();
        ready.publish_mbaff_complete(1, [1, 2], false).unwrap();
        assert_eq!(ready.available_kind(0, [0, 15]).unwrap(), 2);
        assert_eq!(ready.available_kind(0, [0, 16]).unwrap(), 1);
        assert_eq!(ready.available_kind(1, [0, 7]).unwrap(), 2);
        assert_eq!(ready.available_kind(1, [0, 8]).unwrap(), 1);
    }
    #[test]
    fn field_readiness_never_leaks_into_other_field_component_or_slice() {
        let mut ready = Readiness420::new(1, 2, true, 13).unwrap();
        assert!(Readiness420::new(1, 2, true, 12).is_err());
        ready.publish(0, 0, [0, 0, 4, 4], true).unwrap();
        for y in 0..32 {
            assert_eq!(ready.available(0, [0, y]).unwrap(), y % 2 == 0);
        }
        assert!(!ready.available(1, [0, 0]).unwrap());
        assert_eq!(
            prediction_edges::<4>(&[99; 512], 16, [4, 9], 2, |sample| ready
                .available(0, sample)
                .unwrap())
            .unwrap(),
            (None, None, None)
        );
        assert_eq!(
            prediction_edges::<4>(&[99; 512], 16, [4, 8], 2, |sample| ready
                .available(0, sample)
                .unwrap())
            .unwrap(),
            (Some([99; 4]), Some([99; 4]), Some(99))
        );
        ready.publish(0, 1, [0, 0, 1, 1], true).unwrap();
        assert!(ready.available(1, [3, 6]).unwrap());
        assert!(!ready.available(1, [4, 6]).unwrap());
        assert!(!ready.available(1, [3, 7]).unwrap());
        assert!(ready.publish(1, 0, [0, 0, 4, 4], false).is_err());
        assert!(!ready.available(0, [0, 1]).unwrap());
        ready.reset_slice();
        assert!(!ready.available(0, [0, 0]).unwrap());
        ready.publish(0, 0, [0, 0, 4, 4], false).unwrap();
        assert!(ready.available(0, [0, 15]).unwrap());
        assert!(!ready.available(0, [0, 16]).unwrap());
    }
    #[test]
    fn malformed_dimensions_and_addresses_are_rejected() {
        for (a, w, h, m, f, s) in [
            (0, 0, 4, true, false, [1, 1]),
            (0, 3, 3, true, false, [1, 1]),
            (12, 3, 4, true, false, [1, 1]),
            (0, usize::MAX, 4, true, false, [1, 1]),
            (0, 3, 4, false, true, [1, 1]),
            (0, 3, 4, true, false, [3, 1]),
        ] {
            assert!(layout(a, w, h, m, f, s).is_err());
        }
    }
}
