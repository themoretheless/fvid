//! LD-specific LTP side information. Parsing does not commit channel history.
use super::{
    Result,
    aac_ltp_syntax::{LtpData, Usage},
    bits::BitReader,
    invalid,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LdLtpData {
    pub lag_update: Option<u16>,
    pub coefficient_index: u8,
    pub used: Vec<bool>,
}
impl LdLtpData {
    /// ISO table 4.49. Caller commits the resolved lag only after the complete
    /// channel/frame succeeds, preserving it across absent predictors and seeks.
    pub fn read(bits: &mut BitReader<'_>, max_sfb: u8, frame_samples: u16) -> Result<Self> {
        if !matches!(frame_samples, 480 | 512) || max_sfb > 37 {
            return Err(invalid("invalid AAC LD LTP geometry"));
        }
        let mut trial = bits.clone();
        let lag_update = if trial.bit()? {
            Some(trial.read(10)? as u16)
        } else {
            None
        };
        let coefficient_index = trial.read(3)? as u8;
        let mut used = Vec::with_capacity(usize::from(max_sfb));
        for _ in 0..max_sfb {
            used.push(trial.bit()?);
        }
        *bits = trial;
        Ok(Self {
            lag_update,
            coefficient_index,
            used,
        })
    }
    pub fn resolve(&self, previous_lag: u16, frame_samples: u16) -> Result<LtpData> {
        let lag = self.lag_update.unwrap_or(previous_lag);
        if !matches!(frame_samples, 480 | 512)
            || lag > 1023
            || self.coefficient_index > 7
            || self.used.len() > 37
        {
            return Err(invalid("invalid AAC LD LTP history or parameters"));
        }
        Ok(LtpData {
            lag,
            coefficient_index: self.coefficient_index,
            usage: Usage::Bands(self.used.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(lag: Option<u16>, coefficient: u8, bands: usize) -> Vec<u8> {
        let mut s = String::from(if lag.is_some() { "1" } else { "0" });
        if let Some(lag) = lag {
            s.push_str(&format!("{lag:010b}"));
        }
        s.push_str(&format!("{coefficient:03b}"));
        for i in 0..bands {
            s.push(if i % 3 == 0 { '1' } else { '0' });
        }
        while s.len() % 8 != 0 {
            s.push('0');
        }
        s.as_bytes()
            .chunks(8)
            .map(|c| c.iter().fold(0, |n, b| (n << 1) | u8::from(*b == b'1')))
            .collect()
    }
    #[test]
    fn lag_update_reuse_all_bands_and_independent_histories() {
        for n in [480, 512] {
            let data = packet(Some(1023), 7, 37);
            let mut bits = BitReader::new(&data);
            let update = LdLtpData::read(&mut bits, 37, n).unwrap();
            assert_eq!(bits.position(), 51);
            assert_eq!(update.resolve(1, n).unwrap().lag, 1023);
            let data = packet(None, 3, 37);
            let mut bits = BitReader::new(&data);
            let reuse = LdLtpData::read(&mut bits, 37, n).unwrap();
            assert_eq!(bits.position(), 41);
            for previous in [0, 13, 960, 1023] {
                assert_eq!(reuse.resolve(previous, n).unwrap().lag, previous);
            }
            assert!(reuse.resolve(1024, n).is_err());
            assert_eq!(reuse.used, (0..37).map(|i| i % 3 == 0).collect::<Vec<_>>());
        }
    }
    #[test]
    fn every_bit_truncation_and_invalid_geometry_leave_cursor_unchanged() {
        for lag in [None, Some(960)] {
            let data = packet(lag, 5, 37);
            let length = if lag.is_some() { 51 } else { 41 };
            // Place each truncated prefix at the end of a byte-aligned buffer,
            // so padding bits cannot accidentally satisfy a missing field.
            for available in 0..length {
                let prefix: Vec<_> = (0..available)
                    .map(|i| (data[i / 8] >> (7 - i % 8)) & 1)
                    .collect();
                let pad = (8 - available % 8) % 8;
                let mut bytes = vec![0u8; (available + pad) / 8];
                for (i, bit) in prefix.iter().enumerate() {
                    bytes[(pad + i) / 8] |= bit << (7 - (pad + i) % 8);
                }
                let mut bits = BitReader::new(&bytes);
                bits.skip(pad).unwrap();
                assert!(LdLtpData::read(&mut bits, 37, 480).is_err());
                assert_eq!(bits.position(), pad);
            }
        }
        let data = packet(Some(1023), 0, 0);
        let mut bits = BitReader::new(&data);
        assert!(LdLtpData::read(&mut bits, 0, 1024).is_err());
        assert_eq!(bits.position(), 0);
    }
}
