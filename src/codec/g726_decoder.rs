//! G.726 ADPCM: one code of two to five bits in, one sample out.
//!
//! The coding is what G.722's two bands are made of, run once instead of twice: a code
//! stands for a difference, a second-order pole and a sixth-order zero section predict
//! the next sample from the differences and reconstructions before it, and a quantizer
//! step follows the signal's own level. Four rates come out of that one arithmetic -
//! two, three, four or five bits per code, which at the 8 kHz the format is named for is
//! 16, 24, 32 or 40 kbit/s - and they differ only in which of four table sets a code
//! indexes and in how many of a byte's bits it takes.
//!
//! What makes it fiddly is not the predictor but the arithmetic it is written in. The
//! specification states its products in an 11-bit floating form - one sign bit, four of
//! exponent, six of mantissa - and the reference implements exactly that, so every
//! product of two coefficients goes through a quantising round trip rather than a plain
//! multiply. That form is narrow by construction: a mantissa lands between 32 and 63 and
//! an exponent between 0 and 17, which is what keeps a difference inside sixteen bits
//! with no clipping anywhere. It also means a product leaves the multiply as a 16-bit
//! quantity, so a wide one wraps into that width before it is summed. Matching the
//! reference sample for sample means wrapping where it wraps rather than widening the
//! arithmetic to look tidy.
//!
//! Four truncations carry that lesson, each pinned by a test: the step a code asks for
//! comes back as sixteen bits, so a wide one wraps into that width; a product of two of
//! these floats is returned as sixteen bits; the reconstructed sample is held to sixteen
//! bits before the predictor looks at it; and the answer that reaches the stream is four
//! times that - past a sample's range whenever the signal is loud - and is cut back to
//! sixteen bits again, wrapping rather than saturating at the last step.
//!
//! Codes are read most-significant-bit first, so a byte holds the two codes of a 32
//! kbit/s stream with the earlier one on top, the same way G.722 packs its two bands. A
//! code may straddle a byte - which happens at 24 and 40 kbit/s, where three and five
//! bits do not divide a byte - so the reader works in bit positions rather than bytes.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};

/// Samples one code stands for. Unlike G.722, whose byte codes a pair of samples, a
/// G.726 code is one sample.
pub const SAMPLES_PER_CODE: usize = 1;

/// The narrowest and widest code a G.726 stream may state, in bits.
const CODE_SIZE_MIN: u32 = 2;
const CODE_SIZE_MAX: u32 = 5;

/// A code's reconstruction step in Q10-ish log form, the step's own magnitude change,
/// and how far the magnitude average moves: three tables indexed by the whole code, sign
/// bit included, so the two ends of each are the largest steps down and up.
#[derive(Clone, Copy, Debug)]
struct Tables {
    iquant: &'static [i16],
    w: &'static [i16],
    f: &'static [u8],
}

/// What a code that cannot be inverted states in the reconstruction table.
const NO_STEP: i16 = i16::MIN;

/// 16 kbit/s, two bits a code.
const TABLES_2: Tables = Tables {
    iquant: &[116, 365, 365, 116],
    w: &[-22, 439, 439, -22],
    f: &[0, 7, 7, 0],
};
/// 24 kbit/s, three bits a code.
const TABLES_3: Tables = Tables {
    iquant: &[NO_STEP, 135, 273, 373, 373, 273, 135, NO_STEP],
    w: &[-4, 30, 137, 582, 582, 137, 30, -4],
    f: &[0, 1, 2, 7, 7, 2, 1, 0],
};
/// 32 kbit/s, four bits a code - the rate the format is usually carried at, and the one
/// this build's encoder writes by default.
const TABLES_4: Tables = Tables {
    iquant: &[
        NO_STEP, 4, 135, 213, 273, 323, 373, 425, 425, 373, 323, 273, 213, 135, 4, NO_STEP,
    ],
    w: &[
        -12, 18, 41, 64, 112, 198, 355, 1122, 1122, 355, 198, 112, 64, 41, 18, -12,
    ],
    f: &[0, 0, 0, 1, 1, 1, 3, 7, 7, 3, 1, 1, 1, 0, 0, 0],
};
/// 40 kbit/s, five bits a code.
const TABLES_5: Tables = Tables {
    iquant: &[
        NO_STEP, -66, 28, 104, 169, 224, 274, 318, 358, 395, 429, 459, 488, 514, 539, 566, 566,
        539, 514, 488, 459, 429, 395, 358, 318, 274, 224, 169, 104, 28, -66, NO_STEP,
    ],
    w: &[
        14, 14, 24, 39, 40, 41, 58, 100, 141, 179, 219, 280, 358, 440, 529, 696, 696, 529, 440,
        358, 280, 219, 179, 141, 100, 58, 41, 40, 39, 24, 14, 14,
    ],
    f: &[
        0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 3, 4, 5, 6, 6, 6, 6, 5, 4, 3, 2, 1, 1, 1, 1, 1, 0, 0, 0,
        0, 0,
    ],
};

