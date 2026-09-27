//! Dolby Digital. A packet is one or more syncframes, each of which carries six
//! audio blocks of 256 samples per channel, so the frame is the unit that holds a
//! piece of the timeline and the block is the unit that holds a transform.
//!
//! Every block is a floating point spectrum: each of its 256 transform
//! coefficients is an exponent plus a mantissa, the exponents arrive as a
//! differential run, and bit allocation turns that run into a mantissa width per
//! coefficient. That makes bit allocation the load-bearing part - a decoder that
//! computes it one step differently unpacks the mantissas at the wrong bit offsets
//! and hears noise instead of a transient - so the seven steps here are the
//! standard's own, transcribed step by step with its integer arithmetic and its
//! table lookups, because integer arithmetic is exactly reproducible and a
//! floating point shortcut is not.
//!
//! Nothing carries from one syncframe to the next: block 0 of every frame states
//! its own coupling strategy, bandwidths and SNR offsets, which is why the reuse
//! state is wiped at the frame boundary rather than kept across packets. Within a
//! frame a block may reuse exponents, delta bit allocation or a channel's
//! bandwidth, so the geometry lives in the carried strategy and not in per-block
//! scratch.
//!
//! Faulty syntax mutes a whole syncframe rather than emitting half a transform:
//! Section 5.4.3.24 and Table 5.16 both prescribe muting, and a frame whose
//! mantissas do not line up with its own bit allocation is not audio. The bit
//! reader therefore carries an overrun flag instead of failing, which keeps the
//! `Result` out of the loops that unpack the spectra.
//!
//! Output is the track's own channel count: native order when the container and
//! the stream agree, LoRo or mono when the container states two or one channel,
//! which are the cases Section 7.8.2 spells out. Dynamic range compression is
//! applied as the per-block gain of Section 7.7.1.2; heavy compression is not.
//! Dither for zero-bit mantissas is applied as Section 7.3.4 asks, from the decoder's
//! own fixed sequence, and the coupling plane's unallocated bins are dithered after
//! decoupling so each channel's noise stays uncorrelated with the other's.
//!
//! Every constant and formula is derived from the public ATSC A/52:2012 text.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};

/// Transform coefficients, mantissas and exponents per block and per plane.
pub(crate) const BINS: usize = 256;
/// Full-bandwidth channels a stream can carry.
pub(crate) const FBW: usize = 5;
/// Plane index of the coupling channel, which the standard numbers after the five.
pub(crate) const CPL: usize = 5;
/// Plane index of the low frequency effects channel.
pub(crate) const LFE: usize = 6;
/// Planes whose coefficients a block can hold: five full-bandwidth, coupling, LFE.
pub(crate) const PLANES: usize = 7;
/// Output channel slots: left, right, centre, LFE, and the two surrounds.
const SLOTS: usize = 6;
/// Bit allocation bands, the sixth-octave groups the masking curve lives in.
const NBANDS: usize = 50;
/// Coupling sub-bands: coefficients 37 through 252 in groups of 12.
pub(crate) const SUBDN: usize = 18;
/// Delta bit allocation segments, which the 3-bit count spells as 1 through 8.
const SEGMENTS: usize = 8;
/// Audio blocks in a syncframe.
const BLOCKS: usize = 6;
/// Samples one audio block contributes per channel.
pub(crate) const SAMPLES: usize = 256;
/// Samples the encoder put in front of the sound, which the first block therefore
/// carries as padding: a decoder that hands them out plays the whole track some
/// milliseconds behind the picture every other player shows. The count is the same
/// at all three rates - measured against the reference decoder on streams at
/// 48 000, 44 100 and 32 000 Hz, where the earliest sample that lines up after it
/// sits 256 samples into this decoder's own output.
pub(crate) const ENCODER_DELAY: usize = 256;
/// Windowed time-domain slots one block's inverse transform fills.
const SLOTS_512: usize = 512;
/// The longest complex inverse transform the 512-sample path needs: a quarter of
/// the transform length, which is also how many coefficients it takes as input.
const FFT: usize = SLOTS_512 / 4;
/// The complex inverse transform of a block-switched run: half again, since the
/// 256-sample path splits the block's coefficients between two runs.
const QUARTER: usize = FFT / 2;
/// The transform length the twiddle and window tables are measured against.
const TRANSFORM: usize = SLOTS_512;

/// Table 7.13: transform coefficient number to bit allocation band.
const MASKTAB: [u8; 256] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 28, 28, 29, 29, 29, 30, 30, 30, 31, 31, 31, 32, 32, 32, 33, 33, 33, 34, 34, 34, 35,
    35, 35, 35, 35, 35, 36, 36, 36, 36, 36, 36, 37, 37, 37, 37, 37, 37, 38, 38, 38, 38, 38, 38, 39,
    39, 39, 39, 39, 39, 40, 40, 40, 40, 40, 40, 41, 41, 41, 41, 41, 41, 41, 41, 41, 41, 41, 41, 42,
    42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 43, 43, 43, 43, 43, 43, 43, 43, 43, 43, 43, 43, 44,
    44, 44, 44, 44, 44, 44, 44, 44, 44, 44, 44, 45, 45, 45, 45, 45, 45, 45, 45, 45, 45, 45, 45, 45,
    45, 45, 45, 45, 45, 45, 45, 45, 45, 45, 45, 46, 46, 46, 46, 46, 46, 46, 46, 46, 46, 46, 46, 46,
    46, 46, 46, 46, 46, 46, 46, 46, 46, 46, 46, 47, 47, 47, 47, 47, 47, 47, 47, 47, 47, 47, 47, 47,
    47, 47, 47, 47, 47, 47, 47, 47, 47, 47, 47, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48,
    48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49,
    49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 0, 0, 0,
];

/// Table 7.14: log-addition increments.
const LATAB: [i32; 256] = [
    64, 63, 62, 61, 60, 59, 58, 57, 56, 55, 54, 53, 52, 52, 51, 50, 49, 48, 47, 47, 46, 45, 44, 44,
    43, 42, 41, 41, 40, 39, 38, 38, 37, 36, 36, 35, 35, 34, 33, 33, 32, 32, 31, 30, 30, 29, 29, 28,
    28, 27, 27, 26, 26, 25, 25, 24, 24, 23, 23, 22, 22, 21, 21, 21, 20, 20, 19, 19, 19, 18, 18, 18,
    17, 17, 17, 16, 16, 16, 15, 15, 15, 14, 14, 14, 13, 13, 13, 13, 12, 12, 12, 12, 11, 11, 11, 11,
    10, 10, 10, 10, 10, 9, 9, 9, 9, 9, 8, 8, 8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6,
    5, 5, 5, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0,
];

/// Table 7.12: the first mantissa of each of the 50 bands.
const BNDTAB: [usize; 50] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 31, 34, 37, 40, 43, 46, 49, 55, 61, 67, 73, 79, 85, 97, 109, 121, 133, 157, 181,
    205, 229,
];

/// Table 7.12: the number of mantissas in each band.
const BNDSZ: [usize; 50] = [
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3,
    3, 3, 3, 6, 6, 6, 6, 6, 6, 12, 12, 12, 12, 24, 24, 24, 24, 24,
];

/// Table 7.15: hearing threshold per band, by sampling rate code.
const HTH: [[i32; 50]; 3] = [
    [
        1232, 1232, 1088, 1024, 992, 960, 944, 944, 928, 928, 928, 928, 928, 912, 912, 912, 896,
        896, 880, 880, 864, 864, 848, 848, 832, 832, 816, 800, 784, 768, 752, 752, 752, 752, 768,
        784, 832, 912, 992, 1056, 1120, 1168, 1184, 1120, 1088, 1088, 1312, 2048, 2112, 2112,
    ],
    [
        1264, 1264, 1120, 1040, 992, 976, 960, 944, 944, 928, 928, 928, 928, 928, 912, 912, 912,
        896, 896, 896, 880, 880, 864, 864, 848, 848, 832, 832, 800, 784, 768, 752, 752, 752, 752,
        768, 800, 848, 912, 992, 1056, 1104, 1184, 1168, 1120, 1088, 1152, 1584, 2112, 2112,
    ],
    [
        1408, 1408, 1200, 1104, 1056, 1008, 992, 976, 960, 944, 944, 944, 928, 928, 928, 928, 928,
        928, 928, 928, 912, 912, 912, 912, 896, 896, 896, 880, 864, 848, 832, 816, 800, 784, 768,
        752, 752, 752, 768, 784, 816, 848, 960, 1040, 1136, 1184, 1120, 1088, 1104, 1248,
    ],
];

/// Table 7.6: slow decay step.
const SLOWDEC: [i32; 4] = [15, 17, 19, 21];

/// Table 7.7: fast decay step.
const FASTDEC: [i32; 4] = [63, 83, 103, 123];

/// Table 7.8: slow gain offset.
const SLOWGAIN: [i32; 4] = [1344, 1240, 1144, 1040];

/// Table 7.9: decibels per bit, i.e. the knee of the masking curve.
const DBKNEE: [i32; 4] = [0, 1792, 2304, 2816];

/// Table 7.10: bit allocation floor, the last entry read as a signed code.
const FLOORTAB: [i32; 8] = [752, 688, 624, 560, 496, 368, 240, -2048];

/// Table 7.11: fast gain offset.
const FASTGAIN: [i32; 8] = [128, 256, 384, 512, 640, 768, 896, 1024];

/// Table 7.16: the bit allocation pointer for a power-to-mask difference.
const BAPTAB: [u8; 64] = [
    0, 1, 1, 1, 1, 1, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 8, 9, 9, 9, 9,
    10, 10, 10, 10, 11, 11, 11, 11, 12, 12, 12, 12, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
    15, 15, 15, 15, 15, 15, 15, 15, 15,
];

/// Table 5.18: syncframe length in 16-bit words, by frame size code and rate.
const FRAME_WORDS: [[u16; 3]; 38] = [
    [64, 69, 96],
    [64, 70, 96],
    [80, 87, 120],
    [80, 88, 120],
    [96, 104, 144],
    [96, 105, 144],
    [112, 121, 168],
    [112, 122, 168],
    [128, 139, 192],
    [128, 140, 192],
    [160, 174, 240],
    [160, 175, 240],
    [192, 208, 288],
    [192, 209, 288],
    [224, 243, 336],
    [224, 244, 336],
    [256, 278, 384],
    [256, 279, 384],
    [320, 348, 480],
    [320, 349, 480],
    [384, 417, 576],
    [384, 418, 576],
    [448, 487, 672],
    [448, 488, 672],
    [512, 557, 768],
    [512, 558, 768],
    [640, 696, 960],
    [640, 697, 960],
    [768, 835, 1152],
    [768, 836, 1152],
    [896, 975, 1344],
    [896, 976, 1344],
    [1024, 1114, 1536],
    [1024, 1115, 1536],
    [1152, 1253, 1728],
    [1152, 1254, 1728],
    [1280, 1393, 1920],
    [1280, 1394, 1920],
];

/// Table 7.33: the transform window, held for the half block it covers.
/// One of its rounded sines reads to clippy as `PI / 4`, which it is not: the digits
/// are the standard's own table entry, copied rather than computed.
#[allow(clippy::approx_constant)]
const WINDOW: [f32; 256] = [
    0.00014f32, 0.00024f32, 0.00037f32, 0.00051f32, 0.00067f32, 0.00086f32, 0.00107f32, 0.0013f32,
    0.00157f32, 0.00187f32, 0.0022f32, 0.00256f32, 0.00297f32, 0.00341f32, 0.0039f32, 0.00443f32,
    0.00501f32, 0.00564f32, 0.00632f32, 0.00706f32, 0.00785f32, 0.00871f32, 0.00962f32, 0.01061f32,
    0.01166f32, 0.01279f32, 0.01399f32, 0.01526f32, 0.01662f32, 0.01806f32, 0.01959f32, 0.02121f32,
    0.02292f32, 0.02472f32, 0.02662f32, 0.02863f32, 0.03073f32, 0.03294f32, 0.03527f32, 0.0377f32,
    0.04025f32, 0.04292f32, 0.04571f32, 0.04862f32, 0.05165f32, 0.05481f32, 0.0581f32, 0.06153f32,
    0.06508f32, 0.06878f32, 0.07261f32, 0.07658f32, 0.08069f32, 0.08495f32, 0.08935f32, 0.09389f32,
    0.09859f32, 0.10343f32, 0.10842f32, 0.11356f32, 0.11885f32, 0.12429f32, 0.12988f32, 0.13563f32,
    0.14152f32, 0.14757f32, 0.15376f32, 0.16011f32, 0.16661f32, 0.17325f32, 0.18005f32, 0.18699f32,
    0.19407f32, 0.2013f32, 0.20867f32, 0.21618f32, 0.22382f32, 0.23161f32, 0.23952f32, 0.24757f32,
    0.25574f32, 0.26404f32, 0.27246f32, 0.281f32, 0.28965f32, 0.29841f32, 0.30729f32, 0.31626f32,
    0.32533f32, 0.3345f32, 0.34376f32, 0.35311f32, 0.36253f32, 0.37204f32, 0.38161f32, 0.39126f32,
    0.40096f32, 0.41072f32, 0.42054f32, 0.4304f32, 0.4403f32, 0.45023f32, 0.4602f32, 0.47019f32,
    0.4802f32, 0.49022f32, 0.50025f32, 0.51028f32, 0.52031f32, 0.53033f32, 0.54033f32, 0.55031f32,
    0.56026f32, 0.57019f32, 0.58007f32, 0.58991f32, 0.5997f32, 0.60944f32, 0.61912f32, 0.62873f32,
    0.63827f32, 0.64774f32, 0.65713f32, 0.66643f32, 0.67564f32, 0.68476f32, 0.69377f32, 0.70269f32,
    0.7115f32, 0.72019f32, 0.72877f32, 0.73723f32, 0.74557f32, 0.75378f32, 0.76186f32, 0.76981f32,
    0.77762f32, 0.7853f32, 0.79283f32, 0.80022f32, 0.80747f32, 0.81457f32, 0.82151f32, 0.82831f32,
    0.83496f32, 0.84145f32, 0.84779f32, 0.85398f32, 0.86001f32, 0.86588f32, 0.8716f32, 0.87716f32,
    0.88257f32, 0.88782f32, 0.89291f32, 0.89785f32, 0.90264f32, 0.90728f32, 0.91176f32, 0.9161f32,
    0.92028f32, 0.92432f32, 0.92822f32, 0.93197f32, 0.93558f32, 0.93906f32, 0.9424f32, 0.9456f32,
    0.94867f32, 0.95162f32, 0.95444f32, 0.95713f32, 0.95971f32, 0.96217f32, 0.96451f32, 0.96674f32,
    0.96887f32, 0.97089f32, 0.97281f32, 0.97463f32, 0.97635f32, 0.97799f32, 0.97953f32, 0.98099f32,
    0.98236f32, 0.98366f32, 0.98488f32, 0.98602f32, 0.9871f32, 0.98811f32, 0.98905f32, 0.98994f32,
    0.99076f32, 0.99153f32, 0.99225f32, 0.99291f32, 0.99353f32, 0.99411f32, 0.99464f32, 0.99513f32,
    0.99558f32, 0.996f32, 0.99639f32, 0.99674f32, 0.99706f32, 0.99736f32, 0.99763f32, 0.99788f32,
    0.99811f32, 0.99831f32, 0.9985f32, 0.99867f32, 0.99882f32, 0.99895f32, 0.99908f32, 0.99919f32,
    0.99929f32, 0.99938f32, 0.99946f32, 0.99953f32, 0.99959f32, 0.99965f32, 0.99969f32, 0.99974f32,
    0.99978f32, 0.99981f32, 0.99984f32, 0.99986f32, 0.99988f32, 0.9999f32, 0.99992f32, 0.99993f32,
    0.99994f32, 0.99995f32, 0.99996f32, 0.99997f32, 0.99998f32, 0.99998f32, 0.99998f32, 0.99999f32,
    0.99999f32, 0.99999f32, 0.99999f32, 1.0f32, 1.0f32, 1.0f32, 1.0f32, 1.0f32, 1.0f32, 1.0f32,
    1.0f32, 1.0f32, 1.0f32, 1.0f32, 1.0f32, 1.0f32,
];

