//! MPEG-4 EP shortened RS over GF(256), ISO 14496-3 §1.8.4.7.
//! First transmitted octet is the LOWEST polynomial coefficient. All parts'
//! parity trailers follow the complete information class (Figure 1.11).
use super::{Result, invalid};
use std::sync::OnceLock;
struct Field {
    exp: [u8; 510],
    log: [u8; 256],
}
fn field() -> &'static Field {
    static F: OnceLock<Field> = OnceLock::new();
    F.get_or_init(|| {
        let mut exp = [0; 510];
        let mut log = [0; 256];
        let mut x = 1u16;
        for i in 0..255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 256 != 0 {
                x ^= 0x11d;
            }
        }
        for i in 255..510 {
            exp[i] = exp[i - 255];
        }
        Field { exp, log }
    })
}
fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        let f = field();
        f.exp[usize::from(f.log[a as usize]) + usize::from(f.log[b as usize])]
    }
}
fn div(a: u8, b: u8) -> Result<u8> {
    if b == 0 {
        return Err(invalid("singular EP RS locator"));
    }
    if a == 0 {
        return Ok(0);
    }
    let f = field();
    Ok(f.exp[(usize::from(f.log[a as usize]) + 255 - usize::from(f.log[b as usize])) % 255])
}
fn alpha(power: usize) -> u8 {
    field().exp[power % 255]
}
fn geometry(errors: u8) -> Result<(usize, usize)> {
    if errors > 127 {
        return Err(invalid("reserved EP RS correction capability"));
    }
    let r = 2 * usize::from(errors);
    Ok((255 - r, r))
}
fn generator(errors: u8) -> Result<&'static [u8]> {
    geometry(errors)?;
    static G: [OnceLock<Vec<u8>>; 128] = [const { OnceLock::new() }; 128];
    Ok(G[usize::from(errors)].get_or_init(|| {
        let mut g = vec![1];
        for root in 1..=2 * usize::from(errors) {
            let mut next = vec![0; g.len() + 1];
            for (i, &v) in g.iter().enumerate() {
                next[i] ^= v;
                next[i + 1] ^= mul(v, alpha(root));
            }
            g = next;
        }
        g
    }))
}
/// Information size excludes parity. No RS codeword is emitted for zero bits.
pub fn encoded_len(information_bytes: usize, errors: u8) -> Result<usize> {
    let (capacity, r) = geometry(errors)?;
    information_bytes
        .checked_add(
            information_bytes
                .div_ceil(capacity)
                .checked_mul(r)
                .ok_or_else(|| invalid("EP RS length overflow"))?,
        )
        .ok_or_else(|| invalid("EP RS length overflow"))
}
pub fn encode(information: &[u8], errors: u8) -> Result<Vec<u8>> {
    let (capacity, r) = geometry(errors)?;
    let g = generator(errors)?;
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(encoded_len(information.len(), errors)?)
        .map_err(|_| invalid("EP RS output allocation failed"))?;
    encoded.extend_from_slice(information);
    for part in information.chunks(capacity) {
        let mut work: Vec<_> = part
            .iter()
            .rev()
            .copied()
            .chain(std::iter::repeat_n(0, r))
            .collect();
        for i in 0..part.len() {
            let factor = work[i];
            if factor != 0 {
                for j in 1..g.len() {
                    work[i + j] ^= mul(factor, g[j]);
                }
            }
        }
        encoded.extend(work[part.len()..].iter().rev());
    }
    Ok(encoded)
}
fn eval_high(poly: &[u8], x: u8) -> u8 {
    poly.iter().fold(0, |s, &v| mul(s, x) ^ v)
}
fn eval_low(poly: &[u8], x: u8) -> u8 {
    poly.iter().rev().fold(0, |s, &v| mul(s, x) ^ v)
}
fn syndromes(word: &[u8], r: usize) -> Vec<u8> {
    (1..=r).map(|i| eval_high(word, alpha(i))).collect()
}
fn recover(word: &mut [u8], errors: u8) -> Result<usize> {
    let r = 2 * usize::from(errors);
    let s = syndromes(word, r);
    if s.iter().all(|&v| v == 0) {
        return Ok(0);
    }
    // Berlekamp–Massey, ascending locator coefficients.
    let mut locator = vec![0; r + 1];
    locator[0] = 1;
    let mut previous = locator.clone();
    let mut degree = 0;
    let mut shift = 1;
    let mut last = 1;
    for n in 0..r {
        let mut delta = s[n];
        for i in 1..=degree {
            delta ^= mul(locator[i], s[n - i]);
        }
        if delta == 0 {
            shift += 1;
            continue;
        }
        let saved = locator.clone();
        let factor = div(delta, last)?;
        for i in 0..=r - shift {
            locator[i + shift] ^= mul(factor, previous[i]);
        }
        if 2 * degree <= n {
            degree = n + 1 - degree;
            previous = saved;
            last = delta;
            shift = 1;
        } else {
            shift += 1;
        }
    }
    if degree == 0 || degree > usize::from(errors) {
        return Err(invalid("uncorrectable EP RS locator degree"));
    }
    locator.truncate(degree + 1);
    let mut positions = vec![];
    for position in 0..word.len() {
        let inverse = alpha(255 - (word.len() - 1 - position));
        if eval_low(&locator, inverse) == 0 {
            positions.push((position, inverse));
        }
    }
    if positions.len() != degree {
        return Err(invalid("uncorrectable EP RS locator roots"));
    }
    let mut omega = vec![0; r];
    for (i, &a) in s.iter().enumerate() {
        for (j, &b) in locator.iter().enumerate().take(r - i) {
            omega[i + j] ^= mul(a, b);
        }
    }
    let derivative: Vec<_> = locator
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, &v)| if i % 2 == 1 { v } else { 0 })
        .collect();
    for (position, inverse) in positions {
        let magnitude = div(eval_low(&omega, inverse), eval_low(&derivative, inverse))?;
        word[position] ^= magnitude;
    }
    if syndromes(word, r).iter().any(|&v| v != 0) {
        return Err(invalid("uncorrectable EP RS residual syndrome"));
    }
    Ok(degree)
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub information: Vec<u8>,
    pub corrected_symbols: usize,
}
/// Corrects at most k octet errors PER part. Beyond that capability a different
/// legal codeword may be selected; the enclosing EP class CRC remains necessary.
pub fn decode(received: &[u8], information_bytes: usize, errors: u8) -> Result<Decoded> {
    let (capacity, r) = geometry(errors)?;
    if received.len() != encoded_len(information_bytes, errors)? {
        return Err(invalid("EP RS encoded class length mismatch"));
    }
    let mut information = Vec::new();
    information
        .try_reserve_exact(information_bytes)
        .map_err(|_| invalid("EP RS information allocation failed"))?;
    let mut parity = information_bytes;
    let mut corrected = 0;
    for part in received[..information_bytes].chunks(capacity) {
        let mut word: Vec<_> = part
            .iter()
            .rev()
            .copied()
            .chain(received[parity..parity + r].iter().rev().copied())
            .collect();
        corrected += recover(&mut word, errors)?;
        information.extend(word[..part.len()].iter().rev());
        parity += r;
    }
    Ok(Decoded {
        information,
        corrected_symbols: corrected,
    })
}
/// Inverse of encoded_len; rejects lengths falling in an impossible final
/// parity-only fragment. Class parity is a trailer, not interleaved parts.
pub fn information_len(encoded_bytes: usize, errors: u8) -> Result<usize> {
    let (capacity, r) = geometry(errors)?;
    let remainder = encoded_bytes % 255;
    if remainder != 0 && remainder <= r {
        return Err(invalid("EP RS incomplete final part"));
    }
    Ok((encoded_bytes / 255) * capacity + if remainder == 0 { 0 } else { remainder - r })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn hex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    #[test]
    fn field_products_match_independent_carryless_polynomial_oracle() {
        let oracle =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-ep-rs/gf-products.bin");
        for a in 0..256 {
            for b in 0..256 {
                assert_eq!(mul(a as u8, b as u8), oracle[a * 256 + b]);
            }
        }
        assert_eq!(encode(&[1], 1).unwrap(), vec![1, 8, 6]);
        assert_eq!(encode(&[1, 2], 1).unwrap(), vec![1, 2, 104, 62]);
    }
    #[test]
    fn all_capabilities_shortening_and_part_boundaries_match_independent_oracles() {
        let m: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ep-rs/manifest.json"
        ))
        .unwrap();
        let mut seen = [false; 128];
        for row in m["vectors"].as_array().unwrap() {
            let k = row["k"].as_u64().unwrap() as u8;
            seen[k as usize] = true;
            let data = hex(row["data"].as_str().unwrap());
            let encoded = hex(row["encoded"].as_str().unwrap());
            let damaged = hex(row["damaged"].as_str().unwrap());
            assert_eq!(encode(&data, k).unwrap(), encoded, "k {k} n {}", data.len());
            assert_eq!(encoded_len(data.len(), k).unwrap(), encoded.len());
            assert_eq!(information_len(encoded.len(), k).unwrap(), data.len());
            assert_eq!(
                decode(&encoded, data.len(), k).unwrap(),
                Decoded {
                    information: data.clone(),
                    corrected_symbols: 0
                }
            );
            assert_eq!(
                decode(&damaged, data.len(), k).unwrap(),
                Decoded {
                    information: data,
                    corrected_symbols: row["corrected"].as_u64().unwrap() as usize
                },
                "k {k}"
            );
        }
        assert!(seen.into_iter().all(|x| x));
    }
    #[test]
    fn every_single_symbol_error_and_malformed_lengths_are_checked() {
        let data: Vec<_> = (0..700).map(|i| (i * 91 + 7) as u8).collect();
        let encoded = encode(&data, 2).unwrap();
        for at in 0..encoded.len() {
            let mut damaged = encoded.clone();
            damaged[at] ^= 0x5b;
            let recovered = decode(&damaged, data.len(), 2).unwrap();
            assert_eq!(recovered.information, data);
            assert_eq!(recovered.corrected_symbols, 1);
        }
        assert!(decode(&encoded[..encoded.len() - 1], data.len(), 2).is_err());
        assert!(information_len(4, 2).is_err());
        assert!(encoded_len(usize::MAX, 127).is_err());
        assert!(encode(&data, 128).is_err());
    }
}
