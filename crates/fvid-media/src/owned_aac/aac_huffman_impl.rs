use super::{aac_huffman_tables::*, bits::BitReader};

fn table(book: u8) -> Result<(&'static [u32], &'static [u8])> {
    Ok(match book {
        1 => (&SPECTRUM_CODEBOOK1_CODES, &SPECTRUM_CODEBOOK1_LENS),
        2 => (&SPECTRUM_CODEBOOK2_CODES, &SPECTRUM_CODEBOOK2_LENS),
        3 => (&SPECTRUM_CODEBOOK3_CODES, &SPECTRUM_CODEBOOK3_LENS),
        4 => (&SPECTRUM_CODEBOOK4_CODES, &SPECTRUM_CODEBOOK4_LENS),
        5 => (&SPECTRUM_CODEBOOK5_CODES, &SPECTRUM_CODEBOOK5_LENS),
        6 => (&SPECTRUM_CODEBOOK6_CODES, &SPECTRUM_CODEBOOK6_LENS),
        7 => (&SPECTRUM_CODEBOOK7_CODES, &SPECTRUM_CODEBOOK7_LENS),
        8 => (&SPECTRUM_CODEBOOK8_CODES, &SPECTRUM_CODEBOOK8_LENS),
        9 => (&SPECTRUM_CODEBOOK9_CODES, &SPECTRUM_CODEBOOK9_LENS),
        10 => (&SPECTRUM_CODEBOOK10_CODES, &SPECTRUM_CODEBOOK10_LENS),
        11 => (&SPECTRUM_CODEBOOK11_CODES, &SPECTRUM_CODEBOOK11_LENS),
        _ => return Err(invalid("AAC codebook is not spectral")),
    })
}
fn symbol(bits: &mut BitReader<'_>, codes: &[u32], lengths: &[u8]) -> Result<usize> {
    symbol_prefix(bits, codes, lengths)?.ok_or_else(|| invalid("truncated or oversized bit field"))
}
fn symbol_prefix(bits: &mut BitReader<'_>, codes: &[u32], lengths: &[u8]) -> Result<Option<usize>> {
    let mut code = 0;
    for length in 1..=19 {
        if bits.remaining() == 0 {
            return Ok(None);
        }
        code = (code << 1) | bits.read(1)?;
        if let Some(index) = codes
            .iter()
            .zip(lengths)
            .position(|(&c, &n)| n == length && c == code)
        {
            return Ok(Some(index));
        }
    }
    Err(invalid("invalid AAC Huffman codeword"))
}
/// Signed differential scalefactor (-60..=60). Failure preserves cursor.
pub fn scalefactor(bits: &mut BitReader<'_>) -> Result<i16> {
    let mut cursor = bits.clone();
    let value = symbol(&mut cursor, &SCF_CODEBOOK_CODES, &SCF_CODEBOOK_LENS)? as i16 - 60;
    *bits = cursor;
    Ok(value)
}
/// Decode a pair or quad. Unused quad slots in a pair are zero. Signs for
/// unsigned books precede escape magnitudes. Failure preserves the cursor.
pub fn spectral(bits: &mut BitReader<'_>, book: u8) -> Result<([i16; 4], usize)> {
    spectral_prefix(bits, book)?.ok_or_else(|| invalid("truncated or oversized bit field"))
}
/// None means a valid unfinished prefix; both None and errors preserve input.
pub(crate) fn spectral_prefix(
    bits: &mut BitReader<'_>,
    book: u8,
) -> Result<Option<([i16; 4], usize)>> {
    let (codes, lengths) = table(book)?;
    let mut cursor = bits.clone();
    let mut index = match symbol_prefix(&mut cursor, codes, lengths)? {
        Some(i) => i,
        None => return Ok(None),
    };
    let (width, radix, bias): (usize, usize, i16) = match book {
        1 | 2 => (4, 3, 1),
        3 | 4 => (4, 3, 0),
        5 | 6 => (2, 9, 4),
        7 | 8 => (2, 8, 0),
        9 | 10 => (2, 13, 0),
        _ => (2, 17, 0),
    };
    let mut values = [0i16; 4];
    for i in (0..width).rev() {
        values[i] = (index % radix) as i16 - bias;
        index /= radix;
    }
    if bias == 0 {
        for value in &mut values[..width] {
            if *value != 0 {
                if cursor.remaining() == 0 {
                    return Ok(None);
                }
                if cursor.bit()? {
                    *value = -*value;
                }
            }
        }
    }
    if book == 11 {
        for value in &mut values[..width] {
            if value.abs() == 16 {
                let mut width = 4;
                loop {
                    if cursor.remaining() == 0 {
                        return Ok(None);
                    }
                    if !cursor.bit()? {
                        break;
                    }
                    width += 1;
                    if width > 12 {
                        return Err(invalid("AAC escape magnitude exceeds 8191"));
                    }
                }
                if cursor.remaining() < usize::from(width) {
                    return Ok(None);
                }
                let magnitude = (1i16 << width) + cursor.read(width)? as i16;
                *value = value.signum() * magnitude;
            }
        }
    }
    *bits = cursor;
    Ok(Some((values, width)))
}
