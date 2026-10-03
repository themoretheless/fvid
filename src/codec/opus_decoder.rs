//! Opus playback adapter over the local native Rust CELT/SILK core.
//! Container priming and end trimming belong to AudioStream::present_decoded.
use crate::audio::{AudioDecode, AudioPacket};
use crate::{Result, invalid, unsupported};

enum Core {
    Single(fvid_opus::OpusDecoder),
    Multi(fvid_opus::OpusMSDecoder),
}

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
    decoder: Core,
    pcm: Vec<f32>,
    order: Vec<usize>,
    channels: usize,
    failed: bool,
}
impl OpusDecoder {
    pub fn new(data: &[u8], sample_rate: u32, channels: u16) -> Result<Self> {
        let head = header(data)?;
        if sample_rate != 48000 || channels != u16::from(head.channel_count) {
            return Err(invalid(
                "Opus playback requires its declared layout at 48000 Hz",
            ));
        }
        let channels = usize::from(channels);
        let decoder = if head.mapping_family == 0 {
            let mut decoder = fvid_opus::OpusDecoder::new(48000, channels)
                .map_err(|e| invalid(&format!("Opus initialization: {e}")))?;
            decoder.gain_q8 = i32::from(head.output_gain_q8);
            Core::Single(decoder)
        } else {
            let mut decoder = fvid_opus::OpusMSDecoder::new(48000, channels, 1)
                .map_err(|e| invalid(&format!("Opus multistream initialization: {e}")))?;
            for stream in decoder.streams_mut() {
                stream.gain_q8 = i32::from(head.output_gain_q8);
            }
            Core::Multi(decoder)
        };
        // RFC 7845 family 1 is Vorbis order; PCM output uses WAVE speaker order.
        let order = match channels {
            3 => vec![0, 2, 1],
            5 => vec![0, 2, 1, 3, 4],
            6 => vec![0, 2, 1, 5, 3, 4],
            7 => vec![0, 2, 1, 6, 5, 3, 4],
            8 => vec![0, 2, 1, 7, 5, 6, 3, 4],
            _ => (0..channels).collect(),
        };
        Ok(Self {
            decoder,
            pcm: vec![0.0; 5760 * channels],
            order,
            channels,
            failed: false,
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
        if self.failed {
            return Err(invalid("Opus decoder requires reset after an error"));
        }
        let expected = usize::try_from(duration_ns(data)? * 48000 / 1_000_000_000).unwrap();
        let result = match &mut self.decoder {
            Core::Single(d) => d.decode(data, 5760, &mut self.pcm),
            Core::Multi(d) => d.decode(data, 5760, &mut self.pcm),
        };
        let frames = match result {
            Ok(frames) => frames,
            Err(e) => {
                self.failed = true;
                return Err(invalid(&format!("Opus decode: {e}")));
            }
        };
        if frames != expected
            || self.pcm[..frames * self.channels]
                .iter()
                .any(|s| !s.is_finite())
        {
            self.failed = true;
            return Err(invalid("invalid Opus decoded sample geometry"));
        }
        let mut data = Vec::with_capacity(frames * self.channels * 4);
        for frame in self.pcm[..frames * self.channels].chunks_exact(self.channels) {
            for &channel in &self.order {
                data.extend_from_slice(&frame[channel].to_le_bytes());
            }
        }
        Ok(Some(AudioPacket {
            data,
            pts,
            timebase_num: 1,
            timebase_den: 1_000_000_000,
        }))
    }
    fn reset(&mut self) {
        self.failed = match &mut self.decoder {
            Core::Single(d) => d.reset_state().is_err(),
            Core::Multi(d) => d.reset_state().is_err(),
        };
    }
}
