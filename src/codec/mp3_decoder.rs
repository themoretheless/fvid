//! Player packet adapter for the owned MPEG Layer III decoder.
use crate::{
    Result,
    audio::{AudioDecode, AudioPacket},
    invalid,
};
pub struct Mp3Decoder {
    decoder: Box<fvid_codecs::mp3::Decoder>,
    initial: Option<std::sync::Arc<fvid_codecs::mp3::Decoder>>,
    rate: u32,
    channels: u16,
}
pub struct Checkpoint {
    decoder: Box<fvid_codecs::mp3::Decoder>,
    initial: Option<std::sync::Arc<fvid_codecs::mp3::Decoder>>,
    rate: u32,
    channels: u16,
}
impl Mp3Decoder {
    pub fn new(rate: u32, channels: u16) -> Result<Self> {
        fvid_codecs::mp3_spectral::Bands::for_rate(rate)?;
        if !(1..=2).contains(&channels) {
            return Err(invalid("invalid MP3 channel count"));
        }
        Ok(Self {
            decoder: Default::default(),
            initial: None,
            rate,
            channels,
        })
    }
    pub fn prime_info(&mut self, packet: &[u8]) -> Result<()> {
        self.decoder.prime_info(packet)?;
        self.initial = Some(std::sync::Arc::new(self.decoder.as_ref().clone()));
        Ok(())
    }
}
impl AudioDecode for Mp3Decoder {
    fn checkpoint(&self) -> Option<crate::audio::AudioCheckpoint> {
        Some(crate::audio::AudioCheckpoint::Mp3(Checkpoint {
            decoder: self.decoder.clone(),
            initial: self.initial.clone(),
            rate: self.rate,
            channels: self.channels,
        }))
    }
    fn restore(&mut self, state: &crate::audio::AudioCheckpoint) -> Result<()> {
        let crate::audio::AudioCheckpoint::Mp3(state) = state else {
            return Err(invalid("MP3 checkpoint codec mismatch"));
        };
        if state.rate != self.rate || state.channels != self.channels {
            return Err(invalid("MP3 checkpoint geometry mismatch"));
        }
        self.decoder = state.decoder.clone();
        self.initial = state.initial.clone();
        Ok(())
    }
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let header = fvid_codecs::mp3_frame::Header::parse(data)?;
        if header.sample_rate != self.rate || header.channels != self.channels {
            return Err(invalid(
                "MP3 frame geometry differs from the selected track",
            ));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(header.samples() * usize::from(header.channels) * 4)
            .map_err(|_| invalid("MP3 float output allocation failed"))?;
        let pcm = self.decoder.decode(data)?;
        for value in pcm.samples {
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
        if let Some(initial) = &self.initial {
            self.decoder = Box::new(initial.as_ref().clone());
        } else {
            self.decoder.reset();
        }
    }
}
