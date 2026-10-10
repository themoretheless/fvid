// Owned RVLC syntax. Tables are factual ISO/IEC 14496-3:2001 tables4.113-115.
const RVLC_WORDS: [(u8, u32); 15] = [
    (7, 65),
    (9, 257),
    (8, 129),
    (6, 33),
    (5, 17),
    (4, 9),
    (3, 5),
    (1, 0),
    (3, 7),
    (5, 27),
    (6, 51),
    (7, 107),
    (8, 195),
    (9, 427),
    (7, 99),
];
const RVLC_FORBIDDEN: [(u8, u32); 8] = [
    (6, 50),
    (7, 96),
    (9, 256),
    (8, 194),
    (7, 98),
    (6, 52),
    (9, 426),
    (8, 212),
];
const RVLC_ESCAPES: [(u8, u32); 27] = [
    (2, 2),
    (2, 0),
    (3, 6),
    (3, 2),
    (4, 14),
    (5, 31),
    (5, 15),
    (5, 13),
    (6, 61),
    (6, 29),
    (6, 25),
    (6, 24),
    (7, 120),
    (7, 56),
    (8, 242),
    (8, 114),
    (9, 486),
    (9, 230),
    (10, 974),
    (10, 463),
    (11, 1950),
    (11, 1951),
    (11, 925),
    (12, 1848),
    (14, 7399),
    (13, 3698),
    (15, 14797),
];
fn rvlc_layout(books: &[Vec<u8>]) -> Result<()> {
    if books.is_empty()
        || books.len() > 8
        || books
            .iter()
            .any(|g| g.len() > 63 || g.len() != books[0].len())
    {
        return Err(invalid("invalid AAC scalefactor group layout"));
    }
    if books.iter().flatten().any(|&b| b == 12 || b > 31) {
        return Err(invalid("invalid AAC scalefactor codebook"));
    }
    Ok(())
}
fn rvlc_base(code: u32, width: u8) -> Result<Option<i16>> {
    if RVLC_FORBIDDEN.contains(&(width, code)) {
        return Err(invalid("invalid AAC RVLC codeword"));
    }
    Ok(RVLC_WORDS
        .iter()
        .position(|&p| p == (width, code))
        .map(|i| i as i16 - 7))
}
fn rvlc_word(bits: &mut BitReader<'_>, end: usize, escape: bool) -> Result<i16> {
    let mut code = 0;
    for width in 1..=if escape { 20 } else { 9 } {
        if bits.position() >= end {
            return Err(invalid(if escape {
                "truncated AAC RVLC escape"
            } else {
                "truncated AAC RVLC codeword"
            }));
        }
        code = (code << 1) | bits.read(1)?;
        if escape {
            if let Some(i) = RVLC_ESCAPES.iter().position(|&p| p == (width, code)) {
                return Ok(i as i16);
            }
            if width == 20 && (473482..=473503).contains(&code) {
                return Ok((code - 473455) as i16);
            }
            if width == 19 && (236736..=236740).contains(&code) {
                return Ok((code - 236687) as i16);
            }
        } else if let Some(v) = rvlc_base(code, width)? {
            return Ok(v);
        }
    }
    Err(invalid(if escape {
        "invalid AAC RVLC escape codeword"
    } else {
        "invalid AAC RVLC codeword"
    }))
}
fn rvlc_delta(
    sf: &mut BitReader<'_>,
    sf_end: usize,
    esc: &mut BitReader<'_>,
    esc_end: usize,
    bases: &mut Vec<i16>,
) -> Result<i16> {
    let base = rvlc_word(sf, sf_end, false)?;
    bases.push(base);
    Ok(if base.abs() == 7 {
        base + base.signum() * rvlc_word(esc, esc_end, true)?
    } else {
        base
    })
}
fn rvlc_reverse(origin: &BitReader<'_>, start: usize, end: &mut usize) -> Result<i16> {
    let mut code = 0;
    for width in 1..=9 {
        if *end <= start {
            return Err(invalid("truncated AAC reverse RVLC codeword"));
        }
        *end -= 1;
        let mut bit = origin.clone();
        bit.skip(*end - start)?;
        code = (code << 1) | bit.read(1)?;
        if let Some(value) = rvlc_base(code, width)? {
            return Ok(value);
        }
    }
    Err(invalid("invalid AAC reverse RVLC codeword"))
}
/// Class1 header; class2 codewords follow pulse/TNS/gain presence flags and
/// precede deferred ER TNS payload. PNS's first nine bits count toward sf_bits.
#[derive(Clone, Debug)]
pub struct RvlcHeader {
    reverse_gain: u8,
    sf_bits: usize,
    escape_bits: usize,
    first_noise: Option<u16>,
    last_noise: Option<u16>,
}
impl RvlcHeader {
    pub fn read(bits: &mut BitReader<'_>, short: bool, books: &[Vec<u8>]) -> Result<Self> {
        rvlc_layout(books)?;
        let mut cursor = bits.clone();
        let _concealment = cursor.bit()?;
        let reverse_gain = cursor.read(8)? as u8;
        let mut sf_bits = cursor.read(if short { 11 } else { 9 })? as usize;
        let noise = books.iter().flatten().any(|&b| b == 13);
        let first_noise = if noise {
            if sf_bits < 9 {
                return Err(invalid("AAC RVLC noise length is below nine bits"));
            }
            sf_bits -= 9;
            Some(cursor.read(9)? as u16)
        } else {
            None
        };
        let escape_bits = if cursor.bit()? {
            cursor.read(8)? as usize
        } else {
            0
        };
        let last_noise = if noise {
            Some(cursor.read(9)? as u16)
        } else {
            None
        };
        *bits = cursor;
        Ok(Self {
            reverse_gain,
            sf_bits,
            escape_bits,
            first_noise,
            last_noise,
        })
    }
    /// Decode both directions within the declared region, fold ordered escapes,
    /// and verify backward seeds. Corrupt data rejects without consuming input.
    pub fn decode(
        &self,
        bits: &mut BitReader<'_>,
        gain: u8,
        books: &[Vec<u8>],
    ) -> Result<Vec<Vec<BandScale>>> {
        rvlc_layout(books)?;
        if books.iter().flatten().any(|&b| b == 13) != self.first_noise.is_some() {
            return Err(invalid("AAC RVLC noise header disagrees with sections"));
        }
        if self.sf_bits + self.escape_bits > bits.remaining() {
            return Err(invalid("truncated AAC RVLC payload"));
        }
        let origin = bits.clone();
        let start = bits.position();
        let sf_end = start + self.sf_bits;
        let esc_end = sf_end + self.escape_bits;
        let mut sf = bits.clone();
        let mut esc = bits.clone();
        esc.skip(self.sf_bits)?;
        let mut spectral = i16::from(gain);
        let mut noise = i16::from(gain) - 90;
        let mut intensity = 0i16;
        let mut first_noise = true;
        let mut spectral_used = false;
        let mut intensity_used = false;
        let mut bases = Vec::new();
        let mut result = Vec::with_capacity(books.len());
        for group in books {
            let mut scales = Vec::with_capacity(group.len());
            for &book in group {
                let scale = match book {
                    0 => BandScale::Zero,
                    1..=11 | 16..=31 => {
                        spectral_used = true;
                        spectral += rvlc_delta(&mut sf, sf_end, &mut esc, esc_end, &mut bases)?;
                        if !(0..=255).contains(&spectral) {
                            return Err(invalid("AAC spectral scalefactor out of range"));
                        }
                        BandScale::Spectral(spectral as u8)
                    }
                    13 => {
                        noise += if first_noise {
                            first_noise = false;
                            i16::try_from(self.first_noise.unwrap()).unwrap() - 256
                        } else {
                            rvlc_delta(&mut sf, sf_end, &mut esc, esc_end, &mut bases)?
                        };
                        if !(-100..=155).contains(&noise) {
                            return Err(invalid("AAC noise energy out of range"));
                        }
                        BandScale::Noise(noise)
                    }
                    14 | 15 => {
                        intensity_used = true;
                        intensity += rvlc_delta(&mut sf, sf_end, &mut esc, esc_end, &mut bases)?;
                        if !(-155..=100).contains(&intensity) {
                            return Err(invalid("AAC intensity position out of range"));
                        }
                        BandScale::Intensity(intensity)
                    }
                    _ => return Err(invalid("invalid AAC scalefactor codebook")),
                };
                scales.push(scale);
            }
            result.push(scales);
        }
        if intensity_used
            && rvlc_delta(&mut sf, sf_end, &mut esc, esc_end, &mut bases)? != intensity
        {
            return Err(invalid("AAC RVLC reverse intensity mismatch"));
        }
        if sf.position() != sf_end {
            return Err(invalid("AAC RVLC scalefactor length mismatch"));
        }
        if esc.position() != esc_end {
            return Err(invalid("AAC RVLC escape length mismatch"));
        }
        let mut reverse = sf_end;
        for expected in bases.iter().rev() {
            if rvlc_reverse(&origin, start, &mut reverse)? != *expected {
                return Err(invalid("AAC RVLC forward/reverse mismatch"));
            }
        }
        if reverse != start {
            return Err(invalid("AAC RVLC reverse length mismatch"));
        }
        if spectral_used && spectral != i16::from(self.reverse_gain) {
            return Err(invalid("AAC RVLC reverse gain mismatch"));
        }
        if let Some(last) = self.last_noise {
            if noise != i16::from(self.reverse_gain) - 90 - 256 + last as i16 {
                return Err(invalid("AAC RVLC reverse noise mismatch"));
            }
        }
        *bits = esc;
        Ok(result)
    }
}

