//! Creative Technology's ADPCM: a byte of two 4-bit codes, and nothing else - no
//! preamble, no block, no header field that says how fast the run goes.
//!
//! The coding is the Sound Blaster's own compression, and it is the plainest speech
//! coder in the K-Lite list: one predictor and one step size, carried from code to code,
//! where each code says how far and which way to move the predictor. Two things make it
//! unusual among the ADPCM variants this crate already reads. The step size is not an
//! index into a table but the table's product itself, kept as a number between 511 and
//! 32 767 and multiplied by one of eight factors per code; and the predictor is damped
//! before it is updated, so the level a code reaches depends on how far it already is
//! from silence rather than on the code alone. Both are integer arithmetic on 32-bit
//! ints, so matching the reference means matching it sample for sample.
//!
//! Two details of that arithmetic are where a port goes wrong. The eight factors are the
//! first half of the IMA table, and the reference indexes them by `nibble & 7` - the sign
//! bit picks the direction of the step and not its adaptation, so codes 8 and 0 adapt
//! identically while every IMA-shaped decoder you might copy from treats the two as
//! different rows. And the damping is `(predictor * 254) >> 8`, an arithmetic shift on a
//! possibly negative value: rounding toward minus infinity, where dividing by 256 would
//! round toward zero. One code's difference between the two is one sample, and after a
//! few hundred codes it is not.
//!
//! What the container adds is no structure at all. Measured on this build's reference
//! over the shipped Creative take, the same bytes decode to the same 524 192 samples with
//! `wBlockAlign` 1, 258 and 516, with `dwAvgBytesPerSec` stating double what the run
//! costs, with `wBitsPerSample` 4, 8 or 16, and with a `fact` chunk claiming four frames;
//! the only header field it obeys besides the format number is the channel count, and
//! that is because stereo interleaves its nibbles by channel. This file therefore reads
//! one channel, and the reader hands it the whole run cut wherever the window falls.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};

/// The reference's `ff_adpcm_AdaptationTable` cut to the eight rows this coding uses -
/// the factors are a step size's new value over 256, so 230 is a step kept at about its
/// own size and 614 is one nearly doubled.
const ADAPTATION: [i32; 8] = [230, 230, 230, 230, 307, 409, 512, 614];

/// The step's own bounds: it may never fall below a code's worth of resolution, nor
/// above the range a predictor update can use.
const STEP_MIN: i32 = 511;
const STEP_MAX: i32 = 32_767;

/// Creative ADPCM, one channel: a byte in, two samples out.
pub struct AdpcmCtDecoder {
    sample_rate: u32,
    /// The reconstructed level, kept wider than a sample and clipped on the way out.
    predictor: i32,
    /// The current step size, which is a value rather than a table index.
    step: i32,
}

impl AdpcmCtDecoder {
    /// Build a decoder for a track. `sample_rate` is the track's own and this coding
    /// reads it the way GSM does - the numbers it reconstructs are the same at any rate,
    /// and only how fast the player plays them changes.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        if channels != 1 {
            return Err(invalid(&format!(
                "Creative ADPCM reads one channel as a flat nibble run, this track names {channels}"
            )));
        }
        if sample_rate == 0 {
            return Err(invalid("Creative ADPCM track has no sample rate"));
        }
        Ok(Self::cold(sample_rate))
    }

    fn cold(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            predictor: 0,
            step: STEP_MIN,
        }
    }

    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: 1,
            format: SampleFormat::F32,
        }
    }

    /// One code: its high bit the direction, its other three the step's factor. Returns
    /// the predictor as the reference stores it, after the clip that keeps it a sample.
    fn expand(&mut self, code: u8) -> i16 {
        let negative = code & 8 != 0;
        let delta = i32::from(code & 7);
        let difference = ((2 * delta + 1) * self.step) >> 3;
        let damped = (self.predictor * 254) >> 8;
        self.predictor = if negative {
            damped - difference
        } else {
            damped + difference
        }
        .clamp(-32_768, 32_767);
        self.step = ((ADAPTATION[delta as usize] * self.step) >> 8).clamp(STEP_MIN, STEP_MAX);
        self.predictor as i16
    }
}

