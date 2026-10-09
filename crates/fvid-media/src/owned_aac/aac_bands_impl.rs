use super::{aac_ics::IcsInfo, bits::BitReader, config::AacConfig};

pub struct BandTables {
    pub long: &'static [usize],
    pub short: &'static [usize],
}
impl BandTables {
    pub fn for_config(config: &AacConfig) -> Result<Self> {
        if !matches!(config.object_type, 2 | 3) {
            return Err(unsupported("band tables require AAC-LC or AAC-SSR"));
        }
        if !matches!(config.frame_samples, 960 | 1024) {
            return Err(unsupported("AAC band tables require 960 or 1024 samples"));
        }
        if config.object_type == 3 && config.frame_samples != 1024 { return Err(invalid("AAC SSR requires 1024 spectral lines")); }
        if config.sample_rate == 0 {
            return Err(invalid("zero AAC sample rate"));
        }
        let tables = band_geometry::BandTables::new(config.sample_rate, config.frame_samples as usize)
            .map_err(|e| invalid(&e.0))?;
        Ok(Self { long: tables.long, short: tables.short })
    }
    pub fn tns_limit(rate: u32, short: bool) -> usize {
        band_geometry::BandTables::tns_limit(rate, short)
    }
    pub fn ssr_tns_limit(rate:u32, short:bool)->usize {
        let (long, small) = match rate {
            75132.. => (28,7), 55426..=75131 => (27,7), 27713..=55425 => (26,6),
            18783..=27712 => (29,7), 9391..=18782 => (23,8), _ => (19,7),
        };
        if short {small} else {long}
    }
    pub fn read_ics(&self, bits: &mut BitReader<'_>) -> Result<IcsInfo> {
        IcsInfo::read(
            bits,
            ((self.long.len() - 1) as u8, (self.short.len() - 1) as u8),
        )
    }
}
