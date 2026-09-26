//! symphonia decoders for the codecs whose packets need no setup of their own.
//!
//! AAC and Vorbis each carry a container-specific configuration story, so they
//! keep their own wrappers. MP3 frames and FLAC blocks are self-describing
//! enough that one wrapper covers both, and a codec that joins them later only
//! needs its symphonia ID and a tag in the container readers.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::{AudioCodecId, AudioCodecParameters, AudioDecoderOptions};
use symphonia::core::packet::Packet;
use symphonia::default::get_codecs;

/// A symphonia audio decoder plus the fixed output spec the player asked for.
pub struct SymphoniaDecoder {
    label: &'static str,
    decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    sample_rate: u32,
    channels: u16,
}

impl SymphoniaDecoder {
    /// Open `codec` for a stream at this rate and channel count. `configuration`
    /// is whatever the container stored as codec setup data, which may be empty.
    pub fn new(
        label: &'static str,
        codec: AudioCodecId,
        configuration: &[u8],
        sample_rate: u32,
        channels: u16,
    ) -> crate::Result<Self> {
        let mut codec_params = AudioCodecParameters::new();
        codec_params
            .for_codec(codec)
            .with_sample_rate(sample_rate)
            .with_channels(match channels {
                2 => {
                    use symphonia::core::audio::Position;
                    Channels::Positioned(Position::FRONT_LEFT | Position::FRONT_RIGHT)
                }
                count => Channels::Discrete(count),
            });
        if !configuration.is_empty() {
            codec_params.with_extra_data(configuration.to_vec().into_boxed_slice());
        }
        let info = get_codecs()
            .get_audio_decoder(codec)
            .ok_or_else(|| crate::invalid(&format!("{label} decoder not available")))?;
        let decoder = (info.factory)(&codec_params, &AudioDecoderOptions::default())
            .map_err(|error| crate::invalid(&format!("{label} decoder init: {error}")))?;
        Ok(Self {
            label,
            decoder,
            sample_rate,
            channels,
        })
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

impl AudioDecode for SymphoniaDecoder {
    /// Decode one packet to interleaved f32. A packet that the decoder rejects
    /// costs the frames it held, not the stream: the run continues at the next
    /// packet, which is what a damaged or truncated file asks for.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        let packet = Packet::new(0, (pts as i64).into(), duration.into(), data.to_vec());
        let audio = match self.decoder.decode(&packet) {
            Ok(buffer) => buffer,
            Err(symphonia::core::errors::Error::DecodeError(error)) => {
                eprintln!("{} decode error: {error}", self.label);
                return Ok(None);
            }
            Err(error) => return Err(crate::invalid(&format!("{} decode: {error}", self.label))),
        };
        let mut samples: Vec<f32> = Vec::with_capacity(audio.samples_interleaved());
        audio.copy_to_vec_interleaved(&mut samples);
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        Ok(Some(AudioPacket {
            data: bytes,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    fn reset(&mut self) {
        SymphoniaDecoder::reset(self);
    }
}

#[cfg(test)]
mod tests {
    use super::SymphoniaDecoder;
    use symphonia::core::codecs::audio::well_known::{CODEC_ID_FLAC, CODEC_ID_MP3};

    #[test]
    fn a_missing_decoder_is_refused_at_open() {
        // FLAC with no setup data cannot know its layout, so symphonia declines
        // to build a decoder rather than produce silence.
        let error = SymphoniaDecoder::new("FLAC", CODEC_ID_FLAC, &[], 44_100, 2)
            .err()
            .expect("FLAC needs its metadata blocks");
        assert!(error.to_string().contains("FLAC"), "{error}");
    }

    #[test]
    fn an_mp3_stream_needs_no_setup_of_its_own() {
        let decoder = SymphoniaDecoder::new("MP3", CODEC_ID_MP3, &[], 44_100, 2)
            .expect("MP3 frames describe themselves");
        assert_eq!(decoder.spec().sample_rate, 44_100);
        assert_eq!(decoder.spec().channels, 2);
    }
}