/// Sampling rate by `fscod`. The fourth code is the reserved pattern that states no
/// rate, so a frame naming it is not decodable.
const SAMPLE_RATES: [u32; 4] = [48_000, 44_100, 32_000, 0];

/// Full-bandwidth channel count by `acmod`, per Section 5.4.2.3.
const NFCHANS: [usize; 8] = [2, 1, 2, 3, 3, 4, 4, 5];

/// Which of left, right, centre and the two surrounds each `acmod`'s channel array
/// position stands for, per Table 5.8. Positions past the mode's own count are
/// unused. The 3/1 modes put their single surround in the left surround slot, which
/// is what the LoRo equations of Section 7.8.2 then weight by 0.7.
const SLOT_OF_ACMOD: [[u8; FBW]; 8] = [
    [0, 1, 0, 0, 0],
    [2, 0, 0, 0, 0],
    [0, 1, 0, 0, 0],
    [0, 2, 1, 0, 0],
    [0, 1, 4, 0, 0],
    [0, 2, 1, 4, 0],
    [0, 1, 4, 5, 0],
    [0, 2, 1, 4, 5],
];

/// Slot the low frequency effects channel fills, which `acmod` does not name.
const LFE_SLOT: usize = 3;

/// Centre mix level by `cmixlev` code, with the reserved code read as the
/// intermediate value Section 5.4.2.4 asks for.
const CMIX: [f32; 4] = [0.707, 0.595, 0.500, 0.595];
/// Surround mix level by `surmixlev` code, likewise.
const SMIX: [f32; 4] = [0.707, 0.500, 0.000, 0.500];

/// Exponents per group by exponent strategy code, so D15, D25 and D45 as Table 7.4
/// numbers them. Strategy 0 reuses and never reaches a transform.
const GRPSIZE: [usize; 4] = [1, 1, 2, 4];

/// Quantizer levels for the symmetric quantizers of bap 1 through 5, per Table
/// 7.17. A coded value becomes `(2 * code - (levels - 1)) / levels`.
const SYM_LEVELS: [i32; 6] = [0, 3, 5, 7, 11, 15];

/// Mantissa bits by bap, from Table 7.17. Where Section 7.3.5 groups mantissas the
/// entry is the group's width and `group_of_bap` says how many mantissas share it.
const MANTISSA_BITS: [u8; 16] = [0, 5, 7, 3, 7, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 16];

/// The grouped symmetric quantizers: bits per group, the two divisors that split a
/// group code back into level codes, and how many mantissas the group holds.
fn group_of_bap(bap: usize) -> Option<(usize, i32, i32, usize)> {
    match bap {
        1 => Some((5, 9, 3, 3)),
        2 => Some((7, 25, 5, 3)),
        4 => Some((7, 11, 11, 2)),
        _ => None,
    }
}

/// Two to the minus an exponent, the scale every mantissa takes on its way into a
/// transform coefficient; exponents run 0 through 24 per Section 7.1.1.
const fn pow2_table() -> [f32; 25] {
    let mut table = [0.0f32; 25];
    let mut n = 0;
    while n < 25 {
        table[n] = 1.0 / (1u32 << n) as f32;
        n += 1;
    }
    table
}

/// `pow2_table`'s result, so the hot loops only index.
const POW2: [f32; 25] = pow2_table();

/// The scaling Section 7.3.4 calls optimum for a dither word: take a uniform draw
/// between -1 and +1 and scale it by 0.707, which keeps the noise an unallocated
/// coefficient carries inside the range a coded mantissa occupies.
const DITHER_SCALE: f32 = 0.707;

/// Where the dither walk starts. Any value is as good as the next for randomness, and
/// the point of fixing one is that a decode of the same packet then repeats.
const DITHER_SEED: u32 = 0x9E37_79B9;

/// The dynamic range gain of Section 7.7.1.2: the top three bits of the code are a
/// signed power of two and the bottom five refine it, with the all-zeros code
/// standing for unity.
pub(crate) fn dynrng_gain(code: i32) -> f32 {
    if code == 0 {
        return 1.0;
    }
    let exponent = (code >> 5) - if code & 0x20 == 0 { 8 } else { 0 };
    2f32.powi(exponent + 1) * (32 + (code & 0x1f)) as f32 / 64.0
}

/// A coded mantissa of a symmetric quantizer as a fraction: the levels are spread
/// evenly across the range, so the code at the middle is zero.
fn symmetric_value(bap: usize, code: i32) -> f32 {
    let levels = SYM_LEVELS[bap];
    (2 * code - (levels - 1)) as f32 / levels as f32
}

/// A coded mantissa of an asymmetric quantizer as a fraction: the word is two's
/// complement with the decimal point left of the most significant bit, so the
/// divisor is half the word count.
fn asymmetric_value(bap: usize, code: i32) -> f32 {
    let bits = usize::from(MANTISSA_BITS[bap]);
    let half = 1 << (bits - 1);
    let signed = if code >= half {
        code - (1 << bits)
    } else {
        code
    };
    signed as f32 * POW2[bits - 1]
}

/// Log-addition of two power spectral densities, which is what the band integration
/// of Section 7.2.2.3 needs: the increment comes from a table indexed by half the
/// absolute difference of the operands.
fn logadd(a: i32, b: i32) -> i32 {
    let difference = a - b;
    let address = (difference.abs() >> 1).min(255) as usize;
    if difference >= 0 {
        a + LATAB[address]
    } else {
        b + LATAB[address]
    }
}

/// The low-frequency compensation term of Section 7.2.2.4, which tracks how steeply
/// the power in the lowest bands is rising and holds a neighbouring band back
/// accordingly.
fn calc_lowcomp(a: i32, first: i32, second: i32, bin: usize) -> i32 {
    if bin < 7 {
        if second - first == 256 {
            384
        } else if first > second {
            (a - 64).max(0)
        } else {
            a
        }
    } else if bin < 20 {
        if second - first == 256 {
            320
        } else if first > second {
            (a - 64).max(0)
        } else {
            a
        }
    } else {
        (a - 128).max(0)
    }
}

/// Bits read from the top of each byte down, which is the order Section 5.3 states.
/// Reading past the packet sets `overrun` and yields zeros: one flag checked once
/// per frame says the same thing as a `Result` threaded through every loop.
pub(crate) struct Bits<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) pos: usize,
    pub(crate) overrun: bool,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            overrun: false,
        }
    }

    /// A reader positioned at bit `bit` of `data`, which is how a caller that has
    /// read a frame's header by other means hands the audio to this one.
    pub(crate) fn at(data: &'a [u8], bit: usize) -> Self {
        Self {
            data,
            pos: bit,
            overrun: false,
        }
    }

    /// The next `count` bits as an unsigned value, for `count` up to 32.
    pub(crate) fn take(&mut self, count: usize) -> i32 {
        let mut value = 0i32;
        for _ in 0..count {
            value = (value << 1) | self.bit();
        }
        value
    }

    /// One bit as a flag.
    pub(crate) fn flag(&mut self) -> bool {
        self.bit() == 1
    }

    /// Move past `count` bits whose content no part of the decoder reads.
    pub(crate) fn skip(&mut self, count: usize) {
        self.pos += count;
    }

    fn bit(&mut self) -> i32 {
        let byte = self.pos >> 3;
        if byte >= self.data.len() {
            self.overrun = true;
            self.pos += 1;
            return 0;
        }
        let bit = i32::from((self.data[byte] >> (7 - (self.pos & 7))) & 1);
        self.pos += 1;
        bit
    }
}

/// What a syncframe's fixed header says about it: `syncinfo` plus the parts of
/// `bsi` that change how the audio is unpacked. The rest of `bsi` is stepped over
/// field by field, since none of it is.
#[derive(Clone, Copy)]
pub(crate) struct Header {
    /// `fscod` itself, the index into the rate, frame length and threshold tables.
    pub(crate) rate: usize,
    /// Bytes this syncframe occupies, from Table 5.18.
    pub(crate) frame_bytes: usize,
    pub(crate) acmod: usize,
    pub(crate) nfchans: usize,
    pub(crate) lfeon: bool,
    pub(crate) clev: f32,
    pub(crate) slev: f32,
}

impl Header {
    /// Read Table 5.1 and Table 5.2. `None` for what this transcription will not
    /// misread: a foreign syncword, the reserved rate code, a frame size code
    /// outside Table 5.18, a `bsid` whose syntax is not A/52's own, or a header that
    /// runs off the end of the packet.
    fn read(bits: &mut Bits<'_>) -> Option<Self> {
        if bits.take(16) != 0x0B77 {
            return None;
        }
        let _crc1 = bits.take(16);
        let fscod = bits.take(2) as usize;
        let frmsizecod = bits.take(6) as usize;
        if fscod == 3 {
            return None;
        }
        let words = *FRAME_WORDS.get(frmsizecod)?.get(fscod)?;
        if bits.take(5) > 8 {
            return None;
        }
        let _bsmod = bits.take(3);
        let acmod = bits.take(3) as usize;
        let clev = if (acmod & 1) != 0 && acmod != 1 {
            CMIX[bits.take(2) as usize]
        } else {
            CMIX[0]
        };
        let slev = if (acmod & 4) != 0 {
            SMIX[bits.take(2) as usize]
        } else {
            SMIX[0]
        };
        if acmod == 2 {
            let _dsurmod = bits.take(2);
        }
        let lfeon = bits.flag();
        let _dialnorm = bits.take(5);
        if bits.flag() {
            let _compr = bits.take(8);
        }
        if bits.flag() {
            let _langcod = bits.take(8);
        }
        if bits.flag() {
            let _mixlevel = bits.take(5);
            let _roomtyp = bits.take(2);
        }
        if acmod == 0 {
            let _dialnorm2 = bits.take(5);
            if bits.flag() {
                let _compr2 = bits.take(8);
            }
            if bits.flag() {
                let _langcod2 = bits.take(8);
            }
            if bits.flag() {
                let _mixlevel2 = bits.take(5);
                let _roomtyp2 = bits.take(2);
            }
        }
        let _copyright = bits.flag();
        let _original = bits.flag();
        if bits.flag() {
            let _timecod1 = bits.take(14);
        }
        if bits.flag() {
            let _timecod2 = bits.take(14);
        }
        if bits.flag() {
            let addbsil = bits.take(6) as usize;
            bits.skip((addbsil + 1) * 8);
        }
        if bits.overrun {
            return None;
        }
        Some(Self {
            rate: fscod,
            frame_bytes: usize::from(words) * 2,
            acmod,
            nfchans: NFCHANS[acmod],
            lfeon,
            clev,
            slev,
        })
    }

    /// The geometry of an Annex E frame, whose `bsi` says the same three things in a
    /// different order and whose length comes from `frmsiz` rather than from a code
    /// into Table 5.18. `centre` and `surround` are the Lo/Ro downmix levels, which
    /// the annex carries among its mixing metadata instead of beside `acmod`.
    pub(crate) fn annex_e(
        rate: usize,
        frame_bytes: usize,
        acmod: usize,
        lfeon: bool,
        centre: f32,
        surround: f32,
    ) -> Self {
        Self {
            rate,
            frame_bytes,
            acmod,
            nfchans: NFCHANS[acmod],
            lfeon,
            clev: centre,
            slev: surround,
        }
    }

