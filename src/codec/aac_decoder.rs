//! AAC decoder using symphonia.
//!
//! The decoder wraps symphonia's AAC decoder and exposes a simple interface:
//! push encoded packets, receive decoded PCM samples.

use crate::audio::{AudioPacket, AudioSpec, SampleFormat};
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::{
    AudioCodecParameters, AudioDecoderOptions, well_known::CODEC_ID_AAC,
};
use symphonia::core::packet::Packet;
use symphonia::default::get_codecs;

/// AAC decoder state.
pub struct AacDecoder {
    decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    sample_rate: u32,
    channels: u16,
    failed: bool,
}

impl AacDecoder {
    /// Create a new AAC decoder from an MP4 `esds` sample-entry configuration.
    pub fn new(configuration: &[u8], sample_rate: u32, channels: u16) -> crate::Result<Self> {
        // symphonia parses `extra_data` as a bare AudioSpecificConfig, so the
        // descriptor wrapper has to come off before the decoder is built.
        let asc = crate::codec::config::aac_specific_config(configuration)?;
        let parsed = crate::codec::config::AacConfig::parse(asc)?;
        if parsed.sample_rate != sample_rate || u16::from(parsed.channels) != channels {
            return Err(crate::invalid(
                "AAC configuration disagrees with container sample rate or channels",
            ));
        }

        let mut codec_params = AudioCodecParameters::new();
        codec_params
            .for_codec(CODEC_ID_AAC)
            .with_sample_rate(sample_rate);

        let channels_enum = match channels {
            1 => Channels::Discrete(1),
            2 => {
                use symphonia::core::audio::Position;
                Channels::Positioned(Position::FRONT_LEFT | Position::FRONT_RIGHT)
            }
            n => Channels::Discrete(n),
        };
        codec_params.with_channels(channels_enum);

        codec_params.with_extra_data(asc.to_vec().into_boxed_slice());

        let codec_registry = get_codecs();
        let decoder_info = codec_registry
            .get_audio_decoder(CODEC_ID_AAC)
            .ok_or_else(|| crate::invalid("AAC decoder not available"))?;

        let decoder = (decoder_info.factory)(&codec_params, &AudioDecoderOptions::default())
            .map_err(|e| crate::invalid(&format!("AAC decoder init: {e}")))?;

        Ok(Self {
            decoder,
            sample_rate,
            channels,
            failed: false,
        })
    }

    /// Decode an AAC packet and return PCM samples as interleaved f32.
    pub fn decode(
        &mut self,
        packet_data: &[u8],
        pts: u64,
        duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        if self.failed {
            return Err(crate::invalid("AAC decoder requires reset after an error"));
        }
        let timestamp = i64::try_from(pts).map_err(|_| crate::invalid("AAC timestamp overflow"))?;
        let packet = Packet::new(0, timestamp.into(), duration.into(), packet_data.to_vec());

        let audio_buf = match self.decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(e) => {
                self.failed = true;
                return Err(crate::invalid(&format!("AAC decode: {e}")));
            }
        };

        let num_samples = audio_buf.samples_interleaved();
        let mut samples: Vec<f32> = Vec::with_capacity(num_samples);
        audio_buf.copy_to_vec_interleaved(&mut samples);

        let sample_bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();

        Ok(Some(AudioPacket {
            data: sample_bytes,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    /// Current audio specification (sample rate, channels).
    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: self.channels,
            format: SampleFormat::F32,
        }
    }

    /// Reset decoder state (e.g., after a seek).
    pub fn reset(&mut self) {
        self.decoder.reset();
        self.failed = false;
    }
}

impl crate::audio::AudioDecode for AacDecoder {
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        AacDecoder::decode(self, data, pts, duration)
    }

    fn reset(&mut self) {
        AacDecoder::reset(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn decoder() -> AacDecoder {
        let esds = crate::playback_aac::esds_for(&[0x12, 0x10]).unwrap();
        AacDecoder::new(&esds, 44_100, 2).unwrap()
    }
    #[test]
    fn damaged_packet_is_an_error_and_requires_reset() {
        let mut decoder = decoder();
        assert!(decoder.decode(&[0], 0, 1024).is_err());
        let error = decoder.decode(&[0], 1024, 1024).err().unwrap();
        assert!(error.to_string().contains("requires reset"));
        decoder.reset();
        let error = decoder.decode(&[0], 0, 1024).err().unwrap();
        assert!(error.to_string().contains("AAC decode:"));
    }
    #[test]
    fn timestamp_overflow_does_not_wrap_or_poison_codec_state() {
        let mut decoder = decoder();
        let error = decoder.decode(&[0], u64::MAX, 1024).err().unwrap();
        assert!(error.to_string().contains("timestamp overflow"));
        let error = decoder.decode(&[0], 0, 1024).err().unwrap();
        assert!(error.to_string().contains("AAC decode:"));
    }
    #[test]
    fn container_and_aac_config_must_describe_the_same_pcm() {
        let esds = crate::playback_aac::esds_for(&[0x12, 0x10]).unwrap();
        for (rate, channels) in [(48_000, 2), (44_100, 1), (0, 2), (44_100, 0)] {
            let error = AacDecoder::new(&esds, rate, channels).err().unwrap();
            assert!(error.to_string().contains("disagrees with container"));
        }
        let accepted = AacDecoder::new(&esds, 44_100, 2).unwrap();
        assert_eq!(accepted.spec().sample_rate, 44_100);
        assert_eq!(accepted.spec().channels, 2);
    }
}
