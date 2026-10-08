//! VP9 intra and single-reference inter reconstruction. Unsupported tools are rejected before output.
#[cfg(feature = "parallel")]
use super::vp9_parallel;
use super::{
    vp9::{self, Header},
    vp9_bool::BoolDecoder,
    vp9_intra::{self, Mode, References},
    vp9_probs::{CompressedHeader, Counts, count_symbol, counted},
    vp9_residual::{self, Config},
    vp9_tables::*,
    vp9_transform::{self, Kind},
};
use crate::{Result, invalid};
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
    pub planes: [Plane; 3],
    blocks: Vec<Block>,
}
#[derive(Clone, Copy, Default, Debug)]
struct Block {
    skip: bool,
    tx: usize,
    modes: [u8; 4],
    width: usize,
    height: usize,
    reference: u8,
    inter_mode: u8,
    filter: usize,
    mvs: [[i32; 2]; 4],
}
struct Decoder<'a> {
    counts: Counts,
    header: &'a Header,
    ch: &'a CompressedHeader,
    picture: Picture,
    cols: usize,
    rows: usize,
    blocks: Vec<Block>,
    above_partition: Vec<u8>,
    left_partition: Vec<u8>,
    above_nz: [Vec<bool>; 3],
    left_nz: [Vec<bool>; 3],
    tile_col: usize,
    tile_end: usize,
    references: [Option<&'a Picture>; 3],
    previous: Option<&'a Picture>,
    scratch: Vec<i32>,
    pred_scratch: Vec<u16>,
    tx_scratch: Vec<i64>,
    residual_scratch: Vec<i32>,
    coef_values: Vec<i32>,
    coef_cache: Vec<usize>,
    dequant_scratch: Vec<i32>,
}
const MODE_TREE: [i8; 18] = [
    0, 2, -9, 4, -1, 6, 8, 12, -2, 10, -4, -5, -3, 14, -8, 16, -6, -7,
];
fn tree(b: &mut BoolDecoder<'_>, t: &[i8], p: &[u8]) -> Result<u8> {
    let mut at = 0;
    loop {
        let v = t[at + usize::from(b.read(p[at / 2])?)];
        if v <= 0 {
            return Ok((-v) as u8);
        }
        at = v as usize;
    }
}
/// Decode an intra 4:2:0 picture. Budget includes image storage and block/context metadata.
pub fn decode_intra(
    frame: &[u8],
    header: &Header,
    ch: &CompressedHeader,
    budget: usize,
) -> Result<Picture> {
    if !header.is_intra() {
        return Err(invalid("expected VP9 intra frame"));
    }
    decode_frame(frame, header, ch, [None; 3], None, budget)
}
/// Decode an intra or single-reference inter picture using already decoded references.
pub fn decode_frame(
    frame: &[u8],
    header: &Header,
    ch: &CompressedHeader,
    references: [Option<&Picture>; 3],
    previous: Option<&Picture>,
    budget: usize,
) -> Result<Picture> {
    decode_frame_counted(frame, header, ch, references, previous, budget).map(|v| v.0)
}

/// Decode with optional tile parallelism enabled.
#[cfg(feature = "parallel")]
pub fn decode_frame_parallel(
    frame: &[u8],
    header: &Header,
    ch: &CompressedHeader,
    references: [Option<&Picture>; 3],
    previous: Option<&Picture>,
    budget: usize,
) -> Result<(Picture, Counts)> {
    // Try parallel decode if multiple tiles
    if let Some(result) = super::vp9_parallel::try_decode_parallel(frame, header)? {
        return Ok(result);
    }
    // Fallback to sequential
    decode_frame_counted(frame, header, ch, references, previous, budget)
}