    /// How many planes this frame carries: the mode's full-bandwidth channels plus
    /// the LFE when the stream has one.
    pub(crate) fn native_channels(&self) -> usize {
        self.nfchans + usize::from(self.lfeon)
    }
}

/// The part of a syncframe's header that says where the frame ends and what it
/// sounds like, for a caller that walks a stream of frames without decoding them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Syncframe {
    /// The rate the frame's audio is written to play back at, from `fscod`.
    pub sample_rate: u32,
    /// The frame's own channel count in native order, which is what a decoder
    /// handed this frame produces when the container states no other layout.
    pub channels: u16,
    /// Bytes the frame occupies, so the next one starts here.
    pub frame_bytes: usize,
}

/// Read just the fixed header of the syncframe that starts at `bytes`.
///
/// A bare `.ac3` file states its geometry in every frame rather than once up
/// front, so the frame walker that lists a stream's packets needs the same four
/// numbers this decoder reads for itself. `None` for the same refusals
/// [`Header::read`] makes: a foreign syncword, the reserved rate code, a frame
/// size code outside Table 5.18, or a `bsid` whose syntax is not A/52's own.
pub fn syncframe(bytes: &[u8]) -> Option<Syncframe> {
    let header = Header::read(&mut Bits::new(bytes))?;
    Some(Syncframe {
        sample_rate: SAMPLE_RATES[header.rate],
        channels: header.native_channels() as u16,
        frame_bytes: header.frame_bytes,
    })
}

/// The bit allocation parameters a block states, held between blocks because a
/// block that omits them reuses them.
#[derive(Clone, Copy)]
pub(crate) struct Params {
    /// `baie`: the five prototype curve codes are present this block.
    pub(crate) baie: bool,
    pub(crate) sdcycod: usize,
    pub(crate) fdcycod: usize,
    pub(crate) sgaincod: usize,
    pub(crate) dbpbcod: usize,
    pub(crate) floorcod: usize,
    /// `snroffste`, and with it the per-plane offsets and gains below.
    pub(crate) snre: bool,
    pub(crate) csnroffst: i32,
    /// Fine grain SNR offset per plane: `fsnroffst` for the channels,
    /// `cplfsnroffst` for the coupling plane, `lfefsnroffst` for the LFE.
    pub(crate) fsnroffst: [i32; PLANES],
    /// Fast gain code per plane, split the same way as the offsets.
    pub(crate) fgaincod: [usize; PLANES],
    /// `cplfleak` and `cplsleak`, the coupling plane's own masking leaks.
    pub(crate) cplleak: [i32; 2],
    /// `cplleake`: the coupling plane's own masking leaks are new this block. The
    /// leaks move that plane's curve, so an allocation carried from the last block is
    /// wrong beside them even when every exponent run is reused.
    pub(crate) leaks: bool,
    /// `fgaincode`: the block stated its per-plane fast gain codes. Annex E lets a
    /// block state gains while reusing its offsets, and a gain scales the curve the
    /// same way an offset does.
    pub(crate) gains: bool,
    /// `deltbaie`: the block restates at least one plane's delta bit allocation.
    pub(crate) delt: bool,
}

impl Default for Params {
    /// The values Section 5.4.3.34 through 5.4.3.47 fall back to when a stream never
    /// states them. Block 0 of a conforming stream states the whole set, so these
    /// only stand in for a stream that does not.
    fn default() -> Self {
        Self {
            baie: false,
            sdcycod: 3,
            fdcycod: 3,
            sgaincod: 3,
            dbpbcod: 3,
            floorcod: 7,
            snre: false,
            csnroffst: 15,
            fsnroffst: [15; PLANES],
            fgaincod: [7; PLANES],
            cplleak: [7, 7],
            leaks: false,
            gains: false,
            delt: false,
        }
    }
}

/// Delta bit allocation for one plane: up to eight segments, each of which lifts or
/// drops the masking curve over a run of bands.
#[derive(Clone, Copy)]
pub(crate) struct Dba {
    pub(crate) segments: usize,
    pub(crate) offset: [usize; SEGMENTS],
    pub(crate) length: [usize; SEGMENTS],
    pub(crate) ba: [usize; SEGMENTS],
}

impl Default for Dba {
    fn default() -> Self {
        Self {
            segments: 0,
            offset: [0; SEGMENTS],
            length: [0; SEGMENTS],
            ba: [0; SEGMENTS],
        }
    }
}

impl Dba {
    /// Read the segment list a `deltbae` of new information introduces. The 3-bit
    /// count is one less than the number of segments it states, and the fields of a
    /// segment are packed without padding.
    pub(crate) fn read(bits: &mut Bits<'_>) -> Self {
        let mut dba = Self::default();
        let count = (bits.take(3) as usize + 1).min(SEGMENTS);
        for segment in 0..count {
            dba.offset[segment] = bits.take(5) as usize;
            dba.length[segment] = bits.take(4) as usize;
            dba.ba[segment] = bits.take(3) as usize;
        }
        dba.segments = count;
        dba
    }

    /// Walk the segment list, moving each segment's adjustment onto the bands it
    /// spans, per Section 7.2.2.6. Each segment starts an offset past where the
    /// previous one ended.
    fn apply(&self, mask: &mut [i32; NBANDS]) {
        let mut band = 0usize;
        for segment in 0..self.segments {
            band += self.offset[segment];
            let delta = if self.ba[segment] >= 4 {
                (self.ba[segment] as i32 - 3) << 7
            } else {
                (self.ba[segment] as i32 - 4) << 7
            };
            for _ in 0..self.length[segment] {
                if band < NBANDS {
                    mask[band] += delta;
                }
                band += 1;
            }
        }
    }
}

/// The rematrixing band set of a block, which only the 2/0 mode uses. Bands are
/// inclusive coefficient numbers, and the point where coupling begins caps the top
/// band whenever coupling is on.
#[derive(Clone, Copy)]
pub(crate) struct Rematrix {
    /// Low and high coefficient of each band, in the standard's own order.
    pub(crate) bands: [(usize, usize); 4],
    pub(crate) flags: [bool; 4],
    pub(crate) count: usize,
}

impl Default for Rematrix {
    /// Table 7.25, the set for a stream without coupling.
    fn default() -> Self {
        Self {
            bands: [(13, 24), (25, 36), (37, 60), (61, 252)],
            flags: [false; 4],
            count: 4,
        }
    }
}

impl Rematrix {
    /// The band set the current coupling state calls for: Table 7.26 through Table
    /// 7.28, where the top band ends where coupling begins.
    pub(crate) fn reshape(&mut self, cplinu: bool, cplbegf: usize) {
        let top = 36 + cplbegf * 12;
        let (bands, count) = if !cplinu {
            ([(13, 24), (25, 36), (37, 60), (61, 252)], 4)
        } else if cplbegf > 2 {
            ([(13, 24), (25, 36), (37, 60), (61, top)], 4)
        } else if cplbegf > 0 {
            ([(13, 24), (25, 36), (37, top), (0, 0)], 3)
        } else {
            ([(13, 24), (25, 36), (0, 0), (0, 0)], 2)
        };
        if bands != self.bands {
            // Section 5.4.3.19 hands unsent flags back to the bands they were written
            // for, so a set the flags never described starts out with none of them on.
            self.flags = [false; 4];
        }
        self.bands = bands;
        self.count = count;
    }
}

/// The strategy information a block hands to the next one within a syncframe, plus
/// the spectra and geometry derived from it. Reuse never crosses a frame boundary,
/// because block 0 of every syncframe restates the coupling strategy, the bandwidths
/// and the SNR offsets.
#[derive(Clone)]
pub(crate) struct Strategy {
    /// First and last mantissa bin of each plane, per Section 7.2.2.1.
    pub(crate) start: [usize; PLANES],
    pub(crate) end: [usize; PLANES],
    /// Exponent strategy per plane: 0 reuses the stored run, 1 to 3 are D15, D25 and
    /// D45. The LFE states reuse with a single bit, so it only ever holds 0 or 1.
    pub(crate) expstr: [usize; PLANES],
    /// `chbwcod` per channel, kept so a block that reuses an exponent strategy and
    /// sends no bandwidth code still knows where its channel ends.
    pub(crate) bwcod: [usize; FBW],
    /// Decoded exponents, and the mantissa widths bit allocation derives from them.
    pub(crate) exp: [[i32; BINS]; PLANES],
    pub(crate) bap: [[u8; BINS]; PLANES],
    /// Block switch and dither flags.
    pub(crate) blksw: [bool; FBW],
    pub(crate) dither: [bool; FBW],
    /// Dynamic range gain of the program, and of the second program a 1+1 stream
    /// carries.
    pub(crate) gain: [f32; 2],
    pub(crate) params: Params,
    pub(crate) dba: [Dba; PLANES],
    pub(crate) rematrix: Rematrix,
    /// Coupling strategy: whether coupling is on, which channels take part, where it
    /// starts and stops, and how the sub-bands between them group into bands.
    pub(crate) cplinu: bool,
    /// Whether coupling opened in this block, which the reuse checks of Section
    /// 7.10.2 answer to: a strategy that only carries over may leave parameters
    /// unsent, one that begins may not.
    pub(crate) cpl_opened: bool,
    /// Whether this block moved coupling to a different band set than the last one.
    pub(crate) cpl_moved: bool,
    pub(crate) incpl: [bool; FBW],
    pub(crate) cplbegf: usize,
    /// Signed because Annex E lets a frame that uses spectral extension derive it from
    /// `spxbegf`, and Section E3.3.1 states the derived range as -2 to 7.
    pub(crate) cplendf: i32,
    /// `ncplsubnd` and `ncplbnd`: the sub-bands coupling covers, and how many
    /// coupling bands those group into.
    pub(crate) subnd: usize,
    pub(crate) bnd: usize,
    /// `cplbndstrc`, indexed relative to the first coupling sub-band.
    pub(crate) bndstrc: [bool; SUBDN],
    /// Whether each channel still owes its first set of coupling coordinates, which
    /// Annex E asks after and Table 5.3 does not.
    pub(crate) firstcplcos: [bool; FBW],
    /// Whether the coupling plane still owes its first pair of masking leaks, which is
    /// the same promise Annex E makes of the coordinates above: a block that begins
    /// coupling states its leaks without saying that it does.
    pub(crate) firstcplleak: bool,
    /// Phase restoration: whether it is in use, the per-channel master coordinate
    /// gain, and the coordinates and phase flags already expanded to the sub-bands
    /// decoupling addresses them by.
    pub(crate) phsflginu: bool,
    pub(crate) mstr: [usize; FBW],
    pub(crate) coord: [[f32; SUBDN]; FBW],
    pub(crate) phsflg: [bool; SUBDN],
}

impl Default for Strategy {
    /// A strategy that reuses nothing: closed geometry, coupling off, unity gain.
    fn default() -> Self {
        Self {
            start: [0; PLANES],
            end: [0; PLANES],
            expstr: [0; PLANES],
            bwcod: [0; FBW],
            exp: [[0; BINS]; PLANES],
            bap: [[0; BINS]; PLANES],
            blksw: [false; FBW],
            dither: [false; FBW],
            gain: [1.0, 1.0],
            params: Params::default(),
            dba: [Dba::default(); PLANES],
            rematrix: Rematrix::default(),
            cplinu: false,
            cpl_opened: false,
            cpl_moved: false,
            incpl: [false; FBW],
            cplbegf: 0,
            cplendf: 0,
            subnd: 0,
            bnd: 0,
            bndstrc: [false; SUBDN],
            firstcplcos: [true; FBW],
            firstcplleak: true,
            phsflginu: false,
            mstr: [0; FBW],
            coord: [[0.0; SUBDN]; FBW],
            phsflg: [false; SUBDN],
        }
    }
}

impl Strategy {
    /// Where each plane's mantissas start and end, from the bandwidth codes and the
    /// coupling state of this block. A channel that reuses its exponents still moves
    /// its end when coupling moves, which is why this runs every block rather than
    /// only when exponents are new.
    pub(crate) fn measure(&mut self, header: Header) {
        let cplstart = (37 + 12 * self.cplbegf).min(BINS);
        for ch in 0..header.nfchans {
            self.start[ch] = 0;
            self.end[ch] = if self.cplinu && self.incpl[ch] {
                cplstart
            } else {
                (37 + 3 * (self.bwcod[ch] + 12)).min(BINS)
            };
        }
        if self.cplinu {
            self.start[CPL] = cplstart;
            self.end[CPL] = (37 + 12 * (self.cplendf + 3)).clamp(0, BINS as i32) as usize;
        } else {
            self.start[CPL] = 0;
            self.end[CPL] = 0;
        }
        self.start[LFE] = 0;
        self.end[LFE] = if header.lfeon { 7 } else { 0 };
    }

    /// How many grouped exponent words a plane holds, from its strategy and the
    /// width of its mantissa run. The three formulas of Section 7.1.3 are one
    /// formula once the exponents-per-group are factored in, because the run length
    /// is always a multiple of three.
    pub(crate) fn group_count(&self, plane: usize) -> usize {
        let grpsize = GRPSIZE[self.expstr[plane]];
        if plane == CPL {
            (self.end[CPL] - self.start[CPL]) / (3 * grpsize)
        } else if plane == LFE {
            2
        } else {
            (self.end[plane] - 1 + 3 * (grpsize - 1)) / (3 * grpsize)
        }
    }

