//! Sample-rate selection of AAC-LC scale-factor band geometry.
use super::{aac_band_tables::*, aac_ics::IcsInfo, bits::BitReader, config::AacConfig};
use crate::{Result, invalid, unsupported};

pub struct BandTables {
    pub long: &'static [usize],
    pub short: &'static [usize],
}
impl BandTables {
    pub fn for_config(config: &AacConfig) -> Result<Self> {
        if config.object_type != 2 {
            return Err(unsupported("band tables require AAC-LC"));
        }
        if !matches!(config.frame_samples, 960 | 1024) {
            return Err(unsupported("AAC band tables require 960 or 1024 samples"));
        }
        if config.sample_rate == 0 {
            return Err(invalid("zero AAC sample rate"));
        }
        // Rate intervals choose the standard band geometry even when ASC uses
        // an explicit sampling frequency rather than a frequency index.
        let (long, short): (&[usize], &[usize]) = match config.sample_rate {
            75132.. => (&SWB_OFFSET_96K_LONG, &SWB_OFFSET_64K_SHORT),
            55426..=75131 => (&SWB_OFFSET_64K_LONG, &SWB_OFFSET_64K_SHORT),
            37566..=55425 => (&SWB_OFFSET_48K_LONG, &SWB_OFFSET_48K_SHORT),
            27713..=37565 => (&SWB_OFFSET_32K_LONG, &SWB_OFFSET_48K_SHORT),
            18783..=27712 => (&SWB_OFFSET_24K_LONG, &SWB_OFFSET_24K_SHORT),
            9391..=18782 => (&SWB_OFFSET_16K_LONG, &SWB_OFFSET_16K_SHORT),
            _ => (&SWB_OFFSET_8K_LONG, &SWB_OFFSET_8K_SHORT),
        };
        let (long, short): (&[usize], &[usize]) = if config.frame_samples == 960 {
            match config.sample_rate {
                75132.. => (&SWB_960_96K, &SWB_120_64K),
                55426..=75131 => (&SWB_960_64K, &SWB_120_64K),
                37566..=55425 => (&SWB_960_48K, &SWB_120_48K),
                27713..=37565 => (&SWB_960_32K, &SWB_120_48K),
                18783..=27712 => (&SWB_960_24K, &SWB_120_24K),
                9391..=18782 => (&SWB_960_16K, &SWB_120_16K),
                _ => (&SWB_960_8K, &SWB_120_8K),
            }
        } else {
            (long, short)
        };
        Ok(Self { long, short })
    }
    pub fn tns_limit(rate: u32, short: bool) -> usize {
        let (long, small) = match rate {
            75132.. => (31, 9),
            55426..=75131 => (34, 10),
            46009..=55425 => (40, 14),
            37566..=46008 => (42, 14),
            27713..=37565 => (51, 14),
            18783..=27712 => (46, 14),
            9391..=18782 => (42, 14),
            _ => (39, 14),
        };
        if short { small } else { long }
    }
    pub fn read_ics(&self, bits: &mut BitReader<'_>) -> Result<IcsInfo> {
        IcsInfo::read(
            bits,
            ((self.long.len() - 1) as u8, (self.short.len() - 1) as u8),
        )
    }
}

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