impl AudioDecode for AdpcmCtDecoder {
    /// A packet is any number of whole bytes, each of which is two samples, so it always
    /// comes back as one packet and the timestamps stay the caller's. The reference ends
    /// a run at the same byte: half a code has said nothing about a sample.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        let mut out = Vec::with_capacity(data.len() * 2 * 4);
        for &byte in data {
            for code in [byte >> 4, byte & 0x0F] {
                let sample = self.expand(code);
                out.extend_from_slice(&(f32::from(sample) / 32_768.0).to_le_bytes());
            }
        }
        Ok(Some(AudioPacket {
            data: out,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    /// Drop the history: the next run starts as a cold decoder does, silent and with the
    /// smallest step the coding allows.
    fn reset(&mut self) {
        *self = Self::cold(self.sample_rate);
    }
}

#[cfg(test)]
mod tests {
    use super::{AdpcmCtDecoder, STEP_MAX, STEP_MIN};
    use crate::audio::{AudioDecode, AudioStream};
    use crate::playback_wav::{Limits, WavAudioReader};

    /// The take the reference's own FATE suite ships for this coding, and the 524 192
    /// samples its whole 262 096-byte run decodes to - written by the reference itself.
    const WAV: &[u8] = include_bytes!("../../tests/fixtures/adpcm_ct/intro-partial.wav");
    const S16: &[u8] = include_bytes!("../../tests/fixtures/adpcm_ct/intro-partial.s16");
    /// The same run's first 4 096 bytes and their reference answer, for a difference a
    /// failing test can print.
    const HEAD_WAV: &[u8] = include_bytes!("../../tests/fixtures/adpcm_ct/head.wav");
    const HEAD_S16: &[u8] = include_bytes!("../../tests/fixtures/adpcm_ct/head.s16");
    /// Every byte value once, in order, as one run, and the reference's answer for the
    /// 512 samples it codes.
    const CODEBOOK_WAV: &[u8] = include_bytes!("../../tests/fixtures/adpcm_ct/codebook.wav");
    const CODEBOOK_S16: &[u8] = include_bytes!("../../tests/fixtures/adpcm_ct/codebook.s16");

    fn decode(bytes: &[u8]) -> Vec<i16> {
        let mut decoder = AdpcmCtDecoder::new(44_100, 1).expect("mono at a rate");
        let packet = decoder
            .decode_encoded(bytes, 0, 0)
            .expect("decodes")
            .expect("a byte is always two samples");
        packet
            .data
            .chunks_exact(4)
            .map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0).round()
                    as i16
            })
            .collect()
    }

