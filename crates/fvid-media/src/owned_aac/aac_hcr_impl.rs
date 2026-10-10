// Owned inverse of ISO/IEC 14496-3 section4.6.16.3 reordering.
const HCR_MAX_WORD: [u8; 32] = [
    0, 11, 9, 20, 16, 13, 11, 14, 12, 17, 14, 49, 0, 0, 0, 0, 14, 17, 21, 21, 25, 25, 29, 29, 29,
    29, 33, 33, 33, 37, 37, 41,
];
const HCR_PRIORITY: [u8; 32] = [
    99, 21, 21, 20, 20, 19, 19, 18, 18, 17, 17, 0, 99, 99, 99, 99, 16, 15, 14, 13, 12, 11, 10, 9,
    8, 7, 6, 5, 4, 3, 2, 1,
];
#[derive(Clone, Debug)]
pub struct HcrHeader {
    bits: usize,
    longest: u8,
}
impl HcrHeader {
    /// Reserved values adapt to the standard maxima, separately for a CPE.
    pub fn read(bits: &mut BitReader<'_>, pair: bool) -> Result<Self> {
        let mut cursor = bits.clone();
        let length = (cursor.read(14)? as usize).min(if pair { 12288 } else { 6144 });
        let longest = (cursor.read(6)? as u8).min(49);
        if length != 0 && longest == 0 {
            return Err(invalid("AAC HCR nonempty region has zero longest codeword"));
        }
        *bits = cursor;
        Ok(Self {
            bits: length,
            longest,
        })
    }
    /// Restore group/band/window coefficients from priority and rotated,
    /// alternating-direction nonpriority sets. Failure preserves the cursor.
    pub fn decode(
        &self,
        bits: &mut BitReader<'_>,
        info: &IcsInfo,
        offsets: &[usize],
        books: &[Vec<u8>],
        frame_samples: usize,
    ) -> Result<Vec<i16>> {
        let windows = spectral_geometry(info, offsets, books, frame_samples)?;
        if self.bits > bits.remaining() {
            return Err(invalid("truncated AAC HCR spectral region"));
        }
        let mut words = Vec::new();
        let mut count = 0;
        let mut first_window = 0;
        for (group, &length) in books.iter().zip(&info.group_lengths) {
            for (band, &book) in group.iter().enumerate() {
                let width = offsets[band + 1] - offsets[band];
                let base = count;
                count += width * usize::from(length);
                match book {
                    0 | 13..=15 => continue,
                    1..=11 | 16..=31 => {}
                    _ => return Err(invalid("reserved AAC spectral codebook")),
                }
                if !width.is_multiple_of(4) {
                    return Err(invalid("AAC HCR band splits a four-line unit"));
                }
                let dimension = if book <= 4 { 4 } else { 2 };
                for window in 0..usize::from(length) {
                    for line in (offsets[band]..offsets[band + 1]).step_by(4) {
                        for half in 0..4 / dimension {
                            words.push(HcrWord {
                                key: (
                                    HCR_PRIORITY[usize::from(book)],
                                    line,
                                    first_window + window,
                                    half,
                                ),
                                book,
                                output: base + window * width + line - offsets[band]
                                    + half * dimension,
                            });
                        }
                    }
                }
            }
            first_window += usize::from(length);
        }
        debug_assert_eq!(first_window, windows);
        let mut out = vec![0i16; count];
        if words.is_empty() {
            if self.bits != 0 {
                return Err(invalid("AAC HCR unused spectral bits"));
            }
            return Ok(out);
        }
        if self.bits == 0 {
            return Err(invalid("AAC HCR spectral payload is empty"));
        }
        words.sort_unstable_by_key(|word| word.key);
        let mut segments: Vec<HcrSegment> = Vec::new();
        let mut start = 0;
        for word in &words {
            let width = usize::from(HCR_MAX_WORD[usize::from(word.book)].min(self.longest));
            if start + width <= self.bits {
                segments.push(HcrSegment {
                    left: start,
                    right: start + width,
                });
                start += width;
            } else {
                let last = segments
                    .last_mut()
                    .ok_or_else(|| invalid("AAC HCR region cannot hold a priority segment"))?;
                last.right = self.bits;
                start = self.bits;
                break;
            }
        }
        if start != self.bits {
            return Err(invalid("AAC HCR unused spectral bits"));
        }
        let n = segments.len();
        let mut pending = vec![HcrPending::default(); words.len()];
        let mut done = vec![false; words.len()];
        for i in 0..n {
            while let Some(bit) = segments[i].take(bits, true)? {
                if let Some((values, len)) = pending[i].push(bit, words[i].book, self.longest)? {
                    out[words[i].output..words[i].output + len].copy_from_slice(&values[..len]);
                    done[i] = true;
                    break;
                }
            }
            if !done[i] {
                return Err(invalid("AAC HCR incomplete priority codeword"));
            }
        }
        for set in 1..words.len().div_ceil(n) {
            let first = set * n;
            let end = (first + n).min(words.len());
            for trial in 0..n {
                for i in first..end {
                    if done[i] {
                        continue;
                    }
                    let segment = &mut segments[(trial + i - first) % n];
                    while let Some(bit) = segment.take(bits, set.is_multiple_of(2))? {
                        if let Some((values, len)) =
                            pending[i].push(bit, words[i].book, self.longest)?
                        {
                            out[words[i].output..words[i].output + len]
                                .copy_from_slice(&values[..len]);
                            done[i] = true;
                            break;
                        }
                    }
                }
            }
            if done[first..end].contains(&false) {
                return Err(invalid("AAC HCR incomplete nonpriority codeword"));
            }
        }
        if segments.iter().any(|s| s.left != s.right) {
            return Err(invalid("AAC HCR unused spectral bits"));
        }
        bits.skip(self.bits)?;
        Ok(out)
    }
}
struct HcrWord {
    key: (u8, usize, usize, usize),
    book: u8,
    output: usize,
}
struct HcrSegment {
    left: usize,
    right: usize,
}
impl HcrSegment {
    fn take(&mut self, origin: &BitReader<'_>, forward: bool) -> Result<Option<u32>> {
        if self.left == self.right {
            return Ok(None);
        }
        let offset = if forward {
            let p = self.left;
            self.left += 1;
            p
        } else {
            self.right -= 1;
            self.right
        };
        let mut cursor = origin.clone();
        cursor.skip(offset)?;
        Ok(Some(cursor.read(1)?))
    }
}
#[derive(Clone, Default)]
struct HcrPending {
    value: u64,
    len: u8,
}
impl HcrPending {
    fn push(&mut self, bit: u32, book: u8, longest: u8) -> Result<Option<([i16; 4], usize)>> {
        let limit = HCR_MAX_WORD[usize::from(book)].min(longest);
        if self.len >= limit {
            return Err(invalid("AAC HCR codeword exceeds declared longest"));
        }
        self.value = (self.value << 1) | u64::from(bit);
        self.len += 1;
        let bytes = self.value.to_be_bytes();
        let cover = usize::from(self.len).div_ceil(8);
        let mut reader = BitReader::new(&bytes[8 - cover..]);
        reader.skip(cover * 8 - usize::from(self.len))?;
        let result = aac_huffman::spectral_prefix(&mut reader, if book >= 16 { 11 } else { book })?;
        if let Some((values, n)) = &result {
            check_virtual_magnitudes(book, &values[..*n])?;
        }
        if result.is_none() && self.len == limit {
            return Err(invalid("AAC HCR codeword exceeds declared longest"));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod hcr_header_tests {
    use super::*;
    fn pack(s: &str) -> Vec<u8> {
        let mut out = vec![0; s.len().div_ceil(8)];
        for (i, b) in s.bytes().enumerate() {
            out[i / 8] |= (b - b'0') << (7 - i % 8);
        }
        out
    }
    #[test]
    fn reserved_hcr_lengths_adapt_and_header_truncation_is_transactional() {
        for pair in [false, true] {
            let bytes = pack("11111111111111111111");
            let mut bits = BitReader::new(&bytes);
            let header = HcrHeader::read(&mut bits, pair).unwrap();
            assert_eq!(header.bits, if pair { 12288 } else { 6144 });
            assert_eq!(header.longest, 49);
            assert_eq!(bits.position(), 20);
            for keep in 0..20 {
                let prefix = (8 - keep % 8) % 8;
                let bytes = pack(&format!("{}{}", "0".repeat(prefix), "1".repeat(keep)));
                let mut bits = BitReader::new(&bytes);
                bits.skip(prefix).unwrap();
                assert!(HcrHeader::read(&mut bits, pair).is_err());
                assert_eq!(bits.position(), prefix);
            }
        }
    }
}