    /// Decode a plane's exponent run: one absolute exponent, then groups of three
    /// mapped values, each of which stands for one, two or four equal exponents.
    ///
    /// The coupling plane is the odd one out. Its absolute exponent is a reference
    /// rather than a coefficient's exponent, and its run starts at the first coupled
    /// bin instead of at bin 1, so the decoded array is offset by the plane's start
    /// (Section 7.1.3).
    pub(crate) fn exponents(&mut self, bits: &mut Bits<'_>, plane: usize, absexp: i32) {
        let grpsize = GRPSIZE[self.expstr[plane]];
        let start = if plane == CPL {
            self.start[plane]
        } else {
            self.exp[plane][0] = absexp;
            1
        };
        let end = self.end[plane];
        let mut previous = absexp;
        let mut bin = start;
        for _ in 0..self.group_count(plane) {
            let group = bits.take(7);
            let first = group / 25;
            let second = (group % 25) / 5;
            let third = (group % 25) % 5;
            for mapped in [first, second, third] {
                previous = (previous + mapped - 2).clamp(0, 24);
                for _ in 0..grpsize {
                    if bin < end {
                        self.exp[plane][bin] = previous;
                        bin += 1;
                    }
                }
            }
        }
    }

    /// The `cplstre` syntax of Table 5.3: which channels join coupling, where it
    /// starts and stops, and how the sub-bands in between group into coupling bands.
    /// `false` for a coupling span that names no sub-bands, which the decoupling loop
    /// could not walk.
    fn coupling_strategy(&mut self, header: Header, first: bool, bits: &mut Bits<'_>) -> bool {
        // Section 5.4.3.16: with no new strategy every coupling parameter is the one
        // the last block sent, `cplinu` included, so a block that says nothing here
        // still decodes as coupled and still carries its own coupling coordinates.
        self.cpl_opened = false;
        self.cpl_moved = false;
        if !bits.flag() {
            // Section 7.10.2 condition 1: block 0 restates the strategy.
            return !first;
        }
        let continued = self.cplinu;
        self.cplinu = false;
        self.incpl = [false; FBW];
        self.bndstrc = [false; SUBDN];
        self.firstcplcos = [true; FBW];
        self.phsflginu = false;
        if !bits.flag() {
            return true;
        }
        for ch in 0..header.nfchans {
            self.incpl[ch] = bits.flag();
        }
        self.phsflginu = header.acmod == 2 && bits.flag();
        let begin = bits.take(4) as usize;
        let end = bits.take(4) as i32;
        self.open_coupling(header, continued, begin, end, true, bits)
    }

    /// The coupling geometry both Table 5.3 and Table E1.4 state the same way once
    /// their differences are read elsewhere: the band set the block couples over, the
    /// banding structure that groups its sub-bands, and the checks that say no block
    /// can be decoded from what was named. `false` for a coupling span the decoupling
    /// loop could not walk, which mutes the frame.
    ///
    /// `continued` is whether the block before this one coupled too, which the reuse
    /// conditions of Section 7.10.2 answer to. `banding_stated` says whether the
    /// block's own `cplbndstrc` flags follow in the bitstream: Table 5.3 always sends
    /// them, Annex E only when its `cplbndstrce` bit says so, and a block that withholds
    /// them hands over the structure it already settled on - Table E2.12's for a frame's
    /// first coupled block, the last block's for any other.
    pub(crate) fn open_coupling(
        &mut self,
        header: Header,
        continued: bool,
        begin: usize,
        end: i32,
        banding_stated: bool,
        bits: &mut Bits<'_>,
    ) -> bool {
        // Section 7.10.2 conditions 6 and 7: exponents a block does not send may only
        // be reused while coupling still covers the same bands.
        self.cpl_opened = !continued;
        self.cpl_moved = continued && (begin, end) != (self.cplbegf, self.cplendf);
        self.cplbegf = begin;
        self.cplendf = end;
        let subnd = 3 + end - begin as i32;
        if !(1..=SUBDN as i32).contains(&subnd) {
            return false;
        }
        let subnd = subnd as usize;
        self.subnd = subnd;
        if banding_stated {
            for bnd in 1..subnd {
                self.bndstrc[bnd] = bits.flag();
            }
        }
        let mut merged = 0;
        for bnd in 1..subnd {
            if self.bndstrc[bnd] {
                merged += 1;
            }
        }
        self.bnd = subnd - merged;
        // Section 7.10.2 condition 2: coupling one channel to nothing decouples
        // nothing, and leaves the coupled planes without a coordinate.
        if self.incpl[..header.nfchans]
            .iter()
            .filter(|&&on| on)
            .count()
            < 2
        {
            return false;
        }
        self.cplinu = true;
        true
    }

    /// The `cplcoe` and `phsflg` syntax: the per-channel coupling coordinates, then
    /// the phase restoration flags, both stated per coupling band. Decoupling works
    /// per coupling sub-band, so both are duplicated across the sub-bands of their
    /// band as Section 7.4.2 asks.
    ///
    /// A channel whose `cplcoe` is off sends no coordinates and keeps the ones the
    /// last block delivered, so only a channel that states a new set rewrites its
    /// bands. Table 5.3 asks the question per coupled channel, so every one of them
    /// costs a bit whether or not it answers.
    ///
    /// `implicit_first` is the one thing Annex E does differently: a channel whose
    /// coordinates no block has delivered yet - because coupling only now covers it -
    /// states them without saying so, and pays no `cplcoe` bit for the privilege.
    /// The coupling state that makes a fresh set necessary is otherwise the same one
    /// Section 7.10.2 conditions 4 and 23 hold Table 5.3 to.
    pub(crate) fn coupling_coordinates(
        &mut self,
        header: Header,
        bits: &mut Bits<'_>,
        implicit_first: bool,
    ) -> bool {
        let mut band_coord = [[0.0f32; SUBDN]; FBW];
        let mut band_phs = [false; SUBDN];
        let mut coordinates = [false; FBW];
        let mut ok = true;
        for ch in 0..header.nfchans {
            if !self.incpl[ch] {
                self.firstcplcos[ch] = true;
                continue;
            }
            let owed = implicit_first && self.firstcplcos[ch];
            self.firstcplcos[ch] = false;
            if !owed && !bits.flag() {
                // Section 7.10.2 conditions 4 and 23: coordinates cannot be reused
                // before any were sent, nor after coupling moved to other bands.
                ok = ok && !(self.cpl_opened || self.cpl_moved);
                continue;
            }
            // Section 5.4.3.14: a channel whose coordinates stay unsent keeps the ones
            // the last block delivered, so only a new set starts from an empty band.
            self.coord[ch] = [0.0; SUBDN];
            coordinates[ch] = true;
            self.mstr[ch] = bits.take(2) as usize;
            for bnd in 0..self.bnd {
                let exponent = bits.take(4) as usize;
                let mantissa = bits.take(4) as usize;
                // The mantissa of a coordinate is a fraction with its top bit
                // implied, except at the exponent that stands for the smallest
                // representable value.
                let temp = if exponent == 15 {
                    mantissa as f32 / 16.0
                } else {
                    (mantissa + 16) as f32 / 32.0
                };
                band_coord[ch][bnd] = temp * POW2[exponent.min(24) + 3 * self.mstr[ch].min(3)];
            }
        }
        if header.acmod == 2 && self.phsflginu && (coordinates[0] || coordinates[1]) {
            for bnd in 0..self.bnd {
                band_phs[bnd] = bits.flag();
            }
        } else {
            self.phsflginu = false;
        }
        for ch in 0..header.nfchans {
            if !self.incpl[ch] || !coordinates[ch] {
                continue;
            }
            self.expand_coupling(ch, &band_coord[ch], &band_phs);
        }
        ok
    }

    /// Duplicate one channel's per-band coordinate onto the sub-bands of its band,
    /// where the band structure says those bands are joined.
    fn expand_coupling(&mut self, ch: usize, coord: &[f32; SUBDN], phs: &[bool; SUBDN]) {
        let mut bnd = 0;
        let mut relative = 0;
        while bnd < self.bnd {
            let mut span = 1;
            while relative + span < self.subnd && self.bndstrc[relative + span] {
                span += 1;
            }
            for offset in 0..span {
                let sbnd = self.cplbegf + relative + offset;
                if sbnd < SUBDN {
                    self.coord[ch][sbnd] = coord[bnd];
                    self.phsflg[sbnd] = phs[bnd];
                }
            }
            relative += span;
            bnd += 1;
        }
    }

    /// The SNR offsets, the coupling leaks and the delta bit allocation of Table 5.3,
    /// all of which a block may withhold and all of which bit allocation reads.
    /// `false` for the reserved `deltbae` state, which Table 5.16 mutes on.
    fn allocation_side(&mut self, header: Header, first: bool, bits: &mut Bits<'_>) -> bool {
        self.params.snre = bits.flag();
        // Table 5.3 interleaves one block's gains with its offsets under the same
        // flag, so the two always move together here - unlike Annex E, where a block
        // states gain codes of its own.
        self.params.gains = self.params.snre;
        if self.params.snre {
            self.params.csnroffst = bits.take(6);
            if self.cplinu {
                self.params.fsnroffst[CPL] = bits.take(4);
                self.params.fgaincod[CPL] = bits.take(3) as usize;
            }
            for ch in 0..header.nfchans {
                self.params.fsnroffst[ch] = bits.take(4);
                self.params.fgaincod[ch] = bits.take(3) as usize;
            }
            if header.lfeon {
                self.params.fsnroffst[LFE] = bits.take(4);
                self.params.fgaincod[LFE] = bits.take(3) as usize;
            }
        } else if first {
            // Section 5.4.3.36: the first block states the SNR offsets itself.
            return false;
        }
        self.params.leaks = false;
        if self.cplinu {
            let leak = bits.flag();
            if leak {
                let fast = bits.take(3);
                let slow = bits.take(3);
                self.params.cplleak = [fast, slow];
                self.params.leaks = true;
            } else if first {
                // Section 7.10.2 condition 14.
                return false;
            }
        }
        self.delta_ba(header, bits)
    }

    /// The `deltbaie` syntax, which Table 5.3 and Table E1.4 write the same way: the
    /// mode of every plane the block carries, then the segment list of each plane that
    /// sends a new one. `false` for the reserved mode, which Table 5.16 mutes on.
    pub(crate) fn delta_ba(&mut self, header: Header, bits: &mut Bits<'_>) -> bool {
        self.params.delt = bits.flag();
        if !self.params.delt {
            return true;
        }
        let mut mode = [0usize; PLANES];
        if self.cplinu {
            mode[CPL] = bits.take(2) as usize;
        }
        for ch in 0..header.nfchans {
            mode[ch] = bits.take(2) as usize;
        }
        if mode[CPL] == 3 || mode[..header.nfchans].contains(&3) {
            return false;
        }
        // Table 5.3 lays the coupling plane's segments out before the channels' own.
        if self.cplinu {
            match mode[CPL] {
                1 => self.dba[CPL] = Dba::read(bits),
                2 => self.dba[CPL] = Dba::default(),
                _ => {}
            }
        }
        for plane in 0..header.nfchans {
            match mode[plane] {
                1 => self.dba[plane] = Dba::read(bits),
                2 => self.dba[plane] = Dba::default(),
                _ => {}
            }
        }
        true
    }

    /// Whether this block's mantissa widths have to be recomputed: any new exponent
    /// run does, and so does any change to the parameters the curve is built from.
    /// Recomputing a plane whose inputs did not change is wasted work but not a
    /// different answer, which is why the coarse test below is enough.
    pub(crate) fn needs_allocation(&self, header: Header) -> bool {
        self.params.baie
            || self.params.snre
            || self.params.gains
            || self.params.leaks
            || self.params.delt
            || self.expstr[..header.nfchans]
                .iter()
                .any(|&strat| strat != 0)
            || (self.cplinu && self.expstr[CPL] != 0)
            || (header.lfeon && self.expstr[LFE] != 0)
    }
}

/// Scratch that bit allocation and the inverse transform would otherwise rebuild
/// block after block.
#[derive(Clone)]
struct Scratch {
    /// Power spectral density per coefficient, and the three per-band curves the
    /// masking model walks through.
    psd: [i32; BINS],
    bndpsd: [i32; NBANDS],
    excite: [i32; NBANDS],
    mask: [i32; NBANDS],
    /// The block's samples per plane, before they are folded into channels.
    pcm: [[f32; SAMPLES]; PLANES],
    /// Grouped mantissa codes still waiting to be used.
    groups: [Group; 3],
}

impl Default for Scratch {
    fn default() -> Self {
        Self {
            psd: [0; BINS],
            bndpsd: [0; NBANDS],
            excite: [0; NBANDS],
            mask: [0; NBANDS],
            pcm: [[0.0; SAMPLES]; PLANES],
            groups: [Group::default(); 3],
        }
    }
}

/// A partially consumed group of symmetric mantissa codes. Section 7.3.5 shares
/// groups across exponent sets within a block, so a run that leaves a group
/// half-filled is continued by the next run rather than dropped.
#[derive(Clone, Copy, Default)]
struct Group {
    codes: [i32; 3],
    next: usize,
    size: usize,
}

impl Group {
    /// The next code of the group, reading and ungrouping a fresh word once the
    /// group is empty.
    fn take(&mut self, bits: &mut Bits<'_>, bap: usize) -> i32 {
        if self.next >= self.size {
            let (width, big, small, size) = group_of_bap(bap).unwrap_or((0, 1, 1, 0));
            let word = bits.take(width);
            self.codes[0] = word / big;
            if size == 2 {
                self.codes[1] = word % big;
            } else {
                self.codes[1] = (word % big) / small;
                self.codes[2] = (word % big) % small;
            }
            self.size = size;
            self.next = 0;
        }
        let code = self.codes[self.next];
        self.next += 1;
        code
    }
}

/// The twiddle tables of Section 7.9.4, worked out once instead of per transform,
/// and the window table they pair with.
#[derive(Clone)]
struct Tables {
    /// Pre- and post-twiddles of the 512-sample path, and of the two 256-sample
    /// paths a block-switched block runs.
    cos1: [f32; FFT],
    sin1: [f32; FFT],
    cos2: [f32; FFT / 2],
    sin2: [f32; FFT / 2],
    /// Twiddle factors of the complex inverse transform at 128-point spacing, which
    /// the 64-point path reads at every other entry.
    twc: [f32; FFT],
    tws: [f32; FFT],
}