#[cfg(test)]
mod rvlc_tests {
    use super::*;
    fn field(value: u32, width: u8) -> String {
        format!("{value:0width$b}", width = width as usize)
    }
    fn pack(bits: &str) -> Vec<u8> {
        let mut out = vec![0; bits.len().div_ceil(8)];
        for (i, b) in bits.bytes().enumerate() {
            out[i / 8] |= (b - b'0') << (7 - i % 8);
        }
        out
    }
    fn delta(value: i16, sf: &mut String, escapes: &mut String) {
        let base = value.clamp(-7, 7);
        let (width, word) = RVLC_WORDS[(base + 7) as usize];
        sf.push_str(&field(word, width));
        if value.abs() >= 7 {
            let e = (value.abs() - 7) as usize;
            let (width, word) = if e < 27 {
                RVLC_ESCAPES[e]
            } else if e < 49 {
                (20, 473455 + e as u32)
            } else {
                (19, 236687 + e as u32)
            };
            escapes.push_str(&field(word, width));
        }
    }
    #[test]
    fn rvlc_all_signed_deltas_and_escape_words_preserve_adjacent_bits() {
        for value in -60..=60 {
            for prefix in 0..8 {
                let mut sf = String::new();
                let mut esc = String::new();
                delta(value, &mut sf, &mut esc);
                let reverse = (100 + value) as u32;
                let header = format!(
                    "0{}{}{}{}",
                    field(reverse, 8),
                    field(sf.len() as u32, 9),
                    field(u32::from(!esc.is_empty()), 1),
                    if esc.is_empty() {
                        String::new()
                    } else {
                        field(esc.len() as u32, 8)
                    }
                );
                let wire = format!("{}{}{}{}10101010", "0".repeat(prefix), header, sf, esc);
                let bytes = pack(&wire);
                let mut bits = BitReader::new(&bytes);
                bits.skip(prefix).unwrap();
                let books = vec![vec![1]];
                let h = RvlcHeader::read(&mut bits, false, &books).unwrap();
                assert_eq!(
                    h.decode(&mut bits, 100, &books).unwrap(),
                    vec![vec![BandScale::Spectral(reverse as u8)]]
                );
                assert_eq!(bits.read(8).unwrap(), 0xaa);
            }
        }
    }
    #[test]
    fn rvlc_short_groups_keep_separate_spectral_noise_and_intensity_accumulators() {
        let mut sf = String::new();
        let mut esc = String::new();
        let books = vec![vec![16, 13, 14]; 8];
        let mut expected = vec![];
        for g in 0..8 {
            delta(if g % 2 == 0 { -7 } else { 7 }, &mut sf, &mut esc);
            if g != 0 {
                delta(1, &mut sf, &mut esc);
            }
            delta(1, &mut sf, &mut esc);
            expected.push(vec![
                BandScale::Spectral(if g % 2 == 0 { 93 } else { 100 }),
                BandScale::Noise(10 + g),
                BandScale::Intensity(1 + g),
            ]);
        }
        delta(8, &mut sf, &mut esc);
        let header = format!(
            "1{}{}{}1{}{}",
            field(100, 8),
            field(sf.len() as u32 + 9, 11),
            field(256, 9),
            field(esc.len() as u32, 8),
            field(263, 9)
        );
        let bytes = pack(&format!("{header}{sf}{esc}"));
        let mut bits = BitReader::new(&bytes);
        let h = RvlcHeader::read(&mut bits, true, &books).unwrap();
        assert_eq!(h.decode(&mut bits, 100, &books).unwrap(), expected);
        assert_eq!(bits.position(), header.len() + sf.len() + esc.len());
    }
    #[test]
    fn rvlc_forbidden_words_and_every_payload_truncation_preserve_cursor() {
        for (width, word) in RVLC_FORBIDDEN {
            let sf = field(word, width);
            let header = format!("0{}{}0", field(100, 8), field(sf.len() as u32, 9));
            let bytes = pack(&format!("{header}{sf}"));
            let mut bits = BitReader::new(&bytes);
            let h = RvlcHeader::read(&mut bits, false, &[vec![1]]).unwrap();
            let start = bits.position();
            let e = h.decode(&mut bits, 100, &[vec![1]]).unwrap_err();
            assert!(e.to_string().contains("invalid AAC RVLC codeword"));
            assert_eq!(bits.position(), start);
        }
        let mut sf = String::new();
        let mut esc = String::new();
        delta(-60, &mut sf, &mut esc);
        delta(60, &mut sf, &mut esc);
        let header = format!(
            "0{}{}1{}",
            field(100, 8),
            field(sf.len() as u32, 9),
            field(esc.len() as u32, 8)
        );
        let wire = format!("{header}{sf}{esc}");
        for keep in 0..wire.len() {
            let prefix = (8 - keep % 8) % 8;
            let bytes = pack(&format!("{}{}", "0".repeat(prefix), &wire[..keep]));
            let mut bits = BitReader::new(&bytes);
            bits.skip(prefix).unwrap();
            let start = bits.position();
            match RvlcHeader::read(&mut bits, false, &[vec![1, 1]]) {
                Err(_) => assert_eq!(bits.position(), start),
                Ok(h) => {
                    let at = bits.position();
                    assert!(h.decode(&mut bits, 100, &[vec![1, 1]]).is_err());
                    assert_eq!(bits.position(), at);
                }
            }
        }
    }
}
