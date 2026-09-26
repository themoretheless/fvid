//! G.722 ADPCM: one byte of code in, two samples out.
//!
//! The coding splits the band in two with a quadrature mirror filter and codes each
//! half separately: a byte carries the high band's two-bit word in its top bits and the
//! low band's six-bit word below them, which is why the 56 and 48 kbit/s variants -
//! whose low-band word is five or four bits - are the same bytes with one or two of that
//! word's own low bits left out. So both bands run their own adaptive predictor - a
//! pole section, a zero section and a quantizer step that follows the signal's own
//! level - and the two predictions are recombined and put through the synthesis
//! filter to give the pair of samples. Nothing in that depends on where a packet
//! starts as long as it starts on a code boundary, which is why the stream can be cut
//! into windows freely.
//!
//! Every arithmetic step is integer, on widths the specification fixes: the tables are
//! Q10, the predictors are clipped to defined ranges, the coefficient memories are
//! 16-bit registers, and the filter's result is shifted down by 2^11. Keeping those
//! widths and bounds reproduces the reference sample for sample.
//!
//! The state a resumed stream needs is the two bands' predictors and the last 24
//! recombined samples the synthesis filter reads, so a seek has to reset both; a
//! decoder that kept only the bands would start with silence inside the filter's
//! window and take a dozen samples to recover from it.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};

/// Samples one code byte stands for: the byte carries a codeword per band, and the
/// two bands recombine into two output samples.
pub const SAMPLES_PER_CODE: usize = 2;

/// How many past samples the synthesis filter looks back over. It is a twelve-tap
/// filter at the output's half rate, so twelve of its inputs per output are the six
/// pairs before the newest one, and the newest pair completes the other output.
const FILTER_TAPS: usize = 12;

/// The twelve QMF coefficients of ITU-T G.722 Table 11, ordered oldest input first.
/// One output takes them over the even inputs and the other over the odd ones in
/// reverse order - the two phases of a mirror filter.
const QMF: [i32; FILTER_TAPS] = [3, -11, 12, 32, -210, 951, 3876, -805, 362, -156, 53, -11];

/// Reconstruction levels of a codeword, Q10 of the band's step. The low band's table
/// is indexed by the six-bit codeword; the predictor adaptation is measured in the
/// four-bit form of the same word, so it has its own table.
const LOW_INV_QUANT: [i16; 64] = [
    -17, -17, -17, -17, -3101, -2738, -2376, -2088, -1873, -1689, -1535, -1399, -1279, -1170,
    -1072, -982, -899, -822, -750, -682, -618, -558, -501, -447, -396, -347, -300, -254, -211,
    -170, -130, -91, 3101, 2738, 2376, 2088, 1873, 1689, 1535, 1399, 1279, 1170, 1072, 982, 899,
    822, 750, 682, 618, 558, 501, 447, 396, 347, 300, 254, 211, 170, 130, 91, 54, 17, -54, -17,
];
const LOW_INV_QUANT4: [i16; 16] = [
    0, -2557, -1612, -1121, -786, -530, -323, -150, 2557, 1612, 1121, 786, 530, 323, 150, 0,
];
const HIGH_INV_QUANT: [i16; 4] = [-926, -202, 926, 202];

/// How far each codeword moves its band's log step. The high band's two entries are
/// indexed by the parity of its codeword, not its value: only the sign of the
/// reconstruction distinguishes the four.
const LOW_LOG_FACTOR_STEP: [i16; 16] = [
    -60, 3042, 1198, 538, 334, 172, 58, -30, 3042, 1198, 538, 334, 172, 58, -30, -60,
];
const HIGH_LOG_FACTOR_STEP: [i16; 2] = [798, -214];

/// `2 ^ (n / 32)` times 2048, for the fraction of a doubling a log step holds below
/// its whole part.
const INV_LOG2_TABLE: [i32; 32] = [
    2048, 2093, 2139, 2186, 2233, 2282, 2332, 2383, 2435, 2489, 2543, 2599, 2656, 2714, 2774, 2834,
    2896, 2960, 3025, 3091, 3158, 3228, 3298, 3371, 3444, 3520, 3597, 3676, 3756, 3838, 3922, 4008,
];