#[cfg(not(feature = "parallel"))]
pub fn decode_frame_parallel(
    frame: &[u8],
    header: &Header,
    ch: &CompressedHeader,
    references: [Option<&Picture>; 3],
    previous: Option<&Picture>,
    budget: usize,
) -> Result<(Picture, Counts)> {
    decode_frame_counted(frame, header, ch, references, previous, budget)
}
pub(crate) fn decode_frame_counted(
    frame: &[u8],
    header: &Header,
    ch: &CompressedHeader,
    references: [Option<&Picture>; 3],
    previous: Option<&Picture>,
    budget: usize,
) -> Result<(Picture, Counts)> {
    if header.show_existing.is_some()
        || header.picture.format.subsampling != [true; 2]
        || header.segmentation.enabled
        || ch.reference_mode != super::vp9_probs::ReferenceMode::Single
    {
        return Err(crate::unsupported(
            "VP9 picture decoder requires coded 4:2:0 frames, single-reference prediction and no segmentation",
        ));
    }
    let [width, height] = header.picture.size;
    if width == 0
        || height == 0
        || width > 65536
        || height > 65536
        || ![8, 10, 12].contains(&header.picture.format.bit_depth)
        || ch.tx_mode > 4
    {
        return Err(invalid("invalid VP9 picture parameters"));
    }
    let cols = width.div_ceil(8) as usize;
    let rows = height.div_ceil(8) as usize;
    let required = cols
        .checked_mul(rows)
        .and_then(|n| n.checked_mul(384))
        .and_then(|n| n.checked_add((cols + rows + 16) * 64 + 65536))
        .ok_or_else(|| invalid("VP9 picture size overflow"))?;
    if required > budget {
        return Err(invalid("VP9 picture exceeds memory budget"));
    }
    let planes = std::array::from_fn(|p| {
        let sub = usize::from(p > 0);
        let w = (cols * 8) >> sub;
        let h = (rows * 8) >> sub;
        Plane {
            width: w,
            height: h,
            samples: vec![0; w * h],
        }
    });
    let mut d = Decoder {
        counts: Counts::filled([0; 2]),
        header,
        ch,
        picture: Picture {
            size: [width, height],
            depth: header.picture.format.bit_depth,
            planes,
            blocks: Vec::new(),
        },
        cols,
        rows,
        blocks: vec![Block::default(); cols * rows],
        above_partition: vec![0; cols + 8],
        left_partition: vec![0; rows + 8],
        above_nz: std::array::from_fn(|_| vec![false; cols * 2 + 16]),
        left_nz: std::array::from_fn(|_| vec![false; rows * 2 + 16]),
        tile_col: 0,
        tile_end: cols,
        references,
        previous,
        scratch: Vec::new(),
        pred_scratch: Vec::new(),
        tx_scratch: Vec::new(),
        residual_scratch: Vec::new(),
        coef_values: Vec::new(),
        coef_cache: Vec::new(),
        dequant_scratch: Vec::new(),
    };
    for tile in vp9::tiles(frame, header)? {
        let mut b = BoolDecoder::new(tile.data)?;
        d.tile_col = tile.mi_cols.start as usize;
        d.tile_end = tile.mi_cols.end as usize;
        for row in (tile.mi_rows.start as usize..tile.mi_rows.end as usize).step_by(8) {
            d.left_partition.fill(0);
            for p in &mut d.left_nz {
                p.fill(false);
            }
            for col in (tile.mi_cols.start as usize..tile.mi_cols.end as usize).step_by(8) {
                d.partition(&mut b, row, col, 64)?;
            }
        }
        b.finish()?;
    }
    d.filter()?;
    d.picture.blocks = d.blocks;
    Ok((d.picture, d.counts))
}
impl Decoder<'_> {
    fn partition(
        &mut self,
        b: &mut BoolDecoder<'_>,
        r: usize,
        c: usize,
        size: usize,
    ) -> Result<()> {
        if r >= self.rows || c >= self.cols {
            return Ok(());
        }
        let units = size / 8;
        let half = units / 2;
        let has_rows = r + half < self.rows;
        let has_cols = c + half < self.cols;
        let log = units.trailing_zeros() as usize;
        let bit = 1 << (3 - log);
        let above = self.above_partition[c..c + units]
            .iter()
            .any(|&v| v & bit != 0);
        let left = self.left_partition[r..r + units]
            .iter()
            .any(|&v| v & bit != 0);
        let ctx = log * 4 + usize::from(left) * 2 + usize::from(above);
        let p = if self.header.is_intra() {
            KF_PARTITION_PROBS[ctx]
        } else {
            self.ch.probabilities.partition[ctx]
        };
        let part = match (has_rows, has_cols) {
            (true, true) => tree(b, &[0, 2, -1, 4, -2, -3], &p)?,
            (false, true) => {
                if b.read(p[1])? {
                    3
                } else {
                    1
                }
            }
            (true, false) => {
                if b.read(p[2])? {
                    3
                } else {
                    2
                }
            }
            _ => 3,
        };
        count_symbol(
            &[0, 2, -1, 4, -2, -3],
            &mut self.counts.partition[ctx],
            part,
        );
        let (w, h) = match part {
            0 => (size, size),
            1 => (size, size / 2),
            2 => (size / 2, size),
            _ => (size / 2, size / 2),
        };
        if size == 8 || part == 0 {
            self.block(b, r, c, w, h)?;
        } else if part == 1 {
            self.block(b, r, c, w, h)?;
            if has_rows {
                self.block(b, r + half, c, w, h)?;
            }
        } else if part == 2 {
            self.block(b, r, c, w, h)?;
            if has_cols {
                self.block(b, r, c + half, w, h)?;
            }
        } else {
            for (dy, dx) in [(0, 0), (0, half), (half, 0), (half, half)] {
                self.partition(b, r + dy, c + dx, size / 2)?;
            }
        }
        if size == 8 || part != 3 {
            self.above_partition[c..c + units].fill(15 >> (w.trailing_zeros() - 2));
            self.left_partition[r..r + units].fill(15 >> (h.trailing_zeros() - 2));
        }
        Ok(())
    }
    fn block(
        &mut self,
        b: &mut BoolDecoder<'_>,
        r: usize,
        c: usize,
        w: usize,
        h: usize,
    ) -> Result<()> {
        let above = if r > 0 {
            Some(self.blocks[(r - 1) * self.cols + c])
        } else {
            None
        };
        let left = if c > self.tile_col {
            Some(self.blocks[r * self.cols + c - 1])
        } else {
            None
        };
        let ctx =
            usize::from(above.is_some_and(|v| v.skip)) + usize::from(left.is_some_and(|v| v.skip));
        let mut skip = counted(
            b,
            self.ch.probabilities.skip[ctx],
            &mut self.counts.skip[ctx],
        )?;
        let is_inter = if self.header.is_intra() {
            false
        } else {
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
            counted(
                b,
                self.ch.probabilities.is_inter[ctx],
                &mut self.counts.is_inter[ctx],
            )?
        };
        let max_tx = (w.min(h).trailing_zeros() as usize - 2).min(3);
        let mut tx = max_tx.min(usize::from(self.ch.tx_mode.min(3)));
        if self.ch.tx_mode == 4 && w >= 8 && h >= 8 && (!is_inter || !skip) {
            let mut at = above.filter(|v| !v.skip).map_or(max_tx, |v| v.tx);
            let mut lt = left.filter(|v| !v.skip).map_or(max_tx, |v| v.tx);
            if left.is_none() {
                lt = at;
            }
            if above.is_none() {
                at = lt;
            }
            let ctx = usize::from(at + lt > max_tx);
            tx = 0;
            while tx < max_tx
                && counted(
                    b,
                    self.ch.probabilities.tx[max_tx][ctx][tx],
                    &mut self.counts.tx[max_tx][ctx][tx],
                )?
            {
                tx += 1;
            }
        }
        let mut motion = Block {
            width: w,
            height: h,
            ..Block::default()
        };
        let mut modes = [0u8; 4];
        let uv;
        if is_inter {
            motion = self.inter_mode(b, r, c, w, h, above, left)?;
            modes.fill(motion.inter_mode);
            uv = 0;
        } else {
            if w >= 8 && h >= 8 {
                let am = above.map_or(0, |v| v.modes[2]);
                let lm = left.map_or(0, |v| v.modes[1]);
                modes.fill(tree(
                    b,
                    &MODE_TREE,
                    if self.header.is_intra() {
                        &KF_Y_MODE_PROBS[usize::from(am)][usize::from(lm)]
                    } else {
                        &self.ch.probabilities.y_mode[if w < 8 || h < 8 {
                            0
                        } else {
                            (w.min(h).trailing_zeros() as usize - 2).min(3)
                        }]
                    },
                )?);
                count_symbol(
                    &MODE_TREE,
                    &mut self.counts.y_mode[(w.min(h).trailing_zeros() as usize - 2).min(3)],
                    modes[0],
                );
            } else {
                for y in (0..2).step_by(h / 4) {
                    for x in (0..2).step_by(w / 4) {
                        let am = if y > 0 {
                            modes[x]
                        } else {
                            above.map_or(0, |v| v.modes[2 + x])
                        };
                        let lm = if x > 0 {
                            modes[y * 2]
                        } else {
                            left.map_or(0, |v| v.modes[1 + y * 2])
                        };
                        let mode = tree(
                            b,
                            &MODE_TREE,
                            if self.header.is_intra() {
                                &KF_Y_MODE_PROBS[usize::from(am)][usize::from(lm)]
                            } else {
                                &self.ch.probabilities.y_mode[if w < 8 || h < 8 {
                                    0
                                } else {
                                    (w.min(h).trailing_zeros() as usize - 2).min(3)
                                }]
                            },
                        )?;
                        count_symbol(&MODE_TREE, &mut self.counts.y_mode[0], mode);
                        for dy in 0..h / 4 {
                            for dx in 0..w / 4 {
                                modes[(y + dy) * 2 + x + dx] = mode;
                            }
                        }
                    }
                }
            }
            uv = tree(
                b,
                &MODE_TREE,
                if self.header.is_intra() {
                    &KF_UV_MODE_PROBS[usize::from(modes[3])]
                } else {
                    &self.ch.probabilities.uv_mode[usize::from(modes[3])]
                },
            )?;
            count_symbol(
                &MODE_TREE,
                &mut self.counts.uv_mode[usize::from(modes[3])],
                uv,
            );
        }
        let mut any_nonzero = false;
        for plane in 0..3 {
            let sub = usize::from(plane > 0);
            let bw = w.max(8) >> sub;
            let bh = h.max(8) >> sub;
            let size = (4 << tx).min(bw.min(bh));
            let base_x = c * 8 >> sub;
            let base_y = r * 8 >> sub;
            // Distances from the block to the frame edges in eighth-pel units,
            // as in the specification's clipped-block rules. A negative value
            // means the block overflows the frame in that direction.
            let edge_x = (self.cols as isize - (w.max(8) / 8) as isize - c as isize) * 64;
            let edge_y = (self.rows as isize - (h.max(8) / 8) as isize - r as isize) * 64;
            // First four-pixel unit of the block's span that lies outside the
            // frame, or the end of the span when nothing is clipped.
            let bound_x = (base_x / 4 + bw / 4) as isize + (edge_x >> (5 + sub)).min(0);
            let bound_y = (base_y / 4 + bh / 4) as isize + (edge_y >> (5 + sub)).min(0);
            for dy in (0..bh).step_by(size) {
                for dx in (0..bw).step_by(size) {
                    let x = base_x + dx;
                    let y = base_y + dy;
                    let mut nz = false;
                    if x < self.picture.planes[plane].width && y < self.picture.planes[plane].height
                    {
                        let mode = if plane > 0 {
                            uv
                        } else if w < 8 || h < 8 {
                            modes[dy / 4 * 2 + dx / 4]
                        } else {
                            modes[0]
                        };
                        let kind = if self.header.lossless() {
                            Kind::Lossless
                        } else if plane > 0 || size == 32 || is_inter {
                            Kind::DctDct
                        } else {
                            [
                                Kind::DctDct,
                                Kind::AdstDct,
                                Kind::DctAdst,
                                Kind::DctDct,
                                Kind::AdstAdst,
                                Kind::AdstDct,
                                Kind::DctAdst,
                                Kind::DctAdst,
                                Kind::AdstDct,
                                Kind::AdstAdst,
                            ][usize::from(mode)]
                        };
                        let pred_len = size * size;
                        self.pred_scratch.resize(pred_len, 0);
                        if is_inter {
                            let references = self.references;
                            let picture_size = self.picture.size;
                            let picture_depth = self.picture.depth;
                            let rows = self.rows;
                            let cols = self.cols;
                            Self::inter_prediction(
                                references,
                                picture_size,
                                picture_depth,
                                rows,
                                cols,
                                plane,
                                r,
                                c,
                                w,
                                h,
                                x,
                                y,
                                size,
                                &motion,
                                &mut self.scratch,
                                &mut self.pred_scratch,
                            )?;
                        } else {
                            Self::prediction(
                                &self.picture.planes,
                                self.picture.depth,
                                plane,
                                x,
                                y,
                                size,
                                mode,
                                above.is_some() || dy > 0,
                                left.is_some() || dx > 0,
                                dx + size < bw,
                                &mut self.pred_scratch,
                            )?;
                        };
                        if skip {
                            self.residual_scratch.clear();
                        } else {
                            let ctx = usize::from(
                                self.above_nz[plane][x / 4..(x + size) / 4]
                                    .iter()
                                    .any(|v| *v),
                            ) + usize::from(
                                self.left_nz[plane][y / 4..(y + size) / 4]
                                    .iter()
                                    .any(|v| *v),
                            );
                            let coef_nonzero = vp9_residual::read_counted(
                                b,
                                &self.ch.probabilities,
                                &mut self.counts,
                                Config {
                                    size,
                                    kind,
                                    depth: self.picture.depth,
                                    chroma: plane > 0,
                                    inter: is_inter,
                                    initial_context: ctx,
                                },
                                &mut self.coef_values,
                                &mut self.coef_cache,
                            )?;
                            nz = coef_nonzero;
                            let dc = self.header.delta_q[if plane == 0 { 0 } else { 1 }];
                            let ac = if plane == 0 {
                                0
                            } else {
                                self.header.delta_q[2]
                            };
                            vp9_residual::dequantize(
                                &self.coef_values,
                                size,
                                self.picture.depth,
                                self.header.base_q,
                                dc,
                                ac,
                                &mut self.dequant_scratch,
                            )?;
                            vp9_transform::inverse(
                                &self.dequant_scratch,
                                size,
                                self.picture.depth,
                                kind,
                                &mut self.tx_scratch,
                                &mut self.residual_scratch,
                            )?;
                        };
                        let max = (1i32 << self.picture.depth) - 1;
                        let pred = &self.pred_scratch[..pred_len];
                        let residual = &self.residual_scratch;
                        let p = &mut self.picture.planes[plane];
                        for yy in 0..size.min(p.height - y) {
                            for xx in 0..size.min(p.width - x) {
                                let delta = if residual.is_empty() {
                                    0
                                } else {
                                    residual[yy * size + xx]
                                };
                                p.samples[(y + yy) * p.width + x + xx] =
                                    (i32::from(pred[yy * size + xx]) + delta).clamp(0, max) as u16;
                            }
                        }
                    }
                    any_nonzero |= nz;
                    // A transform block that overflows the frame carries its
                    // nonzero context only into the units inside the frame;
                    // the clipped tail of the span is cleared.
                    let ux = x / 4;
                    let uy = y / 4;
                    let ex = (x + size) / 4;
                    let ey = (y + size) / 4;
                    let wx = bound_x.clamp(ux as isize, ex as isize) as usize;
                    let wy = bound_y.clamp(uy as isize, ey as isize) as usize;
                    let above = &mut self.above_nz[plane];
                    above[ux..wx].fill(nz);
                    above[wx..ex].fill(false);
                    let left = &mut self.left_nz[plane];
                    left[uy..wy].fill(nz);
                    left[wy..ey].fill(false);
                }
            }
        }
        if is_inter && w >= 8 && h >= 8 && !any_nonzero {
            skip = true;
        }
        for y in r..(r + h.div_ceil(8)).min(self.rows) {
            for x in c..(c + w.div_ceil(8)).min(self.cols) {
                self.blocks[y * self.cols + x] = Block {
                    skip,
                    tx,
                    modes,
                    width: w,
                    height: h,
                    ..motion
                };
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn prediction(
        planes: &[Plane; 3],
        depth: u8,
        plane: usize,
        x: usize,
        y: usize,
        size: usize,
        mode: u8,
        have_above: bool,
        have_left: bool,
        not_right: bool,
        out: &mut Vec<u16>,
    ) -> Result<()> {
        let p = &planes[plane];
        let mid = 1u16 << (depth - 1);
        let mut a = [mid - 1; 64];
        let mut l = [mid + 1; 32];
        if have_above {
            for (i, v) in a[..size].iter_mut().enumerate() {
                *v = p.samples[(y - 1) * p.width + (x + i).min(p.width - 1)];
            }
        }
        for i in size..2 * size {
            a[i] = if have_above && not_right && size == 4 {
                p.samples[(y - 1) * p.width + (x + i).min(p.width - 1)]
            } else {
                a[size - 1]
            };
        }
        if have_left {
            for (i, v) in l[..size].iter_mut().enumerate() {
                *v = p.samples[(y + i).min(p.height - 1) * p.width + x - 1];
            }
        }
        let corner = if have_above && have_left {
            p.samples[(y - 1) * p.width + x - 1]
        } else if have_above {
            mid + 1
        } else {
            mid - 1
        };
        out.resize(size * size, 0);
        vp9_intra::predict(
            out,
            size,
            depth,
            Mode::try_from(mode)?,
            &References {
                above: &a[..2 * size],
                left: &l[..size],
                corner,
                have_above,
                have_left,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_synthetic_keyframe_matches_ffmpeg_pixel_oracle() {
        let ivf = include_bytes!("../../tests/fixtures/vp9/header.ivf");
        let len = u32::from_le_bytes(ivf[32..36].try_into().unwrap()) as usize;
        let frame = &ivf[44..44 + len];
        let h = vp9::HeaderState::default().parse(frame).unwrap();
        let ch = CompressedHeader::parse(frame, &h, &Default::default()).unwrap();
        let picture = decode_intra(frame, &h, &ch, 1 << 20).unwrap();
        let pixels: Vec<u8> = picture
            .planes
            .iter()
            .flat_map(|p| p.samples.iter().map(|&v| v as u8))
            .collect();
        assert_eq!(
            pixels,
            include_bytes!("../../tests/fixtures/vp9/header-first.yuv")
        );
        assert!(decode_intra(frame, &h, &ch, 64).is_err());
    }
}

impl Decoder<'_> {
    fn filter(&mut self) -> Result<()> {
        let lf = &self.header.loop_filter;
        if lf.level == 0 {
            return Ok(());
        }
        let depth = self.picture.depth;
        let sharpness = lf.sharpness;
        let cols = self.cols;
        let rows = self.rows;
        let blocks = &self.blocks;
        for row in (0..self.rows).step_by(8) {
            for col in (0..self.cols).step_by(8) {
                for plane in 0..3 {
                    let plane_samples = &mut self.picture.planes[plane];
                    let pw = plane_samples.width;
                    let ph = plane_samples.height;
                    for pass in 0..2 {
                        let sub = usize::from(plane > 0);
                        for edge in 0..(16 >> sub) {
                            // `props` answers only what the run detection
                            // below compares — filter level and transform
                            // width — so the sixteen sample positions are
                            // built once per batched line instead of on
                            // every probe that is then discarded.
                            let props = |i: usize| -> Option<(usize, usize, u8, u8)> {
                                let x = col * 8
                                    + if pass == 0 {
                                        edge * (4 << sub)
                                    } else {
                                        i << sub
                                    };
                                let y = row * 8
                                    + if pass == 0 {
                                        i << sub
                                    } else {
                                        edge * (4 << sub)
                                    };
                                if x >= cols * 8
                                    || y >= rows * 8
                                    || (pass == 0 && x == 0)
                                    || (pass == 1 && y == 0)
                                {
                                    return None;
                                }
                                let loop_col = ((x >> 3) >> sub) << sub;
                                let loop_row = ((y >> 3) >> sub) << sub;
                                let block = blocks[loop_row * cols + loop_col];
                                let tx = if plane == 0 {
                                    block.tx
                                } else {
                                    block.tx.min(
                                        ((block.width.max(8).min(block.height.max(8)) >> 1)
                                            .trailing_zeros()
                                            as usize)
                                            - 2,
                                    )
                                };
                                let bw = if sub == 0 {
                                    block.width.max(8)
                                } else {
                                    block.width.max(16)
                                };
                                let bh = if sub == 0 {
                                    block.height.max(8)
                                } else {
                                    block.height.max(16)
                                };
                                // Block dimensions are always powers of two, so
                                // the edge tests hold as masks; as divisions
                                // they stayed on the per-position path.
                                let block_edge = if pass == 0 {
                                    x & (bw - 1) == 0
                                } else {
                                    y & (bh - 1) == 0
                                };
                                let tx_edge = edge & ((1 << tx) - 1) == 0
                                    && !(pass == 1
                                        && sub == 1
                                        && cols % 2 != 0
                                        && edge % 2 != 0
                                        && x + 8 >= cols * 8);
                                if !block_edge
                                    && !(tx_edge && (block.reference == 0 || !block.skip))
                                {
                                    return None;
                                }
                                let mut filter_tx = if tx == 0 && edge % 8 == 0 {
                                    1
                                } else {
                                    tx.min(2)
                                };
                                if sub == 1
                                    && filter_tx == 2
                                    && ((pass == 0 && x >> 3 == cols - 1)
                                        || (pass == 1 && y >> 3 == rows - 1))
                                {
                                    filter_tx = 1;
                                }
                                // The level only matters once the edge survives
                                // the tests above, and most sub-edge positions
                                // inside a block do not.
                                let delta = if lf.delta_enabled {
                                    i32::from(lf.reference_deltas[usize::from(block.reference)])
                                        + if block.reference > 0 {
                                            i32::from(
                                                lf.mode_deltas[usize::from(block.inter_mode != 12)],
                                            )
                                        } else {
                                            0
                                        }
                                } else {
                                    0
                                };
                                let level = (i32::from(lf.level) + (delta << (lf.level >> 5)))
                                    .clamp(0, 63) as u8;
                                Some((x >> sub, y >> sub, level, filter_tx as u8))
                            };
                            let positions = |px: usize, py: usize| -> [usize; 16] {
                                // Only a window that reaches a plane border
                                // needs per-index clamping. An interior one is
                                // a fixed run (pass 0) or a fixed stride
                                // (pass 1) around the edge.
                                if pass == 0 && px >= 8 && px + 8 <= pw {
                                    let first = py * pw + px - 8;
                                    return std::array::from_fn(|j| first + j);
                                }
                                if pass == 1 && py >= 8 && py + 8 <= ph {
                                    let first = (py - 8) * pw + px;
                                    return std::array::from_fn(|j| first + j * pw);
                                }
                                let position = |offset: isize| {
                                    let xx = (px as isize + if pass == 0 { offset } else { 0 })
                                        .clamp(0, pw as isize - 1)
                                        as usize;
                                    let yy = (py as isize + if pass == 1 { offset } else { 0 })
                                        .clamp(0, ph as isize - 1)
                                        as usize;
                                    yy * pw + xx
                                };
                                std::array::from_fn(|j| position(j as isize - 8))
                            };
                            let lim = 64 >> sub;
                            let mut i = 0usize;
                            let mut batch = [(0usize, 0usize, 0u8, 0u8); 4];
                            let mut idxs = [[0usize; 16]; 4];
                            while i < lim {
                                let Some(props0) = props(i) else {
                                    i += 1;
                                    continue;
                                };
                                let (_, _, level, filter_tx) = props0;
                                batch[0] = props0;
                                let mut run = 1usize;
                                while run < 4 && i + run < lim {
                                    match props(i + run) {
                                        Some(next) if next.2 == level && next.3 == filter_tx => {
                                            batch[run] = next;
                                            run += 1;
                                        }
                                        _ => break,
                                    }
                                }
                                let width = 4usize << filter_tx;
                                let mut lines = [[0u16; 16]; 4];
                                let mut runs = [usize::MAX; 4];
                                for k in 0..run {
                                    let (px, py, _, _) = batch[k];
                                    // A pass-0 window clear of the plane border
                                    // is one contiguous run, so it is copied
                                    // rather than gathered through sixteen
                                    // indices.
                                    if pass == 0 && px >= 8 && px + 8 <= pw && py < ph {
                                        let base = py * pw + px - 8;
                                        runs[k] = base;
                                        lines[k].copy_from_slice(
                                            &plane_samples.samples[base..base + 16],
                                        );
                                    } else {
                                        idxs[k] = positions(px, py);
                                        lines[k] = idxs[k].map(|j| plane_samples.samples[j]);
                                    }
                                }
                                for k in run..4 {
                                    lines[k] = lines[0];
                                }
                                let out = super::vp9_filter::filter_batch4(
                                    lines, depth, width, level, sharpness,
                                );
                                for k in 0..run {
                                    if runs[k] != usize::MAX {
                                        plane_samples.samples[runs[k]..runs[k] + 16]
                                            .copy_from_slice(&out[k]);
                                    } else {
                                        for j in 0..16 {
                                            if out[k][j] != lines[k][j] {
                                                plane_samples.samples[idxs[k][j]] = out[k][j];
                                            }
                                        }
                                    }
                                }
                                i += run;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[path = "vp9_picture_inter.rs"]
mod inter;
