//! Owned G.711 A-law and mu-law scalar and interleaved packet conversion.
//! Code words represent signed 16-bit reconstruction levels. Encoding first
//! quantizes linear i16 to the standard 13-bit A-law / 14-bit mu-law domain.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Law {
    ALaw,
    MuLaw,
}
impl Law {
    pub fn decode(self, code: u8) -> i16 {
        let (segment, mantissa, negative) = match self {
            Self::ALaw => {
                let value = code ^ 0x55;
                ((value >> 4) & 7, value & 15, value & 128 == 0)
            }
            Self::MuLaw => {
                let value = !code;
                ((value >> 4) & 7, value & 15, value & 128 != 0)
            }
        };
        let magnitude = match self {
            Self::ALaw => {
                if segment == 0 {
                    i32::from(mantissa) * 16 + 8
                } else {
                    (i32::from(mantissa) * 16 + 264) << (segment - 1)
                }
            }
            Self::MuLaw => ((i32::from(mantissa) * 8 + 132) << segment) - 132,
        };
        (if negative { -magnitude } else { magnitude }) as i16
    }
    pub fn encode(self, sample: i16) -> u8 {
        let shift = if self == Self::ALaw { 3 } else { 2 };
        let sample = i32::from(sample) >> shift;
        let negative = sample < 0;
        let mut magnitude = if negative {
            -sample - if self == Self::ALaw { 1 } else { 0 }
        } else {
            sample
        };
        let (mask, segment, mantissa) = match self {
            Self::ALaw => {
                let segment = (31 - magnitude.max(1).leading_zeros() as i32 - 4).clamp(0, 7) as u8;
                let mantissa = (magnitude >> if segment < 2 { 1 } else { segment }) as u8 & 15;
                (if negative { 0x55 } else { 0xd5 }, segment, mantissa)
            }
            Self::MuLaw => {
                magnitude = magnitude.min(8159) + 33;
                let segment = (31 - magnitude.leading_zeros() as i32 - 5).clamp(0, 8) as u8;
                if segment == 8 {
                    return if negative { 0 } else { 0x80 };
                }
                (
                    if negative { 0x7f } else { 0xff },
                    segment,
                    (magnitude >> (segment + 1)) as u8 & 15,
                )
            }
        };
        ((segment << 4) | mantissa) ^ mask
    }
}
/// Stateless packets still require a whole sample for each interleaved channel.
pub struct Decoder {
    law: Law,
    rate: u32,
    channels: u16,
}
impl Decoder {
    pub fn new(law: Law, rate: u32, channels: u16) -> Result<Self> {
        if rate == 0 || channels == 0 {
            return Err(invalid("G.711 requires a sample rate and channels"));
        }
        Ok(Self {
            law,
            rate,
            channels,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn decode(&self, data: &[u8]) -> Result<Vec<i16>> {
        if !data.len().is_multiple_of(usize::from(self.channels)) {
            return Err(invalid("G.711 packet ends in an incomplete channel frame"));
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(data.len())
            .map_err(|_| invalid("cannot allocate G.711 PCM"))?;
        output.extend(data.iter().map(|&code| self.law.decode(code)));
        Ok(output)
    }
    pub fn encode(&self, data: &[i16]) -> Result<Vec<u8>> {
        if !data.len().is_multiple_of(usize::from(self.channels)) {
            return Err(invalid("G.711 PCM ends in an incomplete channel frame"));
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(data.len())
            .map_err(|_| invalid("cannot allocate G.711 codes"))?;
        output.extend(data.iter().map(|&sample| self.law.encode(sample)));
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_code_matches_saved_independent_segment_levels() {
        for (law, expected) in [
            (
                Law::ALaw,
                include_bytes!("../../../tests/fixtures/playback-errors/g711-alaw-codes.s16le")
                    .as_slice(),
            ),
            (
                Law::MuLaw,
                include_bytes!("../../../tests/fixtures/playback-errors/g711-mulaw-codes.s16le")
                    .as_slice(),
            ),
        ] {
            for code in 0..=255u8 {
                let sample = i16::from_le_bytes(
                    expected[code as usize * 2..code as usize * 2 + 2]
                        .try_into()
                        .unwrap(),
                );
                assert_eq!(law.decode(code), sample, "{law:?} {code:02x}");
                let canonical = if law == Law::MuLaw && code == 0x7f {
                    0xff
                } else {
                    code
                };
                assert_eq!(law.encode(sample), canonical);
            }
            let mut previous = i16::MIN;
            for sample in i16::MIN..=i16::MAX {
                let reconstructed = law.decode(law.encode(sample));
                assert!(reconstructed >= previous);
                previous = reconstructed;
            }
        }
    }
    #[test]
    fn interleaving_geometry_and_stateless_packet_boundaries() {
        for law in [Law::ALaw, Law::MuLaw] {
            assert!(Decoder::new(law, 0, 1).is_err());
            assert!(Decoder::new(law, 8000, 0).is_err());
            let decoder = Decoder::new(law, 8000, 2).unwrap();
            let bytes: Vec<_> = (0..=255u8).collect();
            let expected = decoder.decode(&bytes).unwrap();
            let mut actual = Vec::new();
            for chunk in bytes.chunks(14) {
                actual.extend(decoder.decode(chunk).unwrap());
            }
            assert_eq!(actual, expected);
            assert!(decoder.decode(&[0]).is_err());
            assert!(decoder.encode(&[0]).is_err());
            assert!(decoder.decode(&[]).unwrap().is_empty());
        }
    }
}
