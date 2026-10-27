//! Owned MPEG-4 ErrorProtectionSpecificConfig and access-unit recovery.
//! Syntax: ISO/IEC 14496-3 tables 1.49–1.53. Unsupported mappings remain errors.
use super::{Result, aac_ep_crc, aac_ep_srcpc, bits::BitReader, invalid};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Length {
    Fixed(u16),
    InBand(u8),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Class {
    pub length: Length,
    pub rate: Option<u8>,
    pub crc: Option<u8>,
    pub concatenated: bool,
    pub fec: u8,
    pub terminated: bool,
    pub interleave: Option<u8>,
    pub optional: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Set {
    pub classes: Vec<Class>,
    pub order: Option<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub sets: Vec<Set>,
    pub interleave: u8,
    pub stuffing: bool,
    pub frames: u8,
    pub header: Option<(u8, u8)>,
}
fn crc_width(code: u8) -> Result<u8> {
    match code {
        0..=16 => Ok(code),
        17 => Ok(24),
        18 => Ok(32),
        _ => Err(invalid("reserved EP CRC length code")),
    }
}
impl Config {
    pub fn read_prefix(bytes: &[u8], offset: usize) -> Result<(Self, usize)> {
        let mut b = BitReader::new(bytes);
        b.skip(offset)?;
        let start = b.position();
        let count = b.read(8)? as usize;
        if count == 0 {
            return Err(invalid("EP has no predefined sets"));
        }
        let interleave = b.read(2)? as u8;
        if interleave == 3 {
            return Err(invalid("reserved EP interleave mode"));
        }
        let stuffing = b.read(3)?;
        if stuffing > 1 {
            return Err(invalid("reserved EP stuffing mode"));
        }
        let frames = b.read(3)? as u8;
        if frames == 0 {
            return Err(invalid("reserved EP concatenated frame count"));
        }
        let mut sets = Vec::new();
        for _ in 0..count {
            let n = b.read(6)? as usize;
            let mut classes = Vec::new();
            for _ in 0..n {
                let length_escape = b.bit()?;
                let rate_escape = b.bit()?;
                let crc_escape = b.bit()?;
                let concatenated = if frames != 1 { b.bit()? } else { false };
                let fec = b.read(2)? as u8;
                if fec == 3 {
                    return Err(invalid("reserved EP FEC type"));
                }
                let terminated = if fec == 0 { b.bit()? } else { false };
                let mode = if interleave == 2 {
                    Some(b.read(2)? as u8)
                } else {
                    None
                };
                if fec != 0 && mode == Some(2) {
                    return Err(invalid("reserved RS intraclass interleave mode"));
                }
                let optional = b.bit()?;
                let length = if length_escape {
                    Length::InBand(b.read(4)? as u8)
                } else {
                    Length::Fixed(b.read(16)? as u16)
                };
                let rate = if rate_escape {
                    None
                } else {
                    let rate = b.read(if fec == 0 { 5 } else { 7 })? as u8;
                    if fec == 0 && rate > 24 {
                        return Err(invalid("reserved EP SRCPC code rate"));
                    }
                    Some(rate)
                };
                let crc = if crc_escape {
                    None
                } else {
                    Some(crc_width(b.read(5)? as u8)?)
                };
                if fec != 0 && matches!(length,Length::Fixed(n)if n%8!=0) {
                    return Err(invalid("RS information class is not byte aligned"));
                }
                classes.push(Class {
                    length,
                    rate,
                    crc,
                    concatenated,
                    fec,
                    terminated,
                    interleave: mode,
                    optional,
                });
            }
            let order = if b.bit()? {
                let mut order = Vec::new();
                let mut seen = vec![false; n];
                for _ in 0..n {
                    let index = b.read(6)? as usize;
                    if index >= n || seen[index] {
                        return Err(invalid("invalid EP class output permutation"));
                    }
                    seen[index] = true;
                    order.push(index as u8);
                }
                Some(order)
            } else {
                None
            };
            sets.push(Set { classes, order });
        }
        let header = if b.bit()? {
            let rate = b.read(5)? as u8;
            let width = crc_width(b.read(5)? as u8)?;
            if rate > 24 {
                return Err(invalid("reserved EP header code rate"));
            }
            Some((rate, width))
        } else {
            None
        };
        let used = b.position() - start;
        Ok((
            Self {
                sets,
                interleave,
                stuffing: stuffing != 0,
                frames,
                header,
            },
            used,
        ))
    }
    /// Current application mapping: one class contains the complete raw AAC AU.
    /// EP2 does not normatively define arbitrary application class-to-ESC mapping.
    pub fn validate_raw_access_unit(&self) -> Result<()> {
        if self.interleave != 0 || self.frames != 1 || self.sets.len() != 1 {
            return Err(invalid(
                "EP interleaving or concatenated frame routing is not yet implemented",
            ));
        }
        let set = &self.sets[0];
        if set.classes.len() != 1 || set.classes[0].optional {
            return Err(invalid(
                "EP application class mapping requires an explicit assembler",
            ));
        }
        let c = &set.classes[0];
        if c.fec != 0 {
            return Err(invalid(
                "EP Reed-Solomon class recovery is not yet implemented",
            ));
        }
        let header_bits = self.header_bits();
        if matches!(header_bits, 1 | 2) {
            return Err(invalid("EP short majority header is not yet implemented"));
        }
        Ok(())
    }
    fn header_bits(&self) -> usize {
        let c = &self.sets[0].classes[0];
        let length = match c.length {
            Length::InBand(n) => usize::from(n),
            _ => 0,
        };
        length
            + 3 * usize::from(c.rate.is_none())
            + 3 * usize::from(c.crc.is_none())
            + 3 * usize::from(self.stuffing)
    }
    pub fn decode_raw_access_unit(&self, packet: &[u8]) -> Result<Vec<u8>> {
        self.validate_raw_access_unit()?;
        let c = &self.sets[0].classes[0];
        let all: Vec<u8> = packet
            .iter()
            .flat_map(|&b| (0..8).rev().map(move |i| (b >> i) & 1))
            .collect();
        let (header, offset) = decode_header(&all, self.header_bits(), self.header)?;
        let mut at = 0;
        let mut read = |n: usize| -> usize {
            let value = header[at..at + n]
                .iter()
                .fold(0, |v, &b| (v << 1) | usize::from(b));
            at += n;
            value
        };
        let length = match c.length {
            Length::Fixed(n) => Some(usize::from(n)),
            Length::InBand(0) => None,
            Length::InBand(n) => Some(read(usize::from(n))),
        };
        let rate = c
            .rate
            .unwrap_or_else(|| [0, 3, 4, 6, 8, 12, 16, 24][read(3)]);
        let width = c
            .crc
            .unwrap_or_else(|| [0, 6, 8, 10, 12, 14, 16, 32][read(3)]);
        let stuffing = if self.stuffing { read(3) } else { 0 };
        let end = all
            .len()
            .checked_sub(stuffing)
            .filter(|&end| end >= offset)
            .ok_or_else(|| invalid("truncated EP stuffing/header"))?;
        let size = end - offset;
        let information = if let Some(n) = length {
            if n == 0 {
                if size == 0 {
                    return Ok(vec![]);
                }
                return Err(invalid("nonempty EP encoding for zero-bit class"));
            }
            n.checked_add(usize::from(width))
                .ok_or_else(|| invalid("EP class length overflow"))?
        } else {
            let mut low = 0;
            let mut high = size;
            while low < high {
                let middle = low + (high - low) / 2;
                if aac_ep_srcpc::encoded_len(middle, rate, c.terminated)? >= size {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            low
        };
        if aac_ep_srcpc::encoded_len(information, rate, c.terminated)? != size
            || information <= usize::from(width)
        {
            return Err(invalid("EP class/packet length mismatch"));
        }
        let recovered = aac_ep_srcpc::decode(&all[offset..end], information, rate, c.terminated)?;
        if !aac_ep_crc::verify(&recovered.information, width)? {
            return Err(invalid("EP class CRC mismatch"));
        }
        let bits = &recovered.information[..information - usize::from(width)];
        let mut raw = vec![0; bits.len().div_ceil(8)];
        for (i, &bit) in bits.iter().enumerate() {
            raw[i / 8] |= bit << (7 - i % 8);
        }
        Ok(raw)
    }
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        f.vector(&self.sets)?;
        for set in &self.sets {
            f.vector(&set.classes)?;
            if let Some(order) = &set.order {
                f.vector(order)?;
            }
        }
        Ok(())
    }
}

fn small_code(message: u32, n: usize) -> (u32, usize, usize) {
    let (degree, polynomial, correction) = match n {
        3..=4 => (3, 0xb, 1),
        5..=7 => (8, 0x1d1, 2),
        8..=12 => (11, 0xae3, 3),
        13..=16 => (15, 0x8faf, 3),
        _ => unreachable!(),
    };
    let mut remainder = message << degree;
    while remainder != 0 && 32 - remainder.leading_zeros() > degree as u32 {
        remainder ^= polynomial << (31 - remainder.leading_zeros() - degree as u32);
    }
    ((message << degree) | remainder, degree, correction)
}
fn decode_header(bits: &[u8], n: usize, extended: Option<(u8, u8)>) -> Result<(Vec<u8>, usize)> {
    if n == 0 {
        return Ok((vec![], 0));
    }
    if n <= 16 {
        if n < 3 {
            return Err(invalid("EP majority header is not yet implemented"));
        }
        let (_, degree, cap) = small_code(0, n);
        let count = n + degree;
        if bits.len() < count {
            return Err(invalid("truncated EP BCH/Golay header"));
        }
        let received = bits[..count]
            .iter()
            .fold(0u32, |w, &b| (w << 1) | u32::from(b));
        let clean = received >> degree;
        if small_code(clean, n).0 == received {
            return Ok((
                (0..n).rev().map(|i| ((clean >> i) & 1) as u8).collect(),
                count,
            ));
        }
        let mut best = usize::MAX;
        let mut value = 0;
        let mut ties = false;
        static WORDS: [std::sync::OnceLock<Vec<u32>>; 17] =
            [const { std::sync::OnceLock::new() }; 17];
        let words =
            WORDS[n].get_or_init(|| (0..1u32 << n).map(|word| small_code(word, n).0).collect());
        for (word, &encoded) in words.iter().enumerate() {
            let word = word as u32;
            let distance = (encoded ^ received).count_ones() as usize;
            if distance < best {
                best = distance;
                value = word;
                ties = false;
            } else if distance == best {
                ties = true;
            }
        }
        if ties || best > cap {
            return Err(invalid("uncorrectable EP BCH/Golay header"));
        }
        return Ok((
            (0..n).rev().map(|i| ((value >> i) & 1) as u8).collect(),
            count,
        ));
    }
    let (rate, width) = extended.unwrap_or((8, 4));
    let count = aac_ep_srcpc::encoded_len(n + usize::from(width), rate, true)?;
    if bits.len() < count {
        return Err(invalid("truncated EP SRCPC header"));
    }
    let decoded = aac_ep_srcpc::decode(&bits[..count], n + usize::from(width), rate, true)?;
    if !aac_ep_crc::verify(&decoded.information, width)? {
        return Err(invalid("EP header CRC mismatch"));
    }
    Ok((decoded.information[..n].to_vec(), count))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bits(text: &str) -> Vec<u8> {
        text.bytes().map(|b| b - b'0').collect()
    }
    #[test]
    fn protected_headers_match_independent_oracles_and_correct_single_bit_errors() {
        let m: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ep-playback/manifest.json"
        ))
        .unwrap();
        let rows = m["header_vectors"].as_array().unwrap();
        assert_eq!(rows.len(), 132);
        for v in rows {
            let expected = bits(v["information"].as_str().unwrap());
            let encoded = bits(v["encoded"].as_str().unwrap());
            let ext = if v["extended"].as_bool().unwrap() {
                Some((24, 16))
            } else {
                None
            };
            assert_eq!(
                decode_header(&encoded, expected.len(), ext).unwrap(),
                (expected.clone(), encoded.len())
            );
            for at in 0..encoded.len() {
                let mut damaged = encoded.clone();
                damaged[at] ^= 1;
                assert_eq!(
                    decode_header(&damaged, expected.len(), ext).unwrap(),
                    (expected.clone(), encoded.len()),
                    "header bits {} flipped {at}",
                    expected.len()
                );
            }
            assert!(decode_header(&encoded[..encoded.len() - 1], expected.len(), ext).is_err());
        }
    }
    #[test]
    fn ep_configuration_is_bounded_and_reserved_modes_remain_errors() {
        let asc =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-ep-srcpc/protected.asc");
        assert!(
            super::super::config::AacConfig::parse(asc)
                .unwrap_err()
                .to_string()
                .contains("extension-aware decoder")
        );
        let (config, used) = Config::read_prefix(asc, 30).unwrap();
        assert!(used > 0);
        config.validate_raw_access_unit().unwrap();
        assert_eq!(config.sets[0].classes[0].rate, Some(24));
        assert_eq!(config.sets[0].classes[0].crc, Some(16));
        for end in 0..asc.len() {
            assert!(Config::read_prefix(&asc[..end], 30).is_err());
        }
        let mut no_sets = asc.to_vec();
        for i in 30..38 {
            no_sets[i / 8] &= !(1 << (7 - i % 8));
        }
        assert!(
            Config::read_prefix(&no_sets, 30)
                .unwrap_err()
                .to_string()
                .contains("no predefined sets")
        );
        let mut bad_interleave = asc.to_vec();
        for i in 38..40 {
            bad_interleave[i / 8] |= 1 << (7 - i % 8);
        }
        assert!(
            Config::read_prefix(&bad_interleave, 30)
                .unwrap_err()
                .to_string()
                .contains("reserved EP interleave")
        );
        let mut bad_fec = asc.to_vec();
        for i in 55..57 {
            bad_fec[i / 8] |= 1 << (7 - i % 8);
        }
        assert!(
            Config::read_prefix(&bad_fec, 30)
                .unwrap_err()
                .to_string()
                .contains("reserved EP FEC")
        );
    }
}
