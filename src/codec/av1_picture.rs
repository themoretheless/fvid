//! Native AV1 picture reconstruction. Unsupported coding tools fail closed.
use super::{
    av1_cdfs::{self, Cdfs},
    av1_frame::Header,
    av1_sequence::Sequence,
    av1_symbol::SymbolDecoder,
    vp9_transform::{self, Kind},
};
use crate::{Result, invalid};
#[path = "av1_picture_inter.rs"]
mod inter;
#[path = "av1_palette.rs"]
mod palette;
#[path = "av1_quant_matrix.rs"]
mod quant_matrix;
#[path = "av1_restoration.rs"]
mod restoration;
#[path = "av1_superres.rs"]
mod superres;

#[derive(Clone, Debug)]
pub struct Plane {
    pub width: usize,
    pub height: usize,
    pub samples: Vec<u16>,
}
#[derive(Clone, Debug)]
pub struct Picture {
    pub size: [u32; 2],
    pub depth: u8,
    /// Coded MI grid dimensions; super-resolution changes display size, not this grid.
    pub segment_grid: [usize; 2],
    /// Segment IDs in padded 4x4 raster order, retained with reference pictures.
    pub segment_ids: Vec<u8>,
    /// Coded palette block counts per Y/UV plane group, for palette sizes 2..=8.
    pub palette_counts: [[u32; 7]; 2],
    /// Palette colors selected from the above/left neighbor cache, per Y/UV group.
    pub palette_cache_hits: [u32; 2],
    /// Palette transform blocks with nonzero dequantized residuals, per Y/UV group.
    pub palette_residual_blocks: [u32; 2],
    pub planes: [Plane; 3],
}
#[derive(Clone, Copy)]
enum CompoundMask {
    Wedge { index: usize, sign: bool },
    Difference { invert: bool },
}
#[derive(Clone, Copy, Default)]
struct Block {
    w: usize,
    h: usize,
    segment: usize,
    mode: usize,
    skip: bool,
    tx: [usize; 2],
    uv_mode: usize,
    reference: usize,
    mv: [i32; 2],
    reference2: usize,
    interintra: bool,
    mv2: [i32; 2],
    skip_mode: bool,
    compound_average: bool,
    compound_mask: Option<CompoundMask>,
    warp: Option<[i64; 6]>,
    filters: [usize; 2],
    palette_sizes: [u8; 2],
    palette_colors: [[u16; 8]; 2],
    delta_lf: [i32; 4],
}
struct Decoder<'a> {
    s: &'a Sequence,
    references: [Option<&'a Picture>; 8],
    biases: [bool; 8],
    distances: [i32; 8],
    h: &'a Header,
    image: Picture,
    cols: usize,
    rows: usize,
    blocks: Vec<Block>,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    above: [Vec<(u8, u8)>; 3],
    left: [Vec<(u8, u8)>; 3],
    decoded: [Vec<bool>; 3],
    tx_sizes: [Vec<[usize; 2]>; 3],
    tx_types: Vec<u8>,
    cdef_indexes: Vec<i8>,
    restoration: restoration::State,
    read_deltas: bool,
    current_q: i32,
    delta_lf: [i32; 4],
    current_segment: usize,
    segment_pre_skip: bool,
    previous_segments: Vec<u8>,
    segment_pred_above: Vec<usize>,
    segment_pred_left: Vec<usize>,
    current_block: [usize; 2],
    scratch: Vec<i32>,
    pred_scratch: Vec<u16>,
    tx_scratch: Vec<i64>,
    residual_scratch: Vec<i32>,
    lossless_scratch: Vec<i64>,
    lossless_out: Vec<i32>,
    inter_pred: Vec<i32>,
    inter_pred2: Vec<i32>,
    compound_weights: Vec<i32>,
}
fn neg_deinterleave(diff: usize, reference: usize, max: usize) -> usize {
    if reference == 0 {
        return diff;
    }
    if reference >= max - 1 {
        return max - diff - 1;
    }
    if 2 * reference < max {
        if diff > 2 * reference {
            return diff;
        }
    } else if diff > 2 * (max - reference - 1) {
        return max - diff - 1;
    }
    if diff & 1 != 0 {
        reference + (diff + 1) / 2
    } else {
        reference - diff / 2
    }
}
const MODE_CONTEXT: [usize; 13] = [0, 1, 2, 3, 4, 4, 4, 4, 3, 0, 1, 2, 0];
fn symbol<const K: usize>(
    d: &mut SymbolDecoder<'_>,
    c: &mut Cdfs,
    id: usize,
    index: [usize; K],
) -> Result<usize> {
    let cdf = c
        .get(id, index)
        .ok_or_else(|| invalid("invalid AV1 CDF table access"))?;
    d.read(cdf)
}