/// The bounds the specification states, in the widths its variables hold.
const POLE_MAX: i32 = 12288;
const POLE_LIMIT_BASE: i32 = 15360;
const LOG_FACTOR_MAX_LOW: i32 = 18432;
const LOG_FACTOR_MAX_HIGH: i32 = 22528;
/// A band's reconstructed signal is held to 15 bits signed; a filter's input to 16.
const RECONSTRUCTED_MAX: i32 = (1 << 14) - 1;
const RECONSTRUCTED_MIN: i32 = -(1 << 14);
const SAMPLE_MAX: i32 = 32767;
const SAMPLE_MIN: i32 = -32768;
/// The synthesis filter's own scale above a sample.
const FILTER_SHIFT: u32 = 11;

fn clamp(value: i32, low: i32, high: i32) -> i32 {
    value.clamp(low, high)
}

/// One band's predictor: a seventh-order zero section and a second-order pole section
/// over the partially reconstructed signal, plus the quantizer step scaling its codes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Band {
    /// The predictor's output: what the band expects the next sample to be.
    s_predictor: i16,
    /// The zero section's output, held between codes.
    s_zero: i32,
    /// Signs of the two previously reconstructed signals.
    part_reconst: [i8; 2],
    prev_qtzd_reconst: i16,
    pole_mem: [i16; 2],
    diff_mem: [i32; 6],
    zero_mem: [i16; 6],
    /// The quantizer step in 2-log form, and its linear equivalent.
    log_factor: i16,
    scale_factor: i16,
}

impl Band {
    /// A band at the start of a stream: every memory zero, the step at the value a
    /// receiver is required to begin with - wide for the low band, tight for the high
    /// one, whose codeword is only two bits.
    fn new(scale_factor: i16) -> Self {
        Self {
            s_predictor: 0,
            s_zero: 0,
            part_reconst: [0, 0],
            prev_qtzd_reconst: 0,
            pole_mem: [0, 0],
            diff_mem: [0; 6],
            zero_mem: [0; 6],
            log_factor: 0,
            scale_factor,
        }
    }

    /// The linear step a log step stands for: a power of two times a 32-part fraction
    /// of it. Below zero the log is a fraction, so the shift goes the other way.
    fn linear_scale_factor(log_factor: i32) -> i16 {
        let mantissa = INV_LOG2_TABLE[((log_factor >> 6) & 31) as usize];
        let exponent = log_factor >> 11;
        let scaled = if exponent < 0 {
            mantissa >> -exponent
        } else {
            mantissa << exponent
        };
        // The step is a 16-bit quantity in the specification's arithmetic.
        scaled as i16
    }

    /// Run the zero section over the newest difference, oldest tap first, each tap
    /// pulling the memory down one step as it goes. The sign a tap adapts to is the
    /// one between the *previous* difference and this one's, which is why the taps
    /// run in sequence over the live memory rather than over a copy of it.
    fn update_zero_section(&mut self, cur_diff: i32) {
        let mut s_zero = 0i32;
        for k in (0..6).rev() {
            let tmp = if k == 0 {
                cur_diff * 2
            } else {
                self.diff_mem[k - 1]
            };
            // A difference of zero freezes the adaptation instead of driving it with
            // a sign: a loud rest and a silence have to stay distinguishable.
            let adapted = ((i32::from(self.zero_mem[k]) * 255) >> 8)
                + if cur_diff == 0 {
                    0
                } else if (self.diff_mem[k] ^ cur_diff) < 0 {
                    -128
                } else {
                    128
                };
            // The coefficient memory is a 16-bit register, so an adaptation past full
            // scale wraps into it rather than saturating.
            self.zero_mem[k] = adapted as i16;
            self.diff_mem[k] = tmp;
            s_zero = s_zero.wrapping_add((tmp.wrapping_mul(i32::from(self.zero_mem[k]))) >> 15);
        }
        self.s_zero = s_zero;
    }

