//! Vorbis decoder using symphonia.
//!
//! The decoder wraps symphonia's Vorbis decoder and exposes a simple interface:
//! push encoded packets, receive decoded PCM samples.

use crate::audio::{AudioPacket, AudioSpec, SampleFormat};
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoderOptions, well_known::CODEC_ID_VORBIS};
use symphonia::core::packet::Packet;
use symphonia::default::get_codecs;

/// Vorbis decoder state.
pub struct VorbisDecoder {
    decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    sample_rate: u32,
    channels: u16,
}

impl VorbisDecoder {
    /// Create a new Vorbis decoder from the track's configuration.
    pub fn new(configuration: &[u8], sample_rate: u32, channels: u16) -> crate::Result<Self> {
        let mut codec_params = AudioCodecParameters::new();
        codec_params
            .for_codec(CODEC_ID_VORBIS)
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

        if !configuration.is_empty() {
            codec_params.with_extra_data(configuration.to_vec().into_boxed_slice());
        }

        let codec_registry = get_codecs();
        let decoder_info = codec_registry
            .get_audio_decoder(CODEC_ID_VORBIS)
            .ok_or_else(|| crate::invalid("Vorbis decoder not available"))?;

        let decoder = (decoder_info.factory)(&codec_params, &AudioDecoderOptions::default())
            .map_err(|e| crate::invalid(&format!("Vorbis decoder init: {e}")))?;

        Ok(Self {
            decoder,
            sample_rate,
            channels,
        })
    }

    /// Decode a Vorbis packet and return PCM samples as interleaved f32.
    pub fn decode(&mut self, packet_data: &[u8], pts: u64, duration: u64) -> crate::Result<Option<AudioPacket>> {
        let packet = Packet::new(0, (pts as i64).into(), duration.into(), packet_data.to_vec());

        let audio_buf = match self.decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(symphonia::core::errors::Error::DecodeError(e)) => {
                eprintln!("Vorbis decode error: {e}");
                return Ok(None);
            }
            Err(e) => return Err(crate::invalid(&format!("Vorbis decode: {e}"))),
        };

        let num_samples = audio_buf.samples_interleaved();
        let mut samples: Vec<f32> = Vec::with_capacity(num_samples);
        audio_buf.copy_to_vec_interleaved(&mut samples);

        let sample_bytes: Vec<u8> = samples
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();

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
    }
}

impl crate::audio::AudioDecode for VorbisDecoder {
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        VorbisDecoder::decode(self, data, pts, duration)
    }

    fn reset(&mut self) {
        VorbisDecoder::reset(self);
    }
}
