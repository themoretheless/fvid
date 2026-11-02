//! Checked RF64 size replacement. Index storage is smaller than its input table.
use super::{Limits, Result, invalid};
pub(super) struct Rf64<'a> {
    pub end: u64,
    pub samples: u64,
    pub next: usize,
    data: u64,
    table: &'a [u8],
    // Original row index, plus consumption count in the first row of each ID.
    index: Vec<(u32, u32)>,
}
impl<'a> Rf64<'a> {
    pub fn parse(bytes: &'a [u8], limits: &Limits) -> Result<Self> {
        if bytes.get(12..16) != Some(b"ds64") {
            return Err(invalid("RF64 ds64 must be the first chunk"));
        }
        let header = bytes
            .get(16..20)
            .ok_or_else(|| invalid("truncated RF64 ds64 header"))?;
        let size = u32::from_le_bytes(header.try_into().unwrap()) as usize;
        let end = 20usize
            .checked_add(size)
            .ok_or_else(|| invalid("RF64 ds64 extent overflow"))?;
        let body = bytes
            .get(20..end)
            .filter(|b| b.len() >= 28)
            .ok_or_else(|| invalid("RF64 ds64 is truncated or too short"))?;
        let count = u32::from_le_bytes(body[24..28].try_into().unwrap());
        let table_bytes = (count as usize)
            .checked_mul(12)
            .ok_or_else(|| invalid("RF64 ds64 table length overflow"))?;
        let table_end = 28usize
            .checked_add(table_bytes)
            .ok_or_else(|| invalid("RF64 ds64 table length overflow"))?;
        let table = body
            .get(28..table_end)
            .ok_or_else(|| invalid("RF64 ds64 table is truncated"))?;
        let outer = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let riff_size = if outer == u32::MAX {
            u64::from_le_bytes(body[..8].try_into().unwrap())
        } else {
            u64::from(outer)
        };
        let riff_end = riff_size
            .checked_add(8)
            .ok_or_else(|| invalid("RF64 RIFF extent overflow"))?;
        if riff_end > limits.file_bytes as u64 {
            return Err(invalid("RF64 RIFF extent exceeds file limit"));
        }
        let next = end
            .checked_add(size & 1)
            .filter(|&n| n <= bytes.len())
            .ok_or_else(|| invalid("RF64 ds64 padding is truncated"))?;
        if riff_end < next as u64 {
            return Err(invalid("RF64 RIFF extent precedes ds64 end"));
        }
        let mut index = Vec::new();
        index
            .try_reserve_exact(count as usize)
            .map_err(|_| invalid("cannot allocate RF64 table index"))?;
        index.extend((0..count).map(|row| (row, 0)));
        index.sort_unstable_by_key(|&(row, _)| {
            let at = row as usize * 12;
            (<[u8; 4]>::try_from(&table[at..at + 4]).unwrap(), row)
        });
        Ok(Self {
            end: riff_end,
            samples: u64::from_le_bytes(body[16..24].try_into().unwrap()),
            next,
            data: u64::from_le_bytes(body[8..16].try_into().unwrap()),
            table,
            index,
        })
    }
    fn tag(&self, row: u32) -> [u8; 4] {
        let at = row as usize * 12;
        self.table[at..at + 4].try_into().unwrap()
    }
    pub fn size(&mut self, tag: [u8; 4], short: u32, limits: &Limits) -> Result<usize> {
        let size = if short != u32::MAX {
            u64::from(short)
        } else if &tag == b"data" {
            self.data
        } else {
            let first = self.index.partition_point(|&(row, _)| self.tag(row) < tag);
            if first == self.index.len() || self.tag(self.index[first].0) != tag {
                return Err(invalid("RF64 chunk has no size override"));
            }
            let at = first
                .checked_add(self.index[first].1 as usize)
                .filter(|&at| at < self.index.len() && self.tag(self.index[at].0) == tag)
                .ok_or_else(|| invalid("RF64 chunk has no remaining size override"))?;
            let row = self.index[at].0 as usize * 12;
            let size = u64::from_le_bytes(self.table[row + 4..row + 12].try_into().unwrap());
            self.index[first].1 += 1;
            size
        };
        if &tag == b"data" {
            if size > limits.data_bytes as u64 {
                return Err(invalid("RF64 data extent exceeds data limit"));
            }
        } else if size > limits.file_bytes as u64 {
            return Err(invalid("RF64 chunk extent exceeds file limit"));
        }
        usize::try_from(size).map_err(|_| invalid("RF64 chunk size exceeds address space"))
    }
}