    /// Adapt both filter sections to a reconstructed difference and predict again.
    fn predict(&mut self, cur_diff: i32) {
        let cur_part_reconst = i32::from(self.s_zero + cur_diff < 0);
        let sg0 = if cur_part_reconst != i32::from(self.part_reconst[0]) {
            1
        } else {
            -1
        };
        let sg1 = if cur_part_reconst == i32::from(self.part_reconst[1]) {
            1
        } else {
            -1
        };
        self.part_reconst[1] = self.part_reconst[0];
        self.part_reconst[0] = cur_part_reconst as i8;

        self.pole_mem[1] = clamp(
            ((sg0 * clamp(i32::from(self.pole_mem[0]), -8191, 8191)) >> 5)
                + sg1 * 128
                + ((i32::from(self.pole_mem[1]) * 127) >> 7),
            -POLE_MAX,
            POLE_MAX,
        ) as i16;
        let limit = POLE_LIMIT_BASE - i32::from(self.pole_mem[1]);
        self.pole_mem[0] = clamp(
            -192 * sg0 + ((i32::from(self.pole_mem[0]) * 255) >> 8),
            -limit,
            limit,
        ) as i16;

        self.update_zero_section(cur_diff);

        let qtzd_reconst = clamp(
            (i32::from(self.s_predictor) + cur_diff) * 2,
            SAMPLE_MIN,
            SAMPLE_MAX,
        ) as i16;
        self.s_predictor = clamp(
            self.s_zero
                + ((i32::from(self.pole_mem[0]) * i32::from(qtzd_reconst)) >> 15)
                + ((i32::from(self.pole_mem[1]) * i32::from(self.prev_qtzd_reconst)) >> 15),
            SAMPLE_MIN,
            SAMPLE_MAX,
        ) as i16;
        self.prev_qtzd_reconst = qtzd_reconst;
    }

    /// Consume a four-bit low-band codeword: reconstruct, adapt, and predict the next.
    fn update_low(&mut self, ilow: usize) {
        let cur_diff = (i32::from(self.scale_factor) * i32::from(LOW_INV_QUANT4[ilow])) >> 10;
        self.predict(cur_diff);
        self.log_factor = clamp(
            ((i32::from(self.log_factor) * 127) >> 7) + i32::from(LOW_LOG_FACTOR_STEP[ilow]),
            0,
            LOG_FACTOR_MAX_LOW,
        ) as i16;
        self.scale_factor = Self::linear_scale_factor(i32::from(self.log_factor) - (8 << 11));
    }

    /// The same for the high band, which is adapted by its codeword's parity and
    /// predicted from a step measured against ten doublings rather than eight.
    fn update_high(&mut self, dhigh: i32, ihigh: usize) {
        self.predict(dhigh);
        self.log_factor = clamp(
            ((i32::from(self.log_factor) * 127) >> 7) + i32::from(HIGH_LOG_FACTOR_STEP[ihigh & 1]),
            0,
            LOG_FACTOR_MAX_HIGH,
        ) as i16;
        self.scale_factor = Self::linear_scale_factor(i32::from(self.log_factor) - (10 << 11));
    }
}

/// G.722 decoder: the two bands' predictors and the synthesis filter's window.
pub struct G722Decoder {
    low: Band,
    high: Band,
    /// The last twelve code pairs, oldest first. The window starts zeroed, which is
    /// what a cold receiver's memory holds, and the newest two entries are the pair
    /// the current code just formed.
    window: [i16; FILTER_TAPS * 2],
    sample_rate: u32,
    channels: u16,
}

