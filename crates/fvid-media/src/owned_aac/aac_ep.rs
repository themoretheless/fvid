//! Owned MPEG-4 ErrorProtectionSpecificConfig and access-unit recovery.
//! Syntax: ISO/IEC 14496-3 tables 1.49–1.53. Unsupported mappings remain errors.
use super::{
    Result, aac_ep_crc, aac_ep_interleave, aac_ep_rs, aac_ep_srcpc, bits::BitReader, invalid,
};
#[path = "aac_ep_classes.rs"]
mod classes;
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
/// One recovered class in original output order. Information is represented
/// as exact MSB-first bits (each value is 0 or 1), without CRC or octet padding.
/// An absent optional class is omitted; a present zero-bit class is retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveredClass {
    pub class_index: usize,
    pub bits: Vec<u8>,
}
/// One source frame after FEC, CRC, interleaving and frame concatenation have
/// been undone. These class boundaries are required for EP3's ESC mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveredFrame {
    /// Original configured set, before optional-class derivation.
    pub set_index: usize,
    pub classes: Vec<RecoveredClass>,
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
                // Byte geometry includes CRC and may span an RS group. Defer
                // it until selection, attributes and grouping are resolved.
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
    /// Number of selectable wire sets after optional-class derivation. No
    /// derived class lists are materialized, including counts exceeding u64.
    pub fn predefined_set_count(&self) -> Result<u128> {
        classes::predefined_count(self)
    }
    /// Recover exact class bits in source-frame and original class order.
    /// Unlike the EP2 raw-AU API, this does not impose octet alignment on a
    /// frame or discard the boundary between error sensitivity categories.
    /// This is protection recovery, not an AAC ESC-to-logical-payload decoder.
    pub fn decode_class_frames(&self, packet: &[u8]) -> Result<Vec<RecoveredFrame>> {
        classes::decode_class_frames(self, packet)
    }
    /// EP2 output classes concatenate, after reordering, to the epConfig=0 AU.
    pub fn validate_raw_access_unit(&self) -> Result<()> {
        classes::validate(self)
    }
    pub fn decode_raw_access_unit(&self, packet: &[u8]) -> Result<Vec<u8>> {
        classes::decode(self, packet)
    }
    /// Recover source AUs in source-frame order, undoing class-wise frame
    /// concatenation before packing each original AU's alignment bits.
    pub fn decode_raw_access_units(&self, packet: &[u8]) -> Result<Vec<Vec<u8>>> {
        classes::decode_units(self, packet)
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

/// Remove one header stage before interpreting the next stage's shape.
fn decode_frame_header(
    mut bits: Vec<u8>,
    n: usize,
    extended: Option<(u8, u8)>,
    interleaved: bool,
) -> Result<(Vec<u8>, Vec<u8>)> {
    if n == 0 {
        return Ok((vec![], bits));
    }
    if !interleaved {
        let (header, used) = decode_header(&bits, n, extended)?;
        let body = bits.split_off(used);
        return Ok((header, body));
    }
    let (count, width) = match n {
        1..=2 => (3 * n, 3),
        3..=16 => {
            let degree = small_code(0, n).1;
            (n + degree, n + degree)
        }
        _ => {
            let (rate, crc) = extended.unwrap_or((8, 4));
            (
                aac_ep_srcpc::encoded_len(n + usize::from(crc), rate, true)?,
                28,
            )
        }
    };
    let (protected, body) = aac_ep_interleave::split_stage(&bits, count, width)?;
    let (header, used) = decode_header(&protected, n, extended)?;
    debug_assert_eq!(used, count);
    Ok((header, body))
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
            if bits.len() < 3 * n {
                return Err(invalid("truncated EP majority header"));
            }
            // Information first, followed by two repetitions as parity.
            let message = (0..n)
                .map(|i| u8::from(bits[i] + bits[n + i] + bits[2 * n + i] >= 2))
                .collect();
            return Ok((message, 3 * n));
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
    fn predefined_set_headers_match_all_independent_selector_words() {
        let m: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ep-sets/manifest.json"
        ))
        .unwrap();
        assert_eq!(m["header_vectors"].as_array().unwrap().len(), 510);
        for row in m["header_vectors"].as_array().unwrap() {
            let information = bits(row["information"].as_str().unwrap());
            let encoded = bits(row["encoded"].as_str().unwrap());
            let n = information.len();
            assert_eq!(
                decode_header(&encoded, n, None).unwrap(),
                (information.clone(), encoded.len())
            );
            for i in 0..encoded.len() {
                let mut damaged = encoded.clone();
                damaged[i] ^= 1;
                assert_eq!(
                    decode_header(&damaged, n, None).unwrap().0,
                    information,
                    "{n}-bit selector, flipped {i}"
                );
            }
            for end in 0..encoded.len() {
                assert!(decode_header(&encoded[..end], n, None).is_err());
            }
        }
        // A two-bit header corrects one error in EACH repetition column.
        // Check all 64 received words, including patterns beyond that radius:
        // majority is total and cannot detect a different legal selector.
        for word in 0..64u8 {
            let received: Vec<_> = (0..6).rev().map(|i| (word >> i) & 1).collect();
            let expected: Vec<_> = (0..2)
                .map(|i| {
                    u8::from(
                        [received[i], received[i + 2], received[i + 4]]
                            .iter()
                            .filter(|&&v| v == 1)
                            .count()
                            >= 2,
                    )
                })
                .collect();
            assert_eq!(decode_header(&received, 2, None).unwrap(), (expected, 6));
        }
        assert!(
            Config {
                sets: vec![],
                interleave: 0,
                stuffing: false,
                frames: 1,
                header: None
            }
            .decode_raw_access_unit(&[])
            .is_err()
        );
    }

    #[test]
    fn every_predefined_set_count_and_selector_has_bounded_dispatch() {
        let m: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ep-sets/manifest.json"
        ))
        .unwrap();
        let mut words: [Vec<Vec<u8>>; 9] = std::array::from_fn(|_| vec![]);
        words[0].push(vec![]);
        for row in m["header_vectors"].as_array().unwrap() {
            words[row["information"].as_str().unwrap().len()]
                .push(bits(row["encoded"].as_str().unwrap()));
        }
        let class = Class {
            length: Length::Fixed(16),
            rate: Some(0),
            crc: Some(0),
            concatenated: false,
            fec: 0,
            terminated: false,
            interleave: None,
            optional: false,
        };
        for count in 1..=255usize {
            let config = Config {
                sets: vec![
                    Set {
                        classes: vec![class.clone()],
                        order: None
                    };
                    count
                ],
                interleave: 0,
                stuffing: true,
                frames: 1,
                header: None,
            };
            let n = (usize::BITS - (count - 1).leading_zeros()) as usize;
            for (selected, prefix) in words[n].iter().enumerate() {
                let stuffing = (8 - (prefix.len() + 6 + 16) % 8) % 8;
                let mut packet = prefix.clone();
                packet.extend(&words[3][stuffing]);
                packet.extend(bits("0110110110100111"));
                packet.resize(packet.len() + stuffing, 0);
                let packed: Vec<u8> = packet
                    .chunks_exact(8)
                    .map(|row| row.iter().fold(0, |v, &b| (v << 1) | b))
                    .collect();
                let result = config.decode_raw_access_unit(&packed);
                if selected < count {
                    assert_eq!(
                        result.unwrap(),
                        [0x6d, 0xa7],
                        "count {count} selector {selected}"
                    );
                } else {
                    assert!(
                        result
                            .unwrap_err()
                            .to_string()
                            .contains("EP predefined set index out of range")
                    );
                }
            }
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

fn decode_rs_class_bits(
    bits: &[u8],
    length: Option<usize>,
    capability: u8,
    width: u8,
) -> Result<Vec<u8>> {
    if bits.len() % 8 != 0 {
        return Err(invalid("EP RS coded class is not octet aligned"));
    }
    let received: Vec<_> = bits
        .chunks_exact(8)
        .map(|row| row.iter().fold(0, |v, &b| (v << 1) | b))
        .collect();
    let information = if let Some(n) = length {
        let total = n
            .checked_add(usize::from(width))
            .ok_or_else(|| invalid("EP RS class length overflow"))?;
        if total % 8 != 0 {
            return Err(invalid("EP RS information plus CRC is not octet aligned"));
        }
        total / 8
    } else {
        aac_ep_rs::information_len(received.len(), capability)?
    };
    let total = information
        .checked_mul(8)
        .ok_or_else(|| invalid("EP RS class length overflow"))?;
    if aac_ep_rs::encoded_len(information, capability)? != received.len()
        || total <= usize::from(width)
    {
        return Err(invalid("EP RS class/packet length mismatch"));
    }
    let recovered = aac_ep_rs::decode(&received, information, capability)?;
    let checked: Vec<_> = recovered
        .information
        .iter()
        .flat_map(|&b| (0..8).rev().map(move |i| (b >> i) & 1))
        .collect();
    if !aac_ep_crc::verify(&checked, width)? {
        return Err(invalid("EP class CRC mismatch"));
    }
    // Class information is exact bits. CRC removal can end inside a storage
    // byte; callers must not expose padding as information for the next class.
    Ok(checked[..total - usize::from(width)].to_vec())
}
