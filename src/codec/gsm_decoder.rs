//! GSM 06.10 full-rate, the way Microsoft ships it in a Wave file: 65 bytes of codes
//! in, 320 samples out.
//!
//! The coding is a speech coder from the phone network, and it is a strange one to meet
//! in a media player - it codes 8 kHz mono at 13 kbit/s, which is what a half-second of
//! telephone conversation costs as 8-bit PCM would cost a tenth of a second. What it
//! spends those bits on is a linear-predictive model of the vocal tract plus a periodic
//! excitation: each 20 ms frame carries eight PARCOR coefficients, and four subframes of
//! ten milliseconds each carry a long-term (pitch) prediction and 13 reconstructed
//! samples that a lattice filter then spreads over 40. Nothing about that is a table a
//! decoder can skip, and nothing about it is wide either: the whole stream is 16-bit
//! integer arithmetic, so matching the reference means matching it sample for sample.
//!
//! Two details of that arithmetic are where a port goes wrong. Products are not plain
//! multiplies: the reference's `gsm_mult` is a 32-bit *unsigned* multiply whose result
//! is reinterpreted as signed before the shift, so a wide product wraps into 32 bits
//! first - and every store into the reconstruction buffer is a 16-bit truncation, which
//! wraps rather than saturating. Only the post filter's output is clipped, and it is
//! clipped twice: once on its way into the residual and once on the sample, whose low
//! three bits are then thrown away. This file wraps and clips where the reference does.
//!
//! The Microsoft variant is not a different bitstream. It is the same 260-bit frames
//! read from the least significant bit of each byte instead of the most significant one
//! - measured, which is why a Wave GSM block defies every byte-shuffle hypothesis
//! before that is known. What the variant adds is a block alignment that states a rate:
//! `ff_msgsm_decode_block` takes the number of three-byte steps a block falls short by as
//! a parameter it names `mode`, and every step takes twelve bits off a frame's RPE codes,
//! chosen by a table the mode indexes. Only the whole 65-byte block is decoded here, so
//! mode 0 - thirteen three-bit codes in each of four subframes - is the only one this
//! file's tables hold.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};

/// A frame's bits at full rate: 36 for the eight PARCOR coefficients, 56 for each of
/// four subframes (7 of lag, 2 of long-term gain, 2 of shift, 6 of the maximum index and
/// thirteen 3-bit codes).
const FRAME_BITS: usize = 260;

/// Bytes one frame pair takes: two frames of 260 bits and no padding, which is the only
/// block alignment this decoder is built for.
pub const BLOCK_BYTES: usize = 65;

/// Bits a block holds, which is the unit a run is read out in: a tail shorter than this
/// codes no sample, since the frame it would start is the one whose bytes the file lacks.
const BLOCK_BITS: usize = 2 * FRAME_BITS;

/// Samples one frame reconstructs: 160 of them at 8 kHz, so 20 ms.
const SAMPLES_PER_FRAME: usize = 160;

/// Samples one block codes, which is what a Wave file counts its run in for this
/// coding - and, measured, what its own `wSamplesPerBlock` states when it states
/// anything at all.
pub const SAMPLES_PER_BLOCK: usize = 2 * SAMPLES_PER_FRAME;

/// Samples of the previous frame the decoder keeps: the long-term prediction looks back
/// up to 120 of them.
const HISTORY: usize = 120;

/// The reference decoder's buffer: the last frame's tail, which the long-term
/// prediction reads back through, then the 160 samples of the frame being built.
const REFERENCE_BUFFER: usize = HISTORY + SAMPLES_PER_FRAME;

/// The four long-term gains, in Q15.
const LONG_TERM_GAIN: [u16; 4] = [3277, 11469, 21299, 32767];