impl G722Decoder {
    /// G.722 codes one channel. A Wave file can state a channel count for it anyway,
    /// and no writer this build has measured agrees on how more than one stream would
    /// be laid out, so a multi-channel track is refused rather than guessed at.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        if channels != 1 {
            return Err(invalid(&format!(
                "G.722 codes one channel, this track names {channels}"
            )));
        }
        if sample_rate == 0 {
            return Err(invalid("G.722 track has no sample rate"));
        }
        Ok(Self::cold(sample_rate))
    }

    fn cold(sample_rate: u32) -> Self {
        Self {
            low: Band::new(8),
            high: Band::new(2),
            window: [0; FILTER_TAPS * 2],
            sample_rate,
            channels: 1,
        }
    }

    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: self.channels,
            format: SampleFormat::F32,
        }
    }

    /// Add the newest pair to the filter's window and read the two output samples off
    /// it: the even taps with the table as written, the odd taps with it reversed, the
    /// two being the mirror halves of the band the coding started from.
    fn synthesize(&mut self, rlow: i32, rhigh: i32) -> [i32; 2] {
        self.window.copy_within(2.., 0);
        self.window[FILTER_TAPS * 2 - 2] = (rlow + rhigh) as i16;
        self.window[FILTER_TAPS * 2 - 1] = (rlow - rhigh) as i16;
        let mut out = [0i32; 2];
        for tap in 0..FILTER_TAPS {
            // Each product is under 2^30 and the twelve of them together a fifth of
            // the range, so the sum needs no wider word than the reference's `int`.
            out[1] += i32::from(self.window[2 * tap]) * QMF[tap];
            out[0] += i32::from(self.window[2 * tap + 1]) * QMF[FILTER_TAPS - 1 - tap];
        }
        out
    }
}