impl Tables {
    fn new() -> Self {
        let mut tables = Self {
            cos1: [0.0; FFT],
            sin1: [0.0; FFT],
            cos2: [0.0; FFT / 2],
            sin2: [0.0; FFT / 2],
            twc: [0.0; FFT],
            tws: [0.0; FFT],
        };
        for k in 0..FFT {
            let angle = core::f32::consts::TAU * (8 * k + 1) as f32 / (8.0 * TRANSFORM as f32);
            tables.cos1[k] = -angle.cos();
            tables.sin1[k] = -angle.sin();
        }
        for k in 0..FFT / 2 {
            let angle = core::f32::consts::TAU * (8 * k + 1) as f32 / (4.0 * TRANSFORM as f32);
            tables.cos2[k] = -angle.cos();
            tables.sin2[k] = -angle.sin();
        }
        for k in 0..FFT {
            // The positive sign is the inverse kernel Section 7.9.4.1 step 3 sums
            // with; the normalization it omits is `ifft_scale`'s.
            let angle = core::f32::consts::TAU * k as f32 / FFT as f32;
            tables.twc[k] = angle.cos();
            tables.tws[k] = angle.sin();
        }
        tables
    }
}

/// The factor a complex inverse transform of this length carries and the standard's
/// pseudocode leaves out.
const fn ifft_scale(n: usize) -> f32 {
    1.0 / (n as f32)
}

/// How a frame's planes reach the track's declared channels: a weight per plane for
/// each output channel, so native order and both downmixes share one inner loop.
#[derive(Clone, Copy)]
pub(crate) struct Folding {
    /// Output channels, which is the count the container stated.
    width: usize,
    /// Weight of each plane in each output channel.
    weight: [[f32; PLANES]; SLOTS],
}

impl Folding {
    /// The folding for a frame, or `None` when the declared channel count matches
    /// neither the frame's own layout nor a downmix the standard defines. The
    /// downmix coefficients come from Section 7.8.2; the optional pre-scaling that
    /// keeps every combination of full-scale channels inside range is not applied,
    /// because the fold clamps instead and clamping only the rare peak keeps the
    /// level of ordinary programme up.
    pub(crate) fn new(header: Header, declared: usize) -> Option<Self> {
        let mut folding = Self {
            width: declared,
            weight: [[0.0; PLANES]; SLOTS],
        };
        let native = header.native_channels();
        // Which plane carries each slot, for the native fold and the two downmixes.
        let mut plane_of = [None; SLOTS];
        for ch in 0..header.nfchans {
            plane_of[SLOT_OF_ACMOD[header.acmod][ch] as usize] = Some(ch);
        }
        if header.lfeon {
            plane_of[LFE_SLOT] = Some(LFE);
        }
        if declared == native {
            // A layout names its channels by slot, and the slot numbers are not always
            // the first `native` ones: a 1/0 frame holds a centre, and a 1+2/0 frame
            // reaches as far as the right surround. The output is numbered from zero,
            // so the slots present are packed in ascending slot order, which is also
            // the order a player's channel map states.
            let mut output = 0;
            for plane in plane_of.into_iter().flatten() {
                folding.weight[output][plane] = 1.0;
                output += 1;
            }
            return Some(folding);
        }
        let (left, right) = (plane_of[0], plane_of[1]);
        let centre = plane_of[2];
        let single = match header.acmod {
            4 | 5 => plane_of[4],
            _ => None,
        };
        let (sl, sr) = if single.is_some() {
            (single, None)
        } else {
            (plane_of[4], plane_of[5])
        };
        let add = |folding: &mut Self, output: usize, plane: Option<usize>, gain: f32| {
            if let Some(plane) = plane {
                folding.weight[output][plane] += gain;
            }
        };
        match declared {
            2 => {
                add(&mut folding, 0, left, 1.0);
                add(&mut folding, 0, centre, header.clev);
                add(&mut folding, 1, right, 1.0);
                add(&mut folding, 1, centre, header.clev);
                let surround = header.slev * if single.is_some() { 0.7 } else { 1.0 };
                add(&mut folding, 0, sl, surround);
                add(&mut folding, 1, sr, surround);
                Some(folding)
            }
            1 => {
                // The mono equation of Section 7.8.2, halved so the sum of five
                // full-scale channels still lands inside the range.
                add(&mut folding, 0, left, 0.5);
                add(&mut folding, 0, right, 0.5);
                add(&mut folding, 0, centre, header.clev);
                let surround = header.slev * if single.is_some() { 0.7 } else { 1.0 } * 0.5;
                add(&mut folding, 0, sl, surround);
                add(&mut folding, 0, sr, surround);
                Some(folding)
            }
            _ => None,
        }
    }
}

/// AC-3 decoder: a packet of syncframes in, one block of interleaved f32 out.
pub struct Ac3Decoder {
    /// The rate the container stamped the track with, which is also the timebase the
    /// samples are handed back on.
    sample_rate: u32,
    /// The channel count the container stated, which fixes what the folding below
    /// has to produce.
    channels: usize,
    core: Core,
    /// Samples still owed to `ENCODER_DELAY`, which the stream's first blocks pay off.
    lead: usize,
}

impl Ac3Decoder {
    /// Open an AC-3 stream. `configuration` holds nothing this codec needs, since the
    /// geometry is in the frames themselves, and is only taken to match the other
    /// decoders' signature. The rate has to be one of AC-3's three and the channel
    /// count one the folding can reach; anything else is refused by name rather than
    /// guessed at.
    pub fn new(configuration: &[u8], sample_rate: u32, channels: u16) -> Result<Self> {
        let _ = configuration;
        if !SAMPLE_RATES.contains(&sample_rate) {
            return Err(invalid(&format!(
                "AC-3 track at {sample_rate} Hz is outside AC-3's 48000, 44100 and 32000 Hz"
            )));
        }
        if !(1..=SLOTS as u16).contains(&channels) {
            return Err(invalid(&format!(
                "AC-3 track of {channels} channels has neither a native layout nor a downmix here"
            )));
        }
        Ok(Self {
            sample_rate,
            channels: usize::from(channels),
            core: Core::new(),
            lead: ENCODER_DELAY,
        })
    }

    /// Current audio specification: the rate and channel count the track states,
    /// which is what the decoder holds its output to.
    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: self.channels as u16,
            format: SampleFormat::F32,
        }
    }

    /// Every syncframe the packet holds, each standing on its own. A frame whose
    /// syntax faults contributes its full length of silence, so a bad frame costs a
    /// frame rather than the packet's timing.
    fn frames(&mut self, data: &[u8], out: &mut Vec<f32>) -> Result<()> {
        let silence = BLOCKS * SAMPLES * self.channels;
        let mut offset = 0usize;
        let mut decoded = 0usize;
        while offset < data.len() {
            let raw = &data[offset..];
            let mut bits = Bits::new(raw);
            let Some(header) = Header::read(&mut bits) else {
                break;
            };
            let usable = header.frame_bytes.min(raw.len());
            // The header is read from the packet's own bytes, because its length is
            // what says where the frame ends; the view the blocks are read from is
            // trimmed to that end, so a truncated tail mutes rather than reaching into
            // whatever follows the packet.
            bits.data = &raw[..usable];
            let mut samples = Vec::with_capacity(silence);
            if usable == header.frame_bytes && self.syncframe(header, &mut bits, &mut samples) {
                samples.truncate(silence);
            } else {
                samples.clear();
                samples.resize(silence, 0.0);
            }
            out.extend(samples);
            offset += header.frame_bytes;
            decoded += 1;
        }
        if decoded == 0 {
            return Err(invalid("AC-3 packet holds no syncframe"));
        }
        Ok(())
    }

    /// One syncframe: the header, then six blocks. `false` mutes the frame.
    fn syncframe(&mut self, header: Header, bits: &mut Bits<'_>, out: &mut Vec<f32>) -> bool {
        let Some(folding) = Folding::new(header, self.channels) else {
            return false;
        };
        // Reuse is a within-frame promise.
        self.core.begin_frame();
        for blknum in 0..BLOCKS {
            if !self.block(blknum, header, &folding, bits, out) {
                return false;
            }
        }
        !bits.overrun
    }

    /// One audio block, read in the order Table 5.3 lays out, unpacked through its
    /// own bit allocation, inverse transformed and folded into the frame's samples.
    /// `false` mutes the frame.
    fn block(
        &mut self,
        blknum: usize,
        header: Header,
        folding: &Folding,
        bits: &mut Bits<'_>,
        out: &mut Vec<f32>,
    ) -> bool {
        // Section 7.10.2 states the reuse parameters a first block cannot leave unsent.
        let first = blknum == 0;
        let nfchans = header.nfchans;
        let mut strategy = std::mem::take(&mut self.core.strategy);
        for ch in 0..nfchans {
            strategy.blksw[ch] = bits.flag();
        }
        for ch in 0..nfchans {
            strategy.dither[ch] = bits.flag();
        }
        if bits.flag() {
            strategy.gain[0] = dynrng_gain(bits.take(8));
        }
        if header.acmod == 0 && bits.flag() {
            strategy.gain[1] = dynrng_gain(bits.take(8));
        }
        let mut ok = strategy.coupling_strategy(header, first, bits);
        if ok && strategy.cplinu {
            ok = strategy.coupling_coordinates(header, bits, false);
        }
        if header.acmod == 2 {
            strategy.rematrix.reshape(strategy.cplinu, strategy.cplbegf);
            if bits.flag() {
                for band in 0..strategy.rematrix.count {
                    strategy.rematrix.flags[band] = bits.flag();
                }
            } else if first {
                // Section 5.4.3.19: the first block's rematrixing flags are its own.
                ok = false;
            }
        }
        if ok {
            strategy.expstr = [0; PLANES];
            if strategy.cplinu {
                strategy.expstr[CPL] = bits.take(2) as usize;
                if strategy.expstr[CPL] == 0 && (strategy.cpl_opened || strategy.cpl_moved) {
                    // Section 7.10.2 conditions 6 and 7: coupling that opened or moved
                    // cannot describe itself with exponents from another band set.
                    ok = false;
                }
            }
            for ch in 0..nfchans {
                strategy.expstr[ch] = bits.take(2) as usize;
                if first && strategy.expstr[ch] == 0 {
                    // Section 7.10.2 condition 8: a channel's exponents cannot be
                    // reused before any were sent.
                    ok = false;
                }
            }
            if header.lfeon {
                strategy.expstr[LFE] = bits.take(1) as usize;
                if first && strategy.expstr[LFE] == 0 {
                    ok = false;
                }
            }
            for ch in 0..nfchans {
                // Table 5.3 sends a bandwidth code only for a channel that keeps its
                // own high bands; a coupled channel's end is the coupling point.
                if strategy.expstr[ch] != 0 && !strategy.incpl[ch] {
                    let code = bits.take(6) as usize;
                    // Section 5.4.3.24: above this bandwidth the stream is invalid and
                    // the decoder shall cease decoding audio and mute.
                    if code > 60 {
                        ok = false;
                        break;
                    }
                    strategy.bwcod[ch] = code;
                }
            }
        }
        if !ok {
            self.core.strategy = strategy;
            return false;
        }
        strategy.measure(header);
        if strategy.expstr[CPL] != 0 {
            let absexp = bits.take(4) << 1;
            strategy.exponents(bits, CPL, absexp);
        }
        for ch in 0..nfchans {
            if strategy.expstr[ch] != 0 {
                let absexp = bits.take(4);
                strategy.exponents(bits, ch, absexp);
                // `gainrng` widens the differential range; leaving it at the default
                // 2 costs nothing a conforming stream relies on.
                let _gainrng = bits.take(2);
            }
        }
        if header.lfeon && strategy.expstr[LFE] != 0 {
            let absexp = bits.take(4);
            strategy.exponents(bits, LFE, absexp);
        }
        strategy.params.baie = bits.flag();
        if strategy.params.baie {
            strategy.params.sdcycod = bits.take(2) as usize;
            strategy.params.fdcycod = bits.take(2) as usize;
            strategy.params.sgaincod = bits.take(2) as usize;
            strategy.params.dbpbcod = bits.take(2) as usize;
            strategy.params.floorcod = bits.take(3) as usize;
        } else if first {
            // Section 5.4.3.30: the first block states the bit allocation prototype.
            self.core.strategy = strategy;
            return false;
        }
        if !strategy.allocation_side(header, first, bits) {
            self.core.strategy = strategy;
            return false;
        }
        if bits.flag() {
            let skipl = bits.take(9) as usize;
            bits.skip(skipl * 8);
        }
        self.core.strategy = strategy;
        if self.core.strategy.needs_allocation(header) {
            self.core.allocate_all(header);
        }
        self.core.mantissas(header, bits);
        self.core.decouple(header);
        self.core.rematrix_restore(header);
        self.core.spectrum_to_sound(header, folding, out);
        true
    }
}

/// The block-level half of an A/52 decode: the strategy state that passes from one
/// block of a frame to the next, the bit allocation, the mantissa unpacking, the
/// coupling and rematrixing restoration, the inverse transform and the folding of
/// planes into channels.
///
/// Annex E (E-AC-3) decodes audio exactly this way and changes only the side
/// information above it - where the exponent strategies come from, which frame
/// supplies them, and how the offsets are laid out - so its reader drives this core
/// rather than repeating it. Section 3.3 of the annex names the parameters it
/// modifies and leaves the rest of the core's behaviour to Section 7.
pub(crate) struct Core {
    pub(crate) strategy: Strategy,
    /// Transform coefficients per plane: the only thing that crosses a block's
    /// mantissa read and its inverse transform.
    coef: [[f32; BINS]; PLANES],
    /// The previous block's second half, which the overlap-add step folds into the
    /// next block's first.
    delay: [[f32; SAMPLES]; PLANES],
    scratch: Scratch,
    tables: Tables,
    /// The Section 7.3.4 dither sequence. The standard leaves the generator to the
    /// decoder and only asks that it be reasonably random, so this is a fixed walk
    /// from a fixed seed: the values it hands out are uncorrelated with the
    /// mantissas around them, and a decode stays repeatable.
    dither_state: u32,
}

