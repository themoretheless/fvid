//! Player adapter for owned MPEG Layer II reconstruction.
use crate::audio::{AudioDecode, AudioPacket};
use crate::{Result, invalid};
pub struct Mp2Decoder {
    decoder: fvid_codecs::mp2::Decoder,
    rate: u32,
}
impl Mp2Decoder {
    pub fn new(rate: u32, channels: u16) -> Result<Self> {
        Ok(Self {
            decoder: fvid_codecs::mp2::Decoder::new(rate, channels)?,
            rate,
        })
    }
}
impl AudioDecode for Mp2Decoder {
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let samples = self.decoder.decode(data)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(samples.len() * 4)
            .map_err(|_| invalid("MP2 float output allocation failed"))?;
        for value in samples {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        Ok(Some(AudioPacket {
            data: bytes,
            pts,
            timebase_num: 1,
            timebase_den: self.rate,
        }))
    }
    fn reset(&mut self) {
        self.decoder.reset();
    }
}