fn tables(code_size: u32) -> Tables {
    match code_size {
        2 => TABLES_2,
        3 => TABLES_3,
        4 => TABLES_4,
        _ => TABLES_5,
    }
}

/// The bounds the reference states, in the widths its variables hold.
const POLE_MAX: i32 = 12288;
const POLE_LIMIT_BASE: i32 = 15360;
/// A first-order coefficient's adaptation is cut at nine signed bits: the clamp keeps a
/// value's magnitude inside two to the eighth, so its range is -256 through 255.
const ADAPTATION_MAX: i32 = 255;
const ADAPTATION_MIN: i32 = -256;
/// The step's two ends: the tightest it may go and the widest.
const STEP_MIN: i32 = 544;
const STEP_MAX: i32 = 5120;
/// A pole this far negative says the signal is ringing, which is what the tone detector
/// reports and what makes the next code's step control jump to full.
const TONE_LIMIT: i32 = -11776;
/// A step no larger than this counts as small enough to push the step control up.
const STEP_SMALL: i32 = 1535;
/// Where the slow step's logarithm and fraction are read from, and the scale a product
/// of two floats is held at.
const SLOW_STEP_WHOLE: u32 = 15;
const SLOW_STEP_FRACTION: u32 = 10;
const PRODUCT_SCALE: i32 = 19;
/// How much of the tone threshold the reference takes, and where the step control and the
/// two magnitude averages sit.
const STEP_CONTROL_MAX: i32 = 256;
const STEP_CONTROL_SHIFT: u32 = 6;

fn clamp(value: i32, low: i32, high: i32) -> i32 {
    value.clamp(low, high)
}

/// The reference's sign function, which calls zero positive - and which is only ever
/// asked about a value the caller has already proved nonzero, except through its negation
/// of a stored sign, where `-0` is the way a code of no difference says so.
fn sign(value: i32) -> i32 {
    if value < 0 { -1 } else { 1 }
}

/// The 11-bit float the specification's arithmetic is written in: a sign, a four-bit
/// exponent and a six-bit mantissa, standing for `mantissa * 2 ^ (exponent - 6)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Float11 {
    sign: i32,
    exponent: i32,
    mantissa: i32,
}

impl Float11 {
    const ZERO: Self = Self {
        sign: 0,
        exponent: 0,
        mantissa: 1 << 5,
    };

    /// Take a value apart into the form. The exponent counts the value's width rather
    /// than its highest bit, so a power of two sits one above its own logarithm and the
    /// mantissa is left with six bits that start at 32.
    fn assign(&mut self, value: i32) {
        let magnitude = value.wrapping_abs();
        self.sign = i32::from(value < 0);
        if magnitude == 0 {
            self.exponent = 0;
            self.mantissa = 1 << 5;
            return;
        }
        self.exponent = (31 - magnitude.leading_zeros()) as i32 + 1;
        self.mantissa = (magnitude << 6) >> self.exponent;
    }