/// One row per 6-bit maximum index: the eight reconstruction levels a subframe's codes
/// can take. Every row is symmetric about zero and every one is 4 times a multiple of
/// 7 up to index 48, which is what makes a wide index's levels spill past a sample.
const DEQUANT: [[i16; 8]; 64] = [
    [-28, -20, -12, -4, 4, 12, 20, 28],
    [-56, -40, -24, -8, 8, 24, 40, 56],
    [-84, -60, -36, -12, 12, 36, 60, 84],
    [-112, -80, -48, -16, 16, 48, 80, 112],
    [-140, -100, -60, -20, 20, 60, 100, 140],
    [-168, -120, -72, -24, 24, 72, 120, 168],
    [-196, -140, -84, -28, 28, 84, 140, 196],
    [-224, -160, -96, -32, 32, 96, 160, 224],
    [-252, -180, -108, -36, 36, 108, 180, 252],
    [-280, -200, -120, -40, 40, 120, 200, 280],
    [-308, -220, -132, -44, 44, 132, 220, 308],
    [-336, -240, -144, -48, 48, 144, 240, 336],
    [-364, -260, -156, -52, 52, 156, 260, 364],
    [-392, -280, -168, -56, 56, 168, 280, 392],
    [-420, -300, -180, -60, 60, 180, 300, 420],
    [-448, -320, -192, -64, 64, 192, 320, 448],
    [-504, -360, -216, -72, 72, 216, 360, 504],
    [-560, -400, -240, -80, 80, 240, 400, 560],
    [-616, -440, -264, -88, 88, 264, 440, 616],
    [-672, -480, -288, -96, 96, 288, 480, 672],
    [-728, -520, -312, -104, 104, 312, 520, 728],
    [-784, -560, -336, -112, 112, 336, 560, 784],
    [-840, -600, -360, -120, 120, 360, 600, 840],
    [-896, -640, -384, -128, 128, 384, 640, 896],
    [-1008, -720, -432, -144, 144, 432, 720, 1008],
    [-1120, -800, -480, -160, 160, 480, 800, 1120],
    [-1232, -880, -528, -176, 176, 528, 880, 1232],
    [-1344, -960, -576, -192, 192, 576, 960, 1344],
    [-1456, -1040, -624, -208, 208, 624, 1040, 1456],
    [-1568, -1120, -672, -224, 224, 672, 1120, 1568],
    [-1680, -1200, -720, -240, 240, 720, 1200, 1680],
    [-1792, -1280, -768, -256, 256, 768, 1280, 1792],
    [-2016, -1440, -864, -288, 288, 864, 1440, 2016],
    [-2240, -1600, -960, -320, 320, 960, 1600, 2240],
    [-2464, -1760, -1056, -352, 352, 1056, 1760, 2464],
    [-2688, -1920, -1152, -384, 384, 1152, 1920, 2688],
    [-2912, -2080, -1248, -416, 416, 1248, 2080, 2912],
    [-3136, -2240, -1344, -448, 448, 1344, 2240, 3136],
    [-3360, -2400, -1440, -480, 480, 1440, 2400, 3360],
    [-3584, -2560, -1536, -512, 512, 1536, 2560, 3584],
    [-4032, -2880, -1728, -576, 576, 1728, 2880, 4032],
    [-4480, -3200, -1920, -640, 640, 1920, 3200, 4480],
    [-4928, -3520, -2112, -704, 704, 2112, 3520, 4928],
    [-5376, -3840, -2304, -768, 768, 2304, 3840, 5376],
    [-5824, -4160, -2496, -832, 832, 2496, 4160, 5824],
    [-6272, -4480, -2688, -896, 896, 2688, 4480, 6272],
    [-6720, -4800, -2880, -960, 960, 2880, 4800, 6720],
    [-7168, -5120, -3072, -1024, 1024, 3072, 5120, 7168],
    [-8063, -5759, -3456, -1152, 1152, 3456, 5760, 8064],
    [-8959, -6399, -3840, -1280, 1280, 3840, 6400, 8960],
    [-9855, -7039, -4224, -1408, 1408, 4224, 7040, 9856],
    [-10751, -7679, -4608, -1536, 1536, 4608, 7680, 10752],
    [-11647, -8319, -4992, -1664, 1664, 4992, 8320, 11648],
    [-12543, -8959, -5376, -1792, 1792, 5376, 8960, 12544],
    [-13439, -9599, -5760, -1920, 1920, 5760, 9600, 13440],
    [-14335, -10239, -6144, -2048, 2048, 6144, 10240, 14336],
    [-16127, -11519, -6912, -2304, 2304, 6912, 11519, 16127],
    [-17919, -12799, -7680, -2560, 2560, 7680, 12799, 17919],
    [-19711, -14079, -8448, -2816, 2816, 8448, 14079, 19711],
    [-21503, -15359, -9216, -3072, 3072, 9216, 15359, 21503],
    [-23295, -16639, -9984, -3328, 3328, 9984, 16639, 23295],
    [-25087, -17919, -10752, -3584, 3584, 10752, 17919, 25087],
    [-26879, -19199, -11520, -3840, 3840, 11520, 19199, 26879],
    [-28671, -20479, -12288, -4096, 4096, 12288, 20479, 28671],
];

