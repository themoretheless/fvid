//! Checked VP9 Boolean arithmetic decoder, specification section 9.2.
use super::bits::BitReader;
use crate::{Result, invalid};

#[derive(Clone)]
pub struct BoolDecoder<'a> {
    bits: BitReader<'a>,
    value: u32,
    range: u32,
}
impl<'a> BoolDecoder<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let mut bits = BitReader::new(data);
        let value = bits.read(8)?;
        let mut decoder = Self {
            bits,
            value,
            range: 255,
        };
        if decoder.read(128)? {
            return Err(invalid("nonzero VP9 arithmetic marker"));
        }
        Ok(decoder)
    }
    pub fn read(&mut self, probability: u8) -> Result<bool> {
        let split = 1 + (((self.range - 1) * u32::from(probability)) >> 8);
        let bit = self.value >= split;
        let (mut range, mut value) = if bit {
            (self.range - split, self.value - split)
        } else {
            (split, self.value)
        };
        let shifts = range.leading_zeros().saturating_sub(24);
        // Check before changing any state, including the input position.
        let suffix = self.bits.read(shifts as u8)?;
        range <<= shifts;
        value = (value << shifts) | suffix;
        self.range = range;
        self.value = value;
        Ok(bit)
    }
    pub fn literal(&mut self, count: u8) -> Result<u32> {
        if count > 32 {
            return Err(invalid("oversized VP9 arithmetic literal"));
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | u32::from(self.read(128)?);
        }
        Ok(value)
    }
    pub fn position(&self) -> usize {
        self.bits.position()
    }
    pub fn finish(mut self) -> Result<()> {
        while self.bits.remaining() != 0 {
            let count = self.bits.remaining().min(32) as u8;
            if self.bits.read(count)? != 0 {
                return Err(invalid("nonzero VP9 arithmetic padding"));
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marker_padding_and_exhaustion() {
        assert!(BoolDecoder::new(&[]).is_err());
        assert!(BoolDecoder::new(&[128, 0]).is_err());
        let mut d = BoolDecoder::new(&[0, 0]).unwrap();
        // The zero marker leaves range 128; eight equiprobable zero symbols
        // consume the remaining input exactly.
        for _ in 0..8 {
            assert!(!d.read(128).unwrap());
        }
        assert_eq!(d.position(), 16);
        assert!(d.read(128).is_err());
        assert_eq!(d.position(), 16);
        d.finish().unwrap();
        assert!(BoolDecoder::new(&[0, 1]).unwrap().finish().is_err());
    }
    #[test]
    fn known_binary_fractions_and_extreme_probabilities() {
        let mut d = BoolDecoder::new(&[0x55, 0xaa, 0]).unwrap();
        assert_eq!(d.literal(7).unwrap(), 0x55);
        assert_eq!(d.literal(8).unwrap(), 0xaa);
        d.finish().unwrap();
        let mut d = BoolDecoder::new(&[0; 4]).unwrap();
        assert!(!d.read(0).unwrap());
        assert!(!d.read(255).unwrap());
    }
}
