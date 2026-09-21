//! AV1 tile layout and tile group payload boundaries.
use super::bits::BitReader;
use crate::{Result, invalid};

pub(crate) fn align(b: &mut BitReader<'_>) -> Result<()> {
    while !b.position().is_multiple_of(8) {
        if b.bit()? {
            return Err(invalid("nonzero AV1 byte alignment"));
        }
    }
    Ok(())
}
pub(crate) fn signed(b: &mut BitReader<'_>, n: u8) -> Result<i32> {
    let x = b.read(n)? as i32;
    Ok((x << (32 - n)) >> (32 - n))
}
fn log2(blk: u32, target: u32) -> u8 {
    let mut k = 0;
    while (blk << k) < target {
        k += 1;
    }
    k
}
fn ns(b: &mut BitReader<'_>, n: u32) -> Result<u32> {
    if n == 1 {
        return Ok(0);
    }
    let w = n.ilog2() as u8 + 1;
    let m = (1 << w) - n;
    let v = b.read(w - 1)?;
    if v < m {
        Ok(v)
    } else {
        Ok((v << 1) - m + b.read(1)?)
    }
}
#[derive(Clone, Debug)]
pub struct Layout {
    pub columns: Vec<u32>,
    pub rows: Vec<u32>,
    pub column_bits: u8,
    pub row_bits: u8,
    pub context_tile: usize,
    pub size_bytes: usize,
}
impl Layout {
    pub fn parse(b: &mut BitReader<'_>, size: [u32; 2], sb128: bool) -> Result<Self> {
        if size.contains(&0) || size.iter().any(|x| *x > 65536) {
            return Err(invalid("invalid AV1 dimensions"));
        }
        let mi = [2 * size[0].div_ceil(8), 2 * size[1].div_ceil(8)];
        let shift = if sb128 { 5 } else { 4 };
        let sb = [mi[0].div_ceil(1 << shift), mi[1].div_ceil(1 << shift)];
        let max_width = 4096 >> (shift + 2);
        let max_area = (4096 * 2304) >> (2 * (shift + 2));
        let min_cols = log2(max_width, sb[0]);
        let min_tiles = min_cols.max(log2(max_area, sb[0] * sb[1]));
        let (mut columns, mut rows) = (Vec::new(), Vec::new());
        let (column_bits, row_bits);
        if b.bit()? {
            let mut cb = min_cols;
            while cb < log2(1, sb[0].min(64)) && b.bit()? {
                cb += 1;
            }
            let width = sb[0].div_ceil(1 << cb);
            for start in (0..sb[0]).step_by(width as usize) {
                columns.push(start << shift);
            }
            let mut rb = min_tiles.saturating_sub(cb);
            while rb < log2(1, sb[1].min(64)) && b.bit()? {
                rb += 1;
            }
            let height = sb[1].div_ceil(1 << rb);
            for start in (0..sb[1]).step_by(height as usize) {
                rows.push(start << shift);
            }
            column_bits = cb;
            row_bits = rb;
        } else {
            let mut start = 0;
            let mut widest = 0;
            while start < sb[0] {
                if columns.len() == 64 {
                    return Err(invalid("too many AV1 tile columns"));
                }
                columns.push(start << shift);
                let width = ns(b, (sb[0] - start).min(max_width))? + 1;
                widest = widest.max(width);
                start += width;
            }
            let area = if min_tiles > 0 {
                (sb[0] * sb[1]) >> (min_tiles + 1)
            } else {
                sb[0] * sb[1]
            };
            let max_height = (area / widest).max(1);
            start = 0;
            while start < sb[1] {
                if rows.len() == 64 {
                    return Err(invalid("too many AV1 tile rows"));
                }
                rows.push(start << shift);
                start += ns(b, (sb[1] - start).min(max_height))? + 1;
            }
            column_bits = log2(1, columns.len() as u32);
            row_bits = log2(1, rows.len() as u32);
        }
        let count = columns.len() * rows.len();
        let (context_tile, size_bytes) = if column_bits + row_bits > 0 {
            (
                b.read(column_bits + row_bits)? as usize,
                b.read(2)? as usize + 1,
            )
        } else {
            (0, 0)
        };
        if count > 512 || context_tile >= count {
            return Err(invalid("invalid AV1 tile count/context"));
        }
        columns.push(mi[0]);
        rows.push(mi[1]);
        Ok(Self {
            columns,
            rows,
            column_bits,
            row_bits,
            context_tile,
            size_bytes,
        })
    }
    pub fn count(&self) -> usize {
        self.columns
            .len()
            .saturating_sub(1)
            .saturating_mul(self.rows.len().saturating_sub(1))
    }
    pub fn group<'a>(&self, payload: &'a [u8]) -> Result<Vec<(usize, &'a [u8])>> {
        let b = &mut BitReader::new(payload);
        let count = self.count();
        if count == 0
            || count > 512
            || self.column_bits > 6
            || self.row_bits > 6
            || self.size_bytes > 4
            || (count > 1 && self.size_bytes == 0)
        {
            return Err(invalid("invalid AV1 tile layout"));
        }
        let (start, end) = if count > 1 && b.bit()? {
            let bits = self.column_bits + self.row_bits;
            (b.read(bits)? as usize, b.read(bits)? as usize)
        } else {
            (0, count - 1)
        };
        if start > end || end >= count {
            return Err(invalid("invalid AV1 tile group range"));
        }
        align(b)?;
        let mut offset = b.position() / 8;
        let mut result = Vec::with_capacity(end - start + 1);
        for tile in start..=end {
            let size = if tile == end {
                payload.len() - offset
            } else {
                let bytes = payload
                    .get(offset..offset + self.size_bytes)
                    .ok_or_else(|| invalid("truncated AV1 tile size"))?;
                offset += self.size_bytes;
                let mut value = 0u64;
                for (i, x) in bytes.iter().enumerate() {
                    value |= u64::from(*x) << (8 * i);
                }
                usize::try_from(value + 1).map_err(|_| invalid("AV1 tile size overflow"))?
            };
            if size == 0 {
                return Err(invalid("empty AV1 tile"));
            }
            let end = offset
                .checked_add(size)
                .ok_or_else(|| invalid("AV1 tile size overflow"))?;
            result.push((
                tile,
                payload
                    .get(offset..end)
                    .ok_or_else(|| invalid("AV1 tile exceeds group"))?,
            ));
            offset = end;
        }
        Ok(result)
    }
}