impl AudioDecode for G722Decoder {
    /// Every code byte yields two samples, so a packet always comes back a packet and
    /// the timestamps stay the caller's.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        let mut out = Vec::with_capacity(data.len() * SAMPLES_PER_CODE * 4);
        for &code in data {
            // The byte is packed from the top down: the high band's two bits first,
            // then the low band's six.
            let ihigh = usize::from(code >> 6);
            let ilow = usize::from(code & 0x3f);
            let rlow = clamp(
                ((i32::from(self.low.scale_factor) * i32::from(LOW_INV_QUANT[ilow])) >> 10)
                    + i32::from(self.low.s_predictor),
                RECONSTRUCTED_MIN,
                RECONSTRUCTED_MAX,
            );
            self.low.update_low(ilow >> 2);
            let dhigh =
                (i32::from(self.high.scale_factor) * i32::from(HIGH_INV_QUANT[ihigh])) >> 10;
            let rhigh = clamp(
                dhigh + i32::from(self.high.s_predictor),
                RECONSTRUCTED_MIN,
                RECONSTRUCTED_MAX,
            );
            self.high.update_high(dhigh, ihigh);
            for sample in self.synthesize(rlow, rhigh) {
                // The filter runs 2^11 above a sample and can land past full scale,
                // so the shift is where the clipping belongs.
                let scaled = f32::from(clamp(sample >> FILTER_SHIFT, SAMPLE_MIN, SAMPLE_MAX) as i16)
                    / 32768.0;
                out.extend_from_slice(&scaled.to_le_bytes());
            }
        }
        Ok(Some(AudioPacket {
            data: out,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    /// Drop both bands' memories and the filter's window: the next stream starts as a
    /// cold receiver does.
    fn reset(&mut self) {
        *self = Self::cold(self.sample_rate);
    }
}

#[cfg(test)]
mod tests {
    use super::{G722Decoder, SAMPLES_PER_CODE};
    use crate::audio::{AudioDecode, AudioStream, SampleFormat};
    use crate::playback_wav::{Limits, WavAudioReader};

    /// Decode `data` as one packet and hand back the samples as f32.
    fn decode_with(decoder: &mut G722Decoder, data: &[u8]) -> Vec<f32> {
        let packet = decoder
            .decode_encoded(data, 0, 0)
            .expect("decode")
            .expect("every code yields samples");
        assert_eq!((packet.timebase_num, packet.timebase_den), (1, 16_000));
        packet
            .data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four bytes")))
            .collect()
    }

    fn decode(data: &[u8]) -> Vec<f32> {
        decode_with(
            &mut G722Decoder::new(16_000, 1).expect("mono at a rate"),
            data,
        )
    }

    /// The same run as whole numbers, which is how the reference decoder states them.
    fn samples(data: &[u8]) -> Vec<i16> {
        decode(data)
            .iter()
            .map(|sample| (sample * 32768.0) as i16)
            .collect()
    }

    /// What the player does with one of these files: read the windows the container
    /// hands out and decode them as one stream, sample after sample.
    fn through_the_player(wav: &[u8]) -> Vec<f32> {
        let mut reader = WavAudioReader::open(wav, Limits::default()).expect("opens");
        assert_eq!(reader.codec(), "adpcm_g722");
        let mut decoder = crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("G.722 arm");
        let rate = reader.sample_rate();
        let mut heard = Vec::new();
        while let Some(packet) = reader.next_packet().expect("packet") {
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("decode")
                .expect("codes always yield samples");
            assert_eq!((audio.timebase_num, audio.timebase_den), (1, rate));
            heard.extend(
                audio
                    .data
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four bytes"))),
            );
        }
        heard
    }

    /// The reference's own bytes: `ffmpeg -i NAME.wav -f s16le NAME.s16`.
    fn reference(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(2)
            .map(|chunk| {
                f32::from(i16::from_le_bytes(chunk.try_into().expect("two bytes"))) / 32_768.0
            })
            .collect()
    }

    #[test]
    fn a_code_byte_is_always_two_samples() {
        for length in [1usize, 2, 7, 64, 512] {
            let data: Vec<u8> = (0..length as u32)
                .map(|index| (index as u8).wrapping_mul(37))
                .collect();
            assert_eq!(
                decode(&data).len(),
                length * SAMPLES_PER_CODE,
                "{length} codes"
            );
        }
    }

    /// The first nine codes, as the reference decoder of the same bytes reports them.
    /// Both bands' cold-start steps, the filter's zeroed window and the quantizer
    /// adaptation all show up here: these nine hold the high band at its first codeword
    /// while the low band walks its first nine words, and the fourth is the first one
    /// wide enough to move the low band's own step.
    #[test]
    fn a_stream_starts_where_the_reference_starts() {
        let codes: Vec<u8> = (0..9).collect();
        assert_eq!(
            samples(&codes),
            vec![0, 0, -1, -1, 0, 0, 0, -1, -1, 0, 0, -6, 2, -6, 1, -9, 6, -7]
        );
    }

    /// Sample for sample against this machine's FFmpeg, over four files: the whole
    /// 256-code codebook in the order a reader meets it, which puts every codeword of
    /// both bands through one running state; two tones, one long enough to reach the
    /// player as four windows of its own sizing, so the run is checked across the
    /// boundaries a reader puts into it as well as in one call, and the other coded at
    /// half the rate its header states, which a decoder that trusted the coding's
    /// nominal rate over the container would get wrong; and a run of one code repeated,
    /// the idle channel, which drives both bands' steps to their ceilings and the
    /// filter's output onto its clip rails - the most state this coding can be put in.
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=16000:duration=0.4 \
    ///        -c:a g722 -ar 16000 tone-16k.wav
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=0.1 \
    ///        -c:a g722 -ar 8000 tone-8k.wav
    /// ffmpeg -i NAME.wav -f s16le NAME.s16      # the reference for each, alike
    /// ```
    ///
    /// The codebook and the idle run are written from the format rather than by an
    /// encoder: their `data` chunks are the bytes 0x00 through 0xFF and four hundred
    /// zeroes, and their `fact` states the samples that follow them.
    #[test]
    fn every_sample_of_a_real_file_lands_on_the_references_bits() {
        for (name, wav, want) in [
            (
                "the whole codebook",
                include_bytes!("../../tests/fixtures/g722/codebook.wav") as &[u8],
                include_bytes!("../../tests/fixtures/g722/codebook.s16") as &[u8],
            ),
            (
                "a 440 Hz tone at 16 kHz",
                include_bytes!("../../tests/fixtures/g722/tone-16k.wav") as &[u8],
                include_bytes!("../../tests/fixtures/g722/tone-16k.s16") as &[u8],
            ),
            (
                "a 440 Hz tone at 8 kHz",
                include_bytes!("../../tests/fixtures/g722/tone-8k.wav") as &[u8],
                include_bytes!("../../tests/fixtures/g722/tone-8k.s16") as &[u8],
            ),
            (
                "four hundred repetitions of one code",
                include_bytes!("../../tests/fixtures/g722/idle-run.wav") as &[u8],
                include_bytes!("../../tests/fixtures/g722/idle-run.s16") as &[u8],
            ),
        ] {
            let got = through_the_player(wav);
            let want = reference(want);
            assert_eq!(got.len(), want.len(), "{name}");
            for (index, (got, want)) in got.iter().zip(&want).enumerate() {
                assert_eq!(
                    got.to_bits(),
                    want.to_bits(),
                    "{name} sample {index}: this decoder gives {got}, the reference gives {want}"
                );
            }
        }
    }

    #[test]
    fn a_split_stream_decodes_as_one_stream_does() {
        let codes: Vec<u8> = (0..=255).collect();
        let whole = decode(&codes);
        let mut decoder = G722Decoder::new(16_000, 1).expect("mono at a rate");
        let mut split = Vec::new();
        for chunk in codes.chunks(97) {
            split.extend(decode_with(&mut decoder, chunk));
        }
        assert_eq!(whole, split);
    }

    #[test]
    fn reset_forgets_every_memory_the_stream_left_behind() {
        let codes: Vec<u8> = (0..=255).collect();
        let mut decoder = G722Decoder::new(16_000, 1).expect("mono at a rate");
        decoder.decode_encoded(&codes, 0, 0).expect("decode");
        decoder.reset();
        assert_eq!(decode_with(&mut decoder, &codes), decode(&codes));
    }

    /// A repeated zero code is the stream's own runaway, not its silence: both bands
    /// take it as a step down from their cold steps, their log steps wind up, and the
    /// output is on the clip rails from the 49th code on. This is G.722's predictor
    /// loop behaving as specified - the reference numbers the same 800 samples alike -
    /// and the state a decoder can least fake: every memory of both bands, and the
    /// filter's window, are past their ceilings by then.
    #[test]
    fn an_all_zero_stream_runs_away_the_way_the_reference_does() {
        let heard = samples(&[0u8; 400]);
        assert_eq!(
            &heard[..24],
            [
                0, 0, -1, -1, 0, 0, 0, -1, -1, 0, 0, -5, 2, -7, 2, -8, 5, -13, 11, -19, 17, -23,
                21, -30
            ],
            "the first twelve codes, as the reference numbers them"
        );
        assert_eq!(
            heard.iter().position(|sample| *sample == i16::MAX),
            Some(96)
        );
        assert_eq!(
            heard.iter().position(|sample| *sample == i16::MIN),
            Some(97)
        );
        let clipped = heard
            .iter()
            .filter(|sample| **sample == i16::MAX || **sample == i16::MIN)
            .count();
        assert_eq!(clipped, 354);
    }

    /// A stream that asks for the extremes: every code, xor-ed so the run sweeps both
    /// bands' range repeatedly. No sample may leave the scale the pipeline reads f32
    /// at, and the extremes have to be reached for that to be the filter's doing.
    #[test]
    fn output_stays_inside_the_scale_the_samples_are_read_at() {
        let codes: Vec<u8> = (0..=255)
            .cycle()
            .take(4096)
            .map(|code| code ^ 0x5a)
            .collect();
        let heard = samples(&codes);
        assert!(decode(&codes).iter().all(|s| (-1.0..=1.0).contains(s)));
        assert!(
            heard.iter().any(|s| *s == i16::MIN),
            "nothing reached the clip the filter states"
        );
    }

    #[test]
    fn a_second_channel_is_refused_rather_than_guessed() {
        let error = match G722Decoder::new(16_000, 2) {
            Ok(_) => panic!("stereo has no convention here"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("one channel"), "{error}");
        assert!(G722Decoder::new(0, 1).is_err());
    }

    #[test]
    fn the_spec_says_what_the_packets_carry() {
        let decoder = G722Decoder::new(8_000, 1).expect("mono at a rate");
        assert_eq!(decoder.spec().sample_rate, 8_000);
        assert_eq!(decoder.spec().channels, 1);
        assert_eq!(decoder.spec().format, SampleFormat::F32);
    }
}
