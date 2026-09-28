//! Grouped AAC spectral payload decoding. Special bands retain zero placeholders
//! for later noise/intensity reconstruction; they consume no spectral codewords.
use super::{aac_huffman, aac_ics::IcsInfo, aac_synthesis::WindowSequence, bits::BitReader};
use crate::{Result, invalid};

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
        || info.group_lengths.iter().any(|&n| n == 0)
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
                    if width % tuple != 0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{aac_huffman_tables::*, aac_synthesis::WindowShape};
    fn info(sequence: WindowSequence, groups: Vec<u8>, bands: u8) -> IcsInfo {
        IcsInfo {
            sequence,
            shape: WindowShape::Sine,
            max_sfb: bands,
            group_lengths: groups,
        }
    }
    fn pack(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut data = Vec::new();
        let mut n = 0;
        for &(v, width) in fields {
            for b in (0..width).rev() {
                if n % 8 == 0 {
                    data.push(0);
                }
                *data.last_mut().unwrap() |= ((v >> b & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        (data, n)
    }
    #[test]
    fn mixed_books_preserve_group_band_order_and_skip_special_bands() {
        let info = info(WindowSequence::EightShort, vec![3, 5], 2);
        let quad = (SPECTRUM_CODEBOOK1_CODES[80], SPECTRUM_CODEBOOK1_LENS[80]);
        let pair = (SPECTRUM_CODEBOOK5_CODES[0], SPECTRUM_CODEBOOK5_LENS[0]);
        let fields = [vec![quad; 3], vec![pair; 10]].concat();
        let (data, count) = pack(&fields);
        let mut bits = BitReader::new(&data);
        let output = read(
            &mut bits,
            &info,
            &[0, 4, 8, 128],
            &[vec![1, 13], vec![5, 15]],
            1024,
        )
        .unwrap();
        assert_eq!(
            output,
            [vec![1; 12], vec![0; 12], vec![-4; 20], vec![0; 20]].concat()
        );
        assert_eq!(bits.position(), count);
        let mut windows = vec![0.0; 1024];
        info.deinterleave(
            &[0, 4, 8, 128],
            &output.iter().map(|&v| v as f32).collect::<Vec<_>>(),
            &mut windows,
        )
        .unwrap();
        for w in 0..8 {
            assert_eq!(
                &windows[w * 128..w * 128 + 4],
                if w < 3 { &[1.0; 4] } else { &[-4.0; 4] }
            );
        }
    }
    #[test]
    fn truncation_and_tuple_misalignment_do_not_consume_input() {
        let info = info(WindowSequence::OnlyLong, vec![1], 1);
        for (data, offsets, books) in [
            (vec![], vec![0, 4, 1024], vec![1]),
            (vec![0; 8], vec![0, 3, 1024], vec![1]),
            (vec![0; 8], vec![0, 4, 1024], vec![12]),
        ] {
            let mut bits = BitReader::new(&data);
            assert!(read(&mut bits, &info, &offsets, &[books], 1024).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
    #[test]
    fn zero_bands_need_no_payload() {
        for n in [960, 1024] {
            let info = info(WindowSequence::OnlyLong, vec![1], 1);
            let mut bits = BitReader::new(&[]);
            assert_eq!(
                read(&mut bits, &info, &[0, n], &[vec![0]], n).unwrap(),
                vec![0; n]
            );
            assert_eq!(bits.position(), 0);
        }
    }
}
