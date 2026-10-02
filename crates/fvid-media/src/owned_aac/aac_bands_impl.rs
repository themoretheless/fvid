use super::{aac_ics::IcsInfo, bits::BitReader, config::AacConfig};

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
        let tables = band_geometry::BandTables::new(config.sample_rate, config.frame_samples as usize)
            .map_err(|e| invalid(&e.0))?;
        Ok(Self { long: tables.long, short: tables.short })
    }
    pub fn tns_limit(rate: u32, short: bool) -> usize {
        band_geometry::BandTables::tns_limit(rate, short)
    }
    pub fn read_ics(&self, bits: &mut BitReader<'_>) -> Result<IcsInfo> {
        IcsInfo::read(
            bits,
            ((self.long.len() - 1) as u8, (self.short.len() - 1) as u8),
        )
    }
}