    /// The product of two of these, quantised back through the same form and returned as
    /// the sixteen bits the reference's own result type holds.
    fn product(&self, other: &Self) -> i32 {
        let exponent = self.exponent + other.exponent;
        let scaled = (self.mantissa * other.mantissa + 0x30) >> 4;
        let widened = if exponent > PRODUCT_SCALE {
            scaled << (exponent - PRODUCT_SCALE)
        } else {
            scaled >> (PRODUCT_SCALE - exponent)
        };
        let signed = if self.sign ^ other.sign != 0 {
            -widened
        } else {
            widened
        };
        signed as i16 as i32
    }
}

/// The predictor: a second-order pole and a sixth-order zero section over the signal's
/// own reconstruction and differences, and the step its codes are measured against.
#[derive(Clone, Copy, Debug)]
struct Predictor {
    /// The two last reconstructed samples, newest first, in the form the pole
    /// coefficients are multiplied through.
    sr: [Float11; 2],
    /// The six last differences a code stood for, newest first.
    dq: [Float11; 6],
    /// The pole section's coefficients, and the signs of the two last partial
    /// reconstructions it adapts to.
    a: [i32; 2],
    pk: [i32; 2],
    /// The zero section's coefficients.
    b: [i32; 6],
    /// The step in its three forms: the fast and slow averages, the control that decides
    /// how much the fast one counts, and the linear step the next code measures against.
    yu: i32,
    yl: i32,
    ap: i32,
    y: i32,
    /// The short and long averages of the magnitude, whose disagreement is what the step
    /// control reacts to.
    dms: i32,
    dml: i32,
    /// Whether the signal is ringing, and the prediction for the next code: the whole
    /// estimate, and its pole half alone, which is what the adaptation reads.
    td: i32,
    se: i32,
    sez: i32,
}

impl Predictor {
    /// A receiver's starting state: nothing in the memories, the step at the value the
    /// specification says to begin with, and both averages of the magnitude at zero.
    fn new() -> Self {
        Self {
            sr: [Float11::ZERO; 2],
            dq: [Float11::ZERO; 6],
            a: [0; 2],
            pk: [1; 2],
            b: [0; 6],
            yu: STEP_MIN,
            yl: 34_816,
            ap: 0,
            y: STEP_MIN,
            dms: 0,
            dml: 0,
            td: 0,
            se: 0,
            sez: 0,
        }
    }