/// The eight PARCOR codes of a frame: how wide each is, and the pair of numbers that
/// turn it back into a coefficient. The widths come in descending pairs because the
/// coder spends its bits on the first two reflection coefficients and then halves what
/// it gives each later pair.
const LAR_CODES: [(u32, i32, i32); 8] = [
    (6, 13107, 1 << 15),
    (6, 13107, 1 << 15),
    (5, 13107, (1 << 14) + 2048 * 2),
    (5, 13107, (1 << 14) - 2560 * 2),
    (4, 19223, (1 << 13) + 94 * 2),
    (4, 17476, (1 << 13) - 1792 * 2),
    (3, 31454, (1 << 12) - 341 * 2),
    (3, 29708, (1 << 12) - 1144 * 2),
];

/// Where the short-term synthesis switches from one interpolation of the two coefficient
/// sets to the next: the sample ranges the four subframes cover, the first three ten
/// milliseconds apiece and the last the remaining 120.
const SYNTH_STEPS: [(usize, usize); 4] = [(0, 13), (13, 27), (27, 40), (40, SAMPLES_PER_FRAME)];

/// GSM 06.10's whole arithmetic, in one place: the reference multiplies through an
/// unsigned 32-bit product and re-reads the result as signed, so a wide pair of factors
/// wraps before the shift rather than after it.
fn gsm_mult(a: i32, b: i32) -> i32 {
    let wrapped = (a as u32).wrapping_mul(b as u32).wrapping_add(1 << 14);
    wrapped as i32 >> 15
}

/// The reference's `av_clip_int16`.
fn clip_int16(value: i32) -> i32 {
    value.clamp(i32::from(i16::MIN), i32::from(i16::MAX))
}

/// A PARCOR code back into a coefficient: the code scaled to 16 bits, the offset its
/// range starts at taken off, the result run through a Q15 multiply and doubled.
fn decode_log_area(coded: i32, factor: i32, offset: i32) -> i32 {
    gsm_mult((coded << 10) - offset, factor) * 2
}

/// A coefficient set into the reflection coefficients the lattice filter uses, with the
/// quantising expansion the reference applies to keep the filter stable.
fn get_rrp(filtered: i32) -> i32 {
    let magnitude = u32::try_from(filtered.abs()).unwrap_or(u32::MAX);
    let widened = if magnitude < 11059 {
        magnitude << 1
    } else if magnitude < 20070 {
        magnitude + 11059
    } else {
        (magnitude >> 2) + 26112
    };
    let widened = widened as i32;
    if filtered < 0 { -widened } else { widened }
}
/// One of the four ways the two neighbouring frames' coefficients are blended for a
/// stretch of samples: the previous frame's set carries more weight at the start of the
/// frame and none at all by its last three quarters.
fn blend_coefficients(step: usize, previous: i32, current: i32) -> i32 {
    match step {
        0 => (previous >> 2) + (previous >> 1) + (current >> 2),
        1 => (previous >> 1) + (current >> 1),
        2 => (previous >> 2) + (current >> 1) + (current >> 2),
        _ => current,
    }
}

/// A bit cursor over a block, reading from each byte's low bit upward. This is the whole
/// of what makes the Microsoft variant; the frames themselves are the phone network's.
struct Bits<'a> {
    data: &'a [u8],
    position: usize,
}

