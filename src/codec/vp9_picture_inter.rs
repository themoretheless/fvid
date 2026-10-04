use super::*;
use crate::codec::vp9_motion;
const SEARCH: [[[i32; 2]; 8]; 13] = [
    [
        [-1, 0],
        [0, -1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-2, -1],
        [-1, -2],
        [-2, -2],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-2, -1],
        [-1, -2],
        [-2, -2],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-2, -1],
        [-1, -2],
        [-2, -2],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-2, -1],
        [-1, -2],
        [-2, -2],
    ],
    [
        [0, -1],
        [-1, 0],
        [1, -1],
        [-1, -1],
        [0, -2],
        [-2, 0],
        [-2, -1],
        [-1, -2],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, 1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-1, -2],
        [-2, -1],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, 1],
        [1, -1],
        [-1, -1],
        [-3, 0],
        [0, -3],
        [-3, -3],
    ],
    [
        [0, -1],
        [-1, 0],
        [2, -1],
        [-1, -1],
        [-1, 1],
        [0, -3],
        [-3, 0],
        [-3, -3],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, 2],
        [-1, -1],
        [1, -1],
        [-3, 0],
        [0, -3],
        [-3, -3],
    ],
    [
        [-1, 1],
        [1, -1],
        [-1, 2],
        [2, -1],
        [-1, -1],
        [-3, 0],
        [0, -3],
        [-3, -3],
    ],
    [
        [0, -1],
        [-1, 0],
        [4, -1],
        [-1, 2],
        [-1, -1],
        [0, -3],
        [-3, 0],
        [2, -1],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, 4],
        [2, -1],
        [-1, -1],
        [-3, 0],
        [0, -3],
        [-1, 2],
    ],
    [
        [-1, 3],
        [3, -1],
        [-1, 4],
        [4, -1],
        [-1, -1],
        [-1, 0],
        [0, -1],
        [-1, 6],
    ],
];
fn shape(w: usize, h: usize) -> usize {
    [
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
        (32, 64),
        (64, 32),
        (64, 64),
    ]
    .iter()
    .position(|&(x, y)| x == w && y == h)
    .unwrap()
}
fn add(out: &mut Vec<[i32; 2]>, mv: [i32; 2]) {
    if out.len() < 2 && !out.contains(&mv) {
        out.push(mv);
    }
}
impl Decoder<'_> {
    fn candidates(
        &self,
        r: usize,
        c: usize,
        w: usize,
        h: usize,
        reference: u8,
        sub: Option<usize>,
    ) -> ([[i32; 2]; 2], usize) {
        let mut found = Vec::with_capacity(2);
        let mut counter = 0;
        let neighbors: Vec<_> = SEARCH[shape(w, h)]
            .iter()
            .enumerate()
            .filter_map(|(i, &[dy, dx])| {
                let y = r as i32 + dy;
                let x = c as i32 + dx;
                if y < 0
                    || y >= self.rows as i32
                    || x < self.tile_col as i32
                    || x >= self.tile_end as i32
                {
                    None
                } else {
                    Some((i, dx, self.blocks[y as usize * self.cols + x as usize]))
                }
            })
            .collect();
        for &(i, dx, v) in &neighbors {
            if i < 2 {
                counter += if v.reference == 0 {
                    9
                } else {
                    match v.inter_mode {
                        12 => 3,
                        13 => 1,
                        _ => 0,
                    }
                };
            }
            if v.reference == reference {
                let index = if i < 2 {
                    sub.map_or(3, |k| {
                        [[1, 2], [1, 3], [3, 2], [3, 3]][k][usize::from(dx == 0)]
                    })
                } else {
                    3
                };
                add(&mut found, v.mvs[index]);
            }
        }
        let previous = self
            .previous
            .filter(|p| p.size == self.picture.size && !self.header.error_resilient)
            .and_then(|p| p.blocks.get(r * self.cols + c))
            .copied();
        if let Some(v) = previous {
            if v.reference == reference {
                add(&mut found, v.mvs[3]);
            }
        }
        for v in neighbors.iter().map(|v| v.2).chain(previous) {
            if v.reference > 0 && v.reference != reference {
                let mut mv = v.mvs[3];
                if self.header.sign_bias[usize::from(v.reference - 1)]
                    != self.header.sign_bias[usize::from(reference - 1)]
                {
                    mv = mv.map(|v| -v);
                }
                add(&mut found, mv);
            }
        }
        while found.len() < 2 {
            found.push([0; 2]);
        }
        let mut out = [found[0], found[1]];
        for mv in &mut out {
            *mv = self.clamp_mv(*mv, r, c, w, h, 128);
        }
        (
            out,
            [2, 3, 4, 1, 3, 0, 0, 0, 0, 5, 5, 0, 5, 0, 0, 0, 0, 0, 6][counter],
        )
    }
    fn clamp_mv(
        &self,
        mv: [i32; 2],
        r: usize,
        c: usize,
        w: usize,
        h: usize,
        border: i32,
    ) -> [i32; 2] {
        [
            mv[0].clamp(
                -(r as i32) * 64 - border,
                (self.rows as i32 - h.div_ceil(8) as i32 - r as i32) * 64 + border,
            ),
            mv[1].clamp(
                -(c as i32) * 64 - border,
                (self.cols as i32 - w.div_ceil(8) as i32 - c as i32) * 64 + border,
            ),
        ]
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn inter_mode(
        &mut self,
        b: &mut BoolDecoder<'_>,
        r: usize,
        c: usize,
        w: usize,
        h: usize,
        above: Option<Block>,
        left: Option<Block>,
    ) -> Result<Block> {
        let ar = above.map(|v| v.reference);
        let lr = left.map(|v| v.reference);
        let ctx = match (ar, lr) {
            (Some(0), Some(0)) => 2,
            (Some(0), Some(v)) | (Some(v), Some(0)) => 4 * usize::from(v == 1),
            (Some(a), Some(l)) => 2 * usize::from(a == 1) + 2 * usize::from(l == 1),
            (Some(v), None) | (None, Some(v)) => {
                if v == 0 {
                    2
                } else {
                    4 * usize::from(v == 1)
                }
            }
            _ => 2,
        };
        let reference = if !counted(
            b,
            self.ch.probabilities.single_ref[ctx][0],
            &mut self.counts.single_ref[ctx][0],
        )? {
            1
        } else {
            let ctx = match (ar, lr) {
                (Some(0), Some(0)) => 2,
                (Some(0), Some(v)) | (Some(v), Some(0)) => {
                    if v == 1 {
                        3
                    } else {
                        4 * usize::from(v == 2)
                    }
                }
                (Some(1), Some(1)) => 3,
                (Some(1), Some(v)) | (Some(v), Some(1)) => 4 * usize::from(v == 2),
                (Some(a), Some(l)) => 2 * usize::from(a == 2) + 2 * usize::from(l == 2),
                (Some(v), None) | (None, Some(v)) => {
                    if v <= 1 {
                        2
                    } else {
                        4 * usize::from(v == 2)
                    }
                }
                _ => 2,
            };
            if counted(
                b,
                self.ch.probabilities.single_ref[ctx][1],
                &mut self.counts.single_ref[ctx][1],
            )? {
                3
            } else {
                2
            }
        };
        let (mut candidates, ctx) = self.candidates(r, c, w, h, reference, None);
        for mv in &mut candidates {
            if !self.header.high_precision_mv || !vp9_motion::high_precision(*mv) {
                for v in mv.iter_mut() {
                    if *v & 1 != 0 {
                        *v -= v.signum();
                    }
                }
            }
            *mv = self.clamp_mv(*mv, r, c, w, h, 1248);
        }
        let mut mode = if w >= 8 && h >= 8 {
            10 + tree(
                b,
                &[-2, 2, 0, 4, -1, -3],
                &self.ch.probabilities.inter_mode[ctx],
            )?
        } else {
            0
        };
        if w >= 8 && h >= 8 {
            count_symbol(
                &[-2, 2, 0, 4, -1, -3],
                &mut self.counts.inter_mode[ctx],
                mode - 10,
            );
        }
        let filter = if let Some(raw) = self.header.interpolation_filter {
            [1, 0, 2, 3][usize::from(raw)]
        } else {
            let a = above.filter(|v| v.reference > 0).map_or(3, |v| v.filter);
            let l = left.filter(|v| v.reference > 0).map_or(3, |v| v.filter);
            let ctx = if a == l {
                a
            } else if a == 3 {
                l
            } else if l == 3 {
                a
            } else {
                3
            };
            let filter = tree(
                b,
                &[0, 2, -1, -2],
                &self.ch.probabilities.interp_filter[ctx],
            )?;
            count_symbol(&[0, 2, -1, -2], &mut self.counts.interp_filter[ctx], filter);
            usize::from(filter)
        };
        let mut mvs = [[0; 2]; 4];
        if w >= 8 && h >= 8 {
            let mv = match mode {
                10 => candidates[0],
                11 => candidates[1],
                12 => [0; 2],
                _ => vp9_motion::read_vector_counted(
                    b,
                    &self.ch.probabilities,
                    &mut self.counts,
                    candidates[0],
                    self.header.high_precision_mv,
                )?,
            };
            mvs.fill(mv);
        } else {
            for y in (0..2).step_by(h / 4) {
                for x in (0..2).step_by(w / 4) {
                    let block = y * 2 + x;
                    mode = 10
                        + tree(
                            b,
                            &[-2, 2, 0, 4, -1, -3],
                            &self.ch.probabilities.inter_mode[ctx],
                        )?;
                    count_symbol(
                        &[-2, 2, 0, 4, -1, -3],
                        &mut self.counts.inter_mode[ctx],
                        mode - 10,
                    );
                    let mv = match mode {
                        12 => [0; 2],
                        13 => vp9_motion::read_vector_counted(
                            b,
                            &self.ch.probabilities,
                            &mut self.counts,
                            candidates[0],
                            self.header.high_precision_mv,
                        )?,
                        _ => {
                            let (refs, _) = self.candidates(r, c, w, h, reference, Some(block));
                            let mut list = Vec::with_capacity(2);
                            if block == 0 {
                                for mv in refs {
                                    add(&mut list, mv);
                                }
                            } else if block <= 2 {
                                add(&mut list, mvs[0]);
                            } else {
                                for i in [2, 1, 0] {
                                    add(&mut list, mvs[i]);
                                }
                            }
                            for mv in refs {
                                add(&mut list, mv);
                            }
                            while list.len() < 2 {
                                list.push([0; 2]);
                            }
                            list[usize::from(mode == 11)]
                        }
                    };
                    for dy in 0..h / 4 {
                        for dx in 0..w / 4 {
                            mvs[(y + dy) * 2 + x + dx] = mv;
                        }
                    }
                }
            }
        }
        Ok(Block {
            width: w,
            height: h,
            reference,
            inter_mode: mode,
            filter,
            mvs,
            ..Block::default()
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn inter_prediction(
        references: [Option<&Picture>; 3],
        picture_size: [u32; 2],
        picture_depth: u8,
        rows: usize,
        cols: usize,
        plane: usize,
        r: usize,
        c: usize,
        w: usize,
        h: usize,
        x: usize,
        y: usize,
        size: usize,
        block: &Block,
        scratch: &mut Vec<i32>,
        out: &mut Vec<u16>,
    ) -> Result<()> {
        let reference = references[usize::from(block.reference - 1)]
            .ok_or_else(|| invalid("missing decoded VP9 reference"))?;
        if reference.depth != picture_depth {
            return Err(invalid(
                "VP9 reference depth differs from the current picture",
            ));
        }
        if reference.size != picture_size {
            return Err(crate::unsupported(
                "VP9 scaled references are not yet supported",
            ));
        }
        let sub = usize::from(plane > 0);
        let mut mv = block.mvs[0];
        if w < 8 || h < 8 {
            if plane == 0 {
                let idx = ((y - r * 8) / 4) * 2 + (x - c * 8) / 4;
                mv = block.mvs[idx];
            } else {
                for comp in 0..2 {
                    let sum: i32 = block.mvs.iter().map(|v| v[comp]).sum();
                    mv[comp] = (sum + if sum < 0 { -2 } else { 2 }) / 4;
                }
            }
        }
        let units = [h.div_ceil(8) as i32, w.div_ceil(8) as i32];
        let coords = [r as i32, c as i32];
        let dims = [rows as i32, cols as i32];
        for i in 0..2 {
            let near = (-coords[i] * 128) >> sub;
            let far = ((dims[i] - units[i] - coords[i]) * 128) >> sub;
            let extend = (4 + ((units[i] * 8) >> sub)) * 16;
            mv[i] = ((2 * mv[i]) >> sub).clamp(near - extend, far + extend - 16);
        }
        vp9_motion::interpolate(
            &reference.planes[plane],
            [
                reference.size[0].div_ceil(1 << sub) as usize,
                reference.size[1].div_ceil(1 << sub) as usize,
            ],
            [
                (x * 16) as i64 + i64::from(mv[1]),
                (y * 16) as i64 + i64::from(mv[0]),
            ],
            [16, 16],
            [size, size],
            block.filter,
            picture_depth,
            scratch,
            out,
        )
    }
}