    /// Reference samples, from the file libavcodec wrote for the same bytes.
    fn reference(bytes: &[u8]) -> Vec<i16> {
        bytes
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes(chunk.try_into().expect("two bytes")))
            .collect()
    }

    /// Decode through the container and the dispatch, the way the player does, and hand
    /// back whole samples.
    fn through_the_player(wav: &[u8]) -> Vec<i16> {
        let mut reader = WavAudioReader::open(wav, Limits::default()).expect("opens");
        assert_eq!(reader.codec(), "adpcm_ct");
        let mut decoder = crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("Creative ADPCM arm");
        let rate = reader.sample_rate();
        let mut heard = Vec::new();
        while let Some(packet) = reader.next_packet().expect("packet") {
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("decode")
                .expect("a byte always yields samples");
            assert_eq!((audio.timebase_num, audio.timebase_den), (1, rate));
            heard.extend(audio.data.chunks_exact(4).map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0).round()
                    as i16
            }));
        }
        heard
    }

    /// Every number asserted below is the reference's own answer for those bytes, taken
    /// from `ffmpeg -f s16le` over a one-channel, four-bit track at 44 100 Hz.
    #[test]
    fn a_byte_decodes_its_high_code_first() {
        // The cold step is 511, so code 0's difference is (2·0+1)·511/8 = 63 and its
        // second sample sees the step the first left behind - 63 then 125, not 63
        // then 126, because 230/256 of 511 clamps straight back to the floor.
        assert_eq!(decode(&[0x00]), [63, 125]);
        assert_eq!(decode(&[0x08]), [63, -1]);
        assert_eq!(decode(&[0x80]), [-63, 0]);
        // Code F is the largest step upward and the sign bit only turns it around.
        assert_eq!(decode(&[0x77]), [958, 3246]);
        assert_eq!(decode(&[0xFF]), [-958, -3247]);
    }

    #[test]
    fn a_level_run_climbs_toward_the_damping_s_equilibrium() {
        // Code 0 from a cold start gains 63 a sample at first and less as it grows,
        // because each sample is damped by two two-hundred-and-fifty-sixths of itself.
        assert_eq!(decode(&[0x00; 4]), [63, 125, 187, 248, 309, 369, 429, 488]);
        let long = decode(&[0x00; 64]);
        assert_eq!(long[long.len() - 4..], [5002, 5025, 5048, 5071]);
        for pair in long.windows(2) {
            assert!(
                pair[1] > pair[0],
                "a run of one code keeps climbing: {} then {}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(long[1] - long[0], 62);
        assert!(long[127] - long[126] < long[1] - long[0]);
    }

    #[test]
    fn the_damping_rounds_down_so_the_coding_is_not_sign_symmetric() {
        // Codes 0 and 8 from a silent predictor are exact mirrors, 63 and -63. One code
        // later they are not: `>> 8` on a negative product rounds toward minus infinity,
        // so the positive branch loses a step of its own damping downward while the
        // negative one gains one.
        assert_eq!(decode(&[0x08; 4]), [63, -1, 62, -2, 61, -3, 60, -4]);
        assert_eq!(decode(&[0x80; 4]), [-63, 0, -63, 0, -63, 0, -63, 0]);
        assert_eq!(decode(&[0x08; 64])[126..], [0, -63]);
    }

    #[test]
    fn the_predictor_is_clamped_at_both_ends_of_a_sample() {
        // Code 7 every time puts the step at its largest factor, so four codes are enough
        // to run past what a sample holds, and the clip is what the reference stores.
        assert_eq!(
            decode(&[0x77; 4]),
            [958, 3246, 8728, 21870, 32767, 32767, 32767, 32767]
        );
        assert_eq!(
            decode(&[0xFF; 4]),
            [-958, -3247, -8730, -21873, -32768, -32768, -32768, -32768]
        );
        // The clip starts at the same code on both sides, and the two bounds are not
        // mirrors of one another.
        assert_eq!(decode(&[0x77; 4])[4], 32_767);
        assert_eq!(decode(&[0xFF; 4])[4], -32_768);
    }

    #[test]
    fn the_sign_bit_moves_the_direction_and_not_the_adaptation() {
        // The eight factors are indexed by `code & 7`, so a code and its mirror leave the
        // same step behind - and from a silent predictor that sameness shows in the
        // sample exactly, with no damping to round it.
        for delta in 0..8u8 {
            let mut up = AdpcmCtDecoder::new(44_100, 1).expect("mono");
            let mut down = AdpcmCtDecoder::new(44_100, 1).expect("mono");
            let high = up.expand(delta);
            let low = down.expand(delta | 8);
            assert_eq!(high, -low, "codes {delta} and {}", delta | 8);
            assert_eq!(up.step, down.step, "codes {delta} and {}", delta | 8);
        }
        // And the step's own bounds, which no run of codes can take it past: the largest
        // factor applied forever sits at the top, the smallest at the bottom.
        let mut loud = AdpcmCtDecoder::new(44_100, 1).expect("mono");
        for _ in 0..64 {
            loud.expand(7);
        }
        assert_eq!(loud.step, STEP_MAX);
        let mut quiet = AdpcmCtDecoder::new(44_100, 1).expect("mono");
        for _ in 0..64 {
            quiet.expand(0);
        }
        assert_eq!(quiet.step, STEP_MIN);
    }

    #[test]
    fn every_byte_value_lands_on_the_reference_codebook() {
        let heard = through_the_player(CODEBOOK_WAV);
        assert_eq!(heard, reference(CODEBOOK_S16), "512 samples");
    }

    #[test]
    fn a_track_that_is_not_mono_has_no_geometry_to_decode() {
        assert!(
            AdpcmCtDecoder::new(44_100, 2)
                .err()
                .expect("a stereo Creative run is refused")
                .to_string()
                .contains("one channel")
        );
        assert!(
            AdpcmCtDecoder::new(0, 1)
                .err()
                .expect("a track with no rate is refused")
                .to_string()
                .contains("no sample rate")
        );
    }

    #[test]
    fn every_sample_of_a_real_file_lands_on_the_reference() {
        let heard = through_the_player(WAV);
        let wanted = reference(S16);
        assert_eq!(heard.len(), wanted.len());
        for (index, (from_decoder, from_ffmpeg)) in heard.iter().zip(&wanted).enumerate() {
            assert_eq!(
                from_decoder,
                from_ffmpeg,
                "sample {index} of {}: this decoder gives {from_decoder}, the reference gives {from_ffmpeg}",
                wanted.len()
            );
        }
    }

    #[test]
    fn the_head_of_the_run_lands_on_the_same_reference_samples() {
        let heard = through_the_player(HEAD_WAV);
        assert_eq!(heard, reference(HEAD_S16), "8 192 samples");
        let whole = through_the_player(WAV);
        assert_eq!(&whole[..heard.len()], &heard[..]);
    }

    #[test]
    fn the_state_carries_across_the_packets_the_container_hands_over() {
        // One window's worth of bytes at a time is the same run read whole, because this
        // coding has no block to align to: 4 096 bytes in one packet against eight packets
        // of 512 must give one answer.
        let data = &WAV[48..48 + 4_096];
        let whole = decode(data);
        let mut decoder = AdpcmCtDecoder::new(44_100, 1).expect("mono");
        let mut piecewise = Vec::new();
        for chunk in data.chunks_exact(512) {
            let packet = decoder
                .decode_encoded(chunk, 0, 0)
                .expect("decodes")
                .expect("samples");
            piecewise.extend(packet.data.chunks_exact(4).map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0).round()
                    as i16
            }));
        }
        assert_eq!(piecewise, whole);
    }

    #[test]
    fn a_seek_restarts_the_coding_from_silence() {
        let mut decoder = AdpcmCtDecoder::new(44_100, 1).expect("mono");
        let first = decoder
            .decode_encoded(&[0xFF; 8], 0, 0)
            .expect("decodes")
            .expect("samples");
        decoder.reset();
        let second = decoder
            .decode_encoded(&[0xFF; 8], 0, 0)
            .expect("decodes")
            .expect("samples");
        assert_eq!(first.data, second.data, "reset must be a cold decoder");
        // and a cold decoder is what the reference starts a run with, measured over eight
        // of the loudest codes: the predictor holds at the clip it reached at the fifth.
        assert_eq!(
            decode(&[0xFF; 8]),
            [-958, -3247, -8730, -21873]
                .into_iter()
                .chain(std::iter::repeat_n(-32_768, 12))
                .collect::<Vec<i16>>()
        );
    }
}
