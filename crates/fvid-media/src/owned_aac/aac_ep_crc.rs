//! MPEG-4 EP inverted CRC parity (§1.8.4.5), distinct from AAC ADTS/SBR CRC.
use super::{Result, invalid};
fn polynomial(width: u8) -> Result<u32> {
    Ok(match width {
        0 => 0,
        1 => 0x1,
        2 => 0x3,
        3 => 0x3,
        4 => 0xd,
        5 => 0x15,
        6 => 0x2f,
        7 => 0x45,
        8 => 0x07,
        9 => 0x127,
        10 => 0x233,
        11 => 0x41b,
        12 => 0x80f,
        13 => 0x10f5,
        14 => 0x202d,
        15 => 0x4cc5,
        16 => 0x1021,
        24 => 0x800063,
        32 => 0x04c11db7,
        _ => return Err(invalid("reserved MPEG-4 EP CRC width")),
    })
}
fn valid(bits: &[u8]) -> Result<()> {
    if bits.iter().any(|&b| b > 1) {
        return Err(invalid("EP CRC input contains a non-bit"));
    }
    Ok(())
}
/// Inverted remainder of M(x)*x^k, as transmitted, highest order first.
pub fn parity(information: &[u8], width: u8) -> Result<Vec<u8>> {
    valid(information)?;
    let poly = u64::from(polynomial(width)?);
    if width == 0 {
        return Ok(vec![]);
    }
    let mask = (1u64 << width) - 1;
    let mut register = 0u64;
    for &bit in information {
        let feedback = ((register >> (width - 1)) as u8) ^ bit;
        register = (register << 1) & mask;
        if feedback != 0 {
            register ^= poly;
        }
    }
    let transmitted = register ^ mask;
    Ok((0..width)
        .rev()
        .map(|i| ((transmitted >> i) & 1) as u8)
        .collect())
}
pub fn append(information: &[u8], width: u8) -> Result<Vec<u8>> {
    let crc = parity(information, width)?;
    let capacity = information
        .len()
        .checked_add(crc.len())
        .ok_or_else(|| invalid("EP CRC length overflow"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(capacity)
        .map_err(|_| invalid("EP CRC output allocation failed"))?;
    out.extend_from_slice(information);
    out.extend_from_slice(&crc);
    Ok(out)
}
/// False reports a class integrity failure, not an instruction to conceal it.
pub fn verify(class: &[u8], width: u8) -> Result<bool> {
    polynomial(width)?;
    valid(class)?;
    if class.len() < usize::from(width) {
        return Err(invalid("truncated EP CRC parity"));
    }
    let split = class.len() - usize::from(width);
    Ok(parity(&class[..split], width)? == class[split..])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inverted_parity_matches_independent_polynomial_long_division() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ep-srcpc/manifest.json"
        ))
        .unwrap();
        let rows = manifest["crc"].as_array().unwrap();
        assert_eq!(rows.len(), 114);
        for row in rows {
            let bits = |name: &str| {
                row[name]
                    .as_str()
                    .unwrap()
                    .bytes()
                    .map(|b| b - b'0')
                    .collect::<Vec<_>>()
            };
            let input = bits("message");
            let width = row["width"].as_u64().unwrap() as u8;
            let expected = bits("parity");
            assert_eq!(parity(&input, width).unwrap(), expected);
            let encoded = append(&input, width).unwrap();
            assert!(verify(&encoded, width).unwrap());
            if width != 0 {
                for at in 0..encoded.len() {
                    let mut damaged = encoded.clone();
                    damaged[at] ^= 1;
                    assert!(!verify(&damaged, width).unwrap(), "width {width} bit {at}");
                }
            }
        }
        assert!(parity(&[2], 16).is_err());
        assert!(parity(&[1], 17).is_err());
        assert!(verify(&[1; 7], 8).is_err());
        assert!(verify(&[2; 8], 8).is_err());
    }
}