    /// Consume one code and give back the sample it reconstructs.
    fn decode(&mut self, code: usize, code_size: u32, tables: Tables) -> i16 {
        let sign_bit = (code as i32) >> (code_size - 1);
        // A code's step is its table entry moved up by the current step, and read back
        // out of the log form that entry is held in. Below full scale the entry is the
        // sentinel, and a step that has come out at zero is no difference at all. The
        // entry is returned as sixteen bits, so a wide one wraps into that width.
        let sum = i32::from(tables.iquant[code]) + (self.y >> 2);
        let mut difference = if sum < 0 {
            0
        } else {
            let exponent = (sum >> 7) & 0xf;
            let mantissa = (1 << 7) + (sum & 0x7f);
            ((mantissa << exponent) >> 7) as i16 as i32
        };

        // The ringing test measures the difference against a fraction of the slow step,
        // in the step's own log form: five bits of mantissa over the whole part of it.
        let whole = self.yl >> SLOW_STEP_WHOLE;
        let fraction = (self.yl >> SLOW_STEP_FRACTION) & 0x1f;
        let threshold = if whole > 9 {
            0x1f << 10
        } else {
            (0x20 + fraction) << whole
        };
        let ringing = self.td == 1 && difference > (3 * threshold >> 2);

        if sign_bit != 0 {
            difference = -difference;
        }
        // The reconstruction is what the code asked for, added to the prediction, and it
        // is held to a sample's width before anything else - including itself - looks at
        // it.
        let reconstructed = (self.se + difference) as i16 as i32;

        let pk0 = if self.sez + difference != 0 {
            sign(self.sez + difference)
        } else {
            0
        };
        let dq0 = if difference != 0 { sign(difference) } else { 0 };

        if ringing {
            // A ring this strong is the predictor's own doing, so both sections are
            // thrown away rather than adapted once more.
            self.a = [0; 2];
            self.b = [0; 6];
        } else {
            let adaptation = clamp(
                (-self.a[0] * self.pk[0] * pk0) >> 5,
                ADAPTATION_MIN,
                ADAPTATION_MAX,
            );
            self.a[1] = clamp(
                self.a[1] + 128 * pk0 * self.pk[1] + adaptation - (self.a[1] >> 7),
                -POLE_MAX,
                POLE_MAX,
            );
            // The first coefficient's own limit closes as the second one fills: together
            // the two may not reach past the range the format states.
            let limit = POLE_LIMIT_BASE - self.a[1];
            self.a[0] = clamp(
                self.a[0] + 192 * pk0 * self.pk[0] - (self.a[0] >> 8),
                -limit,
                limit,
            );
            for k in 0..6 {
                // Each tap follows the sign *between* the difference stored there and the
                // one arriving now, which is why the memories are read before they shift.
                self.b[k] += 128 * dq0 * sign(-self.dq[k].sign) - (self.b[k] >> 8);
            }
        }

        self.pk[1] = self.pk[0];
        self.pk[0] = if pk0 != 0 { pk0 } else { 1 };
        self.sr[1] = self.sr[0];
        self.sr[0].assign(reconstructed);
        for k in (1..6).rev() {
            self.dq[k] = self.dq[k - 1];
        }
        self.dq[0].assign(difference);
        // A difference of no magnitude keeps its code's sign anyway, which is the one
        // thing the memory holds that the value itself cannot say.
        self.dq[0].sign = sign_bit;

        self.td = i32::from(self.a[1] < TONE_LIMIT);

        self.dms += (i32::from(tables.f[code]) << 4) + ((-self.dms) >> 5);
        self.dml += (i32::from(tables.f[code]) << 4) + ((-self.dml) >> 7);
        if ringing {
            self.ap = STEP_CONTROL_MAX;
        } else {
            self.ap += (-self.ap) >> 4;
            if self.y <= STEP_SMALL
                || self.td == 1
                || ((self.dms << 2) - self.dml).abs() >= (self.dml >> 3)
            {
                self.ap += 0x20;
            }
        }

        self.yu = clamp(
            self.y + i32::from(tables.w[code]) + ((-self.y) >> 5),
            STEP_MIN,
            STEP_MAX,
        );
        self.yl += self.yu + ((-self.yl) >> 6);
        let control = if self.ap >= STEP_CONTROL_MAX {
            1 << STEP_CONTROL_SHIFT
        } else {
            self.ap >> 2
        };
        self.y = (self.yl + (self.yu - (self.yl >> 6)) * control) >> 6;

        // Both sections are re-read through the 11-bit form, the zero section first so the
        // pole half of the estimate can be handed to the next code's adaptation alone.
        let mut se = 0;
        let mut factor = Float11::ZERO;
        for k in 0..6 {
            factor.assign(self.b[k] >> 2);
            se += factor.product(&self.dq[k]);
        }
        self.sez = se >> 1;
        for k in 0..2 {
            factor.assign(self.a[k] >> 2);
            se += factor.product(&self.sr[k]);
        }
        self.se = se >> 1;

        // The sample the stream carries is four times the reconstruction the predictor
        // worked at - the scale the coding itself never states - and that product is cut
        // back to sixteen bits by wrapping, as the reference's own result type does.
        clamp(reconstructed * 4, -0xffff, 0xffff) as i16
    }
}

/// G.726 decoder: the predictor, and the width its codes are read at.
pub struct G726Decoder {
    predictor: Predictor,
    tables: Tables,
    code_size: usize,
    sample_rate: u32,
    channels: u16,
}

