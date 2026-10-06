//! AV1 single-reference motion compensation and spatial motion-vector prediction.
use super::*;
const SIZES: [(usize, usize); 22] = [
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
fn lower(mut mv: [i32; 2], integer: bool, high: bool) -> [i32; 2] {
    for v in &mut mv {
        if integer {
            *v = v.signum() * ((v.abs() + 3) >> 3) * 8;
        } else if !high {
            *v -= *v % 2;
        }
    }
    mv
}
struct Stack {
    mv: Vec<([[i32; 2]; 2], usize)>,
    new: usize,
    reference: usize,
}
struct Search<'a, 'b> {
    decoder: &'a Decoder<'b>,
    origin: [usize; 2],
    size: [usize; 2],
    refs: [usize; 2],
    stack: Vec<([[i32; 2]; 2], usize)>,
    new: usize,
}
impl Search<'_, '_> {
    fn add(&mut self, mv: [[i32; 2]; 2], weight: usize) {
        if let Some(v) = self.stack.iter_mut().find(|v| v.0 == mv) {
            v.1 += weight;
        } else if self.stack.len() < 8 {
            self.stack.push((mv, weight));
        }
    }
    fn candidate(&mut self, b: Block, weight: usize) -> bool {
        let mut found = false;
        if self.refs[1] > 0 {
            if [b.reference, b.reference2] == self.refs {
                self.add(
                    [b.mv, b.mv2].map(|mv| {
                        lower(
                            mv,
                            self.decoder.h.integer_mv,
                            self.decoder.h.high_precision_mv,
                        )
                    }),
                    weight,
                );
                self.new += usize::from(matches!(b.mode, 16 | 19 | 20 | 21 | 22 | 24));
                found = true;
            }
        } else {
            for (reference, mv) in [(b.reference, b.mv), (b.reference2, b.mv2)] {
                if reference == self.refs[0] {
                    self.add(
                        [
                            lower(
                                mv,
                                self.decoder.h.integer_mv,
                                self.decoder.h.high_precision_mv,
                            ),
                            [0; 2],
                        ],
                        weight,
                    );
                    self.new += usize::from(matches!(b.mode, 16 | 19 | 20 | 21 | 22 | 24));
                    found = true;
                }
            }
        }
        found
    }
    fn scan(&mut self, mut dx: isize, mut dy: isize, axis: Option<bool>) -> bool {
        let [x, y] = self.origin;
        let [w, h] = self.size;
        let len = match axis {
            Some(false) => w.min(self.decoder.cols - x).min(16),
            Some(true) => h.min(self.decoder.rows - y).min(16),
            None => 1,
        };
        if dy < -1 {
            dy += (y & 1) as isize;
            dx = 1 - (x & 1) as isize;
        }
        if dx < -1 {
            dx += (x & 1) as isize;
            dy = 1 - (y & 1) as isize;
        }
        let mut i = 0;
        let mut found = false;
        while i < len {
            let col = x as isize + dx + if axis == Some(false) { i as isize } else { 0 };
            let row = y as isize + dy + if axis == Some(true) { i as isize } else { 0 };
            let Some(b) = self.decoder.candidate(col, row) else {
                break;
            };
            let step = match axis {
                Some(false) => w
                    .min(b.w)
                    .max(if dy.abs() > 1 { 2 } else { 1 })
                    .max(if w >= 16 { 4 } else { 1 }),
                Some(true) => h
                    .min(b.h)
                    .max(if dx.abs() > 1 { 2 } else { 1 })
                    .max(if h >= 16 { 4 } else { 1 }),
                None => 2,
            };
            found |= self.candidate(b, 2 * step);
            i += step;
        }
        found
    }
}
impl Decoder<'_> {
    fn candidate(&self, x: isize, y: isize) -> Option<Block> {
        if x < self.x0 as isize
            || y < self.y0 as isize
            || x >= self.x1 as isize
            || y >= self.y1 as isize
        {
            return None;
        }
        let b = self.blocks[y as usize * self.cols + x as usize];
        (b.w != 0).then_some(b)
    }
    fn motion_stack(&self, x: usize, y: usize, w: usize, h: usize, refs: [usize; 2]) -> Stack {
        let mut search = Search {
            decoder: self,
            origin: [x, y],
            size: [w, h],
            refs,
            stack: Vec::new(),
            new: 0,
        };
        let mut above = search.scan(0, -1, Some(false));
        let mut left = search.scan(-1, 0, Some(true));
        if w.max(h) <= 16 {
            above |= search.scan(w as isize, -1, None);
        }
        let close = usize::from(above) + usize::from(left);
        let nearest = search.stack.len();
        let num_new = search.new;
        for v in &mut search.stack {
            v.1 += 640;
        }
        above |= search.scan(-1, -1, None);
        above |= search.scan(0, -3, Some(false));
        left |= search.scan(-3, 0, Some(true));
        if h > 1 {
            above |= search.scan(0, -5, Some(false));
        }
        if w > 1 {
            left |= search.scan(-5, 0, Some(true));
        }
        let mut stack = search.stack;
        stack[..nearest].sort_by_key(|v| std::cmp::Reverse(v.1));
        stack[nearest..].sort_by_key(|v| std::cmp::Reverse(v.1));
        if stack.len() < 2 {
            let count = w.min(h).min(16).min(self.cols - x).min(self.rows - y);
            let mut same: [Vec<[i32; 2]>; 2] = std::array::from_fn(|_| Vec::new());
            let mut diff = same.clone();
            for pass in 0..2 {
                let mut i = 0;
                while i < count && stack.len() < 2 {
                    let Some(b) = self.candidate(
                        x as isize + if pass == 0 { i as isize } else { -1 },
                        y as isize + if pass == 0 { -1 } else { i as isize },
                    ) else {
                        break;
                    };
                    for (reference, mv) in [(b.reference, b.mv), (b.reference2, b.mv2)] {
                        if reference == 0 {
                            continue;
                        }
                        for list in 0..if refs[1] > 0 { 2 } else { 1 } {
                            let converted = if self.biases[reference] != self.biases[refs[list]] {
                                mv.map(|v| -v)
                            } else {
                                mv
                            };
                            if refs[1] > 0 {
                                if reference == refs[list] && same[list].len() < 2 {
                                    same[list].push(mv);
                                } else if diff[list].len() < 2 {
                                    diff[list].push(converted);
                                }
                            } else if !stack.iter().any(|v| v.0[0] == converted) {
                                stack.push(([converted, [0; 2]], 2));
                            }
                        }
                    }
                    i += if pass == 0 { b.w } else { b.h };
                }
            }
            if refs[1] > 0 {
                let mut combined = [[[0; 2]; 2]; 2];
                for list in 0..2 {
                    for (i, mv) in same[list].iter().chain(&diff[list]).take(2).enumerate() {
                        combined[i][list] = *mv;
                    }
                }
                if stack.len() == 1 {
                    stack.push((
                        if stack[0].0 == combined[0] {
                            combined[1]
                        } else {
                            combined[0]
                        },
                        2,
                    ));
                } else {
                    stack.extend(combined.map(|m| (m, 2)));
                }
            }
        }
        for (mvs, _) in &mut stack {
            for mv in mvs {
                mv[0] = mv[0].clamp(
                    -(((y + h) * 32) as i32) - 128,
                    ((self.rows - y) * 32) as i32 + 128,
                );
                mv[1] = mv[1].clamp(
                    -(((x + w) * 32) as i32) - 128,
                    ((self.cols - x) * 32) as i32 + 128,
                );
            }
        }
        let total = usize::from(above) + usize::from(left);
        let (new, reference) = match close {
            0 => (total.min(1), total),
            1 => (3 - num_new.min(1), 2 + total),
            _ => (5 - num_new.min(1), 5),
        };
        Stack {
            mv: stack,
            new,
            reference,
        }
    }
    pub(super) fn inter_block(
        &mut self,
        d: &mut SymbolDecoder<'_>,
        c: &mut Cdfs,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        skip: bool,
        skip_mode: bool,
    ) -> Result<()> {
        let (above, left) = self.neighbors(x, y);
        let compound = if skip_mode {
            true
        } else if self.h.reference_select && w.min(h) >= 2 {
            let ctx = match (above, left) {
                (Some(a), Some(l)) => match (a.reference2 > 0, l.reference2 > 0) {
                    (false, false) => usize::from(a.reference >= 5) ^ usize::from(l.reference >= 5),
                    (false, true) => 2 + usize::from(a.reference >= 5 || a.reference == 0),
                    (true, false) => 2 + usize::from(l.reference >= 5 || l.reference == 0),
                    _ => 4,
                },
                (Some(a), None) | (None, Some(a)) => {
                    if a.reference2 > 0 {
                        3
                    } else {
                        usize::from(a.reference >= 5)
                    }
                }
                _ => 1,
            };
            symbol(d, c, av1_cdfs::COMP_MODE, [ctx])? != 0
        } else {
            false
        };
        let mut counts = [0usize; 8];
        for b in above.into_iter().chain(left) {
            counts[b.reference] += 1;
            counts[b.reference2] += 1;
        }
        let ctx = |a: &[usize], b: &[usize]| {
            let a = a.iter().map(|i| counts[*i]).sum::<usize>();
            let b = b.iter().map(|i| counts[*i]).sum::<usize>();
            if a < b {
                0
            } else if a == b {
                1
            } else {
                2
            }
        };
        let refs = if skip_mode {
            self.h.skip_mode.unwrap()
        } else if compound {
            let a = above.unwrap_or_default();
            let l = left.unwrap_or_default();
            let ac = a.reference2 > 0;
            let lc = l.reference2 > 0;
            let au = (a.reference >= 5) == (a.reference2 >= 5);
            let lu = (l.reference >= 5) == (l.reference2 >= 5);
            let same = (a.reference >= 5) == (l.reference >= 5);
            let t = if above.is_some() && a.reference > 0 && left.is_some() && l.reference > 0 {
                match (ac, lc) {
                    (false, false) => 1 + 2 * usize::from(same),
                    (false, true) => {
                        if !lu {
                            1
                        } else {
                            3 + usize::from(same)
                        }
                    }
                    (true, false) => {
                        if !au {
                            1
                        } else {
                            3 + usize::from(same)
                        }
                    }
                    _ => {
                        if !au && !lu {
                            0
                        } else if !au || !lu {
                            2
                        } else {
                            3 + usize::from((a.reference == 5) == (l.reference == 5))
                        }
                    }
                }
            } else if above.is_some() && left.is_some() {
                if ac {
                    1 + 2 * usize::from(au)
                } else if lc {
                    1 + 2 * usize::from(lu)
                } else {
                    2
                }
            } else if ac {
                4 * usize::from(au)
            } else if lc {
                4 * usize::from(lu)
            } else {
                2
            };
            if symbol(d, c, av1_cdfs::COMP_REF_TYPE, [t])? == 0 {
                if symbol(
                    d,
                    c,
                    av1_cdfs::UNI_COMP_REF,
                    [ctx(&[1, 2, 3, 4], &[5, 6, 7]), 0],
                )? != 0
                {
                    [5, 7]
                } else if symbol(d, c, av1_cdfs::UNI_COMP_REF, [ctx(&[2], &[3, 4]), 1])? != 0 {
                    [
                        1,
                        if symbol(d, c, av1_cdfs::UNI_COMP_REF, [ctx(&[3], &[4]), 2])? != 0 {
                            4
                        } else {
                            3
                        },
                    ]
                } else {
                    [1, 2]
                }
            } else {
                let forward = if symbol(d, c, av1_cdfs::COMP_REF, [ctx(&[1, 2], &[3, 4]), 0])? == 0
                {
                    if symbol(d, c, av1_cdfs::COMP_REF, [ctx(&[1], &[2]), 1])? != 0 {
                        2
                    } else {
                        1
                    }
                } else if symbol(d, c, av1_cdfs::COMP_REF, [ctx(&[3], &[4]), 2])? != 0 {
                    4
                } else {
                    3
                };
                let backward =
                    if symbol(d, c, av1_cdfs::COMP_BWD_REF, [ctx(&[5, 6], &[7]), 0])? != 0 {
                        7
                    } else if symbol(d, c, av1_cdfs::COMP_BWD_REF, [ctx(&[5], &[6]), 1])? != 0 {
                        6
                    } else {
                        5
                    };
                [forward, backward]
            }
        } else {
            let mut bit = |i, a: &[usize], b: &[usize]| -> Result<bool> {
                Ok(symbol(d, c, av1_cdfs::SINGLE_REF, [ctx(a, b), i])? != 0)
            };
            let r = if bit(0, &[1, 2, 3, 4], &[5, 6, 7])? {
                if bit(1, &[5, 6], &[7])? {
                    7
                } else if bit(5, &[5], &[6])? {
                    6
                } else {
                    5
                }
            } else if bit(2, &[1, 2], &[3, 4])? {
                if bit(4, &[3], &[4])? {
                    4
                } else {
                    3
                }
            } else if bit(3, &[1], &[2])? {
                2
            } else {
                1
            };
            [r, 0]
        };
        let reference = refs[0];
        let stack = self.motion_stack(x, y, w, h, refs);
        let mode = if skip_mode {
            17
        } else if compound {
            let ctx = [[0, 1, 1, 1, 1], [1, 2, 3, 4, 4], [4, 4, 5, 6, 7]][stack.reference >> 1]
                [stack.new.min(4)];
            17 + symbol(d, c, av1_cdfs::COMPOUND_MODE, [ctx])?
        } else if symbol(d, c, av1_cdfs::NEW_MV, [stack.new])? == 0 {
            16
        } else if symbol(d, c, av1_cdfs::ZERO_MV, [0])? == 0 {
            15
        } else if symbol(d, c, av1_cdfs::REF_MV, [stack.reference])? == 0 {
            13
        } else {
            14
        };
        let near = matches!(mode, 14 | 18 | 21 | 22);
        let mut index = usize::from(near);
        if mode == 16 || mode == 24 || near {
            let start = usize::from(near);
            for i in start..start + 2 {
                if stack.mv.len() > i + 1 {
                    let ctx = if stack.mv[i].1 >= 640 {
                        usize::from(stack.mv[i + 1].1 < 640)
                    } else {
                        2
                    };
                    if symbol(d, c, av1_cdfs::DRL_MODE, [ctx])? == 0 {
                        index = i;
                        break;
                    }
                    index = i + 1;
                }
            }
        }
        let mut mvs = [[0; 2]; 2];
        for list in 0..if compound { 2 } else { 1 } {
            let single = if !compound {
                mode
            } else {
                [
                    [13, 13],
                    [14, 14],
                    [13, 16],
                    [16, 13],
                    [14, 16],
                    [16, 14],
                    [15, 15],
                    [16, 16],
                ][mode - 17][list]
            };
            let pos = if single == 13 || (single == 16 && stack.mv.len() <= 1) {
                0
            } else {
                index
            };
            let mut mv = if single == 15 {
                [0; 2]
            } else {
                stack.mv.get(pos).map_or([0; 2], |v| v.0[list])
            };
            if single == 16 {
                let joint = symbol(d, c, av1_cdfs::MV_JOINT, [0])?;
                for (comp, v) in mv.iter_mut().enumerate() {
                    if joint == 3 || joint == if comp == 0 { 2 } else { 1 } {
                        let sign = symbol(d, c, av1_cdfs::MV_SIGN, [0, comp])? != 0;
                        let class = symbol(d, c, av1_cdfs::MV_CLASS, [0, comp])?;
                        let mag = if class == 0 {
                            let bit = symbol(d, c, av1_cdfs::MV_CLASS0_BIT, [0, comp])?;
                            let fr = if self.h.integer_mv {
                                3
                            } else {
                                symbol(d, c, av1_cdfs::MV_CLASS0_FR, [0, comp, bit])?
                            };
                            let hp = if self.h.high_precision_mv {
                                symbol(d, c, av1_cdfs::MV_CLASS0_HP, [0, comp])?
                            } else {
                                1
                            };
                            (bit << 3) + (fr << 1) + hp + 1
                        } else {
                            let mut bits = 0;
                            for i in 0..class {
                                bits |= symbol(d, c, av1_cdfs::MV_BIT, [0, comp, i])? << i;
                            }
                            let fr = if self.h.integer_mv {
                                3
                            } else {
                                symbol(d, c, av1_cdfs::MV_FR, [0, comp])?
                            };
                            let hp = if self.h.high_precision_mv {
                                symbol(d, c, av1_cdfs::MV_HP, [0, comp])?
                            } else {
                                1
                            };
                            (2 << (class + 2)) + (bits << 3) + (fr << 1) + hp + 1
                        } as i32;
                        *v += if sign { -mag } else { mag };
                        if !(-32768..=32767).contains(v) {
                            return Err(invalid("AV1 motion vector out of range"));
                        }
                    }
                }
            }
            mvs[list] = mv;
        }
        let mv = mvs[0];
        let size_id = SIZES
            .iter()
            .position(|v| *v == (w, h))
            .ok_or_else(|| invalid("invalid AV1 inter block size"))?;
        if !compound
            && self.s.interintra_compound
            && (3..=9).contains(&size_id)
            && symbol(
                d,
                c,
                av1_cdfs::INTER_INTRA,
                [(w.min(h).ilog2() as usize) - 1],
            )? != 0
        {
            return Err(crate::unsupported(
                "AV1 inter-intra blending not implemented",
            ));
        }
        let mut warp = None;
        let mut local_warp = false;
        if !compound && self.h.motion_mode_switchable && w.min(h) >= 2 {
            let overlap = (y > self.y0
                && (x..(x + w).min(self.cols))
                    .step_by(2)
                    .any(|xx| self.blocks[(y - 1) * self.cols + (xx | 1)].reference > 0))
                || (x > self.x0
                    && (y..(y + h).min(self.rows))
                        .step_by(2)
                        .any(|yy| self.blocks[(yy | 1) * self.cols + x - 1].reference > 0));
            if overlap {
                let motion = if self.h.warped_motion
                    && !self.h.integer_mv
                    && !self.warp_samples(x, y, w, h, reference, mv).is_empty()
                {
                    symbol(d, c, av1_cdfs::MOTION_MODE, [size_id])?
                } else {
                    symbol(d, c, av1_cdfs::USE_OBMC, [size_id])?
                };
                if motion == 2 {
                    local_warp = true;
                    warp = super::super::av1_warp::estimate(
                        &self.warp_samples(x, y, w, h, reference, mv),
                        [x, y],
                        [w, h],
                        mv,
                    );
                }
                if motion == 1 {
                    return Err(crate::unsupported(
                        "AV1 overlapped motion compensation not implemented",
                    ));
                }
            }
        }
        let mut compound_average = true;
        if compound && !skip_mode {
            let ctx = above
                .into_iter()
                .chain(left)
                .map(|b| {
                    if b.reference2 > 0 {
                        0
                    } else {
                        3 * usize::from(b.reference == 7)
                    }
                })
                .sum::<usize>()
                .min(5);
            if self.s.masked_compound && symbol(d, c, av1_cdfs::COMP_GROUP_IDX, [ctx])? != 0 {
                return Err(crate::unsupported(
                    "AV1 masked compound prediction not implemented",
                ));
            }
            if self.s.joint_compound {
                let equal = self.distances[refs[0]].abs() == self.distances[refs[1]].abs();
                let ctx = 3 * usize::from(equal)
                    + above
                        .into_iter()
                        .chain(left)
                        .map(|b| {
                            if b.reference2 > 0 {
                                usize::from(b.compound_average)
                            } else {
                                usize::from(b.reference == 7)
                            }
                        })
                        .sum::<usize>();
                compound_average = symbol(d, c, av1_cdfs::COMPOUND_IDX, [ctx])? != 0;
            }
        }
        let mut filters = [self.h.interpolation_filter; 2];
        if filters[0] == 4 {
            filters = [0; 2];
            if !skip_mode && !local_warp && !(w.min(h) >= 2 && matches!(mode, 15 | 23)) {
                for dir in 0..if self.s.dual_filter { 2 } else { 1 } {
                    let a = above
                        .filter(|b| b.reference == reference || b.reference2 == reference)
                        .map_or(3, |b| b.filters[dir]);
                    let l = left
                        .filter(|b| b.reference == reference || b.reference2 == reference)
                        .map_or(3, |b| b.filters[dir]);
                    let ctx = 8 * dir
                        + 4 * usize::from(compound)
                        + if a == l || l == 3 {
                            a
                        } else if a == 3 {
                            l
                        } else {
                            3
                        };
                    filters[dir] = symbol(d, c, av1_cdfs::INTERP_FILTER, [ctx])?;
                }
            }
            if !self.s.dual_filter {
                filters[1] = filters[0];
            }
        }
        let tx = if self.h.lossless[0] {
            [4; 2]
        } else {
            [(w * 4).min(64), (h * 4).min(64)]
        };
        let block = Block {
            w,
            h,
            mode,
            skip,
            tx,
            uv_mode: 0,
            reference,
            reference2: refs[1],
            mv2: mvs[1],
            skip_mode,
            compound_average,
            warp,
            mv,
            filters,
        };
        for yy in y..(y + h).min(self.rows) {
            for xx in x..(x + w).min(self.cols) {
                self.blocks[yy * self.cols + xx] = block;
            }
        }
        if self.h.tx_mode == 2 && !skip && !self.h.lossless[0] {
            for yy in (y..y + h).step_by(tx[1] / 4) {
                for xx in (x..x + w).step_by(tx[0] / 4) {
                    self.read_var_tx(d, c, xx, yy, tx, 0, [x, y], [w, h])?;
                }
            }
        }
        let chroma = !self.s.color.monochrome && !(w == 1 && x % 2 == 0 || h == 1 && y % 2 == 0);
        for p in 0..if chroma { 3 } else { 1 } {
            let sub = usize::from(p > 0);
            let px = (x >> sub) * 4;
            let py = (y >> sub) * 4;
            let pw = ((w * 4) >> sub).max(4);
            let ph = ((h * 4) >> sub).max(4);
            // Sub-8x8 chroma can use a distinct vector for each constituent luma block.
            let mixed = p > 0
                && (w == 1 || h == 1)
                && (0..(ph / 2)).any(|yy| {
                    (0..(pw / 2)).any(|xx| {
                        self.blocks[((y & !1) + yy) * self.cols + (x & !1) + xx].reference == 0
                    })
                });
            let step_x = if p > 0 && w == 1 && !mixed { 2 } else { pw };
            let step_y = if p > 0 && h == 1 && !mixed { 2 } else { ph };
            for yy in (0..ph).step_by(step_y) {
                for xx in (0..pw).step_by(step_x) {
                    let b = if p > 0 && (w == 1 || h == 1) && !mixed {
                        self.blocks[((y & !1) + yy / 2) * self.cols + (x & !1) + xx / 2]
                    } else {
                        block
                    };
                    if b.reference == 0 {
                        return Err(crate::unsupported(
                            "AV1 sub-8x8 mixed intra/inter chroma not implemented",
                        ));
                    }
                    self.motion_predict(p, px + xx, py + yy, [step_x, step_y], b)?;
                }
            }
        }
        for cy in 0..h.div_ceil(16) {
            for cx in 0..w.div_ceil(16) {
                for p in 0..if chroma { 3 } else { 1 } {
                    let sub = usize::from(p > 0);
                    let bw = (w >> sub).max(1);
                    let bh = (h >> sub).max(1);
                    let cw = (w.min(16) >> sub).max(1);
                    let ch = (h.min(16) >> sub).max(1);
                    let bx = (x >> sub) + cx * (16 >> sub);
                    let by = (y >> sub) + cy * (16 >> sub);
                    let size = if self.h.lossless[0] {
                        [4; 2]
                    } else if p == 0 {
                        tx
                    } else {
                        [(bw * 4).min(32), (bh * 4).min(32)]
                    };
                    let mut transforms = Vec::new();
                    if p == 0 && !self.h.lossless[0] {
                        self.transform_order(bx, by, [cw * 4, ch * 4], &mut transforms);
                    } else {
                        for yy in (by..by + ch).step_by(size[1] / 4) {
                            for xx in (bx..bx + cw).step_by(size[0] / 4) {
                                transforms.push((xx, yy, size));
                            }
                        }
                    }
                    for (xx, yy, size) in transforms {
                        let [tw, th] = size;
                        if xx >= self.cols >> sub || yy >= self.rows >> sub {
                            continue;
                        }
                        let residual: &[i32] = if skip {
                            for i in 0..tw / 4 {
                                self.above[p][xx + i] = (0, 0);
                            }
                            for i in 0..th / 4 {
                                self.left[p][yy + i] = (0, 0);
                            }
                            self.residual_scratch.resize(tw * th, 0);
                            self.residual_scratch.fill(0);
                            &self.residual_scratch
                        } else {
                            let (coeff, kind) =
                                self.coefficients(d, c, p, xx, yy, bw, bh, size, 0)?;
                            if self.h.lossless[0] {
                                vp9_transform::inverse(
                                    &coeff,
                                    4,
                                    self.s.color.depth,
                                    Kind::Lossless,
                                    &mut self.lossless_scratch,
                                    &mut self.lossless_out,
                                )?;
                                &self.lossless_out[..4 * 4]
                            } else {
                                super::super::av1_transform::inverse(
                                    &coeff,
                                    size,
                                    self.s.color.depth,
                                    kind,
                                    &mut self.tx_scratch,
                                    &mut self.residual_scratch,
                                )?;
                                &self.residual_scratch[..size[0] * size[1]]
                            }
                        };
                        let plane = &mut self.image.planes[p];
                        for r in 0..th.min(plane.height - yy * 4) {
                            for col in 0..tw.min(plane.width - xx * 4) {
                                let i = (yy * 4 + r) * plane.width + xx * 4 + col;
                                plane.samples[i] = (i32::from(plane.samples[i])
                                    + residual[r * tw + col])
                                    .clamp(0, (1 << self.s.color.depth) - 1)
                                    as u16;
                            }
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
        Ok(())
    }
    fn warp_samples(
        &self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        reference: usize,
        mv: [i32; 2],
    ) -> Vec<[i64; 4]> {
        let mut out = Vec::new();
        let mut scanned = 0;
        let mut fallback = None;
        let mut add = |xx: isize, yy: isize| {
            if scanned >= 8 {
                return;
            }
            let Some(b) = self.candidate(xx, yy) else {
                return;
            };
            if b.reference != reference || b.reference2 != 0 {
                return;
            }
            let mx = ((xx as usize & !(b.w - 1)) * 4 + b.w * 2 - 1) as i64;
            let my = ((yy as usize & !(b.h - 1)) * 4 + b.h * 2 - 1) as i64;
            let candidate = [
                my * 8,
                mx * 8,
                my * 8 + i64::from(b.mv[0]),
                mx * 8 + i64::from(b.mv[1]),
            ];
            scanned += 1;
            if scanned == 1 {
                fallback = Some(candidate);
            }
            if (b.mv[0] - mv[0]).abs() + (b.mv[1] - mv[1]).abs()
                <= (w.max(h) * 4).clamp(16, 112) as i32
            {
                out.push(candidate);
            }
        };
        let mut tl = true;
        let mut tr = true;
        if let Some(a) = self.candidate(x as isize, y as isize - 1) {
            if w <= a.w {
                let offset = x & (a.w - 1);
                if offset > 0 {
                    tl = false;
                }
                if a.w - offset > w {
                    tr = false;
                }
                add(x as isize, y as isize - 1);
            } else {
                let mut i = 0;
                while i < w.min(self.cols - x) {
                    let Some(b) = self.candidate((x + i) as isize, y as isize - 1) else {
                        break;
                    };
                    add((x + i) as isize, y as isize - 1);
                    i += w.min(b.w);
                }
            }
        }
        if let Some(l) = self.candidate(x as isize - 1, y as isize) {
            if h <= l.h {
                if y & (l.h - 1) > 0 {
                    tl = false;
                }
                add(x as isize - 1, y as isize);
            } else {
                let mut i = 0;
                while i < h.min(self.rows - y) {
                    let Some(b) = self.candidate(x as isize - 1, (y + i) as isize) else {
                        break;
                    };
                    add(x as isize - 1, (y + i) as isize);
                    i += h.min(b.h);
                }
            }
        }
        if tl {
            add(x as isize - 1, y as isize - 1);
        }
        if tr && w.max(h) <= 16 {
            add((x + w) as isize, y as isize - 1);
        }
        if out.is_empty() {
            if let Some(v) = fallback {
                out.push(v);
            }
        }
        out
    }
    fn read_var_tx(
        &mut self,
        d: &mut SymbolDecoder<'_>,
        c: &mut Cdfs,
        x: usize,
        y: usize,
        size: [usize; 2],
        depth: usize,
        origin: [usize; 2],
        block: [usize; 2],
    ) -> Result<()> {
        if x >= self.cols || y >= self.rows {
            return Ok(());
        }
        let [w, h] = size;
        let split = if size == [4; 2] || depth == 2 {
            false
        } else {
            let above = if y == origin[1] && y == self.y0 {
                64
            } else {
                let b = self.blocks[(y - 1) * self.cols + x];
                if y == origin[1] && b.skip && b.reference > 0 {
                    b.w * 4
                } else {
                    b.tx[0]
                }
            };
            let left = if x == origin[0] && x == self.x0 {
                64
            } else {
                let b = self.blocks[y * self.cols + x - 1];
                if x == origin[0] && b.skip && b.reference > 0 {
                    b.h * 4
                } else {
                    b.tx[1]
                }
            };
            let max = (block[0].max(block[1]) * 4).min(64);
            let ctx = usize::from(w.max(h) != max) * 3
                + (6 - max.ilog2() as usize) * 6
                + usize::from(above < w)
                + usize::from(left < h);
            symbol(d, c, av1_cdfs::TXFM_SPLIT, [ctx])? != 0
        };
        if split {
            let sub = if w > h {
                [w / 2, h]
            } else if h > w {
                [w, h / 2]
            } else {
                [w / 2, h / 2]
            };
            for yy in (y..y + h / 4).step_by(sub[1] / 4) {
                for xx in (x..x + w / 4).step_by(sub[0] / 4) {
                    self.read_var_tx(d, c, xx, yy, sub, depth + 1, origin, block)?;
                }
            }
        } else {
            for yy in y..(y + h / 4).min(self.rows) {
                for xx in x..(x + w / 4).min(self.cols) {
                    self.blocks[yy * self.cols + xx].tx = size;
                }
            }
        }
        Ok(())
    }
    fn transform_order(
        &self,
        x: usize,
        y: usize,
        size: [usize; 2],
        out: &mut Vec<(usize, usize, [usize; 2])>,
    ) {
        if x >= self.cols || y >= self.rows {
            return;
        }
        let [w, h] = size;
        let tx = self.blocks[y * self.cols + x].tx;
        if w <= tx[0] && h <= tx[1] {
            out.push((x, y, size));
            return;
        }
        let sub = if w > h {
            [w / 2, h]
        } else if h > w {
            [w, h / 2]
        } else {
            [w / 2, h / 2]
        };
        for yy in (y..y + h / 4).step_by(sub[1] / 4) {
            for xx in (x..x + w / 4).step_by(sub[0] / 4) {
                self.transform_order(xx, yy, sub, out);
            }
        }
    }
    fn motion_predict(
        &mut self,
        p: usize,
        x: usize,
        y: usize,
        size: [usize; 2],
        b: Block,
    ) -> Result<()> {
        let compound = b.reference2 > 0;
        let references = self.references;
        let h_references = self.h.references;
        let h_size = self.h.size;
        let color_depth = self.s.color.depth;
        Self::motion_samples(
            references,
            h_references,
            h_size,
            color_depth,
            p,
            x,
            y,
            size,
            b,
            compound,
            &mut self.scratch,
            &mut self.inter_pred,
        )?;
        if compound {
            Self::motion_samples(
                references,
                h_references,
                h_size,
                color_depth,
                p,
                x,
                y,
                size,
                Block {
                    reference: b.reference2,
                    mv: b.mv2,
                    ..b
                },
                true,
                &mut self.scratch,
                &mut self.inter_pred2,
            )?;
        }
        let first = &self.inter_pred;
        let second = if compound {
            Some(&self.inter_pred2)
        } else {
            None
        };
        let post = if self.s.color.depth == 12 { 2 } else { 4 };
        let weights = if !b.compound_average && compound {
            let d0 = self.distances[b.reference2].abs().min(31);
            let d1 = self.distances[b.reference].abs().min(31);
            let order = usize::from(d0 <= d1);
            let row = if d0 == 0 || d1 == 0 {
                3
            } else {
                (0..3)
                    .find(|i| {
                        let t = [[2, 3], [2, 5], [2, 7]][*i];
                        if order == 1 {
                            d0 * t[order] > d1 * t[1 - order]
                        } else {
                            d0 * t[order] < d1 * t[1 - order]
                        }
                    })
                    .unwrap_or(3)
            };
            let t = [[9, 7], [11, 5], [12, 4], [13, 3]][row];
            [t[order], t[1 - order]]
        } else {
            [1, 1]
        };
        let shift = if !compound {
            0
        } else if b.compound_average {
            post + 1
        } else {
            post + 4
        };
        let dst = &mut self.image.planes[p];
        let [w, h] = size;
        for r in 0..h.min(dst.height.saturating_sub(y)) {
            for col in 0..w.min(dst.width.saturating_sub(x)) {
                let i = r * w + col;
                let sum = first[i] * weights[0] + second.as_ref().map_or(0, |v| v[i] * weights[1]);
                let value = if shift == 0 {
                    sum
                } else {
                    (sum + (1 << (shift - 1))) >> shift
                };
                dst.samples[(y + r) * dst.width + x + col] =
                    value.clamp(0, (1 << self.s.color.depth) - 1) as u16;
            }
        }
        Ok(())
    }
    fn motion_samples(
        references: [Option<&Picture>; 8],
        h_references: [usize; 7],
        h_size: [u32; 2],
        color_depth: u8,
        p: usize,
        x: usize,
        y: usize,
        size: [usize; 2],
        b: Block,
        compound: bool,
        temp: &mut Vec<i32>,
        out: &mut Vec<i32>,
    ) -> Result<()> {
        use super::super::av1_tables::SUBPEL_FILTERS;
        let reference = references[h_references[b.reference - 1]]
            .ok_or_else(|| invalid("missing AV1 reference pixels"))?;
        if reference.size != h_size {
            return Err(crate::unsupported(
                "AV1 scaled reference prediction not implemented",
            ));
        }
        if size[0] >= 8 && size[1] >= 8 {
            if let Some(params) = b.warp {
                *out =
                    super::super::av1_warp::predict(reference, p, [x, y], size, params, compound)?;
                return Ok(());
            }
        }
        let sub = usize::from(p > 0);
        let [w, h] = size;
        let src = &reference.planes[p];
        let last_x = (reference.size[0] as usize).div_ceil(1 << sub) as i32 - 1;
        let last_y = (reference.size[1] as usize).div_ceil(1 << sub) as i32 - 1;
        let coord_x = (x as i32) * 16 + (b.mv[1] << (1 - sub));
        let coord_y = (y as i32) * 16 + (b.mv[0] << (1 - sub));
        let filter = |dir: usize, n: usize| {
            let f = b.filters[dir];
            if n <= 4 {
                match f {
                    0 | 2 => 4,
                    1 => 5,
                    _ => f,
                }
            } else {
                f
            }
        };
        let fx = &SUBPEL_FILTERS[filter(1, w)][(coord_x & 15) as usize];
        let fy = &SUBPEL_FILTERS[filter(0, h)][(coord_y & 15) as usize];
        let round0 = if color_depth == 12 { 5 } else { 3 };
        let round1 = if compound { 7 } else { 14 - round0 };
        let temp_len = (h + 7) * w;
        temp.resize(temp_len, 0);
        let temp = &mut temp[..temp_len];
        // One source row slice feeds all eight taps of every output column, so a
        // prediction that stays inside the reference needs no per-tap clamping.
        // Taps span reference columns `base_x..base_x + w + 6`.
        let base_x = (coord_x >> 4) - 3;
        let inside_x = base_x >= 0 && base_x + w as i32 + 6 <= last_x;
        for r in 0..h + 7 {
            let sy = ((coord_y >> 4) + r as i32 - 3).clamp(0, last_y) as usize;
            let row = &src.samples[sy * src.width..];
            for col in 0..w {
                let mut sum = 0;
                if inside_x {
                    let start = (base_x + col as i32) as usize;
                    for (t, k) in fx.iter().enumerate() {
                        sum += k * i32::from(row[start + t]);
                    }
                } else {
                    for (t, k) in fx.iter().enumerate() {
                        let sx =
                            ((coord_x >> 4) + col as i32 + t as i32 - 3).clamp(0, last_x) as usize;
                        sum += k * i32::from(row[sx]);
                    }
                }
                temp[r * w + col] = (sum + (1 << (round0 - 1))) >> round0;
            }
        }
        out.resize(w * h, 0);
        let output = &mut out[..w * h];
        for r in 0..h {
            for col in 0..w {
                let sum = fy
                    .iter()
                    .enumerate()
                    .map(|(t, k)| k * temp[(r + t) * w + col])
                    .sum::<i32>();
                output[r * w + col] = (sum + (1 << (round1 - 1))) >> round1;
            }
        }
        Ok(())
    }
}
