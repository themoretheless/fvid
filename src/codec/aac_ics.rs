//! AAC-LC individual-channel window and section syntax.
//! Spectral Huffman decoding and sample-rate-specific band tables are separate.
use crate::{Result, invalid, unsupported};

include!("../../crates/fvid-media/src/owned_aac/aac_ics_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    fn packed(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut data = Vec::new();
        let mut count = 0;
        for &(value, width) in fields {
            for shift in (0..width).rev() {
                if count % 8 == 0 {
                    data.push(0);
                }
                *data.last_mut().unwrap() |= (((value >> shift) & 1) as u8) << (7 - count % 8);
                count += 1;
            }
        }
        (data, count)
    }
    #[test]
    fn every_short_grouping_partitions_eight_windows() {
        for mask in 0..128 {
            let (data, count) = packed(&[(0, 1), (2, 2), (1, 1), (12, 4), (mask, 7)]);
            let mut bits = BitReader::new(&data);
            let info = IcsInfo::read(&mut bits, (49, 14)).unwrap();
            assert_eq!(bits.position(), count);
            assert_eq!(info.shape, WindowShape::Kbd);
            assert_eq!(info.group_lengths.iter().sum::<u8>(), 8);
            assert_eq!(info.group_lengths.len(), 8 - mask.count_ones() as usize);
            let mut boundaries = 0;
            for &length in info.group_lengths.iter().take(info.group_lengths.len() - 1) {
                boundaries += length;
                assert_eq!(mask & (1 << (7 - boundaries)), 0);
            }
        }
    }
    #[test]
    fn deinterleave_restores_every_coefficient_for_all_groupings() {
        for n in [960, 1024] {
            let size = n / 8;
            let offsets = [0, 4, 12, 28, size];
            for mask in 0..128 {
                let (data, _) = packed(&[(0, 1), (2, 2), (0, 1), (3, 4), (mask, 7)]);
                let info = IcsInfo::read(&mut BitReader::new(&data), (49, 4)).unwrap();
                // Build the wire ordering by sorting independent coefficient
                // coordinates by (group, band, window, bin).
                let mut coordinates = Vec::new();
                let mut window_groups = Vec::new();
                for (group, &length) in info.group_lengths.iter().enumerate() {
                    window_groups.extend(std::iter::repeat_n(group, length as usize));
                }
                for (window, &group) in window_groups.iter().enumerate() {
                    for bin in 0..28 {
                        let band = offsets.iter().position(|&end| end > bin).unwrap() - 1;
                        coordinates.push((group, band, window, bin));
                    }
                }
                coordinates.sort();
                let grouped: Vec<_> = coordinates
                    .iter()
                    .map(|&(_, _, w, b)| (w * size + b + 1) as f32)
                    .collect();
                let mut output = vec![-1.0; n];
                info.deinterleave(&offsets, &grouped, &mut output).unwrap();
                for (i, &value) in output.iter().enumerate() {
                    assert_eq!(value, if i % size < 28 { (i + 1) as f32 } else { 0.0 });
                }
            }
        }
    }
    #[test]
    fn bad_spectral_layout_preserves_output_and_empty_bands_zero_fill() {
        let mut info = IcsInfo {
            sequence: WindowSequence::OnlyLong,
            shape: WindowShape::Sine,
            max_sfb: 1,
            group_lengths: vec![1],
            prediction: None,
        };
        let mut output = vec![17.0; 1024];
        for offsets in [&[1, 1024][..], &[0, 0, 1024], &[0, 1025], &[0]] {
            assert!(info.deinterleave(offsets, &[1.0; 4], &mut output).is_err());
            assert!(output.iter().all(|&v| v == 17.0));
        }
        assert!(
            info.deinterleave(&[0, 4, 1024], &[f32::NAN; 4], &mut output)
                .is_err()
        );
        info.group_lengths = vec![0, 1];
        assert!(
            info.deinterleave(&[0, 4, 1024], &[0.0; 4], &mut output)
                .is_err()
        );
        assert!(output.iter().all(|&v| v == 17.0));
        info.group_lengths = vec![1];
        info.deinterleave(&[0, 4, 1024], &[1.0, 2.0, 3.0, 4.0], &mut output)
            .unwrap();
        assert_eq!(&output[..4], &[1.0, 2.0, 3.0, 4.0]);
        assert!(output[4..].iter().all(|&v| v == 0.0));
        info.max_sfb = 0;
        info.deinterleave(&[0, 1024], &[], &mut output).unwrap();
        assert!(output.iter().all(|&v| v == 0.0));
    }
    #[test]
    fn long_syntax_and_errors_preserve_cursor() {
        for sequence in [0, 1, 3] {
            let (data, count) = packed(&[(0, 1), (sequence, 2), (0, 1), (49, 6), (0, 1)]);
            let mut bits = BitReader::new(&data);
            assert_eq!(
                IcsInfo::read(&mut bits, (49, 14)).unwrap().group_lengths,
                [1]
            );
            assert_eq!(bits.position(), count);
        }
        for fields in [
            vec![(1, 1)],
            vec![(0, 1), (0, 2), (0, 1), (50, 6), (0, 1)],
            vec![(0, 1), (0, 2), (0, 1), (40, 6), (1, 1)],
        ] {
            let (data, _) = packed(&fields);
            let mut bits = BitReader::new(&data);
            assert!(IcsInfo::read(&mut bits, (49, 14)).is_err());
            assert_eq!(bits.position(), 0);
        }
        let mut bits = BitReader::new(&[0]);
        assert!(IcsInfo::read(&mut bits, (49, 14)).is_err());
        assert_eq!(bits.position(), 0);
    }
    #[test]
    fn short_sections_keep_group_boundaries_and_special_codebooks() {
        let info = IcsInfo {
            sequence: WindowSequence::EightShort,
            shape: WindowShape::Sine,
            max_sfb: 8,
            group_lengths: vec![3, 5],
            prediction: None,
        };
        let (data, count) = packed(&[(13, 4), (7, 3), (1, 3), (14, 4), (4, 3), (15, 4), (4, 3)]);
        let mut bits = BitReader::new(&data);
        assert_eq!(
            info.read_sections(&mut bits).unwrap(),
            vec![vec![13; 8], vec![14, 14, 14, 14, 15, 15, 15, 15]]
        );
        assert_eq!(bits.position(), count);
    }
    #[test]
    fn sections_expand_escape_lengths_and_reject_invalid_runs() {
        let info = IcsInfo {
            sequence: WindowSequence::OnlyLong,
            shape: WindowShape::Sine,
            max_sfb: 40,
            group_lengths: vec![1],
            prediction: None,
        };
        let (data, count) = packed(&[(5, 4), (31, 5), (4, 5), (0, 4), (5, 5)]);
        let mut bits = BitReader::new(&data);
        let books = info.read_sections(&mut bits).unwrap();
        assert_eq!(books[0], [vec![5; 35], vec![0; 5]].concat());
        assert_eq!(bits.position(), count);
        for fields in [
            vec![(12, 4), (1, 5)],
            vec![(5, 4), (0, 5)],
            vec![(5, 4), (31, 5), (10, 5)],
            vec![(5, 4), (31, 5)],
        ] {
            let (data, _) = packed(&fields);
            let mut bits = BitReader::new(&data);
            assert!(info.read_sections(&mut bits).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
}
