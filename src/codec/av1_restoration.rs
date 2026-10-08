//! Native restoration-unit entropy syntax. Filtering is applied separately.
use super::{
    av1_cdfs::{self, Cdfs},
    symbol, Header, SymbolDecoder,
};
use crate::{invalid, Result};
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
pub(super) struct State {
    planes: [Plane; 3],
    wiener: [[[i32; 3]; 2]; 3],
    sgr: [[i32; 2]; 3],
}
impl State {
    pub(super) fn required_bytes(h: &Header) -> Result<usize> {
        let mut bytes = 0usize;
        for p in 0..3 {
            if h.restoration_types[p] != 0 {
                let size = h.restoration_sizes[p] as usize;
                if !matches!(size, 32 | 64 | 128 | 256) {
                    return Err(invalid("invalid AV1 restoration unit size"));
                }
                let sub = usize::from(p > 0);
                let cols = ((h.size[0] as usize).div_ceil(1 << sub) + size / 2) / size;
                let rows = ((h.size[1] as usize).div_ceil(1 << sub) + size / 2) / size;
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
        Ok(bytes)
    }
    pub(super) fn new(h: &Header) -> Self {
        let planes = std::array::from_fn(|p| {
            let size = h.restoration_sizes[p] as usize;
            let sub = usize::from(p > 0);
            let (cols, rows) = if h.restoration_types[p] == 0 {
                (0, 0)
            } else {
                (
                    (((h.size[0] as usize).div_ceil(1 << sub) + size / 2) / size).max(1),
                    (((h.size[1] as usize).div_ceil(1 << sub) + size / 2) / size).max(1),
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
            let scale = 4 >> usize::from(p > 0);
            let size = unit.size;
            let x0 = (x * scale).div_ceil(size);
            let x1 = ((x + sb) * scale).div_ceil(size).min(unit.cols);
            let y0 = (y * scale).div_ceil(size);
            let y1 = ((y + sb) * scale).div_ceil(size).min(unit.rows);
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
