//! Owned AAC-LC individual channel parsing.
use crate::{Result, invalid, unsupported};
use super::aac_bands::BandTables;
use super::aac_tns as tns_syntax;
use crate::Error;
include!("../../crates/fvid-media/src/owned_aac/aac_channel_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{aac_huffman_tables::*, aac_quant, aac_synthesis::LongSineSynthesis};
    fn pack(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut data = Vec::new();
        let mut n = 0;
        for &(v, width) in fields {
            for bit in (0..width).rev() {
                if n % 8 == 0 {
                    data.push(0);
                }
                *data.last_mut().unwrap() |= ((v >> bit & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        (data, n)
    }
    #[test]
    fn channel_payload_reaches_owned_synthesis() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        let (data, count) = pack(&[
            (100, 8), // global gain
            (0, 1),
            (0, 2),
            (0, 1),
            (1, 6),
            (0, 1), // long ICS, one band
            (1, 4),
            (1, 5), // section: one band, book 1
            (SCF_CODEBOOK_CODES[60], SCF_CODEBOOK_LENS[60]),
            (0, 1),
            (0, 1),
            (0, 1), // no pulse, TNS, gain control
            (SPECTRUM_CODEBOOK1_CODES[80], SPECTRUM_CODEBOOK1_LENS[80]),
        ]);
        let mut bits = BitReader::new(&data);
        let channel = ChannelData::read(&mut bits, &config).unwrap();
        assert_eq!(bits.position(), count);
        assert_eq!(channel.quantized, [1, 1, 1, 1]);
        assert_eq!(channel.scales, vec![vec![BandScale::Spectral(100)]]);
        let reconstructed = channel.ordinary_spectrum(&config).unwrap();
        assert_eq!(&reconstructed[..4], &[1.0; 4]);
        assert!(reconstructed[4..].iter().all(|&x| x == 0.0));
        let mut grouped = [0.0; 4];
        aac_quant::inverse_quantize(&channel.quantized, 100, &mut grouped).unwrap();
        let mut spectrum = vec![0.0; 1024];
        let tables = BandTables::for_config(&config).unwrap();
        channel
            .info
            .deinterleave(tables.long, &grouped, &mut spectrum)
            .unwrap();
        let mut pcm = vec![0.0; 1024];
        LongSineSynthesis::new(1024)
            .unwrap()
            .synthesize_shaped(
                channel.info.sequence,
                channel.info.shape,
                &spectrum,
                &mut pcm,
            )
            .unwrap();
        assert!(pcm.iter().all(|x| x.is_finite()));
        assert!(pcm.iter().any(|x| x.abs() > 1e-6));
        for length in 0..data.len() - 1 {
            let mut bits = BitReader::new(&data[..length]);
            assert!(ChannelData::read(&mut bits, &config).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
    #[test]
    fn reconstruction_scales_each_short_group_and_band() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        let mut channel = ChannelData {
            info: IcsInfo {
                sequence: WindowSequence::EightShort,
                shape: crate::codec::aac_synthesis::WindowShape::Sine,
                max_sfb: 2,
                group_lengths: vec![3, 5],
            prediction: None,
            },
            codebooks: vec![vec![5, 5], vec![5, 5]],
            scales: vec![
                vec![BandScale::Spectral(100), BandScale::Spectral(104)],
                vec![BandScale::Spectral(96), BandScale::Spectral(100)],
            ],
            quantized: [vec![8; 12], vec![-8; 12], vec![8; 20], vec![-8; 20]].concat(),
            pulse: None,
            tns: None,
            gain: None,
        };
        let spectrum = channel.ordinary_spectrum(&config).unwrap();
        for window in 0..8 {
            assert_eq!(
                &spectrum[window * 128..window * 128 + 4],
                &[if window < 3 { 16.0 } else { 8.0 }; 4]
            );
            assert_eq!(
                &spectrum[window * 128 + 4..window * 128 + 8],
                &[if window < 3 { -32.0 } else { -16.0 }; 4]
            );
            assert!(
                spectrum[window * 128 + 8..(window + 1) * 128]
                    .iter()
                    .all(|&v| v == 0.0)
            );
        }
        channel.scales[0][0] = BandScale::Noise(0);
        assert!(channel.ordinary_spectrum(&config).is_err());
        channel.codebooks[0][0] = 13;
        assert!(matches!(
            channel.ordinary_spectrum(&config),
            Err(crate::Error::Unsupported(_))
        ));
    }
    #[test]
    fn noise_bands_reconstruct_per_window_and_rollback_on_late_error() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        let mut channel = ChannelData {
            info: IcsInfo {
                sequence: WindowSequence::EightShort,
                shape: crate::codec::aac_synthesis::WindowShape::Sine,
                max_sfb: 2,
                group_lengths: vec![8],
            prediction: None,
            },
            codebooks: vec![vec![13, 13]],
            scales: vec![vec![BandScale::Noise(0), BandScale::Noise(4)]],
            quantized: vec![0; 64],
            pulse: None,
            tns: None,
            gain: None,
        };
        let mut noise = crate::codec::aac_noise::NoiseState::default();
        let spectrum = channel.spectrum_with_noise(&config, &mut noise).unwrap();
        for w in 0..8 {
            for band in 0..2 {
                let start = w * 128 + band * 4;
                let energy: f64 = spectrum[start..start + 4]
                    .iter()
                    .map(|&x| f64::from(x).powi(2))
                    .sum();
                assert!((energy - if band == 0 { 1.0 } else { 4.0 }).abs() < 1e-6);
            }
        }
        let saved = noise.clone();
        channel.scales[0][1] = BandScale::Noise(156);
        assert!(channel.spectrum_with_noise(&config, &mut noise).is_err());
        assert_eq!(noise, saved);
    }
    #[test]
    fn unsupported_tools_do_not_consume_channel_header() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        {
            let (tns, gain) = (0, 1);
            let (data, _) = pack(&[
                (100, 8),
                (0, 1),
                (0, 2),
                (0, 1),
                (0, 6),
                (0, 1),
                (0, 1),
                (tns, 1),
                (gain, 1),
                (1, 2), // one gain band
                (1, 3), // active adjustment, not the newly supported no-op
                (8, 4),
                (0, 5),
            ]);
            let mut bits = BitReader::new(&data);
            assert!(ChannelData::read(&mut bits, &config).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
}