pub fn decode_intra(s: &Sequence, h: &Header, groups: &[&[u8]], budget: usize) -> Result<Picture> {
    if !matches!(h.frame_type, 0 | 2) {
        return Err(invalid("expected AV1 intra frame"));
    }
    Ok(decode(s, h, groups, budget, None, [None; 8], [0; 8])?.0)
}
pub(crate) fn decode(
    s: &Sequence,
    h: &Header,
    groups: &[&[u8]],
    budget: usize,
    initial: Option<&Cdfs>,
    references: [Option<&Picture>; 8],
    distances: [i32; 8],
) -> Result<(Picture, Cdfs)> {
    if h.reference_mvs {
        return Err(crate::unsupported(
            "AV1 temporal motion field not implemented",
        ));
    }

    // Segment reference/skip/global tools use pre-skip IDs in block decoding.
    if h.intrabc {
        return Err(crate::unsupported(
            "AV1 segmentation/intrabc/superres reconstruction not implemented",
        ));
    }
    if s.color.subsampling != [true, true] {
        return Err(crate::unsupported(
            "AV1 native reconstruction requires 4:2:0",
        ));
    }
    let cols = 2 * (h.size[0] as usize).div_ceil(8);
    let rows = 2 * (h.size[1] as usize).div_ceil(8);
    // Complete edge transforms are needed by chroma-from-luma before cropping.
    let storage_cols = cols.div_ceil(16) * 16;
    let storage_rows = rows.div_ceil(16) * 16;
    let restoration_bytes = restoration::State::required_bytes(h)?;
    let upscale_bytes = if h.superres_denom != 8 {
        (h.upscaled_width as usize)
            .checked_mul(h.size[1] as usize)
            .and_then(|n| n.checked_mul(12))
            .ok_or_else(|| invalid("AV1 upscale allocation overflow"))?
    } else {
        0
    };
    let required = cols
        .checked_mul(rows)
        .and_then(|n| n.checked_mul(448))
        .and_then(|n| n.checked_add(storage_cols.checked_mul(storage_rows)?.checked_mul(100)?))
        .and_then(|n| n.checked_add(500_000))
        .and_then(|n| n.checked_add(restoration_bytes))
        .and_then(|n| n.checked_add(upscale_bytes))
        .ok_or_else(|| invalid("AV1 image allocation overflow"))?;
    if required > budget {
        return Err(invalid("AV1 image exceeds memory budget"));
    }
    let planes = std::array::from_fn(|p| {
        let width = storage_cols * 4 >> usize::from(p > 0);
        let height = storage_rows * 4 >> usize::from(p > 0);
        Plane {
            width,
            height,
            samples: vec![0; width * height],
        }
    });
    let mut previous_segments = vec![0; cols * rows];
    if h.primary_reference != 7 {
        if let Some(primary) = references[h.references[h.primary_reference]] {
            if h.segmentation_enabled
                && primary.segment_grid == [cols, rows]
                && primary.segment_ids.len() == previous_segments.len()
            {
                previous_segments.copy_from_slice(&primary.segment_ids);
            }
        }
    }
    let segment_ids = if h.segmentation_enabled && !h.segmentation_update_map {
        previous_segments.clone()
    } else {
        vec![0; cols * rows]
    };
    let image = Picture {
        size: h.size,
        depth: s.color.depth,
        segment_grid: [cols, rows],
        segment_ids,
        palette_counts: [[0; 7]; 2],
        palette_cache_hits: [0; 2],
        palette_residual_blocks: [0; 2],
        planes,
    };
    let mut dec = Decoder {
        s,
        references,
        biases: distances.map(|v| v > 0),
        distances,
        h,
        image,
        cols,
        rows,
        blocks: vec![Block::default(); cols * rows],
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
        above: std::array::from_fn(|_| vec![(0, 0); cols + 32]),
        left: std::array::from_fn(|_| vec![(0, 0); rows + 32]),
        decoded: std::array::from_fn(|_| vec![false; storage_cols * storage_rows]),
        tx_types: vec![0; cols * rows],
        tx_sizes: std::array::from_fn(|_| vec![[4, 4]; storage_cols * storage_rows]),
        cdef_indexes: vec![-1; cols.div_ceil(16) * rows.div_ceil(16)],
        restoration: restoration::State::new(h),
        read_deltas: false,
        current_q: i32::from(h.quant.base),
        delta_lf: [0; 4],
        current_segment: 0,
        segment_pre_skip: h
            .segments
            .iter()
            .any(|segment| segment[5..].iter().any(Option::is_some)),
        previous_segments,
        segment_pred_above: vec![0; cols],
        segment_pred_left: vec![0; rows],
        current_block: [0; 2],
        scratch: Vec::new(),
        pred_scratch: Vec::new(),
        tx_scratch: Vec::new(),
        residual_scratch: Vec::new(),
        lossless_scratch: Vec::new(),
        lossless_out: Vec::new(),
        inter_pred: Vec::new(),
        inter_pred2: Vec::new(),
        compound_weights: Vec::new(),
    };
    let mut next = 0;
    let mut initial = initial.cloned().unwrap_or_else(|| Cdfs::new(h.quant.base));
    initial.reset_counts();
    let mut saved = initial.clone();
    for group in groups {
        for (tile, payload) in h.tiles.group(group)? {
            if tile != next {
                return Err(invalid("AV1 missing or repeated tile"));
            }
            next += 1;
            let tile_cols = h.tiles.columns.len() - 1;
            let x = tile % tile_cols;
            let y = tile / tile_cols;
            dec.x0 = h.tiles.columns[x] as usize;
            dec.x1 = h.tiles.columns[x + 1] as usize;
            dec.y0 = h.tiles.rows[y] as usize;
            dec.y1 = h.tiles.rows[y + 1] as usize;
            for a in &mut dec.above {
                a.fill((0, 0));
            }
            dec.restoration.reset_tile();
            dec.current_q = i32::from(h.quant.base);
            dec.delta_lf = [0; 4];
            dec.segment_pred_above.fill(0);
            let mut c = initial.clone();
            let mut d = SymbolDecoder::new(payload, !h.disable_cdf_update)?;
            let sb = if s.superblock128 { 32 } else { 16 };
            for r in (dec.y0..dec.y1).step_by(sb) {
                for l in &mut dec.left {
                    l.fill((0, 0));
                }
                dec.segment_pred_left.fill(0);
                for col in (dec.x0..dec.x1).step_by(sb) {
                    if h.restoration_types != [0; 3] {
                        dec.restoration.read(&mut d, &mut c, h, col, r, sb)?;
                    }
                    dec.read_deltas = h.quant.delta_resolution.is_some();
                    dec.partition(&mut d, &mut c, col, r, sb)?;
                }
            }
            d.finish()?;
            if tile == h.tiles.context_tile as usize && !h.disable_frame_end_update {
                saved = c;
            }
        }
    }
    if next != h.tiles.count() {
        return Err(invalid("AV1 frame has missing tiles"));
    }
    // Keep public picture storage and filtering grids at their original MI extent.
    for p in 0..3 {
        let sub = usize::from(p > 0);
        let width = cols * 4 >> sub;
        let height = rows * 4 >> sub;
        let plane = &mut dec.image.planes[p];
        let old_width = plane.width;
        for y in 0..height {
            plane
                .samples
                .copy_within(y * old_width..y * old_width + width, y * width);
        }
        plane.samples.truncate(width * height);
        plane.width = width;
        plane.height = height;
        let old_stride = old_width / 4;
        let stride = width / 4;
        for y in 0..height / 4 {
            dec.tx_sizes[p].copy_within(y * old_stride..y * old_stride + stride, y * stride);
        }
        dec.tx_sizes[p].truncate(stride * (height / 4));
    }
    let restore = dec.restoration.active()?;
    dec.filter();
    let before_restoration = restore.then(|| dec.image.planes.clone());
    let skip = dec.blocks.iter().map(|b| b.skip).collect::<Vec<_>>();
    super::av1_filter::cdef(
        &mut dec.image,
        &h.cdef,
        &dec.cdef_indexes,
        &skip,
        s.color.monochrome,
    );
    if h.superres_denom != 8 {
        dec.image.planes =
            superres::upscale(&dec.image.planes, h.size, h.upscaled_width, s.color.depth)?;
        dec.image.size[0] = h.upscaled_width;
    }
    if let Some(before) = before_restoration {
        let before = if h.superres_denom != 8 {
            superres::upscale(&before, h.size, h.upscaled_width, s.color.depth)?
        } else {
            before
        };
        dec.restoration.apply(&mut dec.image, &before)?;
    }
    Ok((dec.image, saved))
}
impl Decoder<'_> {
    fn read_segment(
        &mut self,
        d: &mut SymbolDecoder<'_>,
        c: &mut Cdfs,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        skip: bool,
    ) -> Result<usize> {
        if !self.h.segmentation_enabled {
            return Ok(0);
        }
        let intra = matches!(self.h.frame_type, 0 | 2);
        let mut predicted = 7;
        for yy in y..(y + h).min(self.rows) {
            for xx in x..(x + w).min(self.cols) {
                predicted = predicted.min(usize::from(self.previous_segments[yy * self.cols + xx]));
            }
        }
        if !intra && !self.h.segmentation_update_map {
            return Ok(predicted);
        }
        let temporal = !intra && self.h.segmentation_temporal_update && !skip;
        let use_previous = temporal
            && symbol(
                d,
                c,
                av1_cdfs::SEGMENT_ID_PREDICTED,
                [self.segment_pred_above[x] + self.segment_pred_left[y]],
            )? != 0;
        let segment = if use_previous {
            predicted
        } else {
            let u =
                (y > self.y0).then(|| usize::from(self.image.segment_ids[(y - 1) * self.cols + x]));
            let l =
                (x > self.x0).then(|| usize::from(self.image.segment_ids[y * self.cols + x - 1]));
            let ul = (y > self.y0 && x > self.x0)
                .then(|| usize::from(self.image.segment_ids[(y - 1) * self.cols + x - 1]));
            let pred = match (u, l) {
                (None, None) => 0,
                (Some(v), None) | (None, Some(v)) => v,
                (Some(a), Some(b)) => {
                    if ul == Some(a) {
                        a
                    } else {
                        b
                    }
                }
            };
            if skip {
                pred
            } else {
                let ctx = if ul.is_none() {
                    0
                } else if ul == u && ul == l {
                    2
                } else if ul == u || ul == l || u == l {
                    1
                } else {
                    0
                };
                let diff = symbol(d, c, av1_cdfs::SEGMENT_ID, [ctx])?;
                let max = self
                    .h
                    .segments
                    .iter()
                    .rposition(|s| s.iter().any(Option::is_some))
                    .unwrap_or(0)
                    + 1;
                if diff >= max || pred >= max {
                    return Err(invalid("AV1 segment ID exceeds active segments"));
                }
                neg_deinterleave(diff, pred, max)
            }
        };
        for xx in x..(x + w).min(self.cols) {
            self.segment_pred_above[xx] = usize::from(use_previous);
        }
        for yy in y..(y + h).min(self.rows) {
            self.segment_pred_left[yy] = usize::from(use_previous);
            for xx in x..(x + w).min(self.cols) {
                self.image.segment_ids[yy * self.cols + xx] = segment as u8;
            }
        }
        Ok(segment)
    }

    fn neighbors(&self, x: usize, y: usize) -> (Option<Block>, Option<Block>) {
        (
            if y > self.y0 {
                Some(self.blocks[(y - 1) * self.cols + x])
            } else {
                None
            },
            if x > self.x0 {
                Some(self.blocks[y * self.cols + x - 1])
            } else {
                None
            },
        )
    }
    fn partition(
        &mut self,
        d: &mut SymbolDecoder<'_>,
        c: &mut Cdfs,
        x: usize,
        y: usize,
        n: usize,
    ) -> Result<()> {
        if x >= self.cols || y >= self.rows {
            return Ok(());
        }
        let half = n / 2;
        let has_rows = y + half < self.rows;
        let has_cols = x + half < self.cols;
        let (above, left) = self.neighbors(x, y);
        let ctx = usize::from(above.is_some_and(|b| b.w < n))
            + 2 * usize::from(left.is_some_and(|b| b.h < n));
        let p = if n == 1 {
            0
        } else if !has_rows && !has_cols {
            3
        } else {
            let id = [
                usize::MAX,
                av1_cdfs::PARTITION_W8,
                av1_cdfs::PARTITION_W16,
                av1_cdfs::PARTITION_W32,
                av1_cdfs::PARTITION_W64,
                av1_cdfs::PARTITION_W128,
            ][n.ilog2() as usize];
            if has_rows && has_cols {
                symbol(d, c, id, [ctx])?
            } else {
                let table = c
                    .get(id, [ctx])
                    .ok_or_else(|| invalid("invalid AV1 CDF table access"))?;
                let indexes: &[usize] = if !has_rows {
                    &[2, 3, 4, 6, 7, 9]
                } else {
                    &[1, 3, 4, 5, 6, 8]
                };
                let sum: u16 = indexes
                    .iter()
                    .filter(|i| **i < table.len() - 1)
                    .map(|i| table[*i] - table[*i - 1])
                    .sum();
                if d.read(&mut [32768 - sum, 32768, 0])? == 1 {
                    3
                } else if !has_rows {
                    1
                } else {
                    2
                }
            }
        };
        match p {
            0 => self.block(d, c, x, y, n, n),
            1 => {
                self.block(d, c, x, y, n, half)?;
                if has_rows {
                    self.block(d, c, x, y + half, n, half)?;
                }
                Ok(())
            }
            2 => {
                self.block(d, c, x, y, half, n)?;
                if has_cols {
                    self.block(d, c, x + half, y, half, n)?;
                }
                Ok(())
            }
            3 => {
                for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
                    self.partition(d, c, x + dx, y + dy, half)?;
                }
                Ok(())
            }
            4 => {
                self.block(d, c, x, y, half, half)?;
                self.block(d, c, x + half, y, half, half)?;
                self.block(d, c, x, y + half, n, half)
            }
            5 => {
                self.block(d, c, x, y, n, half)?;
                self.block(d, c, x, y + half, half, half)?;
                self.block(d, c, x + half, y + half, half, half)
            }
            6 => {
                self.block(d, c, x, y, half, half)?;
                self.block(d, c, x, y + half, half, half)?;
                self.block(d, c, x + half, y, half, n)
            }
            7 => {
                self.block(d, c, x, y, half, n)?;
                self.block(d, c, x + half, y, half, half)?;
                self.block(d, c, x + half, y + half, half, half)
            }
            8 | 9 => {
                for i in 0..4 {
                    let (xx, yy, w, h) = if p == 8 {
                        (x, y + i * n / 4, n, n / 4)
                    } else {
                        (x + i * n / 4, y, n / 4, n)
                    };
                    if xx < self.cols && yy < self.rows {
                        self.block(d, c, xx, yy, w, h)?;
                    }
                }
                Ok(())
            }
            _ => Err(invalid("invalid AV1 partition")),
        }
    }
    fn block(
        &mut self,
        d: &mut SymbolDecoder<'_>,
        c: &mut Cdfs,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
    ) -> Result<()> {
        let (above, left) = self.neighbors(x, y);
        self.current_block = [x, y];
        let pre_skip = self.segment_pre_skip;
        if pre_skip {
            self.current_segment = self.read_segment(d, c, x, y, w, h, false)?;
        }
        let forced_reference = self.h.segments[self.current_segment][5];
        let forced_skip = self.h.segments[self.current_segment][6].is_some();
        let forced_global = self.h.segments[self.current_segment][7].is_some();
        let skip_ctx =
            usize::from(above.is_some_and(|b| b.skip)) + usize::from(left.is_some_and(|b| b.skip));
        let skip_mode = !forced_skip
            && !forced_global
            && forced_reference.is_none()
            && self.h.skip_mode.is_some()
            && w >= 2
            && h >= 2
            && symbol(
                d,
                c,
                av1_cdfs::SKIP_MODE,
                [usize::from(above.is_some_and(|b| b.skip_mode))
                    + usize::from(left.is_some_and(|b| b.skip_mode))],
            )? != 0;
        let skip = forced_skip || skip_mode || symbol(d, c, av1_cdfs::SKIP, [skip_ctx])? != 0;
        if !pre_skip {
            self.current_segment = self.read_segment(d, c, x, y, w, h, skip)?;
        }
        if !skip && !self.h.lossless.iter().all(|v| *v) && self.s.cdef && !self.h.intrabc {
            let stride = self.cols.div_ceil(16);
            let index = (y / 16) * stride + x / 16;
            if self.cdef_indexes[index] < 0 {
                let value = d.literal(self.h.cdef.bits)? as i8;
                for yy in (y / 16)..(y / 16) + h.div_ceil(16) {
                    for xx in (x / 16)..(x / 16) + w.div_ceil(16) {
                        if yy < self.rows.div_ceil(16) && xx < stride {
                            self.cdef_indexes[yy * stride + xx] = value;
                        }
                    }
                }
            }
        }
        if self.read_deltas && !(skip && w == if self.s.superblock128 { 32 } else { 16 } && w == h)
        {
            let mut value = symbol(d, c, av1_cdfs::DELTA_Q, [])? as i32;
            if value == 3 {
                let bits = d.literal(3)? as u8 + 1;
                value = d.literal(bits)? as i32 + (1 << bits) + 1;
            }
            if value != 0 {
                if d.bit()? {
                    value = -value;
                }
                self.current_q = (self.current_q
                    + (value << self.h.quant.delta_resolution.unwrap()))
                .clamp(1, 255);
            }
        }
        if self.read_deltas && !(skip && w == if self.s.superblock128 { 32 } else { 16 } && w == h)
        {
            if let Some((resolution, multi)) = self.h.filter.delta_resolution {
                let count = if multi {
                    if self.s.color.monochrome { 2 } else { 4 }
                } else {
                    1
                };
                for i in 0..count {
                    let mut value = if multi {
                        symbol(d, c, av1_cdfs::DELTA_LF_MULTI, [i])?
                    } else {
                        symbol(d, c, av1_cdfs::DELTA_LF, [])?
                    } as i32;
                    if value == 3 {
                        let bits = d.literal(3)? as u8 + 1;
                        value = d.literal(bits)? as i32 + (1 << bits) + 1;
                    }
                    if value != 0 {
                        if d.bit()? {
                            value = -value;
                        }
                        self.delta_lf[i] =
                            (self.delta_lf[i] + (value << resolution)).clamp(-63, 63);
                    }
                }
            }
        }
        self.read_deltas = false;
        if !matches!(self.h.frame_type, 0 | 2) {
            let ctx = match (above, left) {
                (Some(a), Some(l)) => {
                    if a.reference == 0 && l.reference == 0 {
                        3
                    } else {
                        usize::from(a.reference == 0 || l.reference == 0)
                    }
                }
                (Some(a), None) | (None, Some(a)) => 2 * usize::from(a.reference == 0),
                _ => 0,
            };
            let is_inter = if let Some(reference) = forced_reference {
                reference != 0
            } else if forced_global {
                true
            } else {
                skip_mode || symbol(d, c, av1_cdfs::IS_INTER, [ctx])? != 0
            };
            if is_inter {
                return self.inter_block(d, c, x, y, w, h, skip, skip_mode);
            }
        }
        let ac = MODE_CONTEXT[above.map_or(0, |b| if b.reference == 0 { b.mode } else { 0 })];
        let lc = MODE_CONTEXT[left.map_or(0, |b| if b.reference == 0 { b.mode } else { 0 })];
        let mode = if matches!(self.h.frame_type, 0 | 2) {
            symbol(d, c, av1_cdfs::INTRA_FRAME_Y_MODE, [ac, lc])?
        } else {
            symbol(d, c, av1_cdfs::Y_MODE, [(w.min(h).ilog2() as usize).min(3)])?
        };
        let mut angle = 0i32;
        if w >= 2 && h >= 2 && (1..=8).contains(&mode) {
            angle = symbol(d, c, av1_cdfs::ANGLE_DELTA, [mode - 1])? as i32 - 3;
        }
        let has_chroma =
            !self.s.color.monochrome && !(w == 1 && x % 2 == 0 || h == 1 && y % 2 == 0);
        let mut uv = 0;
        let mut uv_angle = 0;
        let mut cfl = [0i32; 2];
        if has_chroma {
            let cfl_allowed = if self.h.lossless[self.current_segment] {
                w <= 2 && h <= 2
            } else {
                w.max(h) <= 8
            };
            uv = symbol(
                d,
                c,
                if cfl_allowed {
                    av1_cdfs::UV_MODE_CFL_ALLOWED
                } else {
                    av1_cdfs::UV_MODE_CFL_NOT_ALLOWED
                },
                [mode],
            )?;
            if uv == 13 {
                let signs = symbol(d, c, av1_cdfs::CFL_SIGN, [])? + 1;
                let (su, sv) = (signs / 3, signs % 3);
                if su != 0 {
                    cfl[0] = (symbol(d, c, av1_cdfs::CFL_ALPHA, [(su - 1) * 3 + sv])? as i32 + 1)
                        * if su == 1 { -1 } else { 1 };
                }
                if sv != 0 {
                    cfl[1] = (symbol(d, c, av1_cdfs::CFL_ALPHA, [(sv - 1) * 3 + su])? as i32 + 1)
                        * if sv == 1 { -1 } else { 1 };
                }
            }
            if w >= 2 && h >= 2 && (1..=8).contains(&uv) {
                uv_angle = symbol(d, c, av1_cdfs::ANGLE_DELTA, [uv - 1])? as i32 - 3;
            }
        }
        let mut palette_sizes = [0u8; 2];
        let mut palette_colors = [[0u16; 8]; 3];
        if self.h.screen_content && w >= 2 && h >= 2 && w <= 16 && h <= 16 {
            let size_ctx = (w * h).ilog2() as usize - 2;
            let ctx = usize::from(above.is_some_and(|b| b.palette_sizes[0] > 0))
                + usize::from(left.is_some_and(|b| b.palette_sizes[0] > 0));
            for plane in 0..2 {
                let eligible = if plane == 0 {
                    mode == 0
                } else {
                    has_chroma && uv == 0
                };
                let present = eligible
                    && if plane == 0 {
                        symbol(d, c, av1_cdfs::PALETTE_Y_MODE, [size_ctx, ctx])? != 0
                    } else {
                        symbol(
                            d,
                            c,
                            av1_cdfs::PALETTE_UV_MODE,
                            [usize::from(palette_sizes[0] > 0)],
                        )? != 0
                    };
                if !present {
                    continue;
                }
                let n = 2 + symbol(
                    d,
                    c,
                    if plane == 0 {
                        av1_cdfs::PALETTE_Y_SIZE
                    } else {
                        av1_cdfs::PALETTE_UV_SIZE
                    },
                    [size_ctx],
                )?;
                palette_sizes[plane] = n as u8;
                self.image.palette_counts[plane][n - 2] += 1;
                let mut cache = Vec::new();
                if y * 4 % 64 != 0 {
                    if let Some(b) = above {
                        cache.extend_from_slice(
                            &b.palette_colors[plane][..b.palette_sizes[plane] as usize],
                        );
                    }
                }
                if let Some(b) = left {
                    cache.extend_from_slice(
                        &b.palette_colors[plane][..b.palette_sizes[plane] as usize],
                    );
                }
                cache.sort_unstable();
                cache.dedup();
                let (colors, hits) = palette::colors(d, &cache, n, self.s.color.depth, plane > 0)?;
                palette_colors[plane] = colors;
                self.image.palette_cache_hits[plane] += hits;
                if plane == 1 {
                    palette_colors[2] = palette::v_colors(d, n, self.s.color.depth)?;
                }
            }
        }
        let mut filter_mode = None;
        if self.s.filter_intra && mode == 0 && palette_sizes[0] == 0 && w <= 8 && h <= 8 {
            let sizes = [
                (1, 1),
                (1, 2),
                (2, 1),
                (2, 2),
                (2, 4),
                (4, 2),
                (4, 4),
                (4, 8),
                (8, 4),
                (8, 8),
                (8, 16),
                (16, 8),
                (16, 16),
                (16, 32),
                (32, 16),
                (32, 32),
                (1, 4),
                (4, 1),
                (2, 8),
                (8, 2),
                (4, 16),
                (16, 4),
            ];
            let size_id = sizes
                .iter()
                .position(|v| *v == (w, h))
                .ok_or_else(|| invalid("invalid AV1 block size"))?;
            if symbol(d, c, av1_cdfs::FILTER_INTRA, [size_id])? != 0 {
                filter_mode = Some(symbol(d, c, av1_cdfs::FILTER_INTRA_MODE, [])?);
            }
        }
        for group in 0..2 {
            let n = palette_sizes[group] as usize;
            if n == 0 {
                continue;
            }
            let bw = (w * 4) >> group;
            let bh = (h * 4) >> group;
            let visible = [
                ((self.cols - x) * 4).min(w * 4) >> group,
                ((self.rows - y) * 4).min(h * 4) >> group,
            ];
            let map = palette::map(d, c, n, group > 0, [bw, bh], visible)?;
            for p in if group == 0 { 0..1 } else { 1..3 } {
                let plane = &mut self.image.planes[p];
                let px = x * 4 >> group;
                let py = y * 4 >> group;
                for row in 0..bh.min(plane.height - py) {
                    for col in 0..bw.min(plane.width - px) {
                        plane.samples[(py + row) * plane.width + px + col] =
                            palette_colors[p][map[row * bw + col] as usize];
                    }
                }
            }
        }
        let mut tx = if self.h.lossless[self.current_segment] {
            [4, 4]
        } else {
            [(w * 4).min(64), (h * 4).min(64)]
        };
        // AV1 read_tx_size returns TX_4X4 immediately for this segment's
        // lossless blocks; no tx_depth symbol is present even in SELECT mode.
        if self.h.tx_mode == 2 && w * h > 1 && !self.h.lossless[self.current_segment] {
            let ctx = usize::from(
                above.is_some_and(|b| (if b.reference > 0 { b.w * 4 } else { b.tx[0] }) >= tx[0]),
            ) + usize::from(
                left.is_some_and(|b| (if b.reference > 0 { b.h * 4 } else { b.tx[1] }) >= tx[1]),
            );
            let depth = tx[0].max(tx[1]).ilog2() - 2;
            let id = [
                usize::MAX,
                av1_cdfs::TX_8X8,
                av1_cdfs::TX_16X16,
                av1_cdfs::TX_32X32,
                av1_cdfs::TX_64X64,
            ][depth as usize];
            for _ in 0..symbol(d, c, id, [ctx])? {
                if tx[0] > tx[1] {
                    tx[0] /= 2;
                } else if tx[1] > tx[0] {
                    tx[1] /= 2;
                } else {
                    tx[0] /= 2;
                    tx[1] /= 2;
                }
            }
        }
        for yy in y..(y + h).min(self.rows) {
            for xx in x..(x + w).min(self.cols) {
                self.blocks[yy * self.cols + xx] = Block {
                    w,
                    h,
                    mode,
                    segment: self.current_segment,
                    skip,
                    tx,
                    uv_mode: uv,
                    delta_lf: self.delta_lf,
                    palette_sizes,
                    palette_colors: [palette_colors[0], palette_colors[1]],
                    ..Block::default()
                };
            }
        }
        for cy in 0..h.div_ceil(16) {
            for cx in 0..w.div_ceil(16) {
                let mut max_luma = [0; 2];
                for p in 0..if has_chroma { 3 } else { 1 } {
                    let sub = usize::from(p > 0);
                    let bw = (w >> sub).max(1);
                    let bh = (h >> sub).max(1);
                    let chunk_w = (w.min(16) >> sub).max(1);
                    let chunk_h = (h.min(16) >> sub).max(1);
                    let base_x = (x >> sub) + cx * (16 >> sub);
                    let base_y = (y >> sub) + cy * (16 >> sub);
                    let size = if self.h.lossless[self.current_segment] {
                        [4, 4]
                    } else if p == 0 {
                        tx
                    } else {
                        [(bw * 4).min(32), (bh * 4).min(32)]
                    };
                    let [tw, th] = size;
                    let smooth_neighbor = above
                        .into_iter()
                        .chain(left)
                        .any(|b| (9..=11).contains(&if p == 0 { b.mode } else { b.uv_mode }));
                    for yy in (base_y..base_y + chunk_h).step_by(th / 4) {
                        for xx in (base_x..base_x + chunk_w).step_by(tw / 4) {
                            if xx >= self.cols >> sub || yy >= self.rows >> sub {
                                continue;
                            }
                            let m = if p == 0 { mode } else { uv };
                            let angle = if p == 0 { angle } else { uv_angle };
                            if palette_sizes[usize::from(p > 0)] == 0 {
                                self.predict(
                                    p,
                                    xx * 4,
                                    yy * 4,
                                    size,
                                    if m == 13 { 0 } else { m },
                                    angle,
                                    if p == 0 { filter_mode } else { None },
                                    smooth_neighbor,
                                )?;
                            }
                            if m == 13 {
                                self.cfl(p, xx * 4, yy * 4, size, cfl[p - 1], max_luma)?;
                            }
                            let (dequant, kind) = if skip {
                                for i in 0..tw / 4 {
                                    self.above[p][xx + i] = (0, 0);
                                }
                                for i in 0..th / 4 {
                                    self.left[p][yy + i] = (0, 0);
                                }
                                (vec![0; tw * th], 0)
                            } else {
                                self.coefficients(
                                    d,
                                    c,
                                    p,
                                    xx,
                                    yy,
                                    bw,
                                    bh,
                                    size,
                                    if p == 0 {
                                        filter_mode.map_or(mode, |f| [0, 1, 2, 6, 0][f])
                                    } else {
                                        uv
                                    },
                                )?
                            };
                            if palette_sizes[usize::from(p > 0)] > 0
                                && dequant.iter().any(|v| *v != 0)
                            {
                                self.image.palette_residual_blocks[usize::from(p > 0)] += 1;
                            }
                            let residual: &[i32] = if self.h.lossless[self.current_segment] {
                                vp9_transform::inverse(
                                    &dequant,
                                    4,
                                    self.s.color.depth,
                                    Kind::Lossless,
                                    &mut self.lossless_scratch,
                                    &mut self.lossless_out,
                                )?;
                                &self.lossless_out[..4 * 4]
                            } else {
                                super::av1_transform::inverse(
                                    &dequant,
                                    size,
                                    self.s.color.depth,
                                    kind,
                                    &mut self.tx_scratch,
                                    &mut self.residual_scratch,
                                )?;
                                &self.residual_scratch[..size[0] * size[1]]
                            };
                            let plane = &mut self.image.planes[p];
                            for r in 0..th {
                                for col in 0..tw {
                                    if yy * 4 + r >= plane.height || xx * 4 + col >= plane.width {
                                        continue;
                                    }
                                    let i = (yy * 4 + r) * plane.width + xx * 4 + col;
                                    plane.samples[i] = (i32::from(plane.samples[i])
                                        + residual[r * tw + col])
                                        .clamp(0, (1 << self.s.color.depth) - 1)
                                        as u16;
                                }
                            }
                            if p == 0 {
                                max_luma = [xx * 4 + tw, yy * 4 + th];
                            }
                            for r in yy..(yy + th / 4).min(plane.height / 4) {
                                for col in xx..(xx + tw / 4).min(plane.width / 4) {
                                    self.decoded[p][r * (plane.width / 4) + col] = true;
                                    self.tx_sizes[p][r * (plane.width / 4) + col] = size;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn predict(
        &mut self,
        p: usize,
        x: usize,
        y: usize,
        size: [usize; 2],
        mode: usize,
        angle: i32,
        filter_mode: Option<usize>,
        smooth_neighbor: bool,
    ) -> Result<()> {
        let [w, h] = size;
        let sub = usize::from(p > 0);
        let above = y > (self.y0 * 4 >> sub);
        let left = x > (self.x0 * 4 >> sub);
        let mid = 1u16 << (self.s.color.depth - 1);
        let plane = &self.image.planes[p];
        let grid = plane.width / 4;
        let logical_width = self.cols * 4 >> sub;
        let logical_height = self.rows * 4 >> sub;
        let above_right =
            above && x + w < logical_width && self.decoded[p][((y - 1) / 4) * grid + (x + w) / 4];
        let below_left =
            left && y + h < logical_height && self.decoded[p][((y + h) / 4) * grid + (x - 1) / 4];
        let top: Vec<u16> = (0..w + h)
            .map(|i| {
                if above {
                    plane.samples[(y - 1) * plane.width
                        + (x + i)
                            .min(logical_width - 1)
                            .min(x + if above_right { 2 * w } else { w } - 1)]
                } else if left {
                    plane.samples[y * plane.width + x - 1]
                } else {
                    mid - 1
                }
            })
            .collect();
        let side: Vec<u16> = (0..w + h)
            .map(|i| {
                if left {
                    plane.samples[(y + i)
                        .min(logical_height - 1)
                        .min(y + if below_left { 2 * h } else { h } - 1)
                        * plane.width
                        + x
                        - 1]
                } else if above {
                    plane.samples[(y - 1) * plane.width + x]
                } else {
                    mid + 1
                }
            })
            .collect();
        let corner = if above && left {
            plane.samples[(y - 1) * plane.width + x - 1]
        } else if above {
            top[0]
        } else if left {
            side[0]
        } else {
            mid
        };
        self.pred_scratch.resize(w * h, 0);
        super::av1_intra::predict(
            size,
            self.s.color.depth,
            mode,
            angle,
            filter_mode,
            super::av1_intra::Edges {
                above: &top,
                left: &side,
                corner,
                have_above: above,
                have_left: left,
                edge_filter: self.s.intra_edge_filter,
                smooth_neighbor,
            },
            &mut self.pred_scratch,
        )?;
        let plane = &mut self.image.planes[p];
        for r in 0..h.min(plane.height - y) {
            for col in 0..w.min(plane.width - x) {
                plane.samples[(y + r) * plane.width + x + col] = self.pred_scratch[r * w + col];
            }
        }
        Ok(())
    }
    fn cfl(
        &mut self,
        p: usize,
        x: usize,
        y: usize,
        size: [usize; 2],
        alpha: i32,
        max_luma: [usize; 2],
    ) -> Result<()> {
        let [w, h] = size;
        let luma = &self.image.planes[0];
        let mut values = vec![0i32; w * h];
        for r in 0..h {
            for col in 0..w {
                let lx = ((x + col) * 2).min(max_luma[0] - 2);
                let ly = ((y + r) * 2).min(max_luma[1] - 2);
                values[r * w + col] = 2 * [
                    luma.samples[ly * luma.width + lx],
                    luma.samples[ly * luma.width + lx + 1],
                    luma.samples[(ly + 1) * luma.width + lx],
                    luma.samples[(ly + 1) * luma.width + lx + 1],
                ]
                .iter()
                .map(|v| i32::from(*v))
                .sum::<i32>();
            }
        }
        let average = (values.iter().sum::<i32>() + (w * h / 2) as i32) / (w * h) as i32;
        let plane = &mut self.image.planes[p];
        for r in 0..h.min(plane.height - y) {
            for col in 0..w.min(plane.width - x) {
                let v = (values[r * w + col] - average) * alpha;
                let delta = v.signum() * ((v.abs() + 32) >> 6);
                let index = (y + r) * plane.width + x + col;
                plane.samples[index] = (i32::from(plane.samples[index]) + delta)
                    .clamp(0, (1 << self.s.color.depth) - 1)
                    as u16;
            }
        }
        Ok(())
    }
    fn filter(&mut self) {
        for pass in 0..2 {
            for p in 0..if self.s.color.monochrome { 1 } else { 3 } {
                let filter_index = if p == 0 { pass } else { p + 1 };
                let base = i32::from(self.h.filter.levels[filter_index]);
                if (p > 0 && base == 0) || (p == 0 && self.h.filter.levels[..2] == [0, 0]) {
                    continue;
                }
                let sub = usize::from(p > 0);
                let plane = &mut self.image.planes[p];
                let stride = plane.width / 4;
                for y in (0..self.rows * 4 >> sub).step_by(4) {
                    for x in (0..self.cols * 4 >> sub).step_by(4) {
                        if x << sub >= self.h.size[0] as usize
                            || y << sub >= self.h.size[1] as usize
                            || (pass == 0 && x == 0)
                            || (pass == 1 && y == 0)
                        {
                            continue;
                        }
                        let row = ((y / 4) << sub) | sub;
                        let col = ((x / 4) << sub) | sub;
                        let block = self.blocks[row * self.cols + col];
                        let block_extent = if pass == 0 {
                            (block.w * 4 >> sub).max(4)
                        } else {
                            (block.h * 4 >> sub).max(4)
                        };
                        if block.skip
                            && block.reference > 0
                            && (if pass == 0 { x } else { y }) % block_extent != 0
                        {
                            continue;
                        }
                        let strength = |b: Block| {
                            let lf_index = if self
                                .h
                                .filter
                                .delta_resolution
                                .is_some_and(|(_, multi)| multi)
                            {
                                filter_index
                            } else {
                                0
                            };
                            let adjusted_base = (base + b.delta_lf[lf_index]).clamp(0, 63);
                            let segment_level = (adjusted_base
                                + self.h.segments[b.segment][1 + filter_index].unwrap_or(0))
                            .clamp(0, 63);
                            if !self.h.filter.deltas_enabled {
                                return segment_level;
                            }
                            let mode = usize::from(b.mode >= 13 && b.mode != 15);
                            let delta = self.h.filter.reference_deltas[b.reference]
                                + if b.reference > 0 {
                                    self.h.filter.mode_deltas[mode]
                                } else {
                                    0
                                };
                            (segment_level + (delta << (segment_level >> 5))).clamp(0, 63)
                        };
                        let mut level = strength(block);
                        if level == 0 {
                            let previous = if pass == 0 {
                                row * self.cols + col - (1 << sub)
                            } else {
                                (row - (1 << sub)) * self.cols + col
                            };
                            level = strength(self.blocks[previous]);
                        }
                        if level == 0 {
                            continue;
                        }
                        let index = (y / 4) * stride + x / 4;
                        let prev = if pass == 0 { index - 1 } else { index - stride };
                        let current = self.tx_sizes[p][index][pass];
                        if (if pass == 0 { x } else { y }) % current != 0 {
                            continue;
                        }
                        let width = current.min(self.tx_sizes[p][prev][pass]).min(if p == 0 {
                            16
                        } else {
                            8
                        });
                        let (dx, dy) = if pass == 0 { (1isize, 0isize) } else { (0, 1) };
                        // An edge keeps its whole 14-sample window inside the plane once
                        // the eight-sample halo around it does; then taps need neither
                        // clamping nor per-sample bounds tests, and a vertical edge even
                        // reads and writes one contiguous run.
                        let straight = if pass == 0 {
                            x >= 7 && x + 7 <= plane.width && y + 4 <= plane.height
                        } else {
                            y >= 7 && y + 7 <= plane.height && x + 4 <= plane.width
                        };
                        let tap = (dx + dy * plane.width as isize) as usize;
                        let line = (dy + dx * plane.width as isize) as usize;
                        let origin = y * plane.width + x;
                        for i in 0..4 {
                            let base = origin + i * line;
                            let xx = x as isize + dy * i as isize;
                            let yy = y as isize + dx * i as isize;
                            let samples = if straight {
                                let mut s = [0u16; 14];
                                if pass == 0 {
                                    s.copy_from_slice(&plane.samples[base - 7..base + 7]);
                                } else {
                                    let mut at = base as isize - 7 * tap as isize;
                                    for v in s.iter_mut() {
                                        *v = plane.samples[at as usize];
                                        at += tap as isize;
                                    }
                                }
                                s
                            } else {
                                std::array::from_fn(|k| {
                                    let t = k as isize - 7;
                                    let sx =
                                        (xx + dx * t).clamp(0, plane.width as isize - 1) as usize;
                                    let sy =
                                        (yy + dy * t).clamp(0, plane.height as isize - 1) as usize;
                                    plane.samples[sy * plane.width + sx]
                                })
                            };
                            let filtered = super::av1_filter::edge(
                                samples,
                                self.s.color.depth,
                                width,
                                p != 0,
                                level,
                                self.h.filter.sharpness,
                            );
                            if straight {
                                if pass == 0 {
                                    plane.samples[base - 7..base + 7].copy_from_slice(&filtered);
                                } else {
                                    let mut at = base as isize - 7 * tap as isize;
                                    for v in filtered {
                                        plane.samples[at as usize] = v;
                                        at += tap as isize;
                                    }
                                }
                            } else {
                                for (k, value) in filtered.into_iter().enumerate() {
                                    let t = k as isize - 7;
                                    let sx = xx + dx * t;
                                    let sy = yy + dy * t;
                                    if sx >= 0
                                        && sy >= 0
                                        && sx < plane.width as isize
                                        && sy < plane.height as isize
                                    {
                                        plane.samples[sy as usize * plane.width + sx as usize] =
                                            value;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    fn coefficients(
        &mut self,
        d: &mut SymbolDecoder<'_>,
        c: &mut Cdfs,
        p: usize,
        x: usize,
        y: usize,
        bw: usize,
        bh: usize,
        size: [usize; 2],
        mode: usize,
    ) -> Result<(Vec<i32>, u8)> {
        use super::av1_tables::*;
        let [w, h] = size;
        let (w4, h4) = (w / 4, h / 4);
        let tw = w.min(32);
        let th = h.min(32);
        let min_log = w.min(h).ilog2() as usize - 2;
        let max_log = w.max(h).ilog2() as usize - 2;
        let txctx = (min_log + max_log + 1) / 2;
        let sub = usize::from(p > 0);
        let max_x = self.cols >> sub;
        let max_y = self.rows >> sub;
        let top_slice = &self.above[p][x..(x + w4).min(max_x)];
        let left_slice = &self.left[p][y..(y + h4).min(max_y)];
        let top = top_slice.iter().map(|v| v.0).max().unwrap_or(0);
        let left = left_slice.iter().map(|v| v.0).max().unwrap_or(0);
        let ctx = if p == 0 {
            if bw == w4 && bh == h4 {
                0
            } else if top == 0 && left == 0 {
                1
            } else if top == 0 || left == 0 {
                2 + usize::from(top.max(left) > 3)
            } else if top.max(left) <= 3 {
                4
            } else if top.min(left) <= 3 {
                5
            } else {
                6
            }
        } else {
            7 + usize::from(top_slice.iter().any(|v| v.0 | v.1 != 0))
                + usize::from(left_slice.iter().any(|v| v.0 | v.1 != 0))
                + 3 * usize::from(bw * bh > w4 * h4)
        };
        let dc_sum = top_slice
            .iter()
            .chain(left_slice)
            .map(|v| {
                if v.1 == 1 {
                    -1
                } else if v.1 == 2 {
                    1
                } else {
                    0
                }
            })
            .sum::<i32>();
        let mut q = vec![0i32; tw * th];
        let ptype = usize::from(p > 0);
        let mut kind = 0;
        let mut total = 0;
        let mut dc_category = 0;
        if symbol(d, c, av1_cdfs::TXB_SKIP, [txctx, ctx])? == 0 {
            let inter = self.blocks[self.current_block[1] * self.cols + self.current_block[0]]
                .reference
                != 0;
            if inter && !self.h.lossless[self.current_segment] && w.max(h) <= 32 {
                if p == 0 && self.h.quant.base > 0 {
                    kind = if self.h.reduced_tx_set || w.max(h) == 32 {
                        [9, 0][symbol(d, c, av1_cdfs::INTER_TX_TYPE_SET3, [min_log])?]
                    } else if w.min(h) == 16 {
                        [9, 10, 11, 0, 1, 2, 4, 5, 3, 6, 7, 8]
                            [symbol(d, c, av1_cdfs::INTER_TX_TYPE_SET2, [])?]
                    } else {
                        [9, 10, 11, 12, 13, 14, 15, 0, 1, 2, 4, 5, 3, 6, 7, 8]
                            [symbol(d, c, av1_cdfs::INTER_TX_TYPE_SET1, [min_log])?]
                    };
                } else if p > 0 {
                    kind = self.tx_types[(y << sub).max(self.current_block[1]) * self.cols
                        + (x << sub).max(self.current_block[0])];
                    if (self.h.reduced_tx_set || w.max(h) == 32) && kind != 0 && kind != 9
                        || w.min(h) == 16 && kind >= 12
                    {
                        kind = 0;
                    }
                }
            } else if !inter && !self.h.lossless[self.current_segment] && w.max(h) < 32 {
                if p == 0 && self.h.quant.base > 0 {
                    kind = if self.h.reduced_tx_set || w.min(h) == 16 {
                        [9, 0, 3, 1, 2]
                            [symbol(d, c, av1_cdfs::INTRA_TX_TYPE_SET2, [min_log, mode])?]
                    } else {
                        [9, 0, 10, 11, 3, 1, 2]
                            [symbol(d, c, av1_cdfs::INTRA_TX_TYPE_SET1, [min_log, mode])?]
                    };
                } else if p > 0 {
                    kind = [0, 1, 2, 0, 3, 1, 2, 2, 1, 3, 1, 2, 3, 0][mode];
                }
            }
            if p == 0 {
                for yy in y..(y + h4).min(self.rows) {
                    for xx in x..(x + w4).min(self.cols) {
                        self.tx_types[yy * self.cols + xx] = kind;
                    }
                }
            }
            let class = match kind {
                10 | 12 | 14 => 2,
                11 | 13 | 15 => 1,
                _ => 0,
            };
            let scan: Vec<usize> = if class == 2 {
                (0..tw * th).collect()
            } else if class == 1 {
                (0..tw * th).map(|i| (i % th) * tw + i / th).collect()
            } else {
                let array: &[i32] = match (tw, th) {
                    (4, 4) => &DEFAULT_SCAN_4X4,
                    (4, 8) => &DEFAULT_SCAN_4X8,
                    (8, 4) => &DEFAULT_SCAN_8X4,
                    (8, 8) => &DEFAULT_SCAN_8X8,
                    (8, 16) => &DEFAULT_SCAN_8X16,
                    (16, 8) => &DEFAULT_SCAN_16X8,
                    (16, 16) => &DEFAULT_SCAN_16X16,
                    (16, 32) => &DEFAULT_SCAN_16X32,
                    (32, 16) => &DEFAULT_SCAN_32X16,
                    (32, 32) => &DEFAULT_SCAN_32X32,
                    (4, 16) => &DEFAULT_SCAN_4X16,
                    (16, 4) => &DEFAULT_SCAN_16X4,
                    (8, 32) => &DEFAULT_SCAN_8X32,
                    (32, 8) => &DEFAULT_SCAN_32X8,
                    _ => return Err(invalid("invalid AV1 coefficient scan size")),
                };
                array.iter().map(|v| *v as usize).collect()
            };
            let eob_multi = tw.ilog2() + th.ilog2() - 4;
            let id = [
                av1_cdfs::EOB_PT_16,
                av1_cdfs::EOB_PT_32,
                av1_cdfs::EOB_PT_64,
                av1_cdfs::EOB_PT_128,
                av1_cdfs::EOB_PT_256,
                av1_cdfs::EOB_PT_512,
                av1_cdfs::EOB_PT_1024,
            ][eob_multi as usize];
            let pt = if eob_multi < 5 {
                symbol(d, c, id, [ptype, usize::from(class != 0)])?
            } else {
                symbol(d, c, id, [ptype])?
            } + 1;
            let mut eob = if pt < 2 { pt } else { (1 << (pt - 2)) + 1 };
            if pt >= 3 {
                eob += symbol(d, c, av1_cdfs::EOB_EXTRA, [txctx, ptype, pt - 3])? << (pt - 3);
                if pt >= 4 {
                    eob += d.literal((pt - 3) as u8)? as usize;
                }
            }
            if eob > q.len() {
                return Err(invalid("AV1 EOB exceeds transform"));
            }
            let shapes = [
                [4, 4],
                [8, 8],
                [16, 16],
                [32, 32],
                [64, 64],
                [4, 8],
                [8, 4],
                [8, 16],
                [16, 8],
                [16, 32],
                [32, 16],
                [32, 64],
                [64, 32],
                [4, 16],
                [16, 4],
                [8, 32],
                [32, 8],
                [16, 64],
                [64, 16],
            ];
            let txid = shapes
                .iter()
                .position(|s| *s == size)
                .ok_or_else(|| invalid("invalid AV1 transform dimensions"))?;
            for i in (0..eob).rev() {
                let pos = scan[i];
                let r = pos / tw;
                let col = pos % tw;
                let mut level = if i == eob - 1 {
                    let ctx = if i == 0 {
                        0
                    } else if i <= q.len() / 8 {
                        1
                    } else if i <= q.len() / 4 {
                        2
                    } else {
                        3
                    };
                    symbol(d, c, av1_cdfs::COEFF_BASE_EOB, [txctx, ptype, ctx])? as i32 + 1
                } else {
                    let neighbors = [
                        [(0, 1), (1, 0), (1, 1), (0, 2), (2, 0)],
                        [(0, 1), (1, 0), (0, 2), (0, 3), (0, 4)],
                        [(0, 1), (1, 0), (2, 0), (3, 0), (4, 0)],
                    ];
                    let mut mag = 0;
                    for (dy, dx) in neighbors[class] {
                        if r + dy < th && col + dx < tw {
                            mag += q[(r + dy) * tw + col + dx].min(3);
                        }
                    }
                    let mut ctx = ((mag + 1) / 2).min(4) as usize;
                    if class == 0 {
                        ctx = if pos == 0 {
                            0
                        } else {
                            ctx + COEFF_BASE_CTX_OFFSET[txid][r.min(4)][col.min(4)] as usize
                        };
                    } else {
                        ctx += 26 + 5 * if class == 2 { r.min(2) } else { col.min(2) };
                    }
                    symbol(d, c, av1_cdfs::COEFF_BASE, [txctx, ptype, ctx])? as i32
                };
                if level > 2 {
                    let neighbors = [
                        [(0, 1), (1, 0), (1, 1)],
                        [(0, 1), (1, 0), (0, 2)],
                        [(0, 1), (1, 0), (2, 0)],
                    ];
                    let mut mag = 0;
                    for (dy, dx) in neighbors[class] {
                        if r + dy < th && col + dx < tw {
                            mag += q[(r + dy) * tw + col + dx].min(15);
                        }
                    }
                    let near = if class == 0 {
                        r < 2 && col < 2
                    } else if class == 1 {
                        col == 0
                    } else {
                        r == 0
                    };
                    let ctx = ((mag + 1) / 2).min(6) as usize
                        + if pos == 0 {
                            0
                        } else if near {
                            7
                        } else {
                            14
                        };
                    for _ in 0..4 {
                        let br =
                            symbol(d, c, av1_cdfs::COEFF_BR, [txctx.min(3), ptype, ctx])? as i32;
                        level += br;
                        if br < 3 {
                            break;
                        }
                    }
                }
                q[pos] = level;
            }
            for &pos in &scan[..eob] {
                let sign = if q[pos] == 0 {
                    false
                } else if pos == 0 {
                    let ctx = if dc_sum < 0 {
                        1
                    } else if dc_sum > 0 {
                        2
                    } else {
                        0
                    };
                    symbol(d, c, av1_cdfs::DC_SIGN, [ptype, ctx])? != 0
                } else {
                    d.bit()?
                };
                if q[pos] > 14 {
                    let mut bits = 0;
                    while !d.bit()? {
                        bits += 1;
                        if bits > 20 {
                            return Err(invalid("AV1 coefficient Golomb overflow"));
                        }
                    }
                    q[pos] = ((1 << bits) + d.literal(bits)? as i32 + 14) & 0xfffff;
                }
                if pos == 0 && q[pos] != 0 {
                    dc_category = if sign { 1 } else { 2 };
                }
                total += q[pos];
                if sign {
                    q[pos] = -q[pos];
                }
            }
        }
        for i in 0..w4 {
            self.above[p][x + i] = (total.min(63) as u8, dc_category);
        }
        for i in 0..h4 {
            self.left[p][y + i] = (total.min(63) as u8, dc_category);
        }
        // A transform block whose levels are all zero dequantizes to zeros, so
        // the pass over every sample is only needed once a coefficient survives.
        let mut dequant = vec![0; w * h];
        if total != 0 {
            let base = (self.current_q + self.h.segments[self.current_segment][0].unwrap_or(0))
                .clamp(0, 255);
            let dc_delta = self.h.quant.delta[if p == 0 { 0 } else { p * 2 - 1 }];
            let ac_delta = if p == 0 { 0 } else { self.h.quant.delta[p * 2] };
            let depth_index = ((self.s.color.depth - 8) / 2) as usize;
            let dc = DC_QLOOKUP[depth_index][(base + dc_delta).clamp(0, 255) as usize];
            let ac = AC_QLOOKUP[depth_index][(base + ac_delta).clamp(0, 255) as usize];
            let shift = if w * h > 1024 {
                2
            } else if w * h >= 512 {
                1
            } else {
                0
            };
            let limit = 1i64 << (7 + self.s.color.depth);
            let matrix = if kind < 9 && !self.h.lossless[self.current_segment] {
                self.h
                    .quant
                    .matrix
                    .map(|levels| levels[p])
                    .filter(|level| *level < 15)
                    .map(|level| quant_matrix::weights(level, p > 0, size))
            } else {
                None
            };
            // Rows of the coefficient block map one to one onto rows of the
            // residual, so walking them keeps the store sequential instead of
            // dividing the linear coefficient index back out per sample.
            for (r, row) in q.chunks(tw).enumerate() {
                for (c, value) in row.iter().enumerate() {
                    let mut step = if r == 0 && c == 0 { dc } else { ac };
                    if let Some(matrix) = matrix {
                        step = (step * i32::from(matrix[r * tw + c]) + 16) >> 5;
                    }
                    let dq = i64::from(*value) * i64::from(step);
                    // The magnitude is truncated towards zero, which for a
                    // power of two is a shift of the absolute value.
                    dequant[r * w + c] = (dq.signum() * ((dq.abs() & 0xffffff) >> shift))
                        .clamp(-limit, limit - 1) as i32;
                }
            }
        }
        Ok((dequant, kind))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::av1::Obus;
    fn decode(data: &[u8]) -> Result<Picture> {
        let obus = Obus::new(data).collect::<Result<Vec<_>>>()?;
        let s = Sequence::parse(obus.iter().find(|o| o.kind == 1).unwrap().payload)?;
        let o = obus.iter().find(|o| o.kind == 6).unwrap();
        let h = Header::parse_intra(&s, o.payload, o.temporal_id, o.spatial_id)?;
        decode_intra(&s, &h, &[&o.payload[h.header_bytes..]], 16 * 1024 * 1024)
    }
    #[test]
    fn flat_lossless_pixels() {
        let pic = decode(include_bytes!("../../tests/fixtures/av1/flat.obu")).unwrap();
        for plane in pic.planes {
            assert!(plane.samples.iter().all(|v| *v == 128));
        }
    }
    #[test]
    fn lossy_and_full_intra_match_oracle() {
        for (name, data, expected) in [
            (
                "svt",
                &include_bytes!("../../tests/fixtures/av1/sequence.obu")[..],
                &include_bytes!("../../tests/fixtures/av1/sequence-first.yuv")[..],
            ),
            (
                "lossy",
                &include_bytes!("../../tests/fixtures/av1/lossy.obu")[..],
                &include_bytes!("../../tests/fixtures/av1/lossy.yuv")[..],
            ),
            (
                "intra64",
                &include_bytes!("../../tests/fixtures/av1/intra64.obu")[..],
                &include_bytes!("../../tests/fixtures/av1/intra64.yuv")[..],
            ),
        ] {
            let pic = decode(data).unwrap_or_else(|e| panic!("{name}: {e}"));
            let actual = pic
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().map(|v| *v as u8))
                .collect::<Vec<_>>();
            assert_eq!(actual.len(), expected.len());
            for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(a, b, "{name} pixel {i}");
            }
        }
    }
    #[test]
    fn ramp_lossless_matches_oracle() {
        let pic = decode(include_bytes!("../../tests/fixtures/av1/ramp.obu")).unwrap();
        let actual = pic
            .planes
            .iter()
            .flat_map(|p| p.samples.iter().map(|v| *v as u8))
            .collect::<Vec<_>>();
        let expected = include_bytes!("../../tests/fixtures/av1/ramp.yuv");
        assert_eq!(actual.len(), expected.len());
        for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(a, b, "pixel {i}");
        }
    }
}
