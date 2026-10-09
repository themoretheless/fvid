//! Owned packet-to-PCM codecs used by the owned container audio timelines.
use crate::{Result, container::mp4::Track, invalid};
pub(crate) enum NativeAudioCheckpoint {
    Aac(crate::codec::aac_native::AacCheckpoint),
    Ps(crate::codec::aac_ps_native::Checkpoint),
}
pub(crate) enum PacketPcmDecoder {
    Pcm(crate::codec::pcm_decoder::PcmDecoder),
    Aac(crate::codec::aac_native::NativeAacDecoder),
    Ps(Box<crate::codec::aac_ps_native::NativePsAacDecoder>),
    Alac {
        decoder: crate::codec::alac_decoder::AlacDecoder,
        rate: u32,
        channels: u16,
    },
}
impl PacketPcmDecoder {
    pub(crate) fn checkpoint(&self) -> Option<NativeAudioCheckpoint> {
        match self {
            Self::Aac(d) => Some(NativeAudioCheckpoint::Aac(d.checkpoint())),
            Self::Ps(d) => Some(NativeAudioCheckpoint::Ps(d.checkpoint())),
            _ => None,
        }
    }
    pub(crate) fn restore_checkpoint(&mut self, state: &NativeAudioCheckpoint) -> Result<bool> {
        match (self, state) {
            (Self::Aac(d), NativeAudioCheckpoint::Aac(s)) => {
                d.restore(s)?;
                Ok(true)
            }
            (Self::Ps(d), NativeAudioCheckpoint::Ps(s)) => {
                d.restore(s)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(crate) const SAMPLE_BYTES: usize = 4;
    pub(crate) fn with_in_band_ps(track: &Track) -> Result<Self> {
        Ok(Self::Ps(Box::new(crate::codec::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(
            crate::codec::config::aac_specific_config(&track.configuration)?, track.sample_rate,
        ).map_err(|e| invalid(&e.0))?)))
    }

    pub(crate) fn new(track: &Track) -> Result<Self> {
        match &track.codec {
            b"mp4a" => {
                let asc = crate::codec::config::aac_specific_config(&track.configuration)?;
                if crate::codec::config::AudioSpecificConfig::parse(asc)?.ps_present == Some(true) {
                    Ok(Self::Ps(Box::new(
                        crate::codec::aac_ps_native::NativePsAacDecoder::new(asc)?,
                    )))
                } else {
                    Ok(Self::Aac(
                        crate::codec::aac_native::NativeAacDecoder::new_with_output_rate(
                            asc,
                            track.sample_rate,
                        )?,
                    ))
                }
            }
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
            "A_AAC" => {
                let config =
                    crate::codec::config::AudioSpecificConfig::parse(&track.codec_private)?;
                if config.ps_present == Some(true) {
                    Ok(Self::Ps(Box::new(
                        crate::codec::aac_ps_native::NativePsAacDecoder::new(&track.codec_private)?,
                    )))
                } else {
                    Ok(Self::Aac(
                        crate::codec::aac_native::NativeAacDecoder::new_with_output_rate(
                            &track.codec_private,
                            rate,
                        )?,
                    ))
                }
            }
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
            Self::Ps(d) => d.sample_rate(),
            Self::Alac { rate, .. } => *rate,
        }
    }
    pub(crate) fn channels(&self) -> u16 {
        match self {
            Self::Pcm(d) => d.spec().channels,
            Self::Aac(d) => u16::from(d.channels()),
            Self::Ps(d) => u16::from(d.channels()),
            Self::Alac { channels, .. } => *channels,
        }
    }
    pub(crate) fn channel_mask(&self) -> Option<u32> {
        match self {
            Self::Aac(decoder) => Some(decoder.channel_mask()),
            Self::Ps(decoder) => Some(decoder.channel_mask()),
            _ => None,
        }
    }
    pub(crate) fn reset(&mut self) {
        match self {
            Self::Aac(d) => d.reset(),
            Self::Ps(d) => d.reset(),
            _ => {}
        }
    }
    pub(crate) fn delayed(&self) -> bool {
        matches!(self, Self::Ps(_)) || matches!(self,Self::Aac(d) if d.delayed())
    }
    pub(crate) fn decode_timed(&mut self,packet:&[u8],pts:u64,duration:u64)->Result<Option<Vec<f32>>> {
        match self {
            Self::Aac(d)=>Ok(d.decode_timed(packet,i64::try_from(pts).map_err(|_|invalid("AAC timestamp overflow"))?,duration)?.map(|f|f.samples)),
            _=>self.decode_delayed(packet),
        }
    }
    pub(crate) fn decode_delayed(&mut self, packet: &[u8]) -> Result<Option<Vec<f32>>> {
        match self {
            Self::Ps(d) => Ok(d.decode(packet)?.map(|f| f.pcm)),
            Self::Aac(d) => Ok(d.decode_timed(packet,0,u64::from(d.core_frame_samples()))?.map(|f|f.samples)),
            _ => Ok(Some(self.decode(packet)?)),
        }
    }
    pub(crate) fn finish_delayed(&mut self) -> Result<Option<Vec<f32>>> {
        match self {
            Self::Ps(d) => Ok(d.finish()?.map(|f| f.pcm)),
            Self::Aac(d) => Ok(d.finish()?.map(|f|f.samples)),
            _ => Ok(None),
        }
    }
    pub(crate) fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        match self {
            Self::Pcm(d) => d.decode_pcm(packet),
            Self::Aac(d) => d.decode(packet),
            Self::Ps(_) => Err(invalid("PS PCM requires delayed decode and EOF drain")),
            Self::Alac { decoder, .. } => decoder.decode_pcm(packet),
        }
    }
}
