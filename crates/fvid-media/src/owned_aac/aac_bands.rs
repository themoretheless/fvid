//! AAC scale-factor band geometry shared by the owned decoder.
use super::{Result, aac_band_tables::*, invalid};
pub struct BandTables {
    pub long: &'static [usize],
    pub short: &'static [usize],
}
impl BandTables {
    pub fn new(sample_rate: u32, frame_samples: usize) -> Result<Self> {
        if sample_rate == 0 {
            return Err(invalid("zero AAC sample rate"));
        }
        if !matches!(frame_samples, 960 | 1024) {
            return Err(invalid("AAC band tables require 960 or 1024 samples"));
        }
        // Rate intervals choose the standard band geometry even when ASC uses
        // an explicit sampling frequency rather than a frequency index.
        let (long, short): (&[usize], &[usize]) = match sample_rate {
            75132.. => (&SWB_OFFSET_96K_LONG, &SWB_OFFSET_64K_SHORT),
            55426..=75131 => (&SWB_OFFSET_64K_LONG, &SWB_OFFSET_64K_SHORT),
            37566..=55425 => (&SWB_OFFSET_48K_LONG, &SWB_OFFSET_48K_SHORT),
            27713..=37565 => (&SWB_OFFSET_32K_LONG, &SWB_OFFSET_48K_SHORT),
            18783..=27712 => (&SWB_OFFSET_24K_LONG, &SWB_OFFSET_24K_SHORT),
            9391..=18782 => (&SWB_OFFSET_16K_LONG, &SWB_OFFSET_16K_SHORT),
            _ => (&SWB_OFFSET_8K_LONG, &SWB_OFFSET_8K_SHORT),
        };
        let (long, short): (&[usize], &[usize]) = if frame_samples == 960 {
            match sample_rate {
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
}
