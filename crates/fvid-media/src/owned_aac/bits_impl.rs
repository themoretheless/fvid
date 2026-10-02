
#[derive(Clone)]
pub struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    pub fn position(&self) -> usize {
        self.position
    }
    pub fn remaining(&self) -> usize {
        self.bytes
            .len()
            .saturating_mul(8)
            .saturating_sub(self.position)
    }
    pub fn read(&mut self, count: u8) -> Result<u32> {
        if count > 32 || usize::from(count) > self.remaining() {
            return Err(invalid("truncated or oversized bit field"));
        }
        if count == 0 {
            return Ok(0);
        }
        // Gather the covering bytes (at most five for 32 bits at any offset)
        // into one word and mask the field out, instead of one loop step per bit.
        let count = usize::from(count);
        let (byte, bit) = (self.position / 8, self.position % 8);
        let needed = (bit + count).div_ceil(8);
        let mut word = 0u64;
        for &b in &self.bytes[byte..byte + needed] {
            word = (word << 8) | u64::from(b);
        }
        let value = (word >> (needed * 8 - bit - count)) & ((1u64 << count) - 1);
        self.position += count;
        Ok(value as u32)
    }
    /// CABAC renormalization consumes at most eight bits at a time.
    #[inline]
    pub(crate) fn read_short(&mut self, count: u8) -> Result<u16> {
        if count > 8 || usize::from(count) > self.remaining() {
            return Err(invalid("truncated or oversized short bit field"));
        }
        if count == 0 {
            return Ok(0);
        }
        let byte = self.position / 8;
        let bit = self.position % 8;
        let count = usize::from(count);
        let first = u16::from(self.bytes[byte]);
        let value = if bit + count <= 8 {
            first >> (8 - bit - count)
        } else {
            ((first << 8) | u16::from(self.bytes[byte + 1])) >> (16 - bit - count)
        };
        self.position += count;
        Ok(value & ((1 << count) - 1))
    }
    pub fn bit(&mut self) -> Result<bool> {
        Ok(self.read(1)? != 0)
    }
    pub fn skip(&mut self, count: usize) -> Result<()> {
        if count > self.remaining() {
            return Err(invalid("truncated bitstream"));
        }
        self.position += count;
        Ok(())
    }
    pub fn unsigned_golomb(&mut self) -> Result<u32> {
        let mut zeros = 0;
        while !self.bit()? {
            zeros += 1;
            if zeros > 31 {
                return Err(invalid("exponential-Golomb value exceeds supported range"));
            }
        }
        Ok(((1u32 << zeros) - 1) + self.read(zeros)?)
    }
    pub fn signed_golomb(&mut self) -> Result<i32> {
        let code = self.unsigned_golomb()?;
        if code & 1 != 0 {
            Ok((code / 2 + 1) as i32)
        } else {
            Ok(-((code / 2) as i32))
        }
    }
    /// Whether anything precedes the final RBSP stop bit and alignment zeros.
    pub fn more_rbsp_data(&self) -> bool {
        if self.remaining() == 0 || self.remaining() > 8 {
            return true;
        }
        let mut copy = self.clone();
        if copy.read(1).ok() != Some(1) {
            return true;
        }
        while copy.remaining() > 0 {
            if copy.read(1).ok() != Some(0) {
                return true;
            }
        }
        false
    }
    /// Consume rbsp_stop_one_bit and alignment zeros; reject trailing bytes.
    pub fn finish_rbsp(&mut self) -> Result<()> {
        if !self.bit()? {
            return Err(invalid("missing RBSP stop bit"));
        }
        while !self.position.is_multiple_of(8) {
            if self.bit()? {
                return Err(invalid("nonzero RBSP alignment bit"));
            }
        }
        if self.remaining() != 0 {
            return Err(invalid("extra data after RBSP trailing bits"));
        }
        Ok(())
    }
}

/// Remove AVC/HEVC emulation-prevention bytes from a NAL payload (header excluded).
/// Allocation cannot exceed the input length. Malformed escape sequences are rejected.
pub fn unescape_rbsp(ebsp: &[u8]) -> Result<Vec<u8>> {
    let mut rbsp = Vec::new();
    rbsp.try_reserve_exact(ebsp.len())
        .map_err(|_| invalid("RBSP allocation failed"))?;
    let mut zeros = 0;
    let mut i = 0;
    while i < ebsp.len() {
        let byte = ebsp[i];
        if zeros >= 2 {
            if byte == 3 {
                if ebsp.get(i + 1).is_none_or(|&next| next > 3) {
                    return Err(invalid("invalid emulation-prevention sequence"));
                }
                zeros = 0;
                i += 1;
                continue;
            }
            if byte <= 2 {
                return Err(invalid("unescaped start-code pattern in NAL payload"));
            }
        }
        rbsp.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        i += 1;
    }
    Ok(rbsp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_fields_match_general_reader_and_preserve_position_on_error() {
        let bytes = [0xa7, 0x93, 0x5c, 0xff, 0, 0x51];
        for length in 0..=bytes.len() {
            for position in 0..=length * 8 {
                for count in 0..=8 {
                    let mut general = BitReader::new(&bytes[..length]);
                    general.skip(position).unwrap();
                    let mut short = general.clone();
                    let a = general.read(count);
                    let b = short.read_short(count).map(u32::from);
                    assert_eq!(a.is_ok(), b.is_ok());
                    if let Ok(a) = a {
                        assert_eq!(a, b.unwrap());
                    }
                    assert_eq!(general.position(), short.position());
                }
            }
        }
        let mut reader = BitReader::new(&bytes);
        assert!(reader.read_short(9).is_err());
        assert_eq!(reader.position(), 0);
    }
    #[test]
    fn crosses_bytes_and_preserves_position_on_short_read() {
        let mut bits = BitReader::new(&[0xab, 0xcd, 0xef, 0x01, 0x23]);
        assert_eq!(bits.read(4).unwrap(), 10);
        assert_eq!(bits.read(32).unwrap(), 0xbcde_f012);
        assert!(bits.read(5).is_err());
        assert_eq!(bits.position(), 36);
        assert_eq!(bits.read(4).unwrap(), 3);
        assert!(bits.bit().is_err());
    }
    #[test]
    fn golomb_and_trailing_bits() {
        // ue(v): 0=1, 1=010, 2=011, 3=00100, then stop and padding.
        let mut bits = BitReader::new(&[0xa6, 0x48]);
        for value in 0..4 {
            assert_eq!(bits.unsigned_golomb().unwrap(), value);
        }
        bits.finish_rbsp().unwrap();
        let mut signed = BitReader::new(&[0xa6, 0x40]);
        for value in [0, 1, -1, 2] {
            assert_eq!(signed.signed_golomb().unwrap(), value);
        }
        assert!(signed.finish_rbsp().is_err());
        assert!(BitReader::new(&[0; 8]).unsigned_golomb().is_err());
    }
    #[test]
    fn unescapes_and_rejects_broken_sequences() {
        assert_eq!(
            unescape_rbsp(&[0, 0, 3, 0, 0, 3, 1, 2]).unwrap(),
            [0, 0, 0, 0, 1, 2]
        );
        for bad in [&[0, 0, 3][..], &[0, 0, 3, 4], &[0, 0, 1], &[0, 0, 2]] {
            assert!(unescape_rbsp(bad).is_err());
        }
    }
}
