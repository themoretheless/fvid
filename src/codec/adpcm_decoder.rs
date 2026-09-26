//! ADPCM in the three spellings QuickTime containers carry it in: Microsoft's
//! (`ms\0\x02`), IMA in a WAV-shaped block (`ms\0\x11`) and Apple's IMA
//! (`ima4`). All three are 4-bit differential codes grouped into blocks, each of
//! which carries its own predictor, so a block decodes the same wherever it sits
//! in the stream - which is what lets a packet of several blocks be handed over
//! one block at a time.
//!
//! The arithmetic belongs to symphonia; what this file settles is the geometry
//! the container has to state and symphonia has to be told. A block's frame
//! count follows from its byte length with the header bytes taken off, and only
//! Microsoft's two tags state a block length at all (in the `WAVEFORMATEX` the
//! container stores as this codec's setup data). Apple's block is fixed by its
//! own format, so it needs no setup data and a track that carries some is read
//! just the same.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};
use symphonia::core::audio::{Channels, Position};
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_ADPCM_IMA_QT, CODEC_ID_ADPCM_IMA_WAV, CODEC_ID_ADPCM_MS,
};
use symphonia::core::codecs::audio::{AudioCodecId, AudioCodecParameters, AudioDecoderOptions};
use symphonia::core::packet::Packet;
use symphonia::default::get_codecs;

/// Bytes one channel's block header takes, and how many of the block's frames
/// those bytes already hold as whole samples: Microsoft's preamble stores two
/// past samples beside its predictor index, IMA's one previous sample.
fn preamble(tag: AudioCodecId) -> Option<(u64, u64)> {
    if tag == CODEC_ID_ADPCM_MS {
        Some((7, 2))
    } else if tag == CODEC_ID_ADPCM_IMA_WAV {
        Some((4, 1))
    } else {
        None
    }
}

/// Frames in one block of `tag`'s coding, from the length the container states
/// for a block. Bits per sample stay out of the arithmetic because every one of
/// these tags codes at 4 bits; a `WAVEFORMATEX` that says otherwise describes a
/// coding this decoder does not have, and the caller hears that as `None`.
fn frames_of(tag: AudioCodecId, block_bytes: u64, channels: u64, bits: u64) -> Option<u64> {
    if bits != 4 || channels == 0 || block_bytes == 0 {
        return None;
    }
    if tag == CODEC_ID_ADPCM_IMA_QT {
        // Apple's block is two header bytes and 32 bytes of codes per channel,
        // which is 64 frames however long the packet the file packs them into is.
        return (block_bytes == 34 * channels).then_some(64);
    }
    let (per_channel, leading) = preamble(tag)?;
    let header = per_channel * channels;
    if block_bytes < header {
        return None;
    }
    // What is left after the header is one 4-bit code per frame for every channel
    // at once, so `channels * 4` bits of code stand between two frames.
    Some(8 * (block_bytes - header) / (4 * channels) + leading)
}

/// Frames one block of `codec` holds, asked in the words a container reader has:
/// the dispatch name is what a demuxer ends up holding after it maps the file's
/// own numbering, and the number it needs is the one the decoder will divide its
/// bytes by, so both readers of the geometry get it from the same arithmetic.
/// A name outside the ADPCM codings states nothing this function answers.
pub fn block_frames(codec: &str, block_bytes: u64, channels: u64, bits: u64) -> Option<u64> {
    let tag = match codec {
        "adpcm_ms" => CODEC_ID_ADPCM_MS,
        "adpcm_ima_wav" => CODEC_ID_ADPCM_IMA_WAV,
        _ => return None,
    };
    frames_of(tag, block_bytes, channels, bits)
}

/// The `WAVEFORMATEX` a Microsoft-spelled ADPCM entry stores as its setup data:
/// `nBlockAlign` at byte 12 and `wBitsPerSample` at 14, both little-endian, which
/// is where the structure puts them. Anything shorter states no geometry.
fn wave_geometry(extra: &[u8]) -> Option<(u64, u64)> {
    let block = extra
        .get(12..14)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))?;
    let bits = extra
        .get(14..16)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))?;
    Some((u64::from(block), u64::from(bits)))
}

/// Frames to hand on from a packet that holds `held` and was declared to hold
/// `declared`. An encoder pads its last block to the block size, so the bytes
/// state a length the audio never had, and the track's own timing is what says
/// how much of it was sound; the reference decoder ends the track there too. A
/// declaration of nothing leaves the block geometry in charge, and one longer
/// than the bytes is a table this decoder does not invent samples for.
fn frames_to_keep(declared: u64, held: u64) -> u64 {
    if declared == 0 {
        held
    } else {
        declared.min(held)
    }
}

