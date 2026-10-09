use super::{aac_huffman, bits::BitReader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandScale {
    Zero,
    Spectral(u8),
    Noise(i16),
    Intensity(i16),
}

/// Read group-major scalefactors. Accumulators carry across group boundaries;
/// only the first noise band uses a nine-bit absolute delta. Failure preserves
/// the cursor, including failures after earlier bands decoded successfully.
pub fn read(bits: &mut BitReader<'_>, gain: u8, books: &[Vec<u8>]) -> Result<Vec<Vec<BandScale>>> {
    if books.is_empty()
        || books.len() > 8
        || books.iter().any(|g| g.len() > 63)
        || books.iter().any(|g| g.len() != books[0].len())
    {
        return Err(invalid("invalid AAC scalefactor group layout"));
    }
    let mut cursor = bits.clone();
    let mut spectral = i16::from(gain);
    let mut noise = i16::from(gain) - 90;
    let mut intensity = 0i16;
    let mut first_noise = true;
    let mut result = Vec::with_capacity(books.len());
    for group in books {
        let mut scales = Vec::with_capacity(group.len());
        for &book in group {
            let value = match book {
                0 => BandScale::Zero,
                1..=11 | 16..=31 => {
                    spectral += aac_huffman::scalefactor(&mut cursor)?;
                    if !(0..=255).contains(&spectral) {
                        return Err(invalid("AAC spectral scalefactor out of range"));
                    }
                    BandScale::Spectral(spectral as u8)
                }
                13 => {
                    noise += if first_noise {
                        first_noise = false;
                        cursor.read(9)? as i16 - 256
                    } else {
                        aac_huffman::scalefactor(&mut cursor)?
                    };
                    if !(-100..=155).contains(&noise) {
                        return Err(invalid("AAC noise energy out of range"));
                    }
                    BandScale::Noise(noise)
                }
                14 | 15 => {
                    intensity += aac_huffman::scalefactor(&mut cursor)?;
                    if !(-155..=100).contains(&intensity) {
                        return Err(invalid("AAC intensity position out of range"));
                    }
                    BandScale::Intensity(intensity)
                }
                _ => return Err(invalid("invalid AAC scalefactor codebook")),
            };
            scales.push(value);
        }
        result.push(scales);
    }
    *bits = cursor;
    Ok(result)
}