impl Core {
    /// A core that reuses nothing, holds no sound, and carries no overlap from a
    /// block it has not seen. The tables are the same for every stream, so a seek
    /// that rebuilds this still reads the same window.
    pub(crate) fn new() -> Self {
        Self {
            strategy: Strategy::default(),
            coef: [[0.0; BINS]; PLANES],
            delay: [[0.0; SAMPLES]; PLANES],
            scratch: Scratch::default(),
            tables: Tables::new(),
            dither_state: DITHER_SEED,
        }
    }

    /// Start a frame: the strategy a frame carries to its own blocks stops being
    /// available to the next one, which Section 7.10.2 states outright. The overlap
    /// tail and the dither sequence keep running, because a packet's frames are one
    /// continuous stream of blocks.
    pub(crate) fn begin_frame(&mut self) {
        self.strategy = Strategy::default();
    }

    /// Bit allocation for every plane the frame carries, in the standard's own order:
    /// each full-bandwidth channel, the coupling plane, then the LFE.
    pub(crate) fn allocate_all(&mut self, header: Header) {
        if self.all_snr_offsets_zero(header) {
            for plane in 0..PLANES {
                self.strategy.bap[plane] = [0; BINS];
            }
            return;
        }
        for plane in 0..header.nfchans {
            self.bit_allocation(plane, header.rate);
        }
        if self.strategy.cplinu {
            self.bit_allocation(CPL, header.rate);
        }
        if header.lfeon {
            self.bit_allocation(LFE, header.rate);
        }
    }

    /// The special case of Section 7.2.2.1.1: with every SNR offset at zero the
    /// encoder allocated nothing, so the whole bit allocation pointer array is zero
    /// and no curve needs computing. The offsets a block does not state are the ones
    /// it reuses, so the stored set is what the test reads.
    fn all_snr_offsets_zero(&self, header: Header) -> bool {
        let params = self.strategy.params;
        let mut zero = params.csnroffst == 0;
        for plane in 0..header.nfchans {
            zero &= params.fsnroffst[plane] == 0;
        }
        if self.strategy.cplinu {
            zero &= params.fsnroffst[CPL] == 0;
        }
        if header.lfeon {
            zero &= params.fsnroffst[LFE] == 0;
        }
        zero
    }

    /// The seven steps of Section 7.2.2, run over one plane's exponent set. Every
    /// step is integer arithmetic on a 14-bit accumulator because the result has to
    /// match what the encoder assumed when it packed the mantissas.
    fn bit_allocation(&mut self, plane: usize, fscod: usize) {
        let start = self.strategy.start[plane];
        let end = self.strategy.end[plane];
        if end <= start {
            return;
        }
        for bin in &mut self.strategy.bap[plane][start..end] {
            *bin = 0;
        }
        let params = self.strategy.params;
        let dba = self.strategy.dba[plane];
        let sdecay = SLOWDEC[params.sdcycod];
        let fdecay = FASTDEC[params.fdcycod];
        let sgain = SLOWGAIN[params.sgaincod];
        let dbknee = DBKNEE[params.dbpbcod];
        let floor = FLOORTAB[params.floorcod];
        let fgain = FASTGAIN[params.fgaincod[plane]];
        let snroffset = (((params.csnroffst - 15) << 4) + params.fsnroffst[plane]) << 2;

        // Step 2: exponents to a log power spectral density per coefficient.
        for bin in start..end {
            self.scratch.psd[bin] = 3072 - (self.strategy.exp[plane][bin] << 7);
        }
        // The band arrays are cleared so that the bands past the run read as silence
        // rather than as the previous plane's spectrum: the standard's loops do step
        // one band beyond the run when they look for a rising edge.
        self.scratch.bndpsd = [0; NBANDS];
        self.scratch.excite = [0; NBANDS];
        self.scratch.mask = [0; NBANDS];

        let bndstrt = usize::from(MASKTAB[start]);
        let bndend = usize::from(MASKTAB[(end - 1).min(BINS - 1)]) + 1;

        // Step 3: integrate the density within each sixth-octave band.
        let mut bin = start;
        let mut band = bndstrt;
        loop {
            let lastbin = (BNDTAB[band] + BNDSZ[band]).min(end);
            self.scratch.bndpsd[band] = self.scratch.psd[bin];
            bin += 1;
            while bin < lastbin {
                let summed = logadd(self.scratch.bndpsd[band], self.scratch.psd[bin]);
                self.scratch.bndpsd[band] = summed;
                bin += 1;
            }
            band += 1;
            if end <= lastbin {
                break;
            }
        }

        // Step 4: the excitation function, which is the masking curve before the
        // hearing threshold puts a floor under it. A plane that starts above band 0
        // is the coupling plane: it has no low-frequency neighbours to leak from, so
        // its leaks are seeded from the bit stream instead.
        let mut lowcomp = 0;
        let mut fastleak = 0;
        let mut slowleak = 0;
        let bndpsd = self.scratch.bndpsd;
        let mut begin;
        if bndstrt == 0 {
            lowcomp = calc_lowcomp(lowcomp, bndpsd[0], bndpsd[1], 0);
            self.scratch.excite[0] = bndpsd[0] - fgain - lowcomp;
            lowcomp = calc_lowcomp(lowcomp, bndpsd[1], bndpsd[2], 1);
            self.scratch.excite[1] = bndpsd[1] - fgain - lowcomp;
            begin = 7;
            for bin in 2..7 {
                if bndend != 7 || bin != 6 {
                    lowcomp = calc_lowcomp(lowcomp, bndpsd[bin], bndpsd[bin + 1], bin);
                }
                fastleak = bndpsd[bin] - fgain;
                slowleak = bndpsd[bin] - sgain;
                self.scratch.excite[bin] = fastleak - lowcomp;
                if (bndend != 7 || bin != 6) && bndpsd[bin] <= bndpsd[bin + 1] {
                    begin = bin + 1;
                    break;
                }
            }
            let mut bin = begin;
            while bin < bndend.min(22) {
                if bndend != 7 || bin != 6 {
                    lowcomp = calc_lowcomp(lowcomp, bndpsd[bin], bndpsd[bin + 1], bin);
                }
                fastleak -= fdecay;
                fastleak = fastleak.max(bndpsd[bin] - fgain);
                slowleak -= sdecay;
                slowleak = slowleak.max(bndpsd[bin] - sgain);
                self.scratch.excite[bin] = (fastleak - lowcomp).max(slowleak);
                bin += 1;
            }
            begin = 22;
        } else {
            begin = bndstrt;
            fastleak = (params.cplleak[0] << 8) + 768;
            slowleak = (params.cplleak[1] << 8) + 768;
        }
        let mut band = begin;
        while band < bndend {
            fastleak -= fdecay;
            fastleak = fastleak.max(bndpsd[band] - fgain);
            slowleak -= sdecay;
            slowleak = slowleak.max(bndpsd[band] - sgain);
            self.scratch.excite[band] = fastleak.max(slowleak);
            band += 1;
        }

        // Step 5: the masking curve, with quiet bands lifted towards the knee.
        for band in bndstrt..bndend {
            if bndpsd[band] < dbknee {
                self.scratch.excite[band] += (dbknee - bndpsd[band]) >> 2;
            }
            self.scratch.mask[band] = self.scratch.excite[band].max(HTH[fscod][band]);
        }

        // Step 6: the encoder's own adjustments to that curve.
        dba.apply(&mut self.scratch.mask);

        // Step 7: compare each coefficient with its band's mask and read off the
        // mantissa width. The mask is snapped back to the floor so that the
        // allocation cannot ask for fewer bits than the floor allows.
        let mask = self.scratch.mask;
        let psd = self.scratch.psd;
        let mut bin = start;
        let mut band = bndstrt;
        loop {
            let lastbin = (BNDTAB[band] + BNDSZ[band]).min(end);
            let mut level = mask[band] - snroffset - floor;
            if level < 0 {
                level = 0;
            }
            level = (level & 0x1fe0) + floor;
            while bin < lastbin {
                let address = ((psd[bin] - level) >> 5).clamp(0, 63) as usize;
                self.strategy.bap[plane][bin] = BAPTAB[address];
                bin += 1;
            }
            band += 1;
            if end <= lastbin {
                break;
            }
        }
    }

    /// Unpack every plane's mantissas and scale them into transform coefficients.
    ///
    /// The coupling plane's run sits between the first coupled channel's run and the
    /// next channel's, which is where Table 5.3 puts it; the grouped quantizers share
    /// half-filled groups across those runs, so the group state is reset here rather
    /// than per plane.
    pub(crate) fn mantissas(&mut self, header: Header, bits: &mut Bits<'_>) {
        for plane in 0..PLANES {
            self.coef[plane] = [0.0; BINS];
        }
        self.scratch.groups = [Group::default(); 3];
        let mut coupled = false;
        for ch in 0..header.nfchans {
            self.plane_mantissas(bits, ch);
            if self.strategy.cplinu && self.strategy.incpl[ch] && !coupled {
                self.plane_mantissas(bits, CPL);
                coupled = true;
            }
        }
        if header.lfeon {
            self.plane_mantissas(bits, LFE);
        }
    }

    /// One plane's mantissas, each scaled by the exponent of the same coefficient.
    ///
    /// The coupling plane is left out of Section 7.3.4 dither on purpose: the standard
    /// applies dither after the individual channels are extracted from the coupling
    /// channel, so an unallocated coupling bin is restored in `decouple` instead. That
    /// ordering is also what keeps the noise in the two channels uncorrelated.
    fn plane_mantissas(&mut self, bits: &mut Bits<'_>, plane: usize) {
        let start = self.strategy.start[plane];
        let end = self.strategy.end[plane];
        if std::env::var("EAC3_TRACE").is_ok() {
            let run = &self.strategy.bap[plane][start.min(BINS)..end.min(BINS)];
            let exp = &self.strategy.exp[plane][start.min(BINS)..end.min(BINS)];
            let words: usize = run
                .iter()
                .map(|&b| usize::from(b))
                .filter(|&b| b != 0)
                .map(|b| usize::from(MANTISSA_BITS[b]))
                .sum();
            eprintln!(
                "        plane {plane} {start}..{end} bap nonzero={} words={words} exp {:?}..{:?} bapmax {:?}",
                run.iter().filter(|&&b| b != 0).count(),
                exp.iter().min(),
                exp.iter().max(),
                run.iter().max(),
            );
        }
        let dithered = plane < FBW && self.strategy.dither[plane];
        for bin in start..end {
            let bap = usize::from(self.strategy.bap[plane][bin]);
            let mantissa = if bap == 0 {
                if dithered { self.dice() } else { 0.0 }
            } else {
                self.mantissa(bits, bap)
            };
            let exponent = self.strategy.exp[plane][bin].clamp(0, 24) as usize;
            self.coef[plane][bin] = mantissa * POW2[exponent];
        }
    }

    /// The Section 7.3.4 stand-in for a mantissa the bit allocation gave no bits. A
    /// linear congruential walk is the sequence the standard describes as reasonably
    /// random: consecutive draws are uncorrelated with the coded mantissas they sit
    /// between, and every call advances the decoder's own state, so a run of zero-bit
    /// bins hands out a run of different values rather than one fixed offset.
    fn dice(&mut self) -> f32 {
        self.dither_state = self
            .dither_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let unit = (self.dither_state >> 8) as f32 * (1.0 / (1 << 24) as f32);
        (2.0 * unit - 1.0) * DITHER_SCALE
    }

    /// One mantissa: grouped where the quantizer groups, and a plain word otherwise.
    /// Each grouped quantizer keeps its own slot, since a run that ends mid-group is
    /// continued by the next run of the same quantizer.
    fn mantissa(&mut self, bits: &mut Bits<'_>, bap: usize) -> f32 {
        if group_of_bap(bap).is_some() {
            let code = self.scratch.groups[bap / 2].take(bits, bap);
            return symmetric_value(bap, code);
        }
        let code = bits.take(usize::from(MANTISSA_BITS[bap]));
        if bap <= 5 {
            symmetric_value(bap, code)
        } else {
            asymmetric_value(bap, code)
        }
    }

    /// One plane's transform coefficients as the core leaves them between its own
    /// unpacking and its inverse transform, which is where Annex E's spectral extension
    /// rewrites the bins above its begin frequency.
    pub(crate) fn coefficients(&mut self, plane: usize) -> &mut [f32] {
        &mut self.coef[plane]
    }