impl Bits<'_> {
    /// The next `count` bits, first read lowest.
    fn take(&mut self, count: u32) -> i32 {
        let mut value = 0i32;
        for index in 0..count {
            let byte = self.data[self.position / 8];
            let bit = i32::from((byte >> (self.position % 8)) & 1);
            value |= bit << index;
            self.position += 1;
        }
        value
    }
}

/// GSM 06.10 full-rate, Microsoft framing: 65 bytes in, 320 samples out, one channel.
pub struct GsmDecoder {
    sample_rate: u32,
    channels: u16,
    /// The last frames' reconstruction, which the long-term prediction reads back.
    reference: [i16; REFERENCE_BUFFER],
    /// The lattice filter's state, carried from sample to sample across frames.
    lattice: [i32; 9],
    /// The two neighbouring frames' coefficients, kept so the next frame can blend with
    /// the one before it; which of the two slots is the current one alternates.
    coefficients: [[i32; 8]; 2],
    current: usize,
    /// The post filter's one-tap memory.
    residual: i32,
}

impl GsmDecoder {
    /// Build a decoder for a track. `sample_rate` is the track's own, which the coding
    /// is named for at 8 kHz but does not insist on: the numbers it reconstructs are the
    /// same at any rate, and only how fast the player plays them changes.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        if channels != 1 {
            return Err(invalid(&format!(
                "GSM 06.10 codes one channel, this track names {channels}"
            )));
        }
        if sample_rate == 0 {
            return Err(invalid("GSM track has no sample rate"));
        }
        Ok(Self::cold(sample_rate))
    }

    fn cold(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            channels: 1,
            reference: [0; REFERENCE_BUFFER],
            lattice: [0; 9],
            coefficients: [[0; 8]; 2],
            current: 0,
            residual: 0,
        }
    }

    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: self.channels,
            format: SampleFormat::F32,
        }
    }

    /// One frame: 260 bits in, 160 samples out, with the decoder's history moved along
    /// by one frame's worth as it goes.
    fn decode_frame(&mut self, bits: &mut Bits<'_>) -> [i16; SAMPLES_PER_FRAME] {
        for (index, (width, factor, offset)) in LAR_CODES.into_iter().enumerate() {
            let coded = bits.take(width);
            self.coefficients[self.current][index] = decode_log_area(coded, factor, offset);
        }
        for subframe in 0..4 {
            // The lag is clipped rather than refused: the reference reads a value that
            // points past the history it keeps as one that points at the nearest
            // distance that is not.
            let lag = bits.take(7).clamp(40, 120) as usize;
            let gain = bits.take(2) as usize;
            let shift = bits.take(2) as usize;
            let start = HISTORY + subframe * 40;
            self.long_term_synthesis(start, lag, gain);
            let maximum = bits.take(6) as usize;
            for index in 0..13 {
                // A three-bit code numbers its row's eight levels directly; the reference
                // reaches them through a requantiser that is the identity at this width.
                let level = bits.take(3) as usize;
                let at = start + shift + 3 * index;
                self.reference[at] =
                    (i32::from(self.reference[at]) + i32::from(DEQUANT[maximum][level])) as i16;
            }
        }
        // Carry the frame's last 120 samples down into the history slot, the way the
        // reference shifts its buffer rather than indexing it mod something.
        self.reference
            .copy_within(SAMPLES_PER_FRAME..SAMPLES_PER_FRAME + HISTORY, 0);
        let mut samples = [0i16; SAMPLES_PER_FRAME];
        self.short_term_synthesis(&mut samples);
        self.post_filter(&mut samples);
        samples
    }

    /// Fill a subframe with a scaled copy of the signal 40 to 120 samples back: the
    /// pitch period the encoder found, and the gain it measured for it.
    fn long_term_synthesis(&mut self, start: usize, lag: usize, gain: usize) {
        let gain = i32::from(LONG_TERM_GAIN[gain]);
        let source = start - lag;
        for index in 0..40 {
            self.reference[start + index] =
                gsm_mult(gain, i32::from(self.reference[source + index])) as i16;
        }
    }

    /// Run the reconstructed excitation through the all-pole filter, with the
    /// coefficients interpolated four ways across the frame, and hand the previous
    /// frame's set to the next one.
    fn short_term_synthesis(&mut self, dst: &mut [i16]) {
        let current = self.coefficients[self.current];
        let previous = self.coefficients[self.current ^ 1];
        let mut reflections = [0i32; 8];
        for (step, (from, to)) in SYNTH_STEPS.iter().enumerate() {
            for index in 0..8 {
                reflections[index] =
                    get_rrp(blend_coefficients(step, previous[index], current[index]));
            }
            for position in *from..*to {
                let excitation = i32::from(self.reference[HISTORY + position]);
                dst[position] = self.lattice_step(reflections, excitation) as i16;
            }
        }
        self.current ^= 1;
    }

    /// One sample through the lattice: the reflection coefficients subtract their
    /// memory from the input and then update it, most recent last.
    fn lattice_step(&mut self, reflections: [i32; 8], input: i32) -> i32 {
        let mut output = input;
        for index in (0..8).rev() {
            output -= gsm_mult(reflections[index], self.lattice[index]);
            self.lattice[index + 1] = self.lattice[index] + gsm_mult(reflections[index], output);
        }
        self.lattice[0] = output;
        output
    }

    /// A one-tap post filter, doubling the result and dropping its three low bits:
    /// the coder's own roughness is part of what it sounds like, and this is where
    /// the reference decides a sample has gone too far.
    fn post_filter(&mut self, data: &mut [i16]) {
        for sample in data.iter_mut() {
            self.residual = clip_int16(i32::from(*sample) + gsm_mult(self.residual, 28180));
            *sample = (clip_int16(self.residual * 2) & !7) as i16;
        }
    }
}

