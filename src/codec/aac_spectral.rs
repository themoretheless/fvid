//! Grouped AAC spectral payload decoding. Special bands retain zero placeholders
//! for later noise/intensity reconstruction; they consume no spectral codewords.
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_aac/aac_spectral_impl.rs");

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