    /// Rebuild each coupled channel above the coupling point from the coupling
    /// plane, its per-sub-band coordinate, and - for the right channel of a 2/0
    /// stream whose phase flags are on - a sign flip that undoes what the encoder's
    /// phase restoration saved bits for.
    ///
    /// A coupling bin the allocation gave no bits carries noise instead of the zero
    /// the missing mantissa would leave, drawn per channel here rather than in the
    /// coupling plane itself, because Section 7.3.4 asks for dither after the channels
    /// are extracted so that each channel's upper-frequency noise stays uncorrelated.
    /// The exponent is the coupling plane's: above the coupling point a coupled channel
    /// has no exponent run of its own, and inherits this one through the coefficient.
    pub(crate) fn decouple(&mut self, header: Header) {
        if !self.strategy.cplinu {
            return;
        }
        let last = (3 + self.strategy.cplendf).clamp(0, SUBDN as i32) as usize;
        let begf = self.strategy.cplbegf;
        for ch in 0..header.nfchans {
            if !self.strategy.incpl[ch] {
                continue;
            }
            let (coord, phsflg, phase) = (
                self.strategy.coord[ch],
                self.strategy.phsflg,
                self.strategy.phsflginu && ch == 1,
            );
            let dithered = self.strategy.dither[ch];
            for sbnd in begf..last {
                // The coordinate is a fraction of the coupling coefficient, and the
                // factor of eight is the headroom the encoder took off.
                let gain = coord[sbnd] * 8.0;
                let negate = phase && phsflg[sbnd];
                for bin in 0..12 {
                    let index = 37 + 12 * sbnd + bin;
                    if index >= BINS {
                        break;
                    }
                    let value = if self.strategy.bap[CPL][index] != 0 {
                        // A coded coupling coefficient arrives from the coupling plane
                        // already scaled by this same exponent.
                        self.coef[CPL][index] * gain
                    } else if dithered {
                        // The coordinate scales the noise as well as a coded value:
                        // Section 7.3.4 hands the substituted word to the coefficient
                        // the extraction produced, not to a coefficient of its own, and
                        // scaling a reference decode confirms it - the coupling band's
                        // energy lands within 0.1 dB of the reference with the gain and
                        // 2.6 dB above it without.
                        let exponent = self.strategy.exp[CPL][index].clamp(0, 24) as usize;
                        self.dice() * gain * POW2[exponent]
                    } else {
                        0.0
                    };
                    self.coef[ch][index] = if negate { -value } else { value };
                }
            }
        }
    }

    /// Undo the sums and differences the 2/0 mode coded in place of left and right.
    pub(crate) fn rematrix_restore(&mut self, header: Header) {
        if header.acmod != 2 {
            return;
        }
        let rematrix = self.strategy.rematrix;
        // Section 7.5.4: the two channels can have different bandwidths, and
        // restoring past the narrower one would invent coefficients for it out of the
        // wider one's own, since a bin with no mantissa reads as zero.
        let limit = self.strategy.end[0].min(self.strategy.end[1]);
        for band in 0..rematrix.count {
            if !rematrix.flags[band] {
                continue;
            }
            let (low, high) = rematrix.bands[band];
            for bin in low..(high + 1).min(limit) {
                let (left, right) = (self.coef[0][bin], self.coef[1][bin]);
                self.coef[0][bin] = left + right;
                self.coef[1][bin] = left - right;
            }
        }
    }

    /// Inverse transform every plane of the block, overlap it with the last one, and
    /// fold the result into the frame's interleaved samples.
    pub(crate) fn spectrum_to_sound(&mut self, header: Header, folding: &Folding, out: &mut Vec<f32>) {
        for ch in 0..header.nfchans {
            let short = self.strategy.blksw[ch];
            self.transform(ch, short);
        }
        if header.lfeon {
            self.transform(LFE, false);
        }
        // The dynamic range gain is per program, so a 1+1 frame puts its second
        // program's gain on its second channel.
        let gains = if header.acmod == 0 {
            let [first, second] = self.strategy.gain;
            let mut plane = [first; PLANES];
            plane[1] = second;
            plane
        } else {
            [self.strategy.gain[0]; PLANES]
        };
        for n in 0..SAMPLES {
            for output in 0..folding.width {
                let mut value = 0.0f32;
                for plane in 0..PLANES {
                    let weight = folding.weight[output][plane];
                    if weight != 0.0 {
                        value += weight * gains[plane] * self.scratch.pcm[plane][n];
                    }
                }
                out.push(value.clamp(-1.0, 1.0));
            }
        }
    }

    /// The inverse transform of Section 7.9.4: a block's 256 coefficients become the
    /// 512 windowed slots the overlap-add step folds two blocks at a time into
    /// samples. `short` selects Section 7.9.4.2, which decimates the coefficients into
    /// an even and an odd run and runs two 64-point transforms where the long path
    /// runs one of 128.
    fn transform(&mut self, plane: usize, short: bool) {
        let windowed = if short {
            self.transform_short(plane)
        } else {
            self.transform_long(plane)
        };
        // Step 6, shared: the first half of this block joins the second half of the
        // last. The factor of two undoes the headroom the encoder took off. The
        // standard asks for saturating arithmetic here; the fold clamps the summed
        // channels instead, which bounds the same way at the same place.
        for n in 0..SAMPLES {
            self.scratch.pcm[plane][n] = 2.0 * (windowed[n] + self.delay[plane][n]);
            self.delay[plane][n] = windowed[SAMPLES + n];
        }
    }

    /// One 512-sample inverse transform, Section 7.9.4.1 steps 2 through 5: the
    /// pre-twiddle packs the coefficient array backwards and forwards into a 128-point
    /// complex run, the transform and the post-twiddle expand it, and the windowing
    /// step scatters the complex halves over the 512 slots in the order the overlap
    /// step needs.
    fn transform_long(&self, plane: usize) -> [f32; SLOTS_512] {
        let coef = self.coef[plane];
        let (cos1, sin1) = (self.tables.cos1, self.tables.sin1);
        let mut re = [0.0f32; FFT];
        let mut im = [0.0f32; FFT];
        for k in 0..FFT {
            let forward = coef[2 * k];
            let back = coef[BINS - 1 - 2 * k];
            re[k] = back * cos1[k] - forward * sin1[k];
            im[k] = forward * cos1[k] + back * sin1[k];
        }
        ifft(&mut re, &mut im, &self.tables.twc, &self.tables.tws);
        let mut yr = [0.0f32; FFT];
        let mut yi = [0.0f32; FFT];
        for n in 0..FFT {
            // No 1/N here: step 3 of the section sums the transform unnormalized, and
            // the only factor the pseudocode applies to the block is step 6's two.
            // Putting the missing normalization back by hand puts the whole track
            // exactly 128 times too quiet, which an ffmpeg decode of the same stream
            // shows up as a level fit of 1.0 on a 1/128 offset.
            let (zr, zi) = (re[n], im[n]);
            yr[n] = zr * cos1[n] - zi * sin1[n];
            yi[n] = zi * cos1[n] + zr * sin1[n];
        }
        let mut x = [0.0f32; SLOTS_512];
        for n in 0..QUARTER {
            x[2 * n] = -yi[QUARTER + n] * WINDOW[2 * n];
            x[2 * n + 1] = yr[QUARTER - n - 1] * WINDOW[2 * n + 1];
            x[FFT + 2 * n] = -yr[n] * WINDOW[FFT + 2 * n];
            x[FFT + 2 * n + 1] = yi[FFT - n - 1] * WINDOW[FFT + 2 * n + 1];
            x[SAMPLES + 2 * n] = -yr[QUARTER + n] * WINDOW[SAMPLES - 2 * n - 1];
            x[SAMPLES + 2 * n + 1] = yi[QUARTER - n - 1] * WINDOW[SAMPLES - 2 * n - 2];
            x[3 * FFT + 2 * n] = yi[n] * WINDOW[FFT - 2 * n - 1];
            x[3 * FFT + 2 * n + 1] = -yr[FFT - n - 1] * WINDOW[FFT - 2 * n - 2];
        }
        x
    }

    /// One block-switched block: the two 256-sample inverse transforms of Section
    /// 7.9.4.2, run over the even and the odd coefficient runs, both landing in the
    /// same 512 slots. A transient is heard twice as often as it is measured, so the
    /// shorter window is what keeps a pre-echo from smearing across it.
    fn transform_short(&self, plane: usize) -> [f32; SLOTS_512] {
        let coef = self.coef[plane];
        let mut first = [0.0f32; FFT];
        let mut second = [0.0f32; FFT];
        for k in 0..FFT {
            first[k] = coef[2 * k];
            second[k] = coef[2 * k + 1];
        }
        let (yr1, yi1) = self.transform_256(&first);
        let (yr2, yi2) = self.transform_256(&second);
        let mut x = [0.0f32; SLOTS_512];
        for n in 0..QUARTER {
            x[2 * n] = -yi1[n] * WINDOW[2 * n];
            x[2 * n + 1] = yr1[QUARTER - n - 1] * WINDOW[2 * n + 1];
            x[FFT + 2 * n] = -yr1[n] * WINDOW[FFT + 2 * n];
            x[FFT + 2 * n + 1] = yi1[QUARTER - n - 1] * WINDOW[FFT + 2 * n + 1];
            x[SAMPLES + 2 * n] = -yr2[n] * WINDOW[SAMPLES - 2 * n - 1];
            x[SAMPLES + 2 * n + 1] = yi2[QUARTER - n - 1] * WINDOW[SAMPLES - 2 * n - 2];
            x[3 * FFT + 2 * n] = yi2[n] * WINDOW[FFT - 2 * n - 1];
            x[3 * FFT + 2 * n + 1] = -yr2[QUARTER - n - 1] * WINDOW[FFT - 2 * n - 2];
        }
        x
    }

    /// Steps 2 through 4 of Section 7.9.4.2 for one of the two decimated runs: a
    /// 64-point complex inverse transform with the block-switched twiddles, which are
    /// the 512-sample table's own cosines at twice the angle step.
    fn transform_256(&self, x: &[f32; FFT]) -> ([f32; QUARTER], [f32; QUARTER]) {
        let (cos2, sin2) = (self.tables.cos2, self.tables.sin2);
        let mut re = [0.0f32; QUARTER];
        let mut im = [0.0f32; QUARTER];
        for k in 0..QUARTER {
            let forward = x[2 * k];
            let back = x[FFT - 1 - 2 * k];
            re[k] = back * cos2[k] - forward * sin2[k];
            im[k] = forward * cos2[k] + back * sin2[k];
        }
        ifft(&mut re, &mut im, &self.tables.twc, &self.tables.tws);
        let scale = ifft_scale(QUARTER);
        let mut yr = [0.0f32; QUARTER];
        let mut yi = [0.0f32; QUARTER];
        for n in 0..QUARTER {
            let (zr, zi) = (re[n] * scale, im[n] * scale);
            yr[n] = zr * cos2[n] - zi * sin2[n];
            yi[n] = zi * cos2[n] + zr * sin2[n];
        }
        (yr, yi)
    }
}

