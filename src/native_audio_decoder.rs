//! Owned packet-to-PCM codecs used by the owned container audio timelines.
use crate::{Result, container::mp4::Track, invalid};
pub(crate) enum PacketPcmDecoder {
    Pcm(crate::codec::pcm_decoder::PcmDecoder),
    Aac(crate::codec::aac_native::NativeAacDecoder),
    Alac {
        decoder: crate::codec::alac_decoder::AlacDecoder,
        rate: u32,
        channels: u16,
    },
}
impl PacketPcmDecoder {
    pub(crate) fn checkpoint(&self) -> Option<crate::codec::aac_native::AacCheckpoint> {
        if let Self::Aac(d) = self {
            Some(d.checkpoint())
        } else {
            None
        }
    }
    pub(crate) fn restore_checkpoint(
        &mut self,
        state: &crate::codec::aac_native::AacCheckpoint,
    ) -> Result<bool> {
        if let Self::Aac(d) = self {
            d.restore(state)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) const SAMPLE_BYTES: usize = 4;
    pub(crate) fn new(track: &Track) -> Result<Self> {
        match &track.codec {
            b"mp4a" => Ok(Self::Aac(crate::codec::aac_native::NativeAacDecoder::new(
                crate::codec::config::aac_specific_config(&track.configuration)?,
            )?)),
            b"raw " => {
                if track.bit_depth != 8 || !(1..=64).contains(&track.channels) {
                    return Err(invalid(
                        "QuickTime raw PCM requires 8 bits and 1..64 channels",
                    ));
                }
                Ok(Self::Pcm(crate::codec::pcm_decoder::PcmDecoder::new(
                    crate::codec::pcm_decoder::PcmFormat::Unsigned8,
                    track.sample_rate,
                    track.channels,
                )?))
            }
            b"alac" => Self::alac(&track.configuration, track.sample_rate, track.channels),
            b"sowt" | b"twos" | b"fl32" | b"fl64" | b"in24" | b"in32" => {
                if !(1..=64).contains(&track.channels) {
                    return Err(invalid("PCM channels must be 1..64"));
                }
                let format = match &track.codec {
                    b"fl32" => crate::codec::pcm_decoder::PcmFormat::Float { bits: 32 },
                    b"fl64" => crate::codec::pcm_decoder::PcmFormat::Float { bits: 64 },
                    _ => crate::codec::pcm_decoder::PcmFormat::Int {
                        bits: u8::try_from(track.bit_depth)
                            .map_err(|_| invalid("PCM bit depth overflow"))?,
                        big_endian: track.codec == *b"twos"
                            || (matches!(&track.codec, b"in24" | b"in32")
                                && track.configuration.first() != Some(&1)),
                    },
                };
                let mut decoder = crate::codec::pcm_decoder::PcmDecoder::new(
                    format,
                    track.sample_rate,
                    track.channels,
                )?;
                decoder.set_float_big_endian(track.configuration.first() != Some(&1));
                Ok(Self::Pcm(decoder))
            }
            _ => Err(invalid(
                "selected MP4 audio codec is not owned by the export path",
            )),
        }
    }
    fn alac(cookie: &[u8], rate: u32, channels: u16) -> Result<Self> {
        if cookie.len() < 24 || u32::from_be_bytes(cookie[20..24].try_into().unwrap()) != rate {
            return Err(invalid("ALAC cookie and track sample rates disagree"));
        }
        Ok(Self::Alac {
            decoder: crate::codec::alac_decoder::AlacDecoder::new(cookie, rate, channels)?,
            rate,
            channels,
        })
    }
    pub(crate) fn from_matroska(track: &crate::container::webm::Track) -> Result<Self> {
        let rate = u32::try_from(track.sample_rate)
            .map_err(|_| invalid("Matroska audio rate overflow"))?;
        let channels = u16::try_from(track.channels)
            .map_err(|_| invalid("Matroska audio channel count overflow"))?;
        match track.codec.as_str() {
            "A_AAC" => Ok(Self::Aac(crate::codec::aac_native::NativeAacDecoder::new(
                &track.codec_private,
            )?)),
            "A_ALAC" => Self::alac(&track.codec_private, rate, channels),
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => {
                if channels == 0 || channels > 64 {
                    return Err(invalid("PCM channels must be 1..64"));
                }
                let bits =
                    u8::try_from(track.bit_depth).map_err(|_| invalid("PCM bit depth overflow"))?;
                let format = if track.codec == "A_PCM/FLOAT/IEEE" {
                    crate::codec::pcm_decoder::PcmFormat::Float { bits }
                } else if bits == 8 {
                    crate::codec::pcm_decoder::PcmFormat::Unsigned8
                } else {
                    crate::codec::pcm_decoder::PcmFormat::Int {
                        bits,
                        big_endian: track.codec == "A_PCM/INT/BIG",
                    }
                };
                Ok(Self::Pcm(crate::codec::pcm_decoder::PcmDecoder::new(
                    format, rate, channels,
                )?))
            }
            _ => Err(invalid(
                "selected Matroska audio codec is not owned by the export path",
            )),
        }
    }
    pub(crate) fn sample_rate(&self) -> u32 {
        match self {
            Self::Pcm(d) => d.spec().sample_rate,
            Self::Aac(d) => d.sample_rate(),
            Self::Alac { rate, .. } => *rate,
        }
    }
    pub(crate) fn channels(&self) -> u16 {
        match self {
            Self::Pcm(d) => d.spec().channels,
            Self::Aac(d) => u16::from(d.channels()),
            Self::Alac { channels, .. } => *channels,
        }
    }
    pub(crate) fn channel_mask(&self) -> Option<u32> {
        match self {Self::Aac(decoder)=>Some(decoder.channel_mask()),_=>None}
    }
    pub(crate) fn reset(&mut self) {
        if let Self::Aac(d) = self {
            d.reset();
        }
    }
    pub(crate) fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        match self {
            Self::Pcm(d) => d.decode_pcm(packet),
            Self::Aac(d) => d.decode(packet),
            Self::Alac { decoder, .. } => decoder.decode_pcm(packet),
        }
    }
}
