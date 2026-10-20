//! Native FLAC playback adapter; all frame reconstruction lives in fvid-codecs.
use crate::audio::{AudioDecode, AudioPacket};
use crate::{Result, invalid};
use fvid_codecs::flac::{Config, decode};
pub struct FlacDecoder {
    config: Config,
}
impl FlacDecoder {
    pub fn new(extra: &[u8], sample_rate: u32, channels: u16) -> Result<Self> {
        let info = if extra.len() == 34 {
            extra
        } else {
            let blocks = extra.strip_prefix(b"fLaC").unwrap_or(extra);
            if blocks.len() < 38 || blocks[0] & 127 != 0 || blocks[1..4] != [0, 0, 34] {
                return Err(invalid("FLAC requires STREAMINFO"));
            }
            &blocks[4..38]
        };
        let geometry = u64::from_be_bytes(info[10..18].try_into().unwrap());
        let config = Config {
            sample_rate: (geometry >> 44) as u32,
            channels: ((geometry >> 41) & 7) as u16 + 1,
            bits: ((geometry >> 36) & 31) as u8 + 1,
        };
        if config.sample_rate == 0
            || !(4..=32).contains(&config.bits)
            || config.sample_rate != sample_rate
            || config.channels != channels
        {
            return Err(invalid("FLAC STREAMINFO disagrees with output geometry"));
        }
        Ok(Self { config })
    }
}
impl AudioDecode for FlacDecoder {
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let frame = decode(data, self.config)?;
        let scale = (1u64 << (self.config.bits - 1)) as f64;
        let mut output = Vec::new();
        output
            .try_reserve_exact(frame.samples.len() * 4)
            .map_err(|_| invalid("FLAC float allocation failed"))?;
        for sample in frame.samples {
            output.extend_from_slice(&((f64::from(sample) / scale) as f32).to_le_bytes());
        }
        Ok(Some(AudioPacket {
            data: output,
            pts,
            timebase_num: 1,
            timebase_den: self.config.sample_rate,
        }))
    }
    fn reset(&mut self) {}
}