impl AudioDecode for GsmDecoder {
    /// A packet is a whole number of 65-byte blocks, so it always comes back as one
    /// packet and the timestamps stay the caller's. Bytes left over cannot fill a frame
    /// pair, and the reference stops at the same place: a run that ends mid-block has
    /// said nothing about the samples it half-codes.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        let mut out = Vec::with_capacity(data.len() / BLOCK_BYTES * SAMPLES_PER_BLOCK * 4);
        let mut bits = Bits { data, position: 0 };
        while bits.position + BLOCK_BITS <= data.len() * 8 {
            for _ in 0..2 {
                for sample in self.decode_frame(&mut bits) {
                    out.extend_from_slice(&(f32::from(sample) / 32768.0).to_le_bytes());
                }
            }
        }
        Ok(Some(AudioPacket {
            data: out,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    /// Drop the history: the next stream starts as a cold decoder does, with silence
    /// behind its first frame's pitch.
    fn reset(&mut self) {
        *self = Self::cold(self.sample_rate);
    }
}

#[cfg(test)]
mod tests {
    use super::{BLOCK_BYTES, Bits, GsmDecoder, SAMPLES_PER_BLOCK, clip_int16, gsm_mult};
    use crate::audio::{AudioDecode, AudioStream};
    use crate::playback_wav::{Limits, Wav, WavAudioReader};

    /// The take this decoder is proven on: GSM 06.10 as FATE files it, 89 blocks of
    /// `ciao.wav`, and the same file's samples as this build's libavcodec writes them.
    const WAV: &[u8] = include_bytes!("../../tests/fixtures/gsm/ciao.wav");
    const S16: &[u8] = include_bytes!("../../tests/fixtures/gsm/ciao.s16");

    fn whole(decoder: &mut GsmDecoder, data: &[u8]) -> Vec<i32> {
        let audio = decoder
            .decode_encoded(data, 0, 0)
            .expect("decode")
            .expect("a block always yields samples");
        assert_eq!((audio.timebase_num, audio.timebase_den), (1, 8_000));
        audio
            .data
            .chunks_exact(4)
            .map(|chunk| {
                let sample = f32::from_le_bytes(chunk.try_into().expect("four bytes"));
                (f64::from(sample) * 32_768.0).round() as i32
            })
            .collect()
    }

    fn decode(data: &[u8]) -> Vec<i32> {
        let mut decoder = GsmDecoder::new(8_000, 1).expect("mono at a rate");
        whole(&mut decoder, data)
    }

    /// The take's coded run, cut the way the player cuts it: a window holds six blocks.
    fn blocks(count: usize) -> Vec<u8> {
        let wav = Wav::parse(WAV, &Limits::default()).expect("the fixture parses");
        assert_eq!(
            (wav.pcm.frames_per_block(), wav.pcm.block_align()),
            (SAMPLES_PER_BLOCK, BLOCK_BYTES)
        );
        let mut run = Vec::new();
        let mut window = 0;
        while run.len() < count * BLOCK_BYTES {
            run.extend_from_slice(wav.packet(window));
            window += 1;
        }
        run.truncate(count * BLOCK_BYTES);
        assert_eq!(run.len(), count * BLOCK_BYTES);
        run
    }

    /// What the player does with one of these files: read the windows the container hands
    /// out and decode them as one stream, sample after sample.
    fn through_the_player(wav: &[u8]) -> Vec<f32> {
        let mut reader = WavAudioReader::open(wav, Limits::default()).expect("opens");
        assert_eq!(reader.codec(), "gsm_ms");
        let mut decoder = crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("GSM arm");
        let rate = reader.sample_rate();
        let mut heard = Vec::new();
        while let Some(packet) = reader.next_packet().expect("packet") {
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("decode")
                .expect("a block always yields samples");
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

    /// The reference's own bytes: `ffmpeg -i ciao.wav -f s16le ciao.s16`.
    fn reference(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(2)
            .map(|chunk| {
                f32::from(i16::from_le_bytes(chunk.try_into().expect("two bytes"))) / 32_768.0
            })
            .collect()
    }

    #[test]
    fn a_microsoft_block_is_read_from_each_byte_s_low_bit() {
        // The one thing the Microsoft variant changes about the phone network's stream.
        // The first byte is 1000 0000: read from its low bit the block's first six-bit code
        // is 0, where the same byte read from its high bit starts the stream with 32. Both
        // are reflection coefficients - 32 states no reflection at all, 0 is the code that
        // rings at full strength - so the choice of end is not two spellings of one filter.
        // The next code straddles the boundary between the two bytes and takes its bits
        // from the low end of each: 14 out of these twelve bits.
        let mut bits = Bits {
            data: &[0x80, 0x03],
            position: 0,
        };
        assert_eq!(bits.take(6), 0);
        assert_eq!(bits.take(6), 14);
    }

    /// A block whose every code is zero is not a block of silence: zero is the *first*
    /// level of the row the subframe's maximum index also names as the smallest, so the
    /// excitation is set at its most negative every third sample and the synthesis rings
    /// from the very first frame. Measured on this decoder against the same reference.
    #[test]
    fn an_all_zero_block_is_the_quietest_row_not_quiet() {
        let heard = decode(&[0; BLOCK_BYTES]);
        assert_eq!(heard.len(), SAMPLES_PER_BLOCK);
        assert_eq!(&heard[..6], &[-56, -56, -64, -120, -120, -128]);
        assert!(heard.iter().any(|sample| *sample != 0));
    }

    #[test]
    fn a_packet_is_a_whole_number_of_blocks() {
        for blocks in [1usize, 2, 6, 7] {
            for extra in [0usize, 1, 32, 64] {
                let run = vec![0x5a; blocks * BLOCK_BYTES + extra];
                assert_eq!(
                    decode(&run).len(),
                    blocks * SAMPLES_PER_BLOCK,
                    "{blocks} block(s) and {extra} leftover byte(s)"
                );
            }
        }
    }

    /// The frame's pitch reaches back over the block boundary, so the same 65 bytes come
    /// out differently depending on what was decoded before them: measured, block two
    /// alone starts at 0 and -96 while the same block after block one starts at 16 and
    /// 24. This is why the take has to be decoded as one stream rather than block by
    /// block, and why a reset means something.
    #[test]
    fn a_frame_reaches_back_into_the_block_before_it() {
        let run = blocks(2);
        let apart: Vec<i32> = [decode(&run[..BLOCK_BYTES]), decode(&run[BLOCK_BYTES..])].concat();
        let together = decode(&run);
        assert_eq!(&together[..SAMPLES_PER_BLOCK], &apart[..SAMPLES_PER_BLOCK]);
        assert_ne!(
            &together[SAMPLES_PER_BLOCK..SAMPLES_PER_BLOCK + 6],
            &apart[SAMPLES_PER_BLOCK..SAMPLES_PER_BLOCK + 6]
        );

        let mut decoder = GsmDecoder::new(8_000, 1).expect("mono at a rate");
        whole(&mut decoder, &run);
        decoder.reset();
        let heard = whole(&mut decoder, &run[..BLOCK_BYTES]);
        assert_eq!(&heard[..6], &together[..6], "a reset starts cold");
    }

    #[test]
    fn a_wide_product_wraps_before_it_shifts() {
        // The reference's multiply is unsigned, so a pair of factors past a 32-bit
        // product loses bits the signed arithmetic this file could have used keeps.
        // Every number below is what the reference's own expression gives.
        assert_eq!(gsm_mult(32_767, 32_767), 32_766);
        assert_eq!(gsm_mult(-32_768, 32_768), -32_768);
        assert_eq!(gsm_mult(100_000, 60_000), 52_033);
        // A negative times a positive that wraps past the sign comes out positive: signed
        // arithmetic would give -73 242 here. A lattice filter's memory and a reflection
        // coefficient both exceed what sixteen bits held, so this is where a port that
        // widened the product rather than wrapping it would part from the reference.
        assert_eq!(gsm_mult(-40_000, 60_000), 57_830);
    }

    /// The post filter is the one place the coding clips rather than wraps, and it clips
    /// twice: the residual to a sample's range, and the doubled answer - whose three low
    /// bits are then dropped, so no output sample is off the eight-point grid.
    #[test]
    fn the_post_filter_clips_where_everywhere_else_wraps() {
        assert_eq!(clip_int16(70_000), 32_767);
        assert_eq!(clip_int16(-70_000), -32_768);
        assert_eq!(clip_int16(32_768), 32_767);
        let heard = decode(&blocks(89));
        assert_eq!(heard.len(), 89 * SAMPLES_PER_BLOCK);
        assert!(
            heard
                .iter()
                .all(|sample| (-32_768..=32_767).contains(sample)),
            "a sample escaped the sixteen-bit range"
        );
        assert!(
            heard.iter().all(|sample| sample % 8 == 0),
            "the reference throws away three low bits of every sample it writes"
        );
        assert!(
            heard.contains(&(-32_768)),
            "the take is loud enough to reach the clip, which is what this test is for"
        );
    }

    #[test]
    fn a_track_that_is_not_mono_has_no_geometry_to_decode() {
        assert!(
            GsmDecoder::new(8_000, 2)
                .err()
                .expect("a stereo GSM track is refused")
                .to_string()
                .contains("one channel")
        );
        assert!(
            GsmDecoder::new(0, 1)
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
                from_decoder.to_bits(),
                from_ffmpeg.to_bits(),
                "sample {index} of {}: this decoder gives {from_decoder}, the reference gives {from_ffmpeg}",
                wanted.len()
            );
        }
    }

    /// A rate the block does not divide whole changes nothing about the run: 65 bytes stay
    /// two frames of 160 samples whatever the header's rate field says, and the byte rate
    /// for such a run is the whole division a muxer itself writes (8 957 of the 8 957.8125
    /// the geometry asks for). This take patched to 44.1 kHz and that byte rate still lands
    /// on every reference sample, which is the reader's boundary stated from the inside:
    /// the floored claim decodes, the rounded-up one is refused as a contradiction.
    #[test]
    fn a_rate_the_block_does_not_divide_whole_decodes_the_same_run() {
        let mut bytes = WAV.to_vec();
        bytes[24..28].copy_from_slice(&44_100u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&8_957u32.to_le_bytes());
        assert_eq!(through_the_player(&bytes), reference(S16));
    }
}
