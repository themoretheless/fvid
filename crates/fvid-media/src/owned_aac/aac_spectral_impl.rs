use super::{aac_huffman, aac_ics::IcsInfo, aac_synthesis::WindowSequence, bits::BitReader};

/// Read coefficients in group/band/window order. Bounds and group geometry are
/// validated before decoding; failure leaves the input bit cursor unchanged.
pub fn read(
    bits: &mut BitReader<'_>,
    info: &IcsInfo,
    offsets: &[usize],
    books: &[Vec<u8>],
    frame_samples: usize,
) -> Result<Vec<i16>> {
    let windows = if info.sequence == WindowSequence::EightShort {
        8
    } else {
        1
    };
    if !matches!(frame_samples, 960 | 1024)
        || info.group_lengths.is_empty()
        || info.group_lengths.len() > windows
        || info.group_lengths.contains(&0)
        || info
            .group_lengths
            .iter()
            .map(|&n| n as usize)
            .sum::<usize>()
            != windows
    {
        return Err(invalid("invalid AAC spectral window geometry"));
    }
    if offsets.first() != Some(&0)
        || offsets.last() != Some(&(frame_samples / windows))
        || offsets.windows(2).any(|p| p[0] >= p[1])
        || info.max_sfb as usize >= offsets.len()
        || books.len() != info.group_lengths.len()
        || books.iter().any(|g| g.len() != info.max_sfb as usize)
    {
        return Err(invalid("invalid AAC spectral band layout"));
    }
    let mut cursor = bits.clone();
    let mut result = Vec::with_capacity(offsets[info.max_sfb as usize] * windows);
    for (group, &count) in books.iter().zip(&info.group_lengths) {
        for (band, &book) in group.iter().enumerate() {
            let width = offsets[band + 1] - offsets[band];
            let count = width * count as usize;
            match book {
                0 | 13..=15 => result.resize(result.len() + count, 0),
                1..=11 => {
                    let tuple = if book <= 4 { 4 } else { 2 };
                    if !width.is_multiple_of(tuple) {
                        return Err(invalid("AAC band splits spectral tuple"));
                    }
                    for _ in 0..count / tuple {
                        let (values, n) = aac_huffman::spectral(&mut cursor, book)?;
                        result.extend_from_slice(&values[..n]);
                    }
                }
                _ => return Err(invalid("reserved AAC spectral codebook")),
            }
        }
    }
    *bits = cursor;
    Ok(result)
}
