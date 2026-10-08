//! Owned AV1 intra block copy: integer displacement syntax and tile-local copying.
use super::*;

// AV1 assign_mv conformance: tile bounds and a 256-luma-sample wavefront delay.
fn valid_source(
    origin: [usize; 2],
    size: [usize; 2],
    mv: [i32; 2],
    tile: [usize; 4],
    sb128: bool,
    chroma: [bool; 2],
) -> bool {
    if mv.iter().any(|v| v.abs() >= 16384 || v & 7 != 0) {
        return false;
    }
    let [x, y] = origin.map(|v| v as i32 * 4);
    let [w, h] = size.map(|v| v as i32 * 4);
    let [x0, y0, x1, y1] = tile.map(|v| v as i32 * 4);
    let left = x + mv[1] / 8;
    let top = y + mv[0] / 8;
    let right = left + w;
    let bottom = top + h;
    if left - i32::from(chroma[0] && w < 8) * 4 < x0
        || top - i32::from(chroma[1] && h < 8) * 4 < y0
        || right > x1
        || bottom > y1
    {
        return false;
    }
    let sb_height = if sb128 { 128 } else { 64 };
    let active_row = y / sb_height;
    let active_col = x / 64;
    let source_row = (bottom - 1) / sb_height;
    let source_col = (right - 1) / 64;
    let columns = (x1 - x0 + 63) / 64;
    let active = active_row * columns + active_col;
    let source = source_row * columns + source_col;
    let wavefront = (5 + i32::from(sb128)) * (active_row - source_row);
    source < active - 4 && source_row <= active_row && source_col < active_col - 4 + wavefront
}

