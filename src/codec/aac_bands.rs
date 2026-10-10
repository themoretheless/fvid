//! Sample-rate selection of AAC-LC scale-factor band geometry.
use crate::{Result, invalid, unsupported};
use fvid_media::owned_aac::aac_bands as band_geometry;
include!("../../crates/fvid-media/src/owned_aac/aac_bands_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    fn config(rate: u32) -> AacConfig {
        AacConfig {
            channel_configuration: 2,
            object_type: 2,
            sample_rate: rate,
            channels: 2,
            frame_samples: 1024,
            core_coder_delay: None,
            section_data_resilience: false,
            scalefactor_data_resilience: false,
            spectral_data_resilience: false,
        }
    }
    #[test]
    fn indexed_rates_have_standard_counts_and_complete_geometry() {
        for (rate, long, short) in [
            (96000, 41, 12),
            (88200, 41, 12),
            (64000, 47, 12),
            (48000, 49, 14),
            (44100, 49, 14),
            (32000, 51, 14),
            (24000, 47, 15),
            (22050, 47, 15),
            (16000, 43, 15),
            (12000, 43, 15),
            (11025, 43, 15),
            (8000, 40, 15),
            (7350, 40, 15),
        ] {
            let tables = BandTables::for_config(&config(rate)).unwrap();
            assert_eq!(
                (tables.long.len() - 1, tables.short.len() - 1),
                (long, short)
            );
            for (offsets, size) in [(tables.long, 1024), (tables.short, 128)] {
                assert_eq!(offsets[0], 0);
                assert_eq!(*offsets.last().unwrap(), size);
                assert!(
                    offsets
                        .windows(2)
                        .all(|p| p[1] > p[0] && (p[1] - p[0]) % 4 == 0)
                );
            }
        }
    }
    #[test]
    fn explicit_rates_switch_at_the_geometry_boundaries() {
        for (edge, below, above) in [
            (9391, 40, 43),
            (18783, 43, 47),
            (27713, 47, 51),
            (37566, 51, 49),
            (55426, 49, 47),
            (75132, 47, 41),
        ] {
            assert_eq!(
                BandTables::for_config(&config(edge - 1))
                    .unwrap()
                    .long
                    .len()
                    - 1,
                below
            );
            assert_eq!(
                BandTables::for_config(&config(edge)).unwrap().long.len() - 1,
                above
            );
        }
        assert!(BandTables::for_config(&config(0)).is_err());
        let mut c = config(48000);
        c.frame_samples = 512;
        assert!(BandTables::for_config(&c).is_err());
    }
    #[test]
    fn short_frame_geometry_covers_all_indexed_rates() {
        for (rate, bands) in [
            (96000, 40),
            (88200, 40),
            (64000, 46),
            (48000, 49),
            (44100, 49),
            (32000, 49),
            (24000, 46),
            (22050, 46),
            (16000, 42),
            (12000, 42),
            (11025, 42),
            (8000, 40),
            (7350, 40),
        ] {
            let mut c = config(rate);
            c.frame_samples = 960;
            let tables = BandTables::for_config(&c).unwrap();
            assert_eq!(tables.long.len() - 1, bands);
            for (offsets, end) in [(tables.long, 960), (tables.short, 120)] {
                assert_eq!(offsets[0], 0);
                assert_eq!(*offsets.last().unwrap(), end);
                assert!(
                    offsets
                        .windows(2)
                        .all(|p| p[0] < p[1] && (p[1] - p[0]) % 4 == 0)
                );
            }
        }
    }
    #[test]
    fn asc_drives_ics_band_limit() {
        let c = AacConfig::parse(&[0x11, 0x90]).unwrap(); // AAC-LC 48 kHz stereo.
        let tables = BandTables::for_config(&c).unwrap();
        // only-long, sine, max_sfb=49, predictor absent.
        let mut valid = BitReader::new(&[0x0c, 0x40]);
        assert_eq!(tables.read_ics(&mut valid).unwrap().max_sfb, 49);
        let mut invalid = BitReader::new(&[0x0c, 0x80]);
        assert!(tables.read_ics(&mut invalid).is_err());
        assert_eq!(invalid.position(), 0);
    }
}
