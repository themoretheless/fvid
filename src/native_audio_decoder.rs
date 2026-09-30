//! Owned packet-to-PCM codecs used by the MP4 audio timeline.
use crate::{Result, container::mp4::Track, invalid};
pub(crate) enum Mp4PcmDecoder {
    Aac(crate::codec::aac_native::NativeAacDecoder),
    Alac {
        decoder: crate::codec::alac_decoder::AlacDecoder,
        rate: u32,
        channels: u16,
    },
}
impl Mp4PcmDecoder {
    pub(crate) fn new(track: &Track) -> Result<Self> {
        match &track.codec {
            b"mp4a" => Ok(Self::Aac(crate::codec::aac_native::NativeAacDecoder::new(
                crate::codec::config::aac_specific_config(&track.configuration)?,
            )?)),
            b"alac" => {
                let cookie = &track.configuration;
                if cookie.len() < 24
                    || u32::from_be_bytes(cookie[20..24].try_into().unwrap()) != track.sample_rate
                {
                    return Err(invalid("ALAC cookie and track sample rates disagree"));
                }
                Ok(Self::Alac {
                    decoder: crate::codec::alac_decoder::AlacDecoder::new(
                        cookie,
                        track.sample_rate,
                        track.channels,
                    )?,
                    rate: track.sample_rate,
                    channels: track.channels,
                })
            }
            _ => Err(invalid(
                "selected MP4 audio codec is not owned by the export path",
            )),
        }
    }
    pub(crate) fn sample_rate(&self) -> u32 {
        match self {
            Self::Aac(d) => d.sample_rate(),
            Self::Alac { rate, .. } => *rate,
        }
    }
    pub(crate) fn channels(&self) -> u16 {
        match self {
            Self::Aac(d) => u16::from(d.channels()),
            Self::Alac { channels, .. } => *channels,
        }
    }
    pub(crate) fn reset(&mut self) {
        if let Self::Aac(d) = self {
            d.reset();
        }
    }
    pub(crate) fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        match self {
            Self::Aac(d) => d.decode(packet),
            Self::Alac { decoder, .. } => decoder.decode_pcm(packet),
        }
    }
}