impl Decoder<'_> {
    pub(in super::super) fn intrabc_block(
        &mut self,
        d: &mut SymbolDecoder<'_>,
        c: &mut Cdfs,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        skip: bool,
    ) -> Result<()> {
        let stack = self.motion_stack(x, y, w, h, [0; 2]);
        let mut mv = stack.mv.first().map_or([0; 2], |v| v.0[0]);
        if mv == [0; 2] {
            mv = stack.mv.get(1).map_or([0; 2], |v| v.0[0]);
        }
        if mv == [0; 2] {
            let sb = if self.s.superblock128 { 32 } else { 16 };
            mv = if y < self.y0 + sb {
                [0, -((sb * 4 + 256) as i32) * 8]
            } else {
                [-(sb as i32) * 32, 0]
            };
        }
        // Separate MV_INTRABC_CONTEXT=1; fractional and high precision symbols
        // are omitted because intra frames force integer displacement vectors.
        let joint = symbol(d, c, av1_cdfs::MV_JOINT, [1])?;
        for (component, value) in mv.iter_mut().enumerate() {
            if joint == 3 || joint == if component == 0 { 2 } else { 1 } {
                let sign = symbol(d, c, av1_cdfs::MV_SIGN, [1, component])? != 0;
                let class = symbol(d, c, av1_cdfs::MV_CLASS, [1, component])?;
                let magnitude = if class == 0 {
                    (symbol(d, c, av1_cdfs::MV_CLASS0_BIT, [1, component])? + 1) * 8
                } else {
                    let mut bits = 0;
                    for i in 0..class {
                        bits |= symbol(d, c, av1_cdfs::MV_BIT, [1, component, i])? << i;
                    }
                    (2 << (class + 2)) + (bits + 1) * 8
                } as i32;
                *value += if sign { -magnitude } else { magnitude };
            }
        }
        let [sub_x, sub_y] = self.s.color.subsampling;
        let chroma = !self.s.color.monochrome
            && !(sub_x && w == 1 && x & 1 == 0 || sub_y && h == 1 && y & 1 == 0);
        if !valid_source(
            [x, y],
            [w, h],
            mv,
            [self.x0, self.y0, self.x1, self.y1],
            self.s.superblock128,
            [chroma && sub_x, chroma && sub_y],
        ) {
            return Err(invalid(&format!(
                "invalid AV1 intra block copy displacement at {x},{y} size {w},{h}: {mv:?}"
            )));
        }
        let block = Block {
            w,
            h,
            skip,
            tx: if self.h.lossless[self.current_segment] {
                [4; 2]
            } else {
                [(w * 4).min(64), (h * 4).min(64)]
            },
            segment: self.current_segment,
            delta_lf: self.delta_lf,
            mv,
            intrabc: true,
            filters: [3; 2],
            ..Block::default()
        };
        self.image.intrabc_blocks += 1;
        self.image.intrabc_sub8_blocks += u32::from(w == 1 || h == 1);
        let parity = [((mv[1] / 8) & 1) as usize, ((mv[0] / 8) & 1) as usize];
        self.image.intrabc_displacement_parities[parity[1] * 2 + parity[0]] += 1;
        if chroma {
            self.image.intrabc_phases
                [parity[1] * usize::from(sub_y) * 2 + parity[0] * usize::from(sub_x)] += 1;
        }
        self.finish_motion_block(d, c, [x, y], block, false, None, None)
    }

    pub(super) fn intrabc_predict(
        &mut self,
        p: usize,
        x: usize,
        y: usize,
        size: [usize; 2],
        mv: [i32; 2],
    ) -> Result<()> {
        let [sub_x, sub_y] = chroma_geometry::shifts(&self.s.color, p);
        // Luma displacement is integral; subsampled chroma can have a half
        // sample phase. Bilinear filtering therefore uses weights 0, 1 or 2.
        let dx = mv[1] / 8;
        let dy = mv[0] / 8;
        let fx = if sub_x == 0 { 0 } else { dx & 1 };
        let fy = if sub_y == 0 { 0 } else { dy & 1 };
        let sx = x as i32 + (dx >> sub_x);
        let sy = y as i32 + (dy >> sub_y);
        let plane = &mut self.image.planes[p];
        let [w, h] = size;
        self.pred_scratch.resize(w * h, 0);
        for row in 0..h {
            for col in 0..w {
                let mut sum = 0;
                for yy in 0..=usize::from(fy != 0) {
                    for xx in 0..=usize::from(fx != 0) {
                        let source_x = sx + col as i32 + xx as i32;
                        let source_y = sy + row as i32 + yy as i32;
                        if source_x < 0
                            || source_y < 0
                            || source_x >= plane.width as i32
                            || source_y >= plane.height as i32
                        {
                            return Err(invalid("AV1 intra block copy source outside storage"));
                        }
                        let (source_x, source_y) = (source_x as usize, source_y as usize);
                        if !self.decoded[p][(source_y / 4) * (plane.width / 4) + source_x / 4] {
                            return Err(invalid(
                                "AV1 intra block copy source has not been decoded",
                            ));
                        }
                        let weight = if fx == 0 { 2 } else { 1 } * if fy == 0 { 2 } else { 1 };
                        sum += i32::from(plane.samples[source_y * plane.width + source_x]) * weight;
                    }
                }
                self.pred_scratch[row * w + col] = ((sum + 2) >> 2) as u16;
            }
        }
        for row in 0..h.min(plane.height.saturating_sub(y)) {
            let target = (y + row) * plane.width + x;
            let width = w.min(plane.width.saturating_sub(x));
            plane.samples[target..target + width]
                .copy_from_slice(&self.pred_scratch[row * w..row * w + width]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sub8_source_tile_margin_only_applies_to_subsampled_axes() {
        // Exact first failing owned 4:2:2 fixture block: top source edge is 0.
        let tile = [0, 0, 128, 66];
        assert!(valid_source(
            [113, 31],
            [1, 1],
            [-992, 0],
            tile,
            false,
            [true, false]
        ));
        assert!(valid_source(
            [113, 31],
            [1, 1],
            [-992, 0],
            tile,
            false,
            [false; 2]
        ));
        assert!(!valid_source(
            [113, 31],
            [1, 1],
            [-992, 0],
            tile,
            false,
            [true; 2]
        ));
        // Analogous left-edge case: full-resolution chroma is legal, 4:2:2 is not.
        assert!(valid_source(
            [80, 16],
            [1, 1],
            [0, -2560],
            tile,
            false,
            [false; 2]
        ));
        assert!(!valid_source(
            [80, 16],
            [1, 1],
            [0, -2560],
            tile,
            false,
            [true, false]
        ));
    }
    #[test]
    fn source_displacement_respects_tile_wavefront_and_integer_precision() {
        let tile = [0, 0, 96, 48];
        assert!(valid_source(
            [80, 0],
            [8, 8],
            [0, -2560],
            tile,
            false,
            [true; 2]
        ));
        assert!(!valid_source(
            [80, 0],
            [8, 8],
            [0, -2048],
            tile,
            false,
            [true; 2]
        ));
        assert!(!valid_source(
            [80, 0],
            [8, 8],
            [0, -2561],
            tile,
            false,
            [true; 2]
        ));
        assert!(!valid_source(
            [80, 0],
            [8, 8],
            [0, -2560],
            [16, 0, 96, 48],
            false,
            [true; 2]
        ));
        assert!(valid_source(
            [0, 16],
            [8, 8],
            [-512, 0],
            tile,
            false,
            [true; 2]
        ));
        assert!(!valid_source(
            [0, 0],
            [8, 8],
            [-512, 0],
            tile,
            false,
            [true; 2]
        ));
        assert!(!valid_source(
            [80, 0],
            [8, 8],
            [0, -16384],
            tile,
            false,
            [true; 2]
        ));
    }
}
