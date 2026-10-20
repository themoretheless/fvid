//! Player adapter for owned MPEG Layer I reconstruction.
use crate::{
    Result,
    audio::{AudioDecode, AudioPacket},
};
pub struct Mp1Decoder {
    core: Box<fvid_codecs::mp1::Decoder>,
    rate: u32,
}
impl Mp1Decoder {
    pub fn new(rate: u32, channels: u16) -> Result<Self> {
        Ok(Self {
            core: Box::new(fvid_codecs::mp1::Decoder::new(rate, channels)?),
            rate,
        })
    }
}
impl AudioDecode for Mp1Decoder {
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let samples = self.core.decode(data)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(samples.len() * 4)
            .map_err(|_| crate::invalid("MP1 float output allocation failed"))?;
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
        self.core.reset();
    }
}
