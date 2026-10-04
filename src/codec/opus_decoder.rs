//! Opus playback adapter over the local native Rust CELT/SILK core.
//! Container priming and end trimming belong to AudioStream::present_decoded.
use crate::audio::{AudioDecode, AudioPacket};
use crate::{Result, invalid, unsupported};

pub(crate) fn header(data: &[u8]) -> Result<fvid_opus::OpusHead> {
    let header =
        fvid_opus::OpusHead::parse(data).map_err(|e| invalid(&format!("Opus header: {e}")))?;
    if header.channel_count > 8 || !matches!(header.mapping_family, 0 | 1) {
        return Err(unsupported(
            "Opus playback supports mapping families 0/1 up to eight channels",
        ));
    }
    if header.mapping_family == 1 {
        let layout = fvid_opus::ChannelLayout::surround(usize::from(header.channel_count), 1)
            .map_err(|e| invalid(&format!("Opus layout: {e}")))?;
        if usize::from(header.stream_count) != layout.nb_streams
            || usize::from(header.coupled_count) != layout.nb_coupled_streams
            || header.channel_mapping != layout.mapping
        {
            return Err(unsupported(
                "Opus playback requires the standard family-1 channel mapping",
            ));
        }
    }
    Ok(header)
}

pub(crate) fn duration_ns(data: &[u8]) -> Result<u64> {
    let samples =
        fvid_opus::packet::samples_48k(data).map_err(|e| invalid(&format!("Opus packet: {e}")))?;
    Ok(samples as u64 * 1_000_000_000 / 48000)
}

pub struct OpusDecoder {
    decoder: fvid_media::owned_opus::OpusDecoder,
}
impl OpusDecoder {
    pub fn new(data: &[u8], sample_rate: u32, channels: u16) -> Result<Self> {
        header(data)?;
        Ok(Self {
            decoder: fvid_media::owned_opus::OpusDecoder::new(data, sample_rate, channels)
                .map_err(|e| invalid(&format!("Opus initialization: {e}")))?,
        })
    }
}
impl AudioDecode for OpusDecoder {
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> Result<Option<AudioPacket>> {
        Ok(Some(AudioPacket {
            data: self
                .decoder
                .decode_bytes(data)
                .map_err(|e| invalid(&format!("Opus decode: {e}")))?,
            pts,
            timebase_num: 1,
            timebase_den: 1_000_000_000,
        }))
    }
    fn reset(&mut self) {
        self.decoder.reset();
    }
}
