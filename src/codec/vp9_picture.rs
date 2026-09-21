//! VP9 intra and single-reference inter reconstruction. Unsupported tools are rejected before output.
use super::{
    vp9::{self, Header},
    vp9_bool::BoolDecoder,
    vp9_intra::{self, Mode, References},
    vp9_probs::CompressedHeader,
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
    if header.show_existing.is_some()
        || header.picture.format.subsampling != [true; 2]
        || header.segmentation.enabled
        || ch.reference_mode != super::vp9_probs::ReferenceMode::Single
    {
        return Err(invalid(
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
    Ok(d.picture)
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
        let mut skip = b.read(self.ch.probabilities.skip[ctx])?;
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
            b.read(self.ch.probabilities.is_inter[ctx])?
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
            while tx < max_tx && b.read(self.ch.probabilities.tx[max_tx][ctx][tx])? {
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
        }
        let mut any_nonzero = false;
        for plane in 0..3 {
            let sub = usize::from(plane > 0);
            let bw = w.max(8) >> sub;
            let bh = h.max(8) >> sub;
            let size = (4 << tx).min(bw.min(bh));
            let base_x = c * 8 >> sub;
            let base_y = r * 8 >> sub;
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
                        let pred = if is_inter {
                            self.inter_prediction(plane, r, c, w, h, x, y, size, &motion)?
                        } else {
                            self.prediction(
                                plane,
                                x,
                                y,
                                size,
                                mode,
                                above.is_some() || dy > 0,
                                left.is_some() || dx > 0,
                                dx + size < bw,
                            )?
                        };
                        let residual = if skip {
                            vec![0; size * size]
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
                            let coef = vp9_residual::read(
                                b,
                                &self.ch.probabilities,
                                Config {
                                    size,
                                    kind,
                                    depth: self.picture.depth,
                                    chroma: plane > 0,
                                    inter: is_inter,
                                    initial_context: ctx,
                                },
                            )?;
                            nz = coef.nonzero_context;
                            let dc = self.header.delta_q[if plane == 0 { 0 } else { 1 }];
                            let ac = if plane == 0 {
                                0
                            } else {
                                self.header.delta_q[2]
                            };
                            let dequant = vp9_residual::dequantize(
                                &coef.values,
                                size,
                                self.picture.depth,
                                self.header.base_q,
                                dc,
                                ac,
                            )?;
                            vp9_transform::inverse(&dequant, size, self.picture.depth, kind)?
                        };
                        let max = (1i32 << self.picture.depth) - 1;
                        let p = &mut self.picture.planes[plane];
                        for yy in 0..size.min(p.height - y) {
                            for xx in 0..size.min(p.width - x) {
                                p.samples[(y + yy) * p.width + x + xx] =
                                    (i32::from(pred[yy * size + xx]) + residual[yy * size + xx])
                                        .clamp(0, max) as u16;
                            }
                        }
                    }
                    any_nonzero |= nz;
                    self.above_nz[plane][x / 4..(x + size) / 4].fill(nz);
                    self.left_nz[plane][y / 4..(y + size) / 4].fill(nz);
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
        &self,
        plane: usize,
        x: usize,
        y: usize,
        size: usize,
        mode: u8,
        have_above: bool,
        have_left: bool,
        not_right: bool,
    ) -> Result<Vec<u16>> {
        let p = &self.picture.planes[plane];
        let mid = 1u16 << (self.picture.depth - 1);
        let mut a = vec![mid - 1; 2 * size];
        let mut l = vec![mid + 1; size];
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
            for (i, v) in l.iter_mut().enumerate() {
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
        vp9_intra::predict(
            size,
            self.picture.depth,
            Mode::try_from(mode)?,
            &References {
                above: &a,
                left: &l,
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
        for row in (0..self.rows).step_by(8) {
            for col in (0..self.cols).step_by(8) {
                for plane in 0..3 {
                    for pass in 0..2 {
                        let sub = usize::from(plane > 0);
                        for edge in 0..(16 >> sub) {
                            for i in 0..(64 >> sub) {
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
                                if x >= self.cols * 8
                                    || y >= self.rows * 8
                                    || (pass == 0 && x == 0)
                                    || (pass == 1 && y == 0)
                                {
                                    continue;
                                }
                                let loop_col = ((x >> 3) >> sub) << sub;
                                let loop_row = ((y >> 3) >> sub) << sub;
                                let block = self.blocks[loop_row * self.cols + loop_col];
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
                                let block_edge = if pass == 0 { x % bw == 0 } else { y % bh == 0 };
                                let tx_edge = edge % (1 << tx) == 0
                                    && !(pass == 1
                                        && sub == 1
                                        && self.cols % 2 != 0
                                        && edge % 2 != 0
                                        && x + 8 >= self.cols * 8);
                                if !block_edge
                                    && !(tx_edge && (block.reference == 0 || !block.skip))
                                {
                                    continue;
                                }
                                let mut filter_tx = if tx == 0 && edge % 8 == 0 {
                                    1
                                } else {
                                    tx.min(2)
                                };
                                if sub == 1
                                    && filter_tx == 2
                                    && ((pass == 0 && x >> 3 == self.cols - 1)
                                        || (pass == 1 && y >> 3 == self.rows - 1))
                                {
                                    filter_tx = 1;
                                }
                                let (px, py) = (x >> sub, y >> sub);
                                let p = &mut self.picture.planes[plane];
                                let position = |offset: isize| {
                                    let xx = (px as isize + if pass == 0 { offset } else { 0 })
                                        .clamp(0, p.width as isize - 1)
                                        as usize;
                                    let yy = (py as isize + if pass == 1 { offset } else { 0 })
                                        .clamp(0, p.height as isize - 1)
                                        as usize;
                                    yy * p.width + xx
                                };
                                let indices: [usize; 16] =
                                    std::array::from_fn(|j| position(j as isize - 8));
                                let samples = indices.map(|j| p.samples[j]);
                                let filtered = super::vp9_filter::filter(
                                    samples,
                                    self.picture.depth,
                                    4 << filter_tx,
                                    level,
                                    lf.sharpness,
                                )?;
                                for j in 0..16 {
                                    if filtered[j] != samples[j] {
                                        p.samples[indices[j]] = filtered[j];
                                    }
                                }
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
