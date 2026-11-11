//! Native restoration-unit entropy syntax. Filtering is applied separately.
use super::{
    Header, SymbolDecoder,
    av1_cdfs::{self, Cdfs},
    symbol,
};
use crate::{Result, invalid};
#[path = "av1_restoration_filter.rs"]
mod filter;
#[derive(Clone, Debug)]
pub(super) enum Unit {
    None,
    Wiener([[i32; 3]; 2]),
    Sgr { set: u8, weights: [i32; 2] },
}
struct Plane {
    size: usize,
    cols: usize,
    rows: usize,
    units: Vec<Option<Unit>>,
}
pub(super) struct DeblockedHalo {
    /// Per plane: packed 4-row halo `[top0, top1, bot0, bot1]` per stripe, each of `width` samples.
    planes: [Vec<u16>; 3],
    widths: [usize; 3],
}
pub(super) struct State {
    subsampling: [bool; 2],
    planes: [Plane; 3],
    wiener: [[[i32; 3]; 2]; 3],
    sgr: [[i32; 2]; 3],
}
impl State {
    pub(super) fn required_bytes(h: &Header, subsampling: [bool; 2]) -> Result<usize> {
        let mut bytes = 0usize;
        for p in 0..3 {
            if h.restoration_types[p] != 0 {
                let size = h.restoration_sizes[p] as usize;
                if !matches!(size, 32 | 64 | 128 | 256) {
                    return Err(invalid("invalid AV1 restoration unit size"));
                }
                let [sub_x, sub_y] = if p == 0 {
                    [0; 2]
                } else {
                    subsampling.map(usize::from)
                };
                let cols = ((h.upscaled_width as usize).div_ceil(1 << sub_x) + size / 2) / size;
                let rows = ((h.size[1] as usize).div_ceil(1 << sub_y) + size / 2) / size;
                bytes = bytes
                    .checked_add(
                        cols.max(1)
                            .checked_mul(rows.max(1))
                            .and_then(|n| n.checked_mul(std::mem::size_of::<Option<Unit>>()))
                            .ok_or_else(|| invalid("AV1 restoration allocation overflow"))?,
                    )
                    .ok_or_else(|| invalid("AV1 restoration allocation overflow"))?;
            }
        }
        if h.restoration_types != [0; 3] {
            let width = (h.upscaled_width as usize).div_ceil(8) * 8;
            let height = (h.size[1] as usize).div_ceil(8) * 8;
            bytes = bytes
                .checked_add(
                    width
                        .checked_mul(height)
                        .and_then(|n| {
                            n.checked_mul(if subsampling == [false, false] { 8 } else { 6 })
                        })
                        .ok_or_else(|| invalid("AV1 restoration source allocation overflow"))?,
                )
                .and_then(|n| n.checked_add(200_000))
                .ok_or_else(|| invalid("AV1 restoration scratch allocation overflow"))?;
        }
        Ok(bytes)
    }
    pub(super) fn new(h: &Header, subsampling: [bool; 2]) -> Self {
        let planes = std::array::from_fn(|p| {
            let size = h.restoration_sizes[p] as usize;
            let [sub_x, sub_y] = if p == 0 {
                [0; 2]
            } else {
                subsampling.map(usize::from)
            };
            let (cols, rows) = if h.restoration_types[p] == 0 {
                (0, 0)
            } else {
                (
                    (((h.upscaled_width as usize).div_ceil(1 << sub_x) + size / 2) / size).max(1),
                    (((h.size[1] as usize).div_ceil(1 << sub_y) + size / 2) / size).max(1),
                )
            };
            Plane {
                size,
                cols,
                rows,
                units: vec![None; cols * rows],
            }
        });
        Self {
            subsampling,
            planes,
            wiener: [[[3, -7, 15]; 2]; 3],
            sgr: [[-32, 31]; 3],
        }
    }
    pub(super) fn reset_tile(&mut self) {
        self.wiener = [[[3, -7, 15]; 2]; 3];
        self.sgr = [[-32, 31]; 3];
    }
    pub(super) fn read(
        &mut self,
        d: &mut SymbolDecoder<'_>,
        c: &mut Cdfs,
        h: &Header,
        x: usize,
        y: usize,
        sb: usize,
    ) -> Result<()> {
        for p in 0..3 {
            if h.restoration_types[p] == 0 {
                continue;
            }
            let unit = &self.planes[p];
            let [sub_x, sub_y] = if p == 0 {
                [0; 2]
            } else {
                self.subsampling.map(usize::from)
            };
            let scale_x = 4 >> sub_x;
            let scale_y = 4 >> sub_y;
            let size = unit.size;
            let denom = usize::from(h.superres_denom);
            let x0 = (x * scale_x * denom).div_ceil(size * 8);
            let x1 = ((x + sb) * scale_x * denom)
                .div_ceil(size * 8)
                .min(unit.cols);
            let y0 = (y * scale_y).div_ceil(size);
            let y1 = ((y + sb) * scale_y).div_ceil(size).min(unit.rows);
            for row in y0..y1 {
                for col in x0..x1 {
                    let kind = match h.restoration_types[p] {
                        1 => {
                            if symbol(d, c, av1_cdfs::USE_WIENER, [])? != 0 {
                                1
                            } else {
                                0
                            }
                        }
                        2 => {
                            if symbol(d, c, av1_cdfs::USE_SGRPROJ, [])? != 0 {
                                2
                            } else {
                                0
                            }
                        }
                        3 => symbol(d, c, av1_cdfs::RESTORATION_TYPE, [])?,
                        _ => return Err(invalid("invalid AV1 restoration type")),
                    };
                    let value = match kind {
                        0 => Unit::None,
                        1 => {
                            let mut taps = [[0; 3]; 2];
                            for pass in 0..2 {
                                for j in usize::from(p > 0)..3 {
                                    let v = signed_subexp(
                                        d,
                                        [-5, -23, -17][j],
                                        [11, 9, 47][j],
                                        [1, 2, 3][j],
                                        self.wiener[p][pass][j],
                                    )?;
                                    taps[pass][j] = v;
                                    self.wiener[p][pass][j] = v;
                                }
                            }
                            Unit::Wiener(taps)
                        }
                        2 => {
                            let set = d.literal(4)? as u8;
                            let mut weights = [0; 2];
                            for i in 0..2 {
                                let radius = if i == 0 {
                                    set < 10 || set >= 14
                                } else {
                                    set < 14
                                };
                                let low = [-96, -32][i];
                                let high = [32, 96][i];
                                let v = if radius {
                                    signed_subexp(d, low, high, 4, self.sgr[p][i])?
                                } else if i == 1 {
                                    (128 - self.sgr[p][0]).clamp(low, high - 1)
                                } else {
                                    0
                                };
                                weights[i] = v;
                                self.sgr[p][i] = v;
                            }
                            Unit::Sgr { set, weights }
                        }
                        _ => return Err(invalid("invalid AV1 unit restoration type")),
                    };
                    let index = row * self.planes[p].cols + col;
                    if self.planes[p].units[index].is_some() {
                        return Err(invalid("duplicate AV1 restoration unit"));
                    }
                    self.planes[p].units[index] = Some(value);
                }
            }
        }
        Ok(())
    }
    /// Capture only the 4 deblocked stripe-boundary rows `[start-2, start-1, end+1, end+2]`
    /// per 64-luma-row stripe (upscaling horizontally on the fly when superres is active),
    /// eliminating full-plane `before_restoration` clones.
    pub(super) fn capture_halo(
        &self,
        planes: &[super::Plane; 3],
        coded_size: [u32; 2],
        upscaled_width: u32,
        depth: u8,
    ) -> DeblockedHalo {
        let superres = upscaled_width > coded_size[0];
        let mut halo_planes: [Vec<u16>; 3] = std::array::from_fn(|_| Vec::new());
        let mut widths = [0usize; 3];
        for p in 0..3 {
            let layout = &self.planes[p];
            if layout.units.is_empty()
                || !layout.units.iter().any(|u| !matches!(u, Some(Unit::None) | None))
            {
                continue;
            }
            let [sub_x, sub_y] = if p == 0 {
                [0; 2]
            } else {
                self.subsampling.map(usize::from)
            };
            let down = (coded_size[0] as usize).div_ceil(1 << sub_x);
            let up = (upscaled_width as usize).div_ceil(1 << sub_x);
            let height = (coded_size[1] as usize).div_ceil(1 << sub_y);
            widths[p] = up;
            let stripe_count = height.div_ceil((56 >> sub_y).max(1)) + 2;
            let mut buf = Vec::with_capacity(stripe_count * 4 * up);
            let mut y = 0usize;
            while y < height {
                let stripe = ((y << sub_y) + 8) / 64;
                let start = (-8 + stripe as i64 * 64) >> sub_y;
                let end = start + (64 >> sub_y) - 1;
                let h = (end as usize + 1 - y).min(height - y).min(64 >> sub_y);
                let max_y = height as i64 - 1;
                let rows = [
                    (start - 2).clamp(0, max_y) as usize,
                    (start - 1).clamp(0, max_y) as usize,
                    (end + 1).clamp(0, max_y) as usize,
                    (end + 2).clamp(0, max_y) as usize,
                ];
                for ry in rows {
                    let src_row = &planes[p].samples[ry * planes[p].width..(ry + 1) * planes[p].width];
                    if superres {
                        let base = buf.len();
                        buf.resize(base + up, 0);
                        super::superres::upscale_row(src_row, down, up, depth, &mut buf[base..]);
                    } else {
                        buf.extend_from_slice(&src_row[..up]);
                    }
                }
                y += h;
            }
            halo_planes[p] = buf;
        }
        DeblockedHalo {
            planes: halo_planes,
            widths,
        }
    }
    pub(super) fn apply(
        &self,
        image: &mut super::Picture,
        halo: &DeblockedHalo,
    ) -> Result<()> {
        self.active()?;
        let depth = image.depth;
        let mut cdef_stripe = Vec::new();
        for p in 0..3 {
            let layout = &self.planes[p];
            if layout.units.is_empty()
                || !layout.units.iter().any(|u| !matches!(u, Some(Unit::None) | None))
            {
                continue;
            }
            let [sub_x, sub_y] = if p == 0 {
                [0; 2]
            } else {
                self.subsampling.map(usize::from)
            };
            let width = (image.size[0] as usize).div_ceil(1 << sub_x);
            let height = (image.size[1] as usize).div_ceil(1 << sub_y);
            debug_assert_eq!(halo.widths[p], width);
            let halo_buf = &halo.planes[p];
            let output = &mut image.planes[p];
            let mut y = 0;
            let mut stripe_slot = 0usize;
            while y < height {
                let stripe = ((y << sub_y) + 8) / 64;
                let start = (-8 + stripe as i64 * 64) >> sub_y;
                let end = start + (64 >> sub_y) - 1;
                let unit_row = ((y + (8 >> sub_y)) / layout.size).min(layout.rows - 1);
                let h = (end as usize + 1 - y).min(height - y).min(64 >> sub_y);
                let stripe_y0 = start.clamp(0, height as i64 - 1) as usize;
                let stripe_y1 = end.clamp(0, height as i64 - 1) as usize;
                let stripe_rows = stripe_y1 - stripe_y0 + 1;
                cdef_stripe.clear();
                for ry in stripe_y0..stripe_y0 + stripe_rows {
                    let base = ry * output.width;
                    cdef_stripe.extend_from_slice(&output.samples[base..base + width]);
                }
                let hb = stripe_slot * 4 * width;
                let source = filter::Source {
                    top: [&halo_buf[hb..hb + width], &halo_buf[hb + width..hb + 2 * width]],
                    bottom: [
                        &halo_buf[hb + 2 * width..hb + 3 * width],
                        &halo_buf[hb + 3 * width..hb + 4 * width],
                    ],
                    cdef_stripe: &cdef_stripe,
                    stripe_y0,
                    width,
                    height,
                    start,
                    end,
                    depth,
                };
                let mut x = 0;
                while x < width {
                    let unit_col = (x / layout.size).min(layout.cols - 1);
                    let limit = if unit_col + 1 == layout.cols {
                        width
                    } else {
                        (unit_col + 1) * layout.size
                    };
                    let w = (limit - x).min(64).min(width - x);
                    let unit = layout.units[unit_row * layout.cols + unit_col]
                        .as_ref()
                        .ok_or_else(|| invalid("missing AV1 restoration unit"))?;
                    filter::block(&source, output, unit, x, y, w, h);
                    x += w;
                }
                y += h;
                stripe_slot += 1;
            }
        }
        Ok(())
    }
    pub(super) fn counts(&self) -> [[u32; 3]; 3] {
        std::array::from_fn(|p| {
            let mut counts = [0; 3];
            for unit in self.planes[p].units.iter().flatten() {
                counts[match unit {
                    Unit::None => 0,
                    Unit::Wiener(_) => 1,
                    Unit::Sgr { .. } => 2,
                }] += 1;
            }
            counts
        })
    }
    pub(super) fn active(&self) -> Result<bool> {
        let mut active = false;
        for plane in &self.planes {
            for unit in &plane.units {
                match unit {
                    None => return Err(invalid("missing AV1 restoration unit")),
                    Some(Unit::None) => {}
                    Some(_) => active = true,
                }
            }
        }
        Ok(active)
    }
}
fn unsigned_ns(d: &mut SymbolDecoder<'_>, n: u32) -> Result<u32> {
    if n <= 1 {
        return Ok(0);
    }
    let w = n.ilog2() + 1;
    let m = (1 << w) - n;
    let v = d.literal((w - 1) as u8)?;
    if v < m {
        Ok(v)
    } else {
        Ok((v << 1) - m + u32::from(d.bit()?))
    }
}
fn inverse_recenter(r: u32, v: u32) -> u32 {
    if v > 2 * r {
        v
    } else if v & 1 != 0 {
        r - ((v + 1) >> 1)
    } else {
        r + (v >> 1)
    }
}
fn signed_subexp(
    d: &mut SymbolDecoder<'_>,
    low: i32,
    high: i32,
    k: u32,
    reference: i32,
) -> Result<i32> {
    let n = (high - low) as u32;
    let r = (reference - low) as u32;
    let mut i = 0;
    let mut mk = 0;
    let v = loop {
        let bits = if i == 0 { k } else { k + i - 1 };
        let a = 1 << bits;
        if n <= mk + 3 * a {
            break mk + unsigned_ns(d, n - mk)?;
        }
        if !d.bit()? {
            break mk + d.literal(bits as u8)?;
        }
        i += 1;
        mk += a;
    };
    let value = if 2 * r <= n {
        inverse_recenter(r, v)
    } else {
        n - 1 - inverse_recenter(n - 1 - r, v)
    };
    Ok(value as i32 + low)
}