impl G726Decoder {
    /// G.726 codes one channel: the reference asks for a patch when named more, and no
    /// container this build has measured lays two streams out in a way a reader could
    /// infer, so a multi-channel track is refused rather than guessed at.
    ///
    /// `code_size` is a code's width in bits, which is what separates the four rates -
    /// and, measured, is the width a Wave file states in its byte rate rather than in the
    /// field named for bit depth.
    pub fn new(sample_rate: u32, channels: u16, code_size: u32) -> Result<Self> {
        if channels != 1 {
            return Err(invalid(&format!(
                "G.726 codes one channel, this track names {channels}"
            )));
        }
        if sample_rate == 0 {
            return Err(invalid("G.726 track has no sample rate"));
        }
        if !(CODE_SIZE_MIN..=CODE_SIZE_MAX).contains(&code_size) {
            return Err(invalid(&format!(
                "G.726 codes are two to five bits, this track says {code_size}"
            )));
        }
        Ok(Self::cold(sample_rate, code_size))
    }

    fn cold(sample_rate: u32, code_size: u32) -> Self {
        Self {
            predictor: Predictor::new(),
            tables: tables(code_size),
            code_size: code_size as usize,
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

    /// The width of a code, which is the rate's whole story.
    pub fn code_size(&self) -> u32 {
        u32::try_from(self.code_size).unwrap_or(CODE_SIZE_MAX)
    }
}

impl AudioDecode for G726Decoder {
    /// A packet is a whole number of codes, so it always comes back as one packet and the
    /// timestamps stay the caller's. Bits that cannot fill a code are left over: the
    /// reference stops at the same place, and a stream that ends mid-code has said nothing
    /// about the sample it half-codes.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        let mut out = Vec::with_capacity(data.len() * 8 / self.code_size * 4);
        let mask = (1u32 << self.code_size) - 1;
        let mut position = 0usize;
        while position + self.code_size <= data.len() * 8 {
            // Top bit first, across the byte boundary when the code's width does not
            // divide eight; the next byte stands in as zeros where the run has none.
            let byte = position / 8;
            let offset = (position % 8) as u32;
            let pair = u32::from(data[byte]) << 8 | u32::from(*data.get(byte + 1).unwrap_or(&0));
            let code = ((pair >> (16 - offset - self.code_size as u32)) & mask) as usize;
            position += self.code_size;
            let sample = self
                .predictor
                .decode(code, self.code_size as u32, self.tables);
            out.extend_from_slice(&(f32::from(sample) / 32768.0).to_le_bytes());
        }
        Ok(Some(AudioPacket {
            data: out,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    /// Drop the predictor: the next stream starts as a cold receiver does.
    fn reset(&mut self) {
        *self = Self::cold(self.sample_rate, self.code_size as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::{CODE_SIZE_MAX, CODE_SIZE_MIN, G726Decoder, SAMPLES_PER_CODE};
    use crate::audio::{AudioDecode, AudioStream};
    use crate::playback_wav::{Limits, WavAudioReader};

    /// Lay codes out most-significant-bit first, the way a stream carries them.
    fn pack(codes: &[usize], code_size: u32) -> Vec<u8> {
        let mut bits = Vec::with_capacity(codes.len() * code_size as usize);
        for code in codes {
            for shift in (0..code_size).rev() {
                bits.push(u8::try_from((code >> shift) & 1).expect("one bit"));
            }
        }
        while bits.len() % 8 != 0 {
            bits.push(0);
        }
        bits.chunks_exact(8)
            .map(|byte| {
                byte.iter().fold(0u8, |built, bit| {
                    (built << 1) | u8::try_from(*bit == 1).unwrap()
                })
            })
            .collect()
    }

    fn decode(data: &[u8], code_size: u32) -> Vec<f32> {
        let mut decoder = G726Decoder::new(8_000, 1, code_size).expect("mono at a rate");
        let packet = decoder
            .decode_encoded(data, 0, 0)
            .expect("decode")
            .expect("codes always yield samples");
        assert_eq!((packet.timebase_num, packet.timebase_den), (1, 8_000));
        packet
            .data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four bytes")))
            .collect()
    }

    /// The same run as whole numbers, which is how the reference states them.
    fn samples(codes: &[usize], code_size: u32) -> Vec<i16> {
        decode(&pack(codes, code_size), code_size)
            .iter()
            .map(|sample| (sample * 32768.0) as i16)
            .collect()
    }

    /// What the player does with one of these files: read the windows the container hands
    /// out and decode them as one stream, sample after sample.
    fn through_the_player(wav: &[u8]) -> Vec<f32> {
        let mut reader = WavAudioReader::open(wav, Limits::default()).expect("opens");
        assert_eq!(reader.codec(), "adpcm_g726");
        let mut decoder = crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("G.726 arm");
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

    /// A whole code is the smallest thing a stream can say: eight bits hold four, two or
    /// one codes depending on the width, and the leftover bits of a run that does not end
    /// on a code boundary stand for nothing.
    #[test]
    fn a_code_is_always_one_sample() {
        for code_size in CODE_SIZE_MIN..=CODE_SIZE_MAX {
            for bytes in [1usize, 2, 3, 5, 7, 64, 512] {
                let data: Vec<u8> = (0..bytes)
                    .map(|index| (index as u8).wrapping_mul(37))
                    .collect();
                assert_eq!(
                    decode(&data, code_size).len(),
                    bytes * 8 / code_size as usize * SAMPLES_PER_CODE,
                    "{bytes} bytes at {code_size} bits"
                );
            }
        }
    }

    /// The first cycle of every code in ascending order, on a cold receiver, as the
    /// reference decoder of the same codes reports it. Each width's own table is reached
    /// here, and the shape the numbers take says what the tables say: a code's top bit is
    /// its sign, so the second half of every cycle comes out negative; the four-bit
    /// cycle's first and last codes are the two ends of the inverse table, which state no
    /// step at all, and the last of them lands on -236 rather than 0 because a code of no
    /// step still carries the prediction the earlier ones left behind. At five bits the
    /// cycle is wide enough to start the predictor ringing inside its own first
    /// thirty-two codes.
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i sine=440:sample_rate=8000:duration=0.4 \
    ///        -c:a g726 -code_size N -strict -2 probe.wav
    /// ffmpeg -i probe.wav -f s16le probe.s16      # the same numbers
    /// ```
    #[test]
    fn a_stream_starts_where_the_reference_starts() {
        for (code_size, want) in [
            (2, vec![12, 60, -68, -20]),
            (3, vec![0, 16, 36, 64, -96, -72, -32, 0]),
            (
                4,
                vec![
                    0, 8, 16, 24, 36, 52, 84, 144, -396, -920, -976, -972, -812, -644, -444, -236,
                ],
            ),
            (
                5,
                vec![
                    0, 4, 8, 12, 20, 28, 40, 52, 76, 112, 160, 264, 436, 764, 1452, 3152, -4800,
                    -12368, -20876, -31668, 24932, 18344, 14500, 14168, 16768, 20128, 26192, 32056,
                    -27288, -21324, -15932, -11008,
                ],
            ),
        ] {
            let codes: Vec<usize> = (0..1usize << code_size).collect();
            assert_eq!(samples(&codes, code_size), want, "{code_size}-bit codes");
        }
    }

    /// One code, held steady: the step it asks for is small but always in the same
    /// direction, so the predictor rings harder with every sample - and where the number
    /// it hands out passes a sample's range the stream carries the wrapped one, not a
    /// clamped one. The reference's 20th sample of this run is -27 488, which is the
    /// 38 048 the coding computed read back through sixteen bits; the four-bit run below
    /// does the same at its 12th code. After the ring peaks the tone detector throws both
    /// sections away and the output collapses to -1 within four codes.
    #[test]
    fn a_steady_code_rings_past_a_samples_range_and_wraps() {
        assert_eq!(
            samples(&[1; 24], 2),
            vec![
                60, 64, 72, 80, 100, 124, 152, 196, 264, 360, 556, 920, 1484, 2440, 4040, 6364,
                10132, 16152, 24136, -27488, -14488, -5168, -1, -1
            ]
        );
        assert_eq!(
            samples(&[7; 16], 4)[10..],
            [14620, -18256, -8712, -1, -1, -1]
        );
    }

    /// The end codes of the four- and five-bit tables state no step at all, so a stream
    /// of them leaves the reconstruction at the prediction: the reference holds its output
    /// at zero through sixteen of them, which is the one case where a code says nothing
    /// and the predictor keeps walking.
    #[test]
    fn a_code_with_no_step_moves_nothing_of_its_own() {
        assert_eq!(samples(&[15; 16], 4), vec![0; 16]);
        assert_eq!(samples(&[0; 16], 4), vec![0; 16]);
    }

    /// How the run is grouped into packets is the container's business, not the coding's:
    /// the same stream split at every block boundary - three bytes at 24 kbit/s, five at
    /// 40, where a code straddles the byte and the split lands between codes - gives the
    /// same samples as one call.
    #[test]
    fn a_stream_split_on_block_boundaries_decodes_as_one_stream_does() {
        for code_size in CODE_SIZE_MIN..=CODE_SIZE_MAX {
            // The block widths the format states, and the same run cut at thirteen of them.
            let block = [1, 3, 1, 5][(code_size - 2) as usize] * 13;
            let codes: Vec<usize> = (0..600).map(|index| index * 7 % (1 << code_size)).collect();
            let data = pack(&codes, code_size);
            let whole = decode(&data, code_size);
            let mut decoder = G726Decoder::new(8_000, 1, code_size).expect("mono at a rate");
            let mut split = Vec::new();
            for chunk in data.chunks(block) {
                split.extend(
                    decoder
                        .decode_encoded(chunk, 0, 0)
                        .expect("decode")
                        .expect("samples")
                        .data
                        .chunks_exact(4)
                        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four bytes"))),
                );
            }
            assert_eq!(whole, split, "{code_size} bits a code");
        }
    }

    /// A packet's own bits end where a code ends, and the reference stops there too: it
    /// takes a packet's sample count as its byte length times eight over the code width,
    /// so of seven bytes at 24 kbit/s the last two bits are read as nothing. The Wave
    /// reader never hands out such a packet - a block of this coding is always a whole
    /// number of codes - which is what keeps the two readings of a ragged run apart.
    #[test]
    fn bits_that_cannot_fill_a_code_are_read_as_nothing() {
        for (code_size, bytes, want) in
            [(2u32, 7usize, 28usize), (3, 7, 18), (4, 7, 14), (5, 7, 11)]
        {
            let data = vec![0x5au8; bytes];
            assert_eq!(
                decode(&data, code_size).len(),
                want,
                "{bytes} bytes at {code_size} bits"
            );
        }
    }

    #[test]
    fn reset_forgets_every_memory_the_stream_left_behind() {
        for code_size in CODE_SIZE_MIN..=CODE_SIZE_MAX {
            let codes: Vec<usize> = (0..200)
                .map(|index| index * 13 % (1 << code_size))
                .collect();
            let data = pack(&codes, code_size);
            let mut decoder = G726Decoder::new(8_000, 1, code_size).expect("mono at a rate");
            decoder.decode_encoded(&data, 0, 0).expect("decode");
            decoder.reset();
            assert_eq!(
                decode(&data, code_size),
                decoder
                    .decode_encoded(&data, 0, 0)
                    .expect("decode")
                    .expect("samples")
                    .data
                    .chunks_exact(4)
                    .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four bytes")))
                    .collect::<Vec<f32>>(),
                "{code_size} bits a code"
            );
        }
    }

    /// A track that names two channels says something this coding cannot mean, and a code
    /// outside two to five bits is not this coding at all: the reference's decoder asks
    /// for a patch on the first and refuses the second, and the numbers a Wave states are
    /// read before either is reached.
    #[test]
    fn a_geometry_the_coding_does_not_have_is_refused() {
        for (channels, want) in [(2u16, "one channel"), (0, "one channel")] {
            let error = G726Decoder::new(8_000, channels, 4)
                .err()
                .expect("not mono");
            assert!(error.to_string().contains(want), "{error}");
        }
        for code_size in [0, 1, 6, 8, 16] {
            let error = G726Decoder::new(8_000, 1, code_size)
                .err()
                .expect("not a G.726 width");
            assert!(
                error.to_string().contains("two to five bits"),
                "code size {code_size}: {error}"
            );
        }
        assert!(
            G726Decoder::new(0, 1, 4).is_err(),
            "no rate to time the run by"
        );
    }

    /// Sample for sample against this machine's FFmpeg, over eight files: a codebook per
    /// width, which cycles every code through one running state often enough to reach past
    /// the window the reader hands out, and a tone per width as this build's encoder
    /// writes it, which is where the headers this player reads - a byte rate that states
    /// the code width, a bit-depth field that does not, a `fact` that agrees with neither
    /// of them - come from a muxer rather than from here.
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=0.4 \
    ///        -c:a g726 -code_size N -strict -2 tone-Nb.wav
    /// ffmpeg -i NAME.wav -f s16le NAME.s16      # the reference for each, alike
    /// ```
    ///
    /// The codebooks are written from the format rather than by an encoder: their `data`
    /// chunks are the codes 0, 1, … 2^N-1 cycling, packed most-significant-bit first, and
    /// their `fact` states the frames that follow.
    #[test]
    fn every_sample_of_a_real_file_lands_on_the_references_bits() {
        for code_size in [2u32, 3, 4, 5] {
            for kind in ["codebook", "tone"] {
                let name = format!("{kind}-{code_size}b");
                let wav: &[u8] = match (kind, code_size) {
                    ("codebook", 2) => include_bytes!("../../tests/fixtures/g726/codebook-2b.wav"),
                    ("codebook", 3) => include_bytes!("../../tests/fixtures/g726/codebook-3b.wav"),
                    ("codebook", 4) => include_bytes!("../../tests/fixtures/g726/codebook-4b.wav"),
                    ("codebook", 5) => include_bytes!("../../tests/fixtures/g726/codebook-5b.wav"),
                    ("tone", 2) => include_bytes!("../../tests/fixtures/g726/tone-2b.wav"),
                    ("tone", 3) => include_bytes!("../../tests/fixtures/g726/tone-3b.wav"),
                    ("tone", 4) => include_bytes!("../../tests/fixtures/g726/tone-4b.wav"),
                    _ => include_bytes!("../../tests/fixtures/g726/tone-5b.wav"),
                };
                let s16: &[u8] = match (kind, code_size) {
                    ("codebook", 2) => include_bytes!("../../tests/fixtures/g726/codebook-2b.s16"),
                    ("codebook", 3) => include_bytes!("../../tests/fixtures/g726/codebook-3b.s16"),
                    ("codebook", 4) => include_bytes!("../../tests/fixtures/g726/codebook-4b.s16"),
                    ("codebook", 5) => include_bytes!("../../tests/fixtures/g726/codebook-5b.s16"),
                    ("tone", 2) => include_bytes!("../../tests/fixtures/g726/tone-2b.s16"),
                    ("tone", 3) => include_bytes!("../../tests/fixtures/g726/tone-3b.s16"),
                    ("tone", 4) => include_bytes!("../../tests/fixtures/g726/tone-4b.s16"),
                    _ => include_bytes!("../../tests/fixtures/g726/tone-5b.s16"),
                };
                let got = through_the_player(wav);
                let want = reference(s16);
                assert_eq!(got.len(), want.len(), "{name}");
                for (index, (got, want)) in got.iter().zip(&want).enumerate() {
                    assert_eq!(
                        got.to_bits(),
                        want.to_bits(),
                        "{name} sample {index}: this decoder gives {got}, the reference gives \
                         {want}"
                    );
                }
            }
        }
    }
}