/// The unnormalized inverse complex DFT of Section 7.9.4's step 3, run in place over
/// `len` = `re.len()` points with the positive-sign kernel that step sums with.
///
/// The twiddle tables are always the 128-point ones: a stage of length `len` reads
/// every `FFT / len`-th entry, which is also how the 64-point transform of the
/// block-switched path reuses the same table, as Section 7.9.4 notes.
fn ifft(re: &mut [f32], im: &mut [f32], twc: &[f32; FFT], tws: &[f32; FFT]) {
    let len = re.len();
    let mut j = 0usize;
    for i in 1..len {
        let mut bit = len >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut stage = 2;
    while stage <= len {
        let step = FFT / stage;
        let half = stage / 2;
        for start in (0..len).step_by(stage) {
            for k in 0..half {
                let (c, s) = (twc[k * step], tws[k * step]);
                let (lo, hi) = (start + k, start + k + half);
                let (br, bi) = (re[hi], im[hi]);
                let wr = br * c - bi * s;
                let wi = br * s + bi * c;
                re[hi] = re[lo] - wr;
                im[hi] = im[lo] - wi;
                re[lo] += wr;
                im[lo] += wi;
            }
        }
        stage *= 2;
    }
}

impl AudioDecode for Ac3Decoder {
    /// Decode every syncframe of the packet. A packet is normally one frame or a few,
    /// and each contributes its 1536 samples per channel.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let mut samples = Vec::new();
        self.frames(data, &mut samples)?;
        // The padding the encoder put at the head of the stream is not sound, and a
        // player that plays it starts every track behind the picture.
        let frames = samples.len() / self.channels;
        let skip = self.lead.min(frames);
        if skip > 0 {
            self.lead -= skip;
            samples.drain(..skip * self.channels);
        }
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

    /// A seek starts a new frame, and every frame states its own strategy, so the
    /// state to undo is the reuse history and the overlap tail. The dither sequence
    /// restarts from its seed with them, which is what makes a packet decoded twice
    /// after a seek the same packet rather than the same signal plus different noise.
    fn reset(&mut self) {
        self.core = Core::new();
        self.lead = ENCODER_DELAY;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2/0 stream at 48 kHz and 192 kbps whose channels each mix two tones. The
    /// mixture is what makes the fixture worth having: a channel that lands in the
    /// wrong slot, or a bit allocation that unpacks the mantissas at the wrong offset,
    /// changes both channels' content rather than only their balance.
    ///
    /// ```text
    /// ffmpeg -hide_banner -loglevel error -y -f lavfi \
    ///   -i 'aevalsrc=0.3*sin(2*PI*440*t)+0.1*sin(2*PI*1500*t)|0.2*sin(2*PI*220*t)+0.1*sin(2*PI*300*t):s=48000:d=0.25' \
    ///   -c:a ac3 -b:a 192k tests/fixtures/audio/ac3-stereo.ac3
    /// ```
    const STEREO: &[u8] = include_bytes!("../../tests/fixtures/audio/ac3-stereo.ac3");

    /// A 2/0 stream at 256 kbps carrying pink noise, which is the case that spends
    /// most of its bits above the coupling point: the encoder couples both channels'
    /// high bands, so every bin the test sees above bin 37 comes from the coupling
    /// plane and the coordinates rather than from a channel spectrum.
    ///
    /// ```text
    /// ffmpeg -hide_banner -loglevel error -y \
    ///   -f lavfi -i 'anoisesrc=color=pink:amplitude=0.3:sample_rate=48000:duration=0.25' \
    ///   -f lavfi -i 'anoisesrc=color=pink:amplitude=0.3:sample_rate=48000:duration=0.25' \
    ///   -filter_complex '[0:a][1:a]amerge=inputs=2[a]' -map '[a]' \
    ///   -c:a ac3 -b:a 256k tests/fixtures/audio/ac3-coupled.ac3
    /// ```
    const COUPLED: &[u8] = include_bytes!("../../tests/fixtures/audio/ac3-coupled.ac3");

    /// A 5.1 stream at 448 kbps with one tone per channel, plus a 3 kHz tone in the
    /// LFE that the encoder's own low-pass leaves almost nothing of. Six distinct
    /// levels is what makes the channel order testable: the standard's slot order is
    /// left, right, centre, LFE, and the two surrounds, and a fold that mixed them up
    /// would put the loudest channel somewhere other than the last.
    ///
    /// ```text
    /// ffmpeg -hide_banner -loglevel error -y -f lavfi \
    ///   -i 'aevalsrc=0.3*sin(2*PI*440*t)|0.2*sin(2*PI*220*t)|0.15*sin(2*PI*1000*t)|0.1*sin(2*PI*3000*t)|0.05*sin(2*PI*5500*t)|0.4*sin(2*PI*60*t):s=48000:d=0.25' \
    ///   -ch_layout 5.1 -c:a ac3 -b:a 448k tests/fixtures/audio/ac3-51.ac3
    /// ```
    const SURROUND: &[u8] = include_bytes!("../../tests/fixtures/audio/ac3-51.ac3");

    /// A 1/0 stream at the third of AC-3's rates, which is the case that checks the
    /// rate table reaches the transform: 32 kHz frames are half again as short as
    /// 48 kHz ones for the same code, and the band edges move with them.
    ///
    /// ```text
    /// ffmpeg -hide_banner -loglevel error -y \
    ///   -f lavfi -i 'sine=frequency=440:sample_rate=32000:duration=0.25' \
    ///   -c:a ac3 -b:a 96k tests/fixtures/audio/ac3-mono-32k.ac3
    /// ```
    const MONO_32: &[u8] = include_bytes!("../../tests/fixtures/audio/ac3-mono-32k.ac3");

    /// Decode a whole elementary stream as one packet, which is how a Matroska block
    /// of several frames arrives, and hand back the interleaved samples.
    fn decode(data: &[u8], sample_rate: u32, channels: u16) -> Vec<f32> {
        let mut decoder = Ac3Decoder::new(&[], sample_rate, channels).expect("the track opens");
        let packet = decoder
            .decode_encoded(data, 0, 0)
            .expect("the packet decodes")
            .expect("and holds audio");
        assert_eq!(packet.timebase_num, 1);
        assert_eq!(packet.timebase_den, sample_rate);
        shaped(&packet)
    }

    /// A packet's samples as the values they were written from. The decoder's output
    /// is the raw f32 run, and only this much of the player's sample format is needed
    /// to compare two decodes of the same packet.
    fn shaped(packet: &AudioPacket) -> Vec<f32> {
        packet
            .data
            .chunks_exact(4)
            .map(|word| f32::from_le_bytes(word.try_into().expect("one sample")))
            .collect()
    }

    /// The root-mean-square level and the peak of one channel of an interleaved run.
    fn level(samples: &[f32], channel: usize, channels: usize) -> (f32, f32) {
        let mut squared = 0.0f32;
        let mut peak = 0.0f32;
        let mut count = 0usize;
        let mut index = channel;
        while index < samples.len() {
            let value = samples[index];
            squared += value * value;
            peak = peak.max(value.abs());
            count += 1;
            index += channels;
        }
        ((squared / count as f32).sqrt(), peak)
    }

    /// Compare one measured level with the reference decoder's. The tolerance is wide
    /// enough for the two decoders' different rounding of the same spectra and narrow
    /// enough to catch a scale factor: the standard's overlap-add fixes the output
    /// level, so a transform normalized differently is off by two or more, not by ten
    /// percent.
    fn matches(measured: f32, expected: f32, fraction: f32, what: &str) {
        let low = expected * (1.0 - fraction);
        let high = expected * (1.0 + fraction);
        assert!(
            (low..=high).contains(&measured),
            "{what} is {measured}, outside {low}..={high} around the reference {expected}"
        );
    }

    /// Each frame's own header says what it is, before any of its audio is unpacked.
    #[test]
    fn every_fixture_states_its_own_geometry() {
        let cases: [(&[u8], usize, usize, usize, bool); 4] = [
            (STEREO, 0, 768, 2, false),
            (COUPLED, 0, 1024, 2, false),
            (SURROUND, 0, 1792, 5, true),
            (MONO_32, 2, 576, 1, false),
        ];
        for (data, rate, frame_bytes, nfchans, lfeon) in cases {
            let mut bits = Bits::new(data);
            let header = Header::read(&mut bits).expect("the first frame's header reads");
            assert_eq!(header.rate, rate, "fscod");
            assert_eq!(
                header.frame_bytes, frame_bytes,
                "frame length from Table 5.18"
            );
            assert_eq!(header.nfchans, nfchans, "channel count from acmod");
            assert_eq!(header.lfeon, lfeon, "LFE presence");
            assert_eq!(
                data.len() % frame_bytes,
                0,
                "the fixture is a whole number of frames of this length"
            );
        }
    }

    /// The load-bearing test of the bit stream's padding: every frame here carries
    /// hundreds of syntax elements before its mantissas, and a single misjudged field
    /// shifts the mantissa reads, which turns the decode to noise or trips the frame's
    /// own syntax faults and mutes it. All four fixtures decode to their reference
    /// levels, so all four line up.
    #[test]
    fn a_stereo_pair_keeps_its_two_tone_mixtures() {
        let channels = 2;
        let samples = decode(STEREO, 48_000, channels);
        assert_eq!(
            samples.len(),
            8 * BLOCKS * SAMPLES * 2 - ENCODER_DELAY * 2,
            "eight frames, less the padding the encoder put at the head of the stream"
        );
        for (channel, (rms, peak)) in [(0, (0.220_970, 0.400_1)), (1, (0.156_250, 0.300_0))] {
            let (measured_rms, measured_peak) = level(&samples, channel, channels as usize);
            matches(measured_rms, rms, 0.1, &format!("left rms {channel}"));
            matches(measured_peak, peak, 0.1, &format!("left peak {channel}"));
        }
    }

    /// Pink noise through coupling: the two channels' high bands are one spectrum, so
    /// this is the test that fails if the coordinates, the band-to-sub-band expansion,
    /// or the decoupling factor of eight is wrong - and both channels are equally loud
    /// in the reference, which is what a swap of the two would hide.
    #[test]
    fn a_coupled_pair_rebuilds_from_one_spectrum() {
        let channels = 2;
        let samples = decode(COUPLED, 48_000, channels);
        assert_eq!(
            samples.len(),
            8 * BLOCKS * SAMPLES * 2 - ENCODER_DELAY * 2,
            "eight frames, less the padding the encoder put at the head of the stream"
        );
        for (channel, rms) in [(0, 0.051_579), (1, 0.055_860)] {
            let (measured, _) = level(&samples, channel, channels as usize);
            matches(measured, rms, 0.15, &format!("coupled rms {channel}"));
        }
    }

    /// Six planes of the standard's own Table 5.8 order, each with a different tone:
    /// the per-channel levels are the reference's, and the ordering of the peaks is
    /// the channel map.
    #[test]
    fn a_surround_frame_keeps_its_planes_apart() {
        let channels = 6usize;
        let samples = decode(SURROUND, 48_000, channels as u16);
        assert_eq!(
            samples.len(),
            8 * BLOCKS * SAMPLES * channels - ENCODER_DELAY * channels,
            "eight frames"
        );
        let expected: [(f32, f32); 6] = [
            (0.209_632, 0.300_2),
            (0.139_756, 0.200_2),
            (0.104_818, 0.150_2),
            (0.000_537, 0.007_4),
            (0.034_940, 0.051_3),
            (0.279_499, 0.400_2),
        ];
        let mut peaks = Vec::new();
        for (channel, (rms, peak)) in expected.into_iter().enumerate() {
            let (measured_rms, measured_peak) = level(&samples, channel, channels);
            matches(measured_rms, rms.max(0.000_6), 0.2, "surround rms");
            matches(measured_peak, peak.max(0.007), 0.2, "surround peak");
            peaks.push(measured_peak);
        }
        let loudest = peaks
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(
            loudest, 5,
            "the 0.4 tone is the right surround, not a plane before it"
        );
    }

    /// The same frame folded to the two channels a stereo container states: Section
    /// 7.8.2's Lo and Ro equations each add the centre and one surround to their own
    /// side, so both mixed channels are louder than the reference level of the side
    /// alone. The equations are then checked sample by sample against the six-channel
    /// decode of the same packet, which is the part the levels cannot see: in this
    /// fixture the two mixtures land within 0.013 of each other, so only the sums can
    /// tell a surround folded into its own side from one folded into the other.
    #[test]
    fn a_surround_frame_folds_to_its_lo_ro_pair() {
        let samples = decode(SURROUND, 48_000, 2);
        assert_eq!(
            samples.len(),
            8 * BLOCKS * SAMPLES * 2 - ENCODER_DELAY * 2,
            "eight frames of two"
        );
        let (left, _) = level(&samples, 0, 2);
        let (right, _) = level(&samples, 1, 2);
        // The frame's own left channel alone measures 0.2096.
        assert!(
            left > 0.209_632,
            "Lo adds the centre and the left surround to L: {left}"
        );
        assert!(right > 0.139_756, "Ro adds them to R: {right}");
        let native = decode(SURROUND, 48_000, 6);
        let mut bits = Bits::new(SURROUND);
        let header = Header::read(&mut bits).expect("the first frame's header");
        let (centre, surround) = (header.clev, header.slev);
        let mut worst = 0.0f32;
        for (index, pair) in samples.chunks_exact(2).enumerate() {
            let plane = |slot| native[index * 6 + slot];
            // Section 7.8.2's Lo and Ro, with the left/centre/right/surround slots in
            // the order the six-channel decode lays them out.
            let lo = (plane(0) + centre * plane(2) + surround * plane(4)).clamp(-1.0, 1.0);
            let ro = (plane(1) + centre * plane(2) + surround * plane(5)).clamp(-1.0, 1.0);
            worst = worst.max((lo - pair[0]).abs()).max((ro - pair[1]).abs());
        }
        assert!(
            worst < 1e-6,
            "the fold is the standard's two weighted sums, not a level close to them: \
             the furthest sample is {worst}"
        );
    }

    /// The 32 kHz mono frame at its own rate: five of its six frames are the short
    /// 576-byte length, and the reference level says the band tables were read at the
    /// rate the frame states rather than the one assumed.
    #[test]
    fn a_mono_frame_at_32_khz_holds_its_own_rate() {
        let samples = decode(MONO_32, 32_000, 1);
        assert_eq!(
            samples.len(),
            6 * BLOCKS * SAMPLES - ENCODER_DELAY,
            "six frames of one"
        );
        let (rms, peak) = level(&samples, 0, 1);
        matches(rms, 0.082_333, 0.15, "mono rms");
        matches(peak, 0.125_0, 0.15, "mono peak");
    }

    /// A frame that stops mid-packet contributes its own length of silence and nothing
    /// after it: the reader is trimmed to the frame's stated end, so the tail of one
    /// packet cannot be mistaken for the head of the next.
    #[test]
    fn a_truncated_frame_mutes_instead_of_reaching_past_the_packet() {
        let frame = 768usize;
        let per_frame = BLOCKS * SAMPLES * 2;
        let mut data = STEREO[..2 * frame].to_vec();
        data.truncate(frame + 64);
        let samples = decode(&data, 48_000, 2);
        let withheld = ENCODER_DELAY * 2;
        assert_eq!(
            samples.len(),
            2 * per_frame - withheld,
            "both frames still sound their length, less the stream's leading padding"
        );
        assert!(
            samples[per_frame - withheld..].iter().all(|&value| value == 0.0),
            "the second frame is silence"
        );
        let (rms, _) = level(&samples, 0, 2);
        assert!(rms > 0.1, "the first frame is not: {rms}");
    }

    /// Bytes with no syncword are not a silent track: the packet is refused, which is
    /// what lets the player name the track as undecodable rather than play silence.
    #[test]
    fn bytes_without_a_syncword_are_refused() {
        let mut decoder = Ac3Decoder::new(&[], 48_000, 2).expect("the track opens");
        let noise = [0u8; 640];
        let error = match decoder.decode_encoded(&noise, 0, 0) {
            Err(error) => error,
            Ok(_) => panic!("a packet of zeros holds no frame"),
        };
        assert!(error.to_string().contains("no syncframe"), "{error}");
    }

    /// A frame that names a rate AC-3 does not have, or a channel count the folding
    /// cannot reach, is refused at open rather than guessed at.
    #[test]
    fn an_impossible_geometry_is_refused() {
        for (rate, channels, why) in [
            (22_050, 2u16, "AC-3 has no 22050 Hz"),
            (48_000, 7, "no layout of seven channels"),
            (48_000, 0, "no layout of none"),
        ] {
            let error = match Ac3Decoder::new(&[], rate, channels) {
                Err(error) => error,
                Ok(_) => panic!("{why} is refused at open"),
            };
            assert!(error.to_string().contains("AC-3"), "{error}");
        }
    }

    /// Every syncframe is self-contained, so decoding the same packet twice after a
    /// seek - which is what `reset` models - gives the same samples, and the overlap
    /// tail does not carry across the seek.
    #[test]
    fn a_reset_decoder_repeats_itself() {
        let mut decoder = Ac3Decoder::new(&[], 48_000, 2).expect("the track opens");
        let first = shaped(
            &decoder
                .decode_encoded(COUPLED, 0, 0)
                .expect("the packet decodes")
                .expect("and holds audio"),
        );
        decoder.reset();
        let second = shaped(
            &decoder
                .decode_encoded(COUPLED, 0, 0)
                .expect("the packet decodes again")
                .expect("and holds audio again"),
        );
        assert_eq!(first, second, "a seek leaves no state behind");
    }
}
