//! Annex I.6.6.1 wedgelet line rasterization and left-side fill.
use crate::{Result, invalid};

/// Indexed I.6.6 candidate table. 32x32 masks share the 16x16 index domain.
pub struct Table {
    size: usize,
    entries: Vec<[u64; 4]>,
}
impl Table {
    pub fn size(&self) -> usize {
        self.size
    }
    /// I.9.3.3.5 and table I.15: fixed-length, MSB-first bypass bins.
    pub fn read_index(&self, bins: &mut impl super::hevc_residual::ResidualBins) -> Result<usize> {
        let width = usize::BITS - (self.entries.len() - 1).leading_zeros();
        let mut index = 0usize;
        for _ in 0..width {
            index = (index << 1) | usize::from(bins.bypass()?);
        }
        if index >= self.entries.len() {
            return Err(invalid("HEVC wedge index exceeds table"));
        }
        Ok(index)
    }
    /// Decode and materialize a selected mask; invalid syntax preserves output.
    pub fn read_pattern(
        &self,
        bins: &mut impl super::hevc_residual::ResidualBins,
        output: &mut [bool],
    ) -> Result<usize> {
        if output.len() != self.size * self.size {
            return Err(invalid("invalid HEVC wedge output size"));
        }
        let index = self.read_index(bins)?;
        self.get(index, output)?;
        Ok(index)
    }
    /// Budget bounds retained entry capacity; scratch uses fixed-size stack arrays.
    pub fn new(size: usize, budget: usize) -> Result<Self> {
        if !matches!(size, 4 | 8 | 16 | 32) {
            return Err(invalid("invalid HEVC wedge table size"));
        }
        let base = size.min(16);
        let shift = usize::from(base < 16);
        let domain = base << shift;
        let mut table = Self {
            size,
            entries: Vec::new(),
        };
        let mut start = 0;
        let mut candidate = [false; 256];
        for orientation in 0..6 {
            let end = table.entries.len();
            if matches!(orientation, 0 | 4) {
                let step_start = if base == 16 { 2 } else { 1 };
                let step_end = if base == 16 && orientation == 0 { 2 } else { 1 };
                for m in (0..domain).step_by(step_start) {
                    for n in (0..domain).step_by(step_end) {
                        pattern(
                            base,
                            [m, 0],
                            if orientation == 0 {
                                [0, n]
                            } else {
                                [n, domain - 1]
                            },
                            &mut candidate[..base * base],
                        )?;
                        table.insert(&candidate[..base * base], budget)?;
                    }
                }
            } else {
                for index in start..end {
                    let source = table.entries[index];
                    for y in 0..base {
                        for x in 0..base {
                            let bit = (base - 1 - x) * base + y;
                            candidate[y * base + x] = source[bit / 64] & (1u64 << (bit % 64)) == 0;
                        }
                    }
                    table.insert(&candidate[..base * base], budget)?;
                }
            }
            start = end;
        }
        Ok(table)
    }
    fn insert(&mut self, pattern: &[bool], budget: usize) -> Result<()> {
        if pattern.iter().all(|&v| v == pattern[0]) {
            return Ok(());
        }
        let mut bits = [0u64; 4];
        let mut inverse = [0u64; 4];
        for (i, &v) in pattern.iter().enumerate() {
            if v {
                bits[i / 64] |= 1 << (i % 64);
            } else {
                inverse[i / 64] |= 1 << (i % 64);
            }
        }
        if self.entries.iter().any(|p| *p == bits || *p == inverse) {
            return Ok(());
        }
        if (self.entries.len() + 1)
            .checked_mul(std::mem::size_of::<[u64; 4]>())
            .is_none_or(|n| n > budget)
        {
            return Err(invalid("HEVC wedge table exceeds budget"));
        }
        self.entries
            .try_reserve_exact(1)
            .map_err(|_| invalid("HEVC wedge allocation failed"))?;
        if self.heap_bytes() > budget {
            return Err(invalid("HEVC wedge table exceeds budget"));
        }
        self.entries.push(bits);
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn heap_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<[u64; 4]>()
    }
    pub fn get(&self, index: usize, output: &mut [bool]) -> Result<()> {
        let bits = self
            .entries
            .get(index)
            .ok_or_else(|| invalid("HEVC wedge index exceeds table"))?;
        if output.len() != self.size * self.size {
            return Err(invalid("invalid HEVC wedge output size"));
        }
        let base = self.size.min(16);
        let shift = usize::from(self.size == 32);
        for y in 0..self.size {
            for x in 0..self.size {
                let bit = (y >> shift) * base + (x >> shift);
                output[y * self.size + x] = bits[bit / 64] & (1 << (bit % 64)) != 0;
            }
        }
        Ok(())
    }
}

/// Generate one candidate mask. Endpoints use half-sample precision for
/// 4/8 blocks and integer precision for 16 blocks, per I.6.6.
/// This does not perform table insertion or assign wedge_full_tab_idx.
pub fn pattern(size: usize, start: [usize; 2], end: [usize; 2], output: &mut [bool]) -> Result<()> {
    if !matches!(size, 4 | 8 | 16) || output.len() != size * size {
        return Err(invalid("invalid HEVC wedge geometry"));
    }
    let shift = usize::from(size < 16);
    let domain = size << shift;
    if start.into_iter().chain(end).any(|v| v >= domain) {
        return Err(invalid("HEVC wedge endpoint exceeds boundary"));
    }
    // These are the endpoint families enumerated by orientations zero/four.
    if start[1] != 0 || !(end[0] == 0 || end[1] == domain - 1) {
        return Err(invalid("invalid HEVC wedge endpoint family"));
    }
    let steep = end[1].abs_diff(start[1]) > end[0].abs_diff(start[0]);
    let [mut x0, mut y0] = if steep { [start[1], start[0]] } else { start };
    let [mut x1, mut y1] = if steep { [end[1], end[0]] } else { end };
    if x0 > x1 {
        std::mem::swap(&mut x0, &mut x1);
        std::mem::swap(&mut y0, &mut y1);
    }
    output.fill(false);
    let mut error = 0isize;
    let mut y = y0 as isize;
    let dx = (x1 - x0) as isize;
    let dy = y1.abs_diff(y0) as isize;
    for x in x0..=x1 {
        let (px, py) = if steep {
            (y as usize >> shift, x >> shift)
        } else {
            (x >> shift, y as usize >> shift)
        };
        output[py * size + px] = true;
        error += 2 * dy;
        if error >= dx {
            y += if y0 < y1 { 1 } else { -1 };
            error -= 2 * dx;
        }
    }
    for row in output.chunks_exact_mut(size).take((end[1] >> shift) + 1) {
        for value in row {
            if *value {
                break;
            }
            *value = true;
        }
    }
    Ok(())
}
