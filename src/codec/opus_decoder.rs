//! Opus playback adapter over the local native Rust CELT/SILK core.
//! Container priming and end trimming belong to AudioStream::present_decoded.
use crate::audio::{AudioDecode, AudioPacket};
use crate::{Result, invalid};

pub(crate) fn header(data: &[u8]) -> Result<fvid_opus::OpusHead> {
    let header =
        fvid_opus::OpusHead::parse(data).map_err(|e| invalid(&format!("Opus header: {e}")))?;
    header
        .decoding_layout()
        .map_err(|e| invalid(&format!("Opus layout: {e}")))?;
    Ok(header)
}

pub(crate) fn duration_ns(configuration: &[u8], data: &[u8]) -> Result<u64> {
    let head = header(configuration)?;
    let samples = fvid_opus::multistream_packet_samples(data, usize::from(head.stream_count))
        .map_err(|e| invalid(&format!("Opus packet: {e}")))?;
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
