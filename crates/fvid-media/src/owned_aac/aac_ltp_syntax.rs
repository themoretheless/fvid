//! Owned ordinary AAC LTP side information. Not ER/LD syntax or PCM synthesis.
use super::{Result, aac_synthesis::WindowSequence, bits::BitReader, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortPrediction {
    /// Raw four-bit short_lag value; interpretation belongs to synthesis.
    pub lag_offset: Option<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Usage {
    Bands(Vec<bool>),
    Windows([Option<ShortPrediction>; 8]),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LtpData {
    pub lag: u16,
    pub coefficient_index: u8,
    pub usage: Usage,
}
impl LtpData {
    /// Read ltp_data after its presence flag. On any error the cursor is unchanged.
    pub fn read(
        bits: &mut BitReader<'_>,
        sequence: WindowSequence,
        max_sfb: u8,
        frame_samples: u16,
    ) -> Result<Self> {
        let short = sequence == WindowSequence::EightShort;
        if !matches!(frame_samples, 960 | 1024) || max_sfb > if short { 15 } else { 63 } {
            return Err(invalid("invalid AAC LTP geometry"));
        }
        let mut trial = bits.clone();
        let lag = trial.read(11)? as u16;
        if lag > 2 * frame_samples {
            return Err(invalid("AAC LTP lag exceeds two frames"));
        }
        let coefficient_index = trial.read(3)? as u8;
        let usage = if short {
            let mut windows = std::array::from_fn(|_| None);
            for slot in &mut windows {
                if trial.bit()? {
                    let lag_offset = if trial.bit()? {
                        Some(trial.read(4)? as u8)
                    } else {
                        None
                    };
                    *slot = Some(ShortPrediction { lag_offset });
                }
            }
            Usage::Windows(windows)
        } else {
            let mut bands = Vec::with_capacity(usize::from(max_sfb.min(40)));
            for _ in 0..max_sfb.min(40) {
                bands.push(trial.bit()?);
            }
            Usage::Bands(bands)
        };
        *bits = trial;
        Ok(Self {
            lag,
            coefficient_index,
            usage,
        })
    }
}
