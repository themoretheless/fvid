//! AAC band scalefactor syntax. Values retain their distinct spectral,
//! noise-energy and intensity-position units until reconstruction.
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_aac/aac_scalefactors_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::aac_huffman_tables::{SCF_CODEBOOK_CODES, SCF_CODEBOOK_LENS};
    fn delta(value: i16) -> (u32, u8) {
        let i = (value + 60) as usize;
        (SCF_CODEBOOK_CODES[i], SCF_CODEBOOK_LENS[i])
    }
    fn pack(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut out = Vec::new();
        let mut n = 0;
        for &(v, width) in fields {
            for b in (0..width).rev() {
                if n % 8 == 0 {
                    out.push(0);
                }
                *out.last_mut().unwrap() |= ((v >> b & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        (out, n)
    }
    #[test]
    fn accumulators_remain_separate_and_cross_groups() {
        let (data, count) = pack(&[delta(3), (256, 9), delta(-4), delta(2), delta(5), delta(1)]);
        let mut bits = BitReader::new(&data);
        let scales = read(&mut bits, 100, &[vec![0, 1, 13, 14], vec![2, 13, 15, 0]]).unwrap();
        assert_eq!(
            scales,
            vec![
                vec![
                    BandScale::Zero,
                    BandScale::Spectral(103),
                    BandScale::Noise(10),
                    BandScale::Intensity(-4)
                ],
                vec![
                    BandScale::Spectral(105),
                    BandScale::Noise(15),
                    BandScale::Intensity(-3),
                    BandScale::Zero
                ]
            ]
        );
        assert_eq!(bits.position(), count);
    }
    #[test]
    fn first_noise_is_absolute_even_after_group_boundary() {
        let (data, count) = pack(&[(255, 9)]);
        let mut bits = BitReader::new(&data);
        assert_eq!(
            read(&mut bits, 90, &[vec![0], vec![13]]).unwrap(),
            vec![vec![BandScale::Zero], vec![BandScale::Noise(-1)]]
        );
        assert_eq!(bits.position(), count);
    }
    #[test]
    fn out_of_range_truncated_and_reserved_inputs_are_transactional() {
        for (gain, books, fields) in [
            (255, vec![1], vec![delta(1)]),
            (0, vec![1], vec![delta(-1)]),
            (100, vec![14, 14], vec![delta(60), delta(60)]),
            (0, vec![13], vec![(0, 9)]),
            (100, vec![12], vec![]),
            (100, vec![13], vec![(0, 1)]),
        ] {
            let (data, _) = pack(&fields);
            let mut bits = BitReader::new(&data);
            assert!(read(&mut bits, gain, &[books]).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
}
