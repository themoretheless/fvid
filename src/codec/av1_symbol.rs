//! AV1 section 8.2 adaptive multi-symbol range decoder.
use super::bits::BitReader;
use crate::{Result, invalid};

pub struct SymbolDecoder<'a> {
    input: BitReader<'a>,
    data: &'a [u8],
    range: u32,
    value: u32,
    available: i64,
    adapt: bool,
    failed: bool,
}
impl<'a> SymbolDecoder<'a> {
    pub fn new(data: &'a [u8], adapt: bool) -> Result<Self> {
        if data.is_empty() || data.len() > (i32::MAX as usize / 8) {
            return Err(invalid("invalid AV1 tile entropy size"));
        }
        let mut input = BitReader::new(data);
        let n = input.remaining().min(15) as u8;
        let value = 32767 ^ (input.read(n)? << (15 - n));
        Ok(Self {
            input,
            data,
            range: 32768,
            value,
            available: data.len() as i64 * 8 - 15,
            adapt,
            failed: false,
        })
    }
    /// Cumulative probabilities in ascending order, followed by adaptation count.
    /// The penultimate entry must be 32768 and count must be at most 32.
    pub fn read(&mut self, cdf: &mut [u16]) -> Result<usize> {
        if self.failed {
            return Err(invalid("AV1 symbol decoder requires reset after error"));
        }
        let result = self.read_inner(cdf);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn read_inner(&mut self, cdf: &mut [u16]) -> Result<usize> {
        if !(3..=17).contains(&cdf.len()) {
            return Err(invalid("invalid AV1 CDF alphabet size"));
        }
        let n = cdf.len() - 1;
        if cdf[n - 1] != 32768 || cdf[n] > 32 || cdf[..n].windows(2).any(|p| p[0] > p[1]) {
            return Err(invalid("invalid AV1 cumulative distribution"));
        }
        let mut current = self.range;
        let mut symbol = 0;
        loop {
            let previous = current;
            let f = 32768 - u32::from(cdf[symbol]);
            current = ((self.range >> 8) * (f >> 6) >> 1) + 4 * (n - symbol - 1) as u32;
            if self.value >= current {
                self.range = previous - current;
                self.value -= current;
                break;
            }
            symbol += 1;
        }
        let bits = (self.range.leading_zeros() - 16) as u8;
        let real_bits = i64::from(bits).min(self.available.max(0)) as u8;
        let padded = self.input.read(real_bits)? << (bits - real_bits);
        self.range <<= bits;
        self.value = padded ^ (((self.value + 1) << bits) - 1);
        self.available -= i64::from(bits);
        if self.available < -14 {
            return Err(invalid("truncated AV1 entropy data"));
        }
        if self.adapt {
            let rate = 3 + u32::from(cdf[n] > 15) + u32::from(cdf[n] > 31) + (n.ilog2()).min(2);
            for (i, p) in cdf[..n - 1].iter_mut().enumerate() {
                if i < symbol {
                    *p -= *p >> rate;
                } else {
                    *p += (32768 - *p) >> rate;
                }
            }
            cdf[n] += u16::from(cdf[n] < 32);
        }
        Ok(symbol)
    }
    pub fn bit(&mut self) -> Result<bool> {
        Ok(self.read(&mut [16384, 32768, 0])? != 0)
    }
    pub fn literal(&mut self, bits: u8) -> Result<u32> {
        if bits > 32 {
            return Err(invalid("AV1 entropy literal exceeds 32 bits"));
        }
        let mut value = 0;
        for _ in 0..bits {
            value = (value << 1) | u32::from(self.bit()?);
        }
        Ok(value)
    }
    pub fn finish(&self) -> Result<()> {
        if self.failed || self.available < -14 {
            return Err(invalid("invalid AV1 entropy state"));
        }
        let back = (self.available + 15).min(15) as usize;
        let start = self
            .input
            .position()
            .checked_sub(back)
            .ok_or_else(|| invalid("invalid AV1 entropy termination"))?;
        for bit in start..self.data.len() * 8 {
            let set = self.data[bit / 8] & (1 << (7 - bit % 8)) != 0;
            if set != (bit == start) {
                return Err(invalid("invalid AV1 entropy trailing bits"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn libaom_encoded_symbols_with_and_without_adaptation() {
        let mut fixture = &include_bytes!("../../tests/fixtures/av1/symbols.bin")[..];
        for adapt in [false, true] {
            for n in 2..=16usize {
                let len = u32::from_le_bytes(fixture[..4].try_into().unwrap()) as usize;
                let data = &fixture[4..4 + len];
                fixture = &fixture[4 + len..];
                let mut decoder = SymbolDecoder::new(data, adapt).unwrap();
                let mut cdf = (1..=n).map(|i| (32768 * i / n) as u16).collect::<Vec<_>>();
                cdf.push(0);
                let mut state = 0x31415926u32;
                for k in 0..1024 {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    let expected = (state >> 16) as usize % n;
                    assert_eq!(
                        decoder.read(&mut cdf).unwrap(),
                        expected,
                        "adapt={adapt} n={n} k={k}"
                    );
                }
                decoder.finish().unwrap();
            }
        }
        assert!(fixture.is_empty());
    }
    #[test]
    fn empty_alphabet_and_truncation_fail_without_panics() {
        assert!(SymbolDecoder::new(&[], true).is_err());
        let mut d = SymbolDecoder::new(&[0], true).unwrap();
        assert!(d.read(&mut [0, 0, 0]).is_err());
        assert!(d.bit().is_err());
        for byte in 0..=255 {
            let data = [byte];
            let mut d = SymbolDecoder::new(&data, true).unwrap();
            let mut cdf = [16384, 32768, 0];
            let mut reads = 0;
            while d.read(&mut cdf).is_ok() {
                reads += 1;
                assert!(reads < 1024);
            }
            assert!(d.finish().is_err());
        }
    }
}