/// ADPCM decoder: hands symphonia one whole block at a time and concatenates the
/// f32 frames, which is the interleaved form the pipeline carries.
pub struct AdpcmDecoder {
    label: &'static str,
    decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    sample_rate: u32,
    channels: u16,
    block_bytes: usize,
    block_frames: u64,
}

impl AdpcmDecoder {
    /// Open `codec` for a stream at this rate and channel count. `configuration`
    /// is the entry's setup data, which the two Microsoft tags carry and Apple's
    /// does not.
    pub fn new(
        label: &'static str,
        codec: AudioCodecId,
        configuration: &[u8],
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self> {
        let channels64 = u64::from(channels);
        let (block_bytes, bits) = if codec == CODEC_ID_ADPCM_IMA_QT {
            (34 * channels64, 4)
        } else {
            let (block, bits) = wave_geometry(configuration).ok_or_else(|| {
                invalid(&format!(
                    "{label} needs a WAVEFORMATEX to know its block size"
                ))
            })?;
            (block, bits)
        };
        let frames = frames_of(codec, block_bytes, channels64, bits).ok_or_else(|| {
            invalid(&format!(
                "{label} block of {block_bytes} bytes is not a geometry this decoder reads"
            ))
        })?;

        let mut codec_params = AudioCodecParameters::new();
        codec_params
            .for_codec(codec)
            .with_sample_rate(sample_rate)
            .with_max_frames_per_packet(frames)
            .with_frames_per_block(frames)
            .with_channels(match channels {
                2 => Channels::Positioned(Position::FRONT_LEFT | Position::FRONT_RIGHT),
                count => Channels::Discrete(count),
            });
        if !configuration.is_empty() {
            codec_params.with_extra_data(configuration.to_vec().into_boxed_slice());
        }
        let info = get_codecs()
            .get_audio_decoder(codec)
            .ok_or_else(|| invalid(&format!("{label} decoder not available")))?;
        let decoder = (info.factory)(&codec_params, &AudioDecoderOptions::default())
            .map_err(|error| invalid(&format!("{label} decoder init: {error}")))?;
        Ok(Self {
            label,
            decoder,
            sample_rate,
            channels,
            block_bytes: block_bytes as usize,
            block_frames: frames,
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
}

impl AudioDecode for AdpcmDecoder {
    /// Decode every whole block the packet's bytes hold, up to the frames the
    /// container declared for the packet. A tail too short for a block is left
    /// out rather than read past, which is what the reference decoder does with a
    /// truncated one too, so a damaged file costs the frames its bytes do not
    /// hold and nothing else.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let blocks = data.len() / self.block_bytes;
        if blocks == 0 {
            return Ok(None);
        }
        let keep = frames_to_keep(duration, blocks as u64 * self.block_frames);
        // The blocks the kept frames need, which stops short of the padded tail
        // rather than decoding samples the file then throws away.
        let wanted = keep.div_ceil(self.block_frames) as usize;
        let blocks = blocks.min(wanted);
        let frames = blocks * self.block_frames as usize * usize::from(self.channels);
        let mut samples: Vec<f32> = Vec::with_capacity(frames);
        for index in 0..blocks {
            let start = index * self.block_bytes;
            let block = &data[start..start + self.block_bytes];
            let packet = Packet::new(
                0,
                (pts as i64).into(),
                self.block_frames.into(),
                block.to_vec(),
            );
            let audio = match self.decoder.decode(&packet) {
                Ok(buffer) => buffer,
                Err(symphonia::core::errors::Error::DecodeError(error)) => {
                    eprintln!("{} decode error: {error}", self.label);
                    return Ok(None);
                }
                Err(error) => {
                    return Err(invalid(&format!("{} decode: {error}", self.label)));
                }
            };
            audio.copy_to_vec_interleaved(&mut samples);
        }
        samples.truncate(keep as usize * usize::from(self.channels));
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

    /// Blocks carry their own predictors, so there is no state between them and
    /// nothing a seek has to undo.
    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::{AdpcmDecoder, block_frames, frames_of, frames_to_keep, wave_geometry};
    use symphonia::core::codecs::audio::well_known::{
        CODEC_ID_ADPCM_IMA_QT, CODEC_ID_ADPCM_IMA_WAV, CODEC_ID_ADPCM_MS,
    };

    #[test]
    fn a_wav_shaped_ima_block_holds_its_predictor_and_its_codes() {
        // 1024 bytes, of which the first 4 are the mono block's header.
        assert_eq!(frames_of(CODEC_ID_ADPCM_IMA_WAV, 1024, 1, 4), Some(2041));
        // Two channels share the codes, so the same block length halves the
        // frames it holds while its header doubles.
        assert_eq!(frames_of(CODEC_ID_ADPCM_IMA_WAV, 2048, 2, 4), Some(2041));
    }

    #[test]
    fn a_microsoft_block_counts_its_two_stored_samples() {
        assert_eq!(frames_of(CODEC_ID_ADPCM_MS, 1024, 1, 4), Some(2036));
        assert_eq!(frames_of(CODEC_ID_ADPCM_MS, 2048, 2, 4), Some(2036));
    }

    #[test]
    fn apples_block_is_the_one_its_format_fixes() {
        assert_eq!(frames_of(CODEC_ID_ADPCM_IMA_QT, 34, 1, 4), Some(64));
        assert_eq!(frames_of(CODEC_ID_ADPCM_IMA_QT, 68, 2, 4), Some(64));
        // A length that is not Apple's own is some other coding, and guessing at
        // it would play the bytes as noise.
        assert_eq!(frames_of(CODEC_ID_ADPCM_IMA_QT, 32, 1, 4), None);
    }

    #[test]
    fn a_coding_wider_than_four_bits_is_refused_not_guessed_at() {
        assert_eq!(frames_of(CODEC_ID_ADPCM_MS, 1024, 1, 8), None);
        assert_eq!(frames_of(CODEC_ID_ADPCM_MS, 6, 1, 4), None);
    }

    #[test]
    fn the_dispatch_names_ask_the_same_arithmetic_as_the_ids() {
        // A container reader has the name the decoder was picked by, and its
        // block must be divided by exactly the number the decoder divides by.
        assert_eq!(block_frames("adpcm_ms", 1024, 1, 4), Some(2036));
        assert_eq!(block_frames("adpcm_ima_wav", 2048, 2, 4), Some(2041));
        // Apple's coding is not spelled by a dispatch name a reader uses here,
        // and a name outside these is not a question this answers by guessing.
        assert_eq!(block_frames("adpcm_ima_qt", 34, 1, 4), None);
        assert_eq!(block_frames("pcm_s16le", 1024, 1, 4), None);
    }

    #[test]
    fn a_padded_last_block_stops_where_the_track_says_it_does() {
        // A block holds 2041 frames and the encoder padded the last one out; the
        // packet's timing says only 1877 of them were ever sound.
        assert_eq!(frames_to_keep(1877, 2041), 1877);
        // Timing that states the whole block, or nothing at all, leaves the bytes
        // in charge.
        assert_eq!(frames_to_keep(2041, 2041), 2041);
        assert_eq!(frames_to_keep(0, 2041), 2041);
        // A declaration longer than the packet's bytes cannot add frames the
        // coding does not hold.
        assert_eq!(frames_to_keep(4096, 2041), 2041);
    }

    #[test]
    fn the_wave_format_reads_its_geometry_where_the_structure_puts_it() {
        // The header a muxer writes for 1024-byte, 4-bit mono blocks.
        let extra = [
            0x11, 0x00, 0x01, 0x00, 0x40, 0x1f, 0x00, 0x00, 0xad, 0x0f, 0x00, 0x00, 0x00, 0x04,
            0x04, 0x00, 0x02, 0x00,
        ];
        assert_eq!(wave_geometry(&extra), Some((1024, 4)));
        assert_eq!(wave_geometry(&extra[..14]), None);
    }

    #[test]
    fn a_microsoft_tag_without_setup_data_has_no_block_to_read() {
        let error = AdpcmDecoder::new("ADPCM IMA WAV", CODEC_ID_ADPCM_IMA_WAV, &[], 8000, 1)
            .err()
            .expect("the block size comes from the entry, not the guess of a reader");
        assert!(error.to_string().contains("WAVEFORMATEX"), "{error}");
    }

    #[test]
    fn apple_ima_needs_no_setup_of_its_own() {
        let decoder = AdpcmDecoder::new("ADPCM IMA QT", CODEC_ID_ADPCM_IMA_QT, &[], 8000, 1)
            .expect("Apple's block is fixed by the format");
        assert_eq!(decoder.spec().sample_rate, 8000);
        assert_eq!(decoder.block_frames, 64);
        assert_eq!(decoder.block_bytes, 34);
    }
}
