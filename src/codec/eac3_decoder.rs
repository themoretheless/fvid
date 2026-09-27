//! Enhanced AC-3: the coding Annex E of ATSC A/52:2012 attaches to AC-3, which is
//! what Dolby Digital Plus delivers. A packet is one or more syncframes, each of
//! which carries up to six audio blocks of 256 samples per channel, so a frame of
//! six spans the same 32 ms an AC-3 frame does in a third of the frames.
//!
//! What is read here is the header: the syncframe's own geometry, and the frame-level
//! state `audfrm` writes ahead of its audio blocks - which of them couple, the
//! exponent strategy codes, the frame's signal-to-noise offsets, and the flags that
//! say what each block will state for itself. That is all a walker needs to list a
//! stream's packets and all a container route needs to say what a track sounds like
//! before a single block is unpacked. Annex E packs every field most significant bit
//! first, unlike the AC-3 `bsi` this syntax lives inside, and it writes the frame's
//! length into the header rather than a code into a table, so `(frmsiz + 1) * 2` is
//! the byte count at which the next syncframe starts.
//!
//! The strategy codes are handed back as the frame writes them, and
//! [`Frame::strategies`] resolves them into what one block codes by: Table E2.10 turns
//! a frame-wide code into the six per-block ones, and Table 7.4 and Table 7.5 are the
//! per-block codes themselves. What a resolved strategy then asks of a block - its band
//! structure and the bit allocation that follows - is a block decoder's work, and a
//! header reader has no use for that answer and every use for the bits, since they sit
//! between the frame flags and the audio.
//!
//! The metadata behind the geometry is stepped over field by field because none of
//! it changes how the audio unpacks - with one exception worth naming where it is,
//! since a decoder that folds a stream down will want it: the `mixmdate` group
//! carries this stream's own Lo/Ro and L/R/Ls/Rs coefficients, which is where an
//! E-AC-3 downmix finds its levels instead of the fixed ones AC-3 mixes by.
//!
//! Every refusal says what it refuses, because a file that needs syntax this module
//! does not have yet is a missing feature and not a broken file, and the caller
//! cannot tell them apart unless the decoder can:
//!
//! * a stream type other than independent - a dependent substream states nothing of
//!   its own and leans on the independent frame before it, and a converted one
//!   repeats AC-3's header, so both need a reader this one is not yet;
//! * a second substream of the same presentation, which is the other half of a pair
//!   the first point refuses;
//! * `bsid` other than 16, which is Annex E's own mark: below it the frame is AC-3,
//!   above it a revision not transcribed;
//! * a frame of one, two or three blocks, which is the shape that carries AC-3's
//!   conversion layer and with it the three-way block switching this decoder will
//!   not have to solve while six-block frames are what exists;
//! * the reduced rates `fscod` = 3 names, whose band edges and thresholds the AC-3
//!   tables this core reuses are keyed to the three full rates;
//! * `ahte`, the adaptive hybrid transform, whose coefficients are windowed and
//!   quantized by rules of their own that reach into every block of the frame;
//! * `blkswe`, which lets a block of the frame switch to AC-3's short transform;
//! * `spxattene`, the spectral extension attenuation data, which is the flag that
//!   high bands were reconstructed rather than coded.
//!
//! The constants and field layouts are transcribed from the public ATSC A/52:2012
//! text, Annex E, sections 2.2.1 to 2.3.2.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::codec::ac3_decoder::{
    Bits, Core, dynrng_gain, ENCODER_DELAY, FBW, Folding, Header, LFE, PLANES, SAMPLES, Strategy as BlockStrategy, SUBDN,
    CPL,
};
use crate::codec::bits::BitReader;
use crate::{Error, Result, invalid, unsupported};

/// The 16-bit mark a syncframe opens with, shared with AC-3 by Table E1.1.
const SYNCWORD: u32 = 0x0B77;
/// The `bsid` that says the syntax is Annex E's rather than AC-3's, section 2.1.
const EAC3_BSID: u32 = 16;
/// Rates the 2-bit `fscod` names, Table E2.2.
const SAMPLE_RATES: [u32; 3] = [48_000, 44_100, 32_000];
/// Full-bandwidth channels per `acmod`, the count Annex E borrows from AC-3.
const NFCHANS: [usize; 8] = [2, 1, 2, 3, 3, 4, 4, 5];
/// Audio blocks per syncframe for `numblkscod`, Table E2.4.
const BLOCK_COUNTS: [usize; 4] = [1, 2, 3, 6];
/// The blocks a frame this reader accepts carries, which is `BLOCK_COUNTS[3]`.
const BLOCKS: usize = 6;
/// The most full-bandwidth channels an `acmod` names: 5.1 without its LFE.
const CHANS: usize = 5;

/// What a syncframe says about itself before any audio is read: its geometry from
/// `syncinfo` and `bsi`, and the frame-level state `audfrm` writes ahead of blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The rate the frame's audio plays back at, from `fscod`.
    pub sample_rate: u32,
    /// `fscod` itself. The rate in hertz is what a caller plays at; the band edges and
    /// the masking thresholds the audio is decoded with are keyed to the row of
    /// Table E2.2 the frame names, and only the code carries that.
    rate_code: u8,
    /// `acmod`: the audio coding mode, which a block reader needs for its own syntax -
    /// a 1+1 frame states a second program's dynamic range, and a 2/0 frame states
    /// rematrixing and may state phase restoration.
    pub layout: u8,
    /// The frame's own channel count in native order, which is what a decoder handed
    /// this frame produces when the container states no other layout.
    pub channels: u16,
    /// `lfeon` itself, since the channel count alone cannot say which channel of a
    /// frame is the low frequency one: three channels are either 3/0 or 2/0 with an
    /// LFE, and the two unpack differently.
    pub low_frequency: bool,
    /// Bytes the frame occupies, from `frmsiz`, so the next one starts here.
    pub frame_bytes: usize,
    /// Audio blocks the frame carries, each of 256 samples per channel.
    pub blocks: usize,
    /// The bit of this syncframe at which the first block begins, which is where the
    /// walk of `syncinfo`, `bsi` and `audfrm` stops.
    pub audio_bit: usize,
    /// `cplinu[blk]`: whether each block folds its high bands into a coupling
    /// channel. A block that inherits the flag from the one before it is recorded
    /// with the flag it inherited, so this is the block's own state.
    pub coupling: [bool; BLOCKS],
    /// `cplstre[blk]`: whether each block states its coupling strategy in the block
    /// rather than taking the one the frame last stated. Block 0 always states its own,
    /// which is why the frame reads its flag outright, and a frame of one or two
    /// channels states nothing at all.
    pub coupling_stated: [bool; BLOCKS],
    /// The Lo/Ro downmix coefficients this stream's mixing metadata carries, which is
    /// where an E-AC-3 fold finds its levels instead of the fixed ones AC-3 mixes by.
    pub downmix: Option<Downmix>,
    /// The exponent strategy codes of the frame, in whichever of the two forms
    /// `expstre` writes them.
    pub exponents: Exponents,
    /// `lfeexpstr[blk]`, the exponent strategy flag of each block's low frequency
    /// channel; all false when the frame has no such channel.
    pub low_frequency_strategies: [bool; BLOCKS],
    /// `convexpstr[ch]`: the strategies the frame names for the AC-3 syncframes it
    /// converts to. Read because they occupy bits ahead of the audio, and used by no
    /// player that sounds out E-AC-3 as itself.
    pub converter_strategies: [u8; CHANS],
    /// `frmcsnroffst` and `frmfsnroffst`, which the frame states only when its blocks
    /// take their offsets from it.
    pub snr_offsets: Option<SnrOffsets>,
    /// What the frame tells its blocks they may state for themselves.
    pub blocks_state: BlocksState,
    /// `blkstrt[blk]`: the bit each block after the first begins at, in order, for a
    /// frame that states them. `None` when it does not, which is what an encoder
    /// writes when it is content for the blocks to divide what the header leaves.
    pub block_starts: Option<[usize; BLOCKS - 1]>,
}

/// The Lo/Ro downmix levels the frame's mixing metadata states, held as the 3-bit
/// codes Table D2.5 and Table D2.6 name them rather than as levels, so that a frame
/// reads back as the bits that were written to it. [`Eac3Decoder`] turns a code into
/// the level the fold mixes by, and supplies the `-3 dB` default for the level a
/// stream does not name: Annex E puts these coefficients inside `bsi`, where AC-3's
/// own 2-bit `cmixlev` and `surmixlev` sit beside `acmod`, and its tables are the
/// wider Lo/Ro ones - the Lt/Rt codes of the same group are for a stream that asks to
/// be folded to Lt/Rt, which is not what a stereo container states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Downmix {
    /// `lorocmixlev`, present only for a mode with a centre channel.
    pub centre: Option<u8>,
    /// `lorosurmixlev`, present only for a mode with surround channels.
    pub surround: Option<u8>,
}

/// Table D2.5, the Lo/Ro centre mix level per `lorocmixlev`.
const LORO_CENTRE: [f32; 8] = [1.414, 1.189, 1.000, 0.841, 0.707, 0.595, 0.500, 0.000];

/// Table D2.6, the Lo/Ro surround mix level per `lorosurmixlev`. The three lowest
/// codes are reserved, and the table's own instruction is to mix by `-1.5 dB` for
/// one of them.
const LORO_SURROUND: [f32; 8] = [0.841, 0.841, 0.841, 0.841, 0.707, 0.595, 0.500, 0.000];

/// The exponent strategy codes a frame states, in one of the two forms `expstre`
/// selects. Both are codes, not band structures: [`Frame::strategies`] resolves them
/// into the [`Strategy`] one block codes by, through Table E2.10 for a frame-wide code
/// and Table 7.4 for a per-block one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exponents {
    /// `expstre`: each block states a 2-bit code for the coupling channel, when it
    /// couples, and one for every full-bandwidth channel.
    PerBlock {
        coupling: [u8; BLOCKS],
        channels: [[u8; CHANS]; BLOCKS],
    },
    /// One 5-bit code for every block of the frame. `coupling` is absent when no
    /// block of the frame couples, since then the frame states no code for it.
    PerFrame {
        coupling: Option<u8>,
        channels: [u8; CHANS],
    },
}

/// What one block codes one channel's exponents by, once a frame's own code is read
/// for that block. The three differential strategies differ only in how many
/// exponents share a coded group, which is what makes one cheaper than another: D15
/// carries three to a group, D25 six, D45 twelve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Reuse the exponents the block before this one decoded, which is what Table
    /// E2.10 names for most of the blocks of most of its rows. Never the strategy of
    /// block 0: the standard states that information is never shared across
    /// syncframes, so a frame's first block always carries a strategy of its own.
    Reuse,
    /// Differentially coded exponents, three of them to a coded group.
    D15,
    /// Differentially coded exponents, six to a group.
    D25,
    /// Differentially coded exponents, twelve to a group.
    D45,
}

impl Strategy {
    /// Table 7.4: the 2-bit code of a frame that states strategies per block.
    fn per_block(code: u8) -> Self {
        match code {
            0 => Strategy::Reuse,
            1 => Strategy::D15,
            2 => Strategy::D25,
            _ => Strategy::D45,
        }
    }

    /// Table 7.5: the low frequency channel is offered reuse or D15, and nothing else.
    fn low_frequency(flag: bool) -> Self {
        if flag {
            Strategy::D15
        } else {
            Strategy::Reuse
        }
    }

    /// Table E2.10: the strategy one of the frame's 5-bit codes names for one block.
    fn per_frame(code: u8, block: usize) -> Self {
        FRAME_STRATEGIES[usize::from(code)][block]
    }

    /// The 2-bit `*expstr` code a block reads for this strategy, Table E2.10's own
    /// numbering: 0 is reuse, and 1, 2, 3 are the three differential lengths.
    fn code(self) -> usize {
        match self {
            Strategy::Reuse => 0,
            Strategy::D15 => 1,
            Strategy::D25 => 2,
            Strategy::D45 => 3,
        }
    }
}

/// Table E2.10, the frame-wide exponent strategy codes expanded into the six blocks of
/// a syncframe. The rows are the codes in order.
const FRAME_STRATEGIES: [[Strategy; BLOCKS]; 32] = [
    //  0
    [Strategy::D15, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse],
    //  1
    [Strategy::D15, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse, Strategy::D45],
    //  2
    [Strategy::D15, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse, Strategy::D25, Strategy::Reuse],
    //  3
    [Strategy::D15, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse, Strategy::D45, Strategy::D45],
    //  4
    [Strategy::D25, Strategy::Reuse, Strategy::Reuse, Strategy::D25, Strategy::Reuse, Strategy::Reuse],
    //  5
    [Strategy::D25, Strategy::Reuse, Strategy::Reuse, Strategy::D25, Strategy::Reuse, Strategy::D45],
    //  6
    [Strategy::D25, Strategy::Reuse, Strategy::Reuse, Strategy::D45, Strategy::D25, Strategy::Reuse],
    //  7
    [Strategy::D25, Strategy::Reuse, Strategy::Reuse, Strategy::D45, Strategy::D45, Strategy::D45],
    //  8
    [Strategy::D25, Strategy::Reuse, Strategy::D15, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse],
    //  9
    [Strategy::D25, Strategy::Reuse, Strategy::D25, Strategy::Reuse, Strategy::Reuse, Strategy::D45],
    // 10
    [Strategy::D25, Strategy::Reuse, Strategy::D25, Strategy::Reuse, Strategy::D25, Strategy::Reuse],
    // 11
    [Strategy::D25, Strategy::Reuse, Strategy::D25, Strategy::Reuse, Strategy::D45, Strategy::D45],
    // 12
    [Strategy::D25, Strategy::Reuse, Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::Reuse],
    // 13
    [Strategy::D25, Strategy::Reuse, Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::D45],
    // 14
    [Strategy::D25, Strategy::Reuse, Strategy::D45, Strategy::D45, Strategy::D25, Strategy::Reuse],
    // 15
    [Strategy::D25, Strategy::Reuse, Strategy::D45, Strategy::D45, Strategy::D45, Strategy::D45],
    // 16
    [Strategy::D45, Strategy::D15, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse],
    // 17
    [Strategy::D45, Strategy::D15, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse, Strategy::D45],
    // 18
    [Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::Reuse, Strategy::D25, Strategy::Reuse],
    // 19
    [Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::Reuse, Strategy::D45, Strategy::D45],
    // 20
    [Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::D25, Strategy::Reuse, Strategy::Reuse],
    // 21
    [Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::D25, Strategy::Reuse, Strategy::D45],
    // 22
    [Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::D45, Strategy::D25, Strategy::Reuse],
    // 23
    [Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::D45, Strategy::D45, Strategy::D45],
    // 24
    [Strategy::D45, Strategy::D45, Strategy::D15, Strategy::Reuse, Strategy::Reuse, Strategy::Reuse],
    // 25
    [Strategy::D45, Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::Reuse, Strategy::D45],
    // 26
    [Strategy::D45, Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::D25, Strategy::Reuse],
    // 27
    [Strategy::D45, Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::D45, Strategy::D45],
    // 28
    [Strategy::D45, Strategy::D45, Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::Reuse],
    // 29
    [Strategy::D45, Strategy::D45, Strategy::D45, Strategy::D25, Strategy::Reuse, Strategy::D45],
    // 30
    [Strategy::D45, Strategy::D45, Strategy::D45, Strategy::D45, Strategy::D25, Strategy::Reuse],
    // 31
    [Strategy::D45, Strategy::D45, Strategy::D45, Strategy::D45, Strategy::D45, Strategy::D45],
];

/// The strategies of one block, which is what a block decoder reads before it reads a
/// single exponent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockStrategies {
    /// The coupling channel's strategy, absent for a block that does not couple and so
    /// codes no coupling channel.
    pub coupling: Option<Strategy>,
    /// One per full-bandwidth channel, in the frame's own channel order. The slots past
    /// [`Frame::full_bandwidth`] are padding the frame never states.
    pub channels: [Strategy; CHANS],
    /// `lfeexpstr[blk]` of Table 7.5, absent when the frame has no low frequency
    /// channel at all.
    pub low_frequency: Option<Strategy>,
}

impl Frame {
    /// The channels of this frame that carry full-bandwidth audio: its own count
    /// without the low frequency one, which is `nfchans` of the `acmod` it states.
    pub fn full_bandwidth(&self) -> usize {
        usize::from(self.channels) - usize::from(self.low_frequency)
    }

    /// The level the centre channel joins a Lo/Ro fold at. A stream whose mixing
    /// metadata names none mixes at `-3 dB`, which is the default AC-3's own
    /// `cmixlev` falls back to and the level Table D2.5 gives for `-3 dB` alike.
    fn centre_level(&self) -> f32 {
        self.downmix
            .and_then(|mix| mix.centre)
            .map_or(0.707, |code| LORO_CENTRE[usize::from(code)])
    }

    /// The level the surround channels join a Lo/Ro fold at, with the same default.
    fn surround_level(&self) -> f32 {
        self.downmix
            .and_then(|mix| mix.surround)
            .map_or(0.707, |code| LORO_SURROUND[usize::from(code)])
    }

    /// What `block` codes its exponents by, resolved from whichever form `expstre`
    /// wrote. `None` for a block index past the ones this frame carries.
    pub fn strategies(&self, block: usize) -> Option<BlockStrategies> {
        if block >= self.blocks {
            return None;
        }
        let coupling = match self.exponents {
            Exponents::PerBlock { coupling, .. } if self.coupling[block] => {
                Some(Strategy::per_block(coupling[block]))
            }
            Exponents::PerFrame {
                coupling: Some(code), ..
            } if self.coupling[block] => Some(Strategy::per_frame(code, block)),
            _ => None,
        };
        let resolved: [Strategy; CHANS] = match self.exponents {
            Exponents::PerBlock { channels, .. } => {
                let codes = channels[block];
                std::array::from_fn(|index| Strategy::per_block(codes[index]))
            }
            Exponents::PerFrame { channels, .. } => {
                std::array::from_fn(|index| Strategy::per_frame(channels[index], block))
            }
        };
        let mut channels = [Strategy::Reuse; CHANS];
        let stated = self.full_bandwidth();
        channels[..stated].copy_from_slice(&resolved[..stated]);
        let low_frequency = self
            .low_frequency
            .then(|| Strategy::low_frequency(self.low_frequency_strategies[block]));
        Some(BlockStrategies {
            coupling,
            channels,
            low_frequency,
        })
    }
}

/// The pair of signal-to-noise offsets a frame states for its blocks, which a block
/// turns into the bit allocation it runs by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnrOffsets {
    /// `frmcsnroffst`, the coarse offset, 6 bits.
    pub coarse: u8,
    /// `frmfsnroffst`, the fast decay offset, 4 bits.
    pub fast: u8,
}

/// The `audfrm` flags that say what each of the frame's blocks writes for itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlocksState {
    /// `dithflage`: each block states its own dither flags. Otherwise no block says
    /// so, and every block dithers, which is what AC-3's own default is.
    pub dither_flags: bool,
    /// `bamode`: each block states its bit allocation parameters. Otherwise it uses
    /// the fixed set Table E1.4 names, and the curves of AC-3's Table 5.20 are not
    /// read at all.
    pub bit_allocation: bool,
    /// `frmfgaincode`: gain codes of some kind are in play for this frame.
    pub gain_codes: bool,
    /// `dbaflde`: each block states dynamic bit allocation parameters.
    pub dynamic_parameters: bool,
    /// `skipflde`: each block states the full skip fields.
    pub skip_fields: bool,
    /// `transproce`: at least one channel carries a transient pre-noise processing
    /// window, whose location and length this reader steps over.
    pub transient_processing: bool,
    /// `snroffststr`: 0 when the blocks take the offsets the frame states, and the
    /// other three codes name the ways a block states its own.
    pub snr_offsets: u8,
}

/// Read the header of the syncframe that starts at `bytes`.
///
/// A bare `.ec3` file states its geometry in every frame rather than once up front,
/// so the walker that lists a stream's packets needs the same numbers the decoder
/// will. `None` for the same refusals `header()` makes.
pub fn frame(bytes: &[u8]) -> Option<Frame> {
    header(&mut BitReader::new(bytes)).ok()
}

/// Read `syncinfo`, step over `bsi` and read `audfrm`, Tables E1.1 to E1.3, leaving
/// the frame's blocks ahead of the reader.
fn header(bits: &mut BitReader<'_>) -> Result<Frame> {
    if bits.read(16)? != SYNCWORD {
        return Err(invalid("no E-AC-3 syncword at the start of the syncframe"));
    }
    // The two fields that say which substream this is come first, and both of the
    // kinds that are not this decoder's are named before anything else is read, so
    // that a stream needing them fails here rather than half a block later.
    let strmtyp = bits.read(2)?;
    if strmtyp != 0 {
        return Err(unsupported(&format!(
            "E-AC-3 stream type {strmtyp}, which is not an independent substream"
        )));
    }
    if bits.read(3)? != 0 {
        return Err(unsupported("a second E-AC-3 substream of the same presentation"));
    }
    // `frmsiz` counts 16-bit words in the syncframe, itself included.
    let frame_bytes = bits.read(11)? as usize * 2 + 2;
    let fscod = bits.read(2)? as usize;
    if fscod == 3 {
        return Err(unsupported("an E-AC-3 frame at a reduced sample rate"));
    }
    let rate = SAMPLE_RATES[fscod];
    let numblkscod = bits.read(2)? as usize;
    if numblkscod != 3 {
        return Err(unsupported(&format!(
            "an E-AC-3 frame of {} blocks, which carries AC-3's conversion layer",
            BLOCK_COUNTS[numblkscod]
        )));
    }
    let acmod = bits.read(3)? as usize;
    let lfeon = bits.bit()?;
    let bsid = bits.read(5)?;
    if bsid != EAC3_BSID {
        return Err(unsupported(&format!(
            "an E-AC-3 stream marked with bsid {bsid}, which names {}",
            if bsid < EAC3_BSID { "AC-3's own syntax" } else { "a later revision" }
        )));
    }
    let downmix = bsi(bits, acmod, lfeon, BLOCKS)?;
    let audio = audfrm(bits, acmod, lfeon, frame_bytes)?;
    let position = bits.position();
    if position > frame_bytes * 8 {
        return Err(invalid("an E-AC-3 header longer than the frame that carries it"));
    }
    Ok(Frame {
        sample_rate: rate,
        rate_code: fscod as u8,
        layout: acmod as u8,
        channels: (NFCHANS[acmod] + usize::from(lfeon)) as u16,
        low_frequency: lfeon,
        frame_bytes,
        blocks: BLOCKS,
        audio_bit: position,
        coupling: audio.coupling,
        coupling_stated: audio.coupling_stated,
        downmix,
        exponents: audio.exponents,
        low_frequency_strategies: audio.low_frequency_strategies,
        converter_strategies: audio.converter_strategies,
        snr_offsets: audio.snr_offsets,
        blocks_state: audio.blocks_state,
        block_starts: audio.block_starts,
    })
}

/// The part of a `Frame` that `audfrm` states, kept apart so that [`header`] can
/// assemble the geometry it read before the walk with what the walk leaves behind.
struct Audio {
    coupling: [bool; BLOCKS],
    coupling_stated: [bool; BLOCKS],
    exponents: Exponents,
    low_frequency_strategies: [bool; BLOCKS],
    converter_strategies: [u8; CHANS],
    snr_offsets: Option<SnrOffsets>,
    blocks_state: BlocksState,
    block_starts: Option<[usize; BLOCKS - 1]>,
}

/// Read the audio frame of Table E1.3: the flags every block of the frame obeys, the
/// exponent strategy codes, and where the blocks start. `numblkscod` is known to be
/// six blocks, which is why `expstre` and `ahte` are fields here rather than defaults
/// the table would have to supply, and why no conversion-layer strategy data is read.
fn audfrm(bits: &mut BitReader<'_>, acmod: usize, lfeon: bool, frame_bytes: usize) -> Result<Audio> {
    let nfchans = NFCHANS[acmod];
    let expstre = bits.bit()?;
    if bits.bit()? {
        return Err(unsupported(
            "an E-AC-3 frame whose blocks carry the adaptive hybrid transform",
        ));
    }
    let snr_strategy = bits.read(2)? as u8;
    if snr_strategy == 3 {
        return Err(unsupported(
            "an E-AC-3 frame whose SNR offset strategy code is the reserved one",
        ));
    }
    let transient_processing = bits.bit()?;
    if bits.bit()? {
        return Err(unsupported(
            "an E-AC-3 frame that switches its blocks to the short transform",
        ));
    }
    let dither_flags = bits.bit()?;
    let bit_allocation = bits.bit()?;
    let gain_codes = bits.bit()?;
    let dynamic_parameters = bits.bit()?;
    let skip_fields = bits.bit()?;
    if bits.bit()? {
        return Err(unsupported(
            "an E-AC-3 frame that attenuates a reconstructed high band",
        ));
    }

    // Coupling: the first block states its own flag, and a later block that does not
    // state one keeps the flag of the block before it.
    let mut coupling = [false; BLOCKS];
    let mut coupling_stated = [false; BLOCKS];
    if acmod > 1 {
        // Block 0's flag is one the frame states, which is what Table E1.4 means by
        // `cplstre[0]`: the block will state its strategy, because the frame has just
        // stated whether it couples at all.
        coupling[0] = bits.bit()?;
        coupling_stated[0] = true;
        for blk in 1..BLOCKS {
            coupling_stated[blk] = bits.bit()?;
            coupling[blk] = if coupling_stated[blk] {
                bits.bit()?
            } else {
                coupling[blk - 1]
            };
        }
    }
    let coupled = coupling.iter().any(|&flag| flag);

    let exponents = if expstre {
        let mut strategy = [0u8; BLOCKS];
        let mut channels = [[0u8; CHANS]; BLOCKS];
        for blk in 0..BLOCKS {
            if coupling[blk] {
                strategy[blk] = bits.read(2)? as u8;
            }
            for channel in channels[blk][..nfchans].iter_mut() {
                *channel = bits.read(2)? as u8;
            }
        }
        Exponents::PerBlock {
            coupling: strategy,
            channels,
        }
    } else {
        let coupling = coupled.then(|| bits.read(5)).transpose()?;
        Exponents::PerFrame {
            coupling: coupling.map(|code| code as u8),
            channels: frame_strategies(bits, nfchans)?,
        }
    };

    let mut low_frequency_strategies = [false; BLOCKS];
    if lfeon {
        for flag in low_frequency_strategies.iter_mut() {
            *flag = bits.bit()?;
        }
    }
    // `strmtyp` is known independent and the frame is known to hold six blocks, so
    // the converter strategies that follow are always there, and always for the
    // full-bandwidth channels alone.
    let mut converter_strategies = [0u8; CHANS];
    for channel in converter_strategies[..nfchans].iter_mut() {
        *channel = bits.read(5)? as u8;
    }

    let snr_offsets = if snr_strategy == 0 {
        Some(SnrOffsets {
            coarse: bits.read(6)? as u8,
            fast: bits.read(4)? as u8,
        })
    } else {
        None
    };
    if transient_processing {
        for _ in 0..nfchans {
            if bits.bit()? {
                bits.skip(10)?; // transprocloc
                bits.skip(8)?; // transproclen
            }
        }
    }
    // Only a frame of a single block omits the flag below, and those are refused
    // above with the rest of the conversion layer.
    let mut block_starts = None;
    if bits.bit()? {
        // The field is exactly wide enough to name a bit of this frame, so an entry
        // reads as the bit that block starts at rather than as an offset.
        let width = 4 + ceil_log2(frame_bytes / 2);
        let mut starts = [0usize; BLOCKS - 1];
        for start in starts.iter_mut() {
            *start = bits.read(width as u8)? as usize;
        }
        validate_starts(&starts, bits.position(), frame_bytes * 8)?;
        block_starts = Some(starts);
    }
    Ok(Audio {
        coupling,
        coupling_stated,
        exponents,
        low_frequency_strategies,
        converter_strategies,
        snr_offsets,
        blocks_state: BlocksState {
            dither_flags,
            bit_allocation,
            gain_codes,
            dynamic_parameters,
            skip_fields,
            transient_processing,
            snr_offsets: snr_strategy,
        },
        block_starts,
    })
}

/// The 5-bit exponent strategy codes one full-bandwidth channel after another, which
/// is the frame-wide form `expstre` = 0 writes.
fn frame_strategies(bits: &mut BitReader<'_>, nfchans: usize) -> Result<[u8; CHANS]> {
    let mut channels = [0u8; CHANS];
    for channel in channels[..nfchans].iter_mut() {
        *channel = bits.read(5)? as u8;
    }
    Ok(channels)
}

/// Check the block starts a frame states against the frame that states them, which
/// is the only check they can be given: a start inside the header, a start that is
/// not ahead of the block before it, or a start past the end of the frame cannot
/// frame a block, and no encoder here writes them to tell us what it meant instead.
fn validate_starts(starts: &[usize; BLOCKS - 1], header_bits: usize, frame_bits: usize) -> Result<()> {
    let mut previous = header_bits;
    for &start in starts {
        if start <= previous {
            return Err(invalid("an E-AC-3 block that starts before the one before it"));
        }
        if start >= frame_bits {
            return Err(invalid("an E-AC-3 block starting past the end of its frame"));
        }
        previous = start;
    }
    Ok(())
}

/// `ceiling (log2 (n))` of Annex E's own wording, which for the block start field is
/// how many bits it takes to count the words of a frame.
fn ceil_log2(n: usize) -> usize {
    (0..).find(|shift| 1usize << shift >= n).unwrap_or(usize::BITS as usize)
}

/// Step over `bsi` past the fields `header` has already read: Table E1.2 from
/// `dialnorm` on. `strmtyp` is known independent and `numblkscod` known six blocks,
/// which is what retires the dependent-stream and conversion fields of the table.
/// The Lo/Ro coefficients the metadata carries come back with the walk.
fn bsi(bits: &mut BitReader<'_>, acmod: usize, lfeon: bool, blocks: usize) -> Result<Option<Downmix>> {
    bits.skip(5)?; // dialnorm
    if bits.bit()? {
        bits.skip(8)?; // compr
    }
    if acmod == 0 {
        // 1+1 mode gives some of the metadata a second value of its own.
        bits.skip(5)?; // dialnorm2
        if bits.bit()? {
            bits.skip(8)?; // compr2
        }
    }
    let downmix = if bits.bit()? {
        mixing(bits, acmod, lfeon, blocks)?
    } else {
        None
    };
    if bits.bit()? {
        information(bits, acmod)?;
    }
    if bits.bit()? {
        let length = bits.read(6)? as usize;
        bits.skip((length + 1) * 8)?; // addbsi
    }
    Ok(downmix)
}

/// Step over the mixing metadata of Table E1.2, and take the two Lo/Ro coefficients a
/// stream's own fold reads from it. Each sits behind its Lt/Rt twin, which is for a
/// stream encoded to surround-aware decoding rather than to a stereo container.
fn mixing(bits: &mut BitReader<'_>, acmod: usize, lfeon: bool, blocks: usize) -> Result<Option<Downmix>> {
    if acmod > 2 {
        bits.skip(2)?; // dmixmod
    }
    let centre = if acmod > 2 && acmod & 1 != 0 {
        bits.skip(3)?; // ltrtcmixlev
        Some(bits.read(3)? as u8) // lorocmixlev
    } else {
        None
    };
    let surround = if acmod & 4 != 0 {
        bits.skip(3)?; // ltrtsurmixlev
        Some(bits.read(3)? as u8) // lorosurmixlev
    } else {
        None
    };
    if lfeon && bits.bit()? {
        bits.skip(5)?; // lfemixlevcod
    }
    if bits.bit()? {
        bits.skip(6)?; // pgmscl
    }
    if acmod == 0 && bits.bit()? {
        bits.skip(6)?; // pgmscl2
    }
    if bits.bit()? {
        bits.skip(6)?; // extpgmscl
    }
    match bits.read(2)? {
        // mixdef: how the mixing definition of an external program is spelled.
        0 => {}
        1 => {
            bits.skip(1)?; // premixcmpsel
            bits.skip(1)?; // drcsrc
            bits.skip(3)?; // premixcmpscl
        }
        2 => bits.skip(12)?, // mixdata
        _ => external(bits)?,
    }
    if acmod < 2 {
        if bits.bit()? {
            bits.skip(8)?; // panmean
            bits.skip(6)?; // paninfo, reserved
        }
        if acmod == 0 && bits.bit()? {
            bits.skip(8)?; // panmean2
            bits.skip(6)?; // paninfo2, reserved
        }
    }
    if bits.bit()? {
        // A frame of one block states its mixing configuration once; six-block
        // frames, which are all this reader accepts, state it per block.
        for _ in 0..blocks {
            if bits.bit()? {
                bits.skip(5)?; // blkmixcfginfo
            }
        }
    }
    Ok(Some(Downmix { centre, surround }))
}

/// Step over the most flexible mixing definition: `mixdeflen` says how many bytes
/// the group it opens spans, and the scaling flags and `mixdata` together fill
/// exactly that, with `mixdatafill` rounding the end up to a byte boundary. Reading
/// it as a total is what keeps the walk honest whatever the flags inside hold.
fn external(bits: &mut BitReader<'_>) -> Result<()> {
    let start = bits.position();
    let group = (bits.read(5)? as usize + 2) * 8; // mixdeflen counts the bytes of the group it opens
    if bits.bit()? {
        bits.skip(1)?; // premixcmpsel
        bits.skip(1)?; // drcsrc
        bits.skip(3)?; // premixcmpscl
        for _ in 0..6 {
            if bits.bit()? {
                bits.skip(4)?; // the left, centre, right, two surround and LFE scales
            }
        }
        if bits.bit()? {
            for _ in 0..2 {
                if bits.bit()? {
                    bits.skip(4)?; // the two auxiliary scales
                }
            }
        }
    }
    if bits.bit()? {
        bits.skip(5)?; // spchdat
        if bits.bit()? {
            bits.skip(5)?; // spchdat1
            bits.skip(2)?; // spchan1att
            if bits.bit()? {
                bits.skip(5)?; // spchdat2
                bits.skip(3)?; // spchan2att
            }
        }
    }
    let used = bits.position() - start;
    let rest = group
        .checked_sub(used)
        .ok_or_else(|| invalid("E-AC-3 mixing parameters longer than the group that holds them"))?;
    bits.skip(rest)?; // mixdata and the fill bits that round it up
    Ok(())
}

/// Step over the informational metadata, which is what a player shows rather than
/// what it mixes by.
fn information(bits: &mut BitReader<'_>, acmod: usize) -> Result<()> {
    bits.skip(3)?; // bsmod
    bits.skip(1)?; // copyrightb
    bits.skip(1)?; // origbs
    if acmod == 2 {
        bits.skip(2)?; // dsurmod
        bits.skip(2)?; // dheadphonmod
    }
    if acmod >= 6 {
        bits.skip(2)?; // dsurexmod
    }
    if bits.bit()? {
        bits.skip(5)?; // mixlevel
        bits.skip(2)?; // roomtyp
        bits.skip(1)?; // adconvtyp
    }
    if acmod == 0 && bits.bit()? {
        bits.skip(5)?; // mixlevel2
        bits.skip(2)?; // roomtyp2
        bits.skip(1)?; // adconvtyp2
    }
    // sourcefscod is there for every rate this reader takes, and the conversion
    // flag that follows it only for a frame of fewer than six blocks.
    bits.skip(1)?;
    Ok(())
}

/// Table E2.12, the coupling banding structure a frame's first coupled block assumes
/// when it withholds its own. Sub-band 0 carries no flag, since the structure is
/// written from the second sub-band of the coupling region up, which is where
/// [`open_coupling`] reads it.
const DEF_CPL_BNDSTRC: [bool; SUBDN] = [
    false, false, false, false, false, false, false, false, true, false, true, true, false, true,
    true, true, true, true,
];

/// Table E3.13, the first transform coefficient of each spectral extension sub-band.
/// The last row is not a sub-band at all: it is the coefficient one past the region a
/// frame with `spxendf` = 7 synthesises, which is why the table reaches to index 17.
const SPX_BAND: [usize; 18] = [
    25, 37, 49, 61, 73, 85, 97, 109, 121, 133, 145, 157, 169, 181, 193, 205, 217, 229,
];
/// The most bands the sub-bands of the synthesised region can group into: `spx_begin_subbnd`
/// starts at 2, `spx_end_subbnd` stops at 17, and the structure is written from the second
/// of them up.
const SPX_BANDS: usize = 16;
/// Table E2.11, the banding structure a frame's first spectral extension block assumes
/// when it withholds its own - the same trick [`DEF_CPL_BNDSTRC`] plays for coupling, and
/// indexed by absolute sub-band for the same reason.
const DEF_SPX_BNDSTRC: [bool; 18] = [
    false, false, false, false, false, false, false, false, false, true, false, true, false, true,
    false, true, false, true,
];

/// What one block of a frame knows about its spectral extension, and what it hands to
/// the blocks after it: where the synthesised region begins and ends, how its sub-bands
/// group into the bands coordinates are sent for, which channels take part and which of
/// them still owes its first set of them, and the envelope and blend factors of the bands
/// themselves.
///
/// Table E1.4 writes this state per block and Section E3.6.1 keeps it to the frame, so
/// [`Spx::begin_frame`] restores it between frames exactly as [`Core::begin_frame`] does
/// the AC-3 strategy - and the default banding structure is in it from the start, which
/// is what makes a block that sends no structure mean Table E2.11's in its first block
/// and the previous block's in any other.
#[derive(Clone, Copy)]
struct Spx {
    /// `spxinu`: whether this block synthesises its high bands at all.
    in_use: bool,
    /// `spxbegf` itself, which is what the end of coupling and the rematrixing band
    /// count are derived from rather than the sub-band below.
    beginf: usize,
    /// `chinspx[ch]`, and `firstspxcos[ch]`: which channels' high bands are synthesized,
    /// and which of them owes a set of coordinates it has never been sent.
    channels: [bool; FBW],
    owed: [bool; FBW],
    /// `spxbandtable[spxstrtf]`, the lowest coefficient the copy region draws from.
    copy_first: usize,
    /// `spxbandtable[spx_begin_subbnd]` and `spxbandtable[spx_end_subbnd]`: the first
    /// synthesized coefficient and the last plus one.
    first: usize,
    last: usize,
    /// `spxbndstrc[]` by absolute sub-band, and `nspxbnds` with `spxbndsztab[]` read out
    /// of it: the coefficients each of the block's bands holds.
    grouping: [bool; 18],
    bands: [usize; SPX_BANDS],
    count: usize,
    /// `spxco[ch][bnd] * 32`, the envelope the blended coefficients are scaled to, and
    /// the two blend factors beside it: one band's share of its energy goes to the noise
    /// drawn for it and the rest to the coefficients copied under it.
    scale: [[f32; SPX_BANDS]; FBW],
    signal: [[f32; SPX_BANDS]; FBW],
    noise: [[f32; SPX_BANDS]; FBW],
}

impl Default for Spx {
    /// A block that synthesises nothing, owes coordinates for every channel, and would
    /// group its bands by Table E2.11.
    fn default() -> Self {
        Self {
            in_use: false,
            beginf: 0,
            channels: [false; FBW],
            owed: [true; FBW],
            copy_first: 0,
            first: 0,
            last: 0,
            grouping: DEF_SPX_BNDSTRC,
            bands: [0; SPX_BANDS],
            count: 0,
            scale: [[0.0; SPX_BANDS]; FBW],
            signal: [[0.0; SPX_BANDS]; FBW],
            noise: [[0.0; SPX_BANDS]; FBW],
        }
    }
}

/// The seed the [`dice_noise`] walk starts on, which is as arbitrary as the generator it
/// seeds and only has to be somewhere other than zero.
const NOISE_SEED: u32 = 0x7F3A_5C19;

/// Section E3.6.4.2.4's pseudo-random number: Annex E asks only that the generator be
/// zero-mean and unity-variance and leaves the rest to the decoder, so this is a
/// linear congruential walk of its own drawn into [-1, 1] and widened to a variance of
/// one. It keeps its own state rather than sharing the Section 7.3.4 dither sequence so
/// that blending high bands cannot disturb the noise a zero-bit bin was given.
fn dice_noise(state: &mut u32) -> f32 {
    *state = state
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    let unit = ((*state >> 8) as f32) * (1.0 / (1 << 24) as f32);
    (2.0 * unit - 1.0) * 1.732_050_8
}

impl Spx {
    /// The state of a frame's first block, which is no block at all: the reuse promise
    /// of Section E3.6.1 reaches one frame and no further.
    fn begin_frame(&mut self) {
        *self = Self::default();
    }

    /// Table E1.4's spectral extension strategy: which channels take part, where the
    /// region begins and ends, and how its sub-bands group into bands. `false` for a
    /// structure no coefficient could be written against.
    fn read_strategy(&mut self, header: Header, bits: &mut Bits<'_>) -> bool {
        self.in_use = true;
        if header.acmod == 1 {
            // A 1+1 block puts the first of its two programs into spectral extension and
            // states nothing about either of them; the second codes its own bands.
            self.channels = [false; FBW];
            self.channels[0] = true;
        } else {
            for ch in 0..header.nfchans {
                self.channels[ch] = bits.flag();
            }
        }
        let copy = bits.take(2) as usize;
        let begin = bits.take(3) as usize;
        let end = bits.take(3) as usize;
        self.copy_first = SPX_BAND[copy];
        self.beginf = begin;
        // Sections E2.3.3.5 and E2.3.3.6: the two codes name sub-bands, and the region
        // is worth writing only if the ones they name hold at least one band each.
        let begin = if begin < 6 { begin + 2 } else { begin * 2 - 3 };
        let end = if end < 3 { end + 5 } else { end * 2 + 3 };
        self.first = SPX_BAND[begin];
        self.last = SPX_BAND[end];
        if begin >= end || self.copy_first >= self.first {
            return false;
        }
        if bits.flag() {
            for bnd in begin + 1..end {
                self.grouping[bnd] = bits.flag();
            }
        }
        // `nspxbnds` and `spxbndsztab[]`, section E3.6.2: a band begins at each sub-band
        // whose flag says so and joins the one before it at each that says otherwise.
        self.bands = [0; SPX_BANDS];
        self.bands[0] = 12;
        self.count = 1;
        for bnd in begin + 1..end {
            if self.grouping[bnd] {
                self.bands[self.count - 1] += 12;
            } else {
                self.bands[self.count] = 12;
                self.count += 1;
            }
        }
        true
    }

    /// A block that leaves spectral extension off clears every channel's part in it and
    /// leaves each of them owing a set of coordinates, which is Table E1.4's own
    /// `firstspxcos[ch] = 1`.
    fn closed(&mut self) {
        self.in_use = false;
        self.channels = [false; FBW];
        self.owed = [true; FBW];
    }

    /// The `cplendf` coupling stops at when the block synthesises what is above it:
    /// Section E3.3.1 takes the 4-bit code out of the stream and derives it from
    /// `spxbegf`, which is also what makes the last coupled coefficient sit one below the
    /// first synthesized one. Signed, because the derived value is -2 for `spxbegf` = 0.
    fn coupling_end(&self) -> i32 {
        let begin = i32::try_from(self.beginf).unwrap_or(0);
        if begin < 6 {
            begin - 2
        } else {
            begin * 2 - 7
        }
    }

    /// Table E1.4's spectral extension coordinates: a `spxcoe` bit per channel that takes
    /// part, forced on the first set that channel is ever sent, and for each channel that
    /// states them its blend, its master gain, and an exponent and mantissa per band.
    /// The blend factors fall out of the coordinates, since a block that reuses them
    /// reuses the way it blends noise with them too.
    fn read_coordinates(&mut self, header: Header, bits: &mut Bits<'_>) {
        for ch in 0..header.nfchans {
            if !self.channels[ch] {
                self.owed[ch] = true;
                continue;
            }
            let stated = if self.owed[ch] {
                self.owed[ch] = false;
                true
            } else {
                bits.flag()
            };
            if !stated {
                continue;
            }
            let blend = bits.take(5) as f32 / 32.0;
            let master = bits.take(2) as usize;
            let mut offset = self.first;
            for bnd in 0..self.count {
                let exponent = bits.take(4) as usize;
                let mantissa = bits.take(2) as f32;
                // Section E3.6.3: an exponent below its top value says the mantissa's
                // leading bit is one and was left out, and the master gain shifts every
                // band of the channel by three exponents apiece. The 32 is the scale
                // Section E3.6.4.3 applies to the blend, folded in here because it is the
                // same for every bin of every band.
                let value = if exponent == 15 {
                    mantissa / 4.0
                } else {
                    (mantissa + 4.0) / 8.0
                };
                let shift = (exponent + 3 * master) as i32;
                self.scale[ch][bnd] = value * 32.0 / (2f32.powi(shift));
                // Section E3.6.4.2.1: how far the band's middle sits above the copy
                // region decides how much of it is noise, and `spxblnd` offsets that.
                let centre = offset as f32 + 0.5 * self.bands[bnd] as f32;
                let ratio = (centre / self.last as f32 - blend).clamp(0.0, 1.0);
                self.noise[ch][bnd] = ratio.sqrt();
                self.signal[ch][bnd] = (1.0 - ratio).sqrt();
                offset += self.bands[bnd];
            }
        }
    }

    /// Section E3.3.3: the coded run of a channel whose high bands are synthesized stops
    /// where they begin, and no bandwidth code is sent to say where that is. A coupled
    /// channel stops earlier still, where coupling begins, which is what
    /// [`BlockStrategy::measure`] already wrote for it.
    fn reach(&self, header: Header, strategy: &mut BlockStrategy) {
        for ch in 0..header.nfchans {
            if self.channels[ch] && !(strategy.cplinu && strategy.incpl[ch]) {
                strategy.end[ch] = self.first;
            }
        }
    }
}

/// Enhanced AC-3 decoder: a packet of Annex E syncframes in, interleaved f32 out.
///
/// Annex E changes the side information around an AC-3 block and almost nothing
/// inside it: the same 256 coefficients, the same 512-point window, the same bit
/// allocation and the same mantissa runs, which is why the unpacking here is the AC-3
/// core's. What this decoder adds is Table E1.4's walk - the order a block's fields
/// arrive in, which of them the frame already answered for it, and which of them say
/// the block is asking for syntax this module does not have - and, above it all, the
/// high bands Section E3.6 synthesises for a block that codes none of them.
pub struct Eac3Decoder {
    /// The rate the container stamped the track with, which is also the timebase the
    /// samples are handed back on.
    sample_rate: u32,
    /// The channel count the container stated, which fixes what the folding has to
    /// produce.
    channels: usize,
    core: Core,
    /// Samples still owed to [`ENCODER_DELAY`], which the stream's first blocks pay off.
    lead: usize,
    /// What this block knows about its spectral extension, and what the frame's next
    /// block inherits from it.
    spx: Spx,
    /// The state of the [`dice_noise`] walk, which is high bands' only randomness.
    noise_state: u32,
}

impl Eac3Decoder {
    /// Open an Enhanced AC-3 stream. `configuration` holds nothing this codec needs,
    /// since the geometry is in the frames themselves, and is only taken to match the
    /// other decoders' signature. The rate has to be one Annex E names and the channel
    /// count one the folding can reach; anything else is refused by name.
    pub fn new(configuration: &[u8], sample_rate: u32, channels: u16) -> Result<Self> {
        let _ = configuration;
        if !SAMPLE_RATES.contains(&sample_rate) {
            return Err(invalid(&format!(
                "E-AC-3 track at {sample_rate} Hz is outside Annex E's 48000, 44100 and 32000 Hz"
            )));
        }
        if !(1..=6).contains(&channels) {
            return Err(invalid(&format!(
                "E-AC-3 track of {channels} channels has neither a native layout nor a downmix here"
            )));
        }
        Ok(Self {
            sample_rate,
            channels: usize::from(channels),
            core: Core::new(),
            lead: ENCODER_DELAY,
            spx: Spx::default(),
            noise_state: NOISE_SEED,
        })
    }

    /// Current audio specification: the rate and channel count the track states, which
    /// is what the decoder holds its output to.
    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: self.channels as u16,
            format: SampleFormat::F32,
        }
    }

    /// The geometry an Annex E frame hands the AC-3 core: the same count of channels
    /// and the same rate code AC-3 reads out of its own `syncinfo`, with the Lo/Ro
    /// levels the frame's mixing metadata states in place of the 2-bit codes AC-3
    /// keeps beside `acmod`.
    fn geometry(frame: &Frame) -> Header {
        Header::annex_e(
            usize::from(frame.rate_code),
            frame.frame_bytes,
            usize::from(frame.layout),
            frame.low_frequency,
            frame.centre_level(),
            frame.surround_level(),
        )
    }

    /// Every syncframe the packet holds, each standing on its own. A frame whose syntax
    /// faults contributes its full length of silence, so a bad frame costs a frame
    /// rather than the packet's timing, while a frame that names syntax this module does
    /// not read ends the packet with the error that says so: a missing feature is not a
    /// broken file, and only the caller can keep them apart.
    fn frames(&mut self, data: &[u8], out: &mut Vec<f32>) -> Result<()> {
        let mut offset = 0usize;
        let mut decoded = 0usize;
        while offset < data.len() {
            let raw = &data[offset..];
            let frame = match header(&mut BitReader::new(raw)) {
                Ok(frame) => frame,
                Err(reason @ Error::Unsupported(_)) => return Err(reason),
                // A header that runs off the end of the packet is the stream ending.
                Err(_) => break,
            };
            let silence = frame.blocks * SAMPLES * self.channels;
            let usable = frame.frame_bytes.min(raw.len());
            // The header is read from the packet's own bytes, because only its length
            // says where the frame ends; the view the blocks are read from is trimmed to
            // that end, so a truncated tail mutes rather than reaching into whatever
            // follows the packet.
            let mut bits = Bits::at(&raw[..usable], frame.audio_bit);
            let mut samples = Vec::with_capacity(silence);
            let sound = usable == frame.frame_bytes
                && self.syncframe(&frame, &mut bits, &mut samples)?;
            if std::env::var("EAC3_TRACE").is_ok() {
                eprintln!(
                    "frame@{offset} bytes={} bits={} audio_bit={} ends={} overrun={} sound={sound}",
                    frame.frame_bytes,
                    frame.frame_bytes * 8,
                    frame.audio_bit,
                    bits.pos,
                    bits.overrun,
                );
            }
            if sound {
                samples.truncate(silence);
            } else {
                samples.clear();
                samples.resize(silence, 0.0);
            }
            out.extend(samples);
            offset += frame.frame_bytes;
            decoded += 1;
        }
        if decoded == 0 {
            return Err(invalid("E-AC-3 packet holds no syncframe"));
        }
        Ok(())
    }

    /// One syncframe: the blocks the frame says it carries, in the order it carries
    /// them. `false` mutes the frame.
    fn syncframe(
        &mut self,
        frame: &Frame,
        bits: &mut Bits<'_>,
        out: &mut Vec<f32>,
    ) -> Result<bool> {
        let header = Self::geometry(frame);
        let Some(folding) = Folding::new(header, self.channels) else {
            return Ok(false);
        };
        // Reuse is a within-frame promise, and so is the coupling state a block
        // inherits when its frame says it states nothing.
        self.core.begin_frame();
        self.spx.begin_frame();
        let starts = frame.block_starts;
        for blk in 0..frame.blocks {
            if let Some(starts) = starts {
                if blk > 0 {
                    // The frame said where this block begins. A frame that states no
                    // starts lets its blocks divide what the header leaves between them.
                    bits.pos = starts[blk - 1];
                }
            }
            if !self.block(blk, frame, header, &folding, bits, out)? {
                return Ok(false);
            }
        }
        Ok(!bits.overrun)
    }

    /// One audio block, read in the order Table E1.4 lays out, then unpacked the way
    /// Section 7.9 does for AC-3. `Ok(false)` mutes the frame, which is what
    /// Section 5.4.3.24 and Table 5.16 prescribe for syntax that cannot describe audio;
    /// `Err` names a shape this module does not read at all.
    fn block(
        &mut self,
        blk: usize,
        frame: &Frame,
        header: Header,
        folding: &Folding,
        bits: &mut Bits<'_>,
        out: &mut Vec<f32>,
    ) -> Result<bool> {
        // Section 7.10.2 states the reuse parameters a first block cannot leave unsent.
        let first = blk == 0;
        let nfchans = header.nfchans;
        let state = frame.blocks_state;
        let mut strategy = std::mem::take(&mut self.core.strategy);

        // Block switching went out with the `blkswe` refusal that closed the frame, so
        // no block says anything here and every one keeps the frame's long window.
        // Dither is the frame's choice too: a block that states no flags dithers, which
        // is Table E1.4's own `dithflag[ch] = 1`.
        for ch in 0..nfchans {
            strategy.dither[ch] = if state.dither_flags { bits.flag() } else { true };
        }
        say("dither", blk, bits.pos);
        // Dynamic range: a block that sends no code keeps the gain the block before it
        // sent, and the first block of a frame that sends none is at 0 dB - which is
        // what `begin_frame` leaves standing.
        if bits.flag() {
            strategy.gain[0] = dynrng_gain(bits.take(8));
        }
        if header.acmod == 0 && bits.flag() {
            strategy.gain[1] = dynrng_gain(bits.take(8));
        }
        say("dynrng", blk, bits.pos);
        // Spectral extension: `spxstre` is implicit at block 0, and a block that puts
        // SPX in use rebuilds its high bands by rules of their own. It is the only
        // block-level shape the frame's flags do not settle beforehand, and it is read
        // here because Table E1.4 writes it ahead of everything else a block says about
        // its bands - coupling stops where the synthesized region begins, and the
        // rematrixing count and each channel's coded run both answer to it.
        let mut ok = true;
        if first || bits.flag() {
            if bits.flag() {
                ok = self.spx.read_strategy(header, bits);
            } else {
                self.spx.closed();
            }
        }
        if ok && self.spx.in_use {
            self.spx.read_coordinates(header, bits);
        }
        say("spx", blk, bits.pos);
        // Coupling strategy: `cplstre[blk]` and `cplinu[blk]` both come from the frame,
        // so a block that inherits a strategy reads no coupling bits here at all, and
        // only the band set of a block that begins or restates coupling is its own.
        strategy.cpl_opened = false;
        strategy.cpl_moved = false;
        if frame.coupling_stated[blk] {
            let continued = strategy.cplinu;
            if frame.coupling[blk] {
                // Enhanced coupling describes an amplitude and an angle per band in
                // place of one coordinate, which is a decoder of its own rather than a
                // variant of this one.
                if bits.flag() {
                    return Err(unsupported(
                        "an E-AC-3 block that couples by enhanced coupling",
                    ));
                }
                strategy.incpl = [false; FBW];
                if header.acmod == 2 {
                    // A 2/0 block that couples couples both channels, and says so with
                    // no bits at all.
                    strategy.incpl[..2].fill(true);
                } else {
                    for ch in 0..nfchans {
                        strategy.incpl[ch] = bits.flag();
                    }
                }
                strategy.phsflginu = header.acmod == 2 && bits.flag();
                let begin = bits.take(4) as usize;
                // Section E3.3.1: the block that synthesises the bands above coupling
                // leaves the end of coupling to the spectral extension begin frequency,
                // and sends no code for it.
                let end = if self.spx.in_use {
                    self.spx.coupling_end()
                } else {
                    bits.take(4) as i32
                };
                let banding = bits.flag(); // `cplbndstrce`
                if !banding && !continued {
                    // Section E2.3.3.15: a frame's first coupled block that sends no
                    // banding structure means Table E2.12's, while any other reuses the
                    // structure the block before it settled on - which is already here.
                    strategy.bndstrc = DEF_CPL_BNDSTRC;
                }
                ok = strategy.open_coupling(header, continued, begin, end, banding, bits);
            } else {
                // A block that does not couple leaves every coupling parameter off, and
                // the next block that couples owes its coordinates and its leaks to
                // itself rather than to a bit saying that it does.
                strategy.cplinu = false;
                strategy.incpl = [false; FBW];
                strategy.phsflginu = false;
                strategy.firstcplcos = [true; FBW];
                strategy.firstcplleak = true;
            }
        }
        say("cplstrategy", blk, bits.pos);
        if ok && strategy.cplinu {
            // Annex E's one change to the coordinates: a channel whose coordinates no
            // block has delivered yet states them without paying a `cplcoe` bit.
            ok = strategy.coupling_coordinates(header, bits, true);
        }
        say("cplcoords", blk, bits.pos);
        if header.acmod == 2 {
            strategy.rematrix.reshape(strategy.cplinu, strategy.cplbegf);
            if !strategy.cplinu && self.spx.in_use {
                // Section E3.3.2: a 2/0 block that synthesises its high bands and couples
                // none of them states one band fewer above the copy region than one that
                // codes them, and the bands themselves stay Table 7.25's.
                strategy.rematrix.count = if self.spx.beginf < 2 { 3 } else { 4 };
            }
            // `rematstr` is implicit at block 0; a later block that withholds its flags
            // keeps the ones the block before it sent.
            if first || bits.flag() {
                for band in 0..strategy.rematrix.count {
                    strategy.rematrix.flags[band] = bits.flag();
                }
            }
        }
        say("rematrix", blk, bits.pos);
        // Exponent strategies: the frame wrote these, in whichever of the two forms
        // `expstre` selects, and [`Frame::strategies`] resolves them for this block, so
        // the block carries no strategy bits of its own.
        strategy.expstr = [0; PLANES];
        if let Some(strategies) = frame.strategies(blk) {
            if let Some(coupling) = strategies.coupling {
                strategy.expstr[CPL] = coupling.code();
            }
            for ch in 0..nfchans {
                strategy.expstr[ch] = strategies.channels[ch].code();
            }
            if let Some(low_frequency) = strategies.low_frequency {
                strategy.expstr[LFE] = low_frequency.code();
            }
        }
        if strategy.expstr[CPL] == 0
            && strategy.cplinu
            && (strategy.cpl_opened || strategy.cpl_moved)
        {
            // Section 7.10.2 conditions 6 and 7, which Annex E keeps: exponents coded
            // for one band set cannot describe another.
            ok = false;
        }
        for ch in 0..nfchans {
            if first && strategy.expstr[ch] == 0 {
                // Section 7.10.2 condition 8: nothing may be reused before anything was
                // sent, and a frame whose own code left a channel at reuse for its first
                // block cannot be read.
                ok = false;
            }
        }
        if first && header.lfeon && strategy.expstr[LFE] == 0 {
            ok = false;
        }
        for ch in 0..nfchans {
            if !ok {
                break;
            }
            // A channel whose high bands went into the coupling plane, or whose are
            // synthesized above the region it codes, states no bandwidth for them.
            if strategy.expstr[ch] != 0 && !strategy.incpl[ch] && !self.spx.channels[ch] {
                let code = bits.take(6) as usize;
                // Section 5.4.3.24: above this bandwidth the stream is invalid and the
                // decoder shall cease decoding audio and mute.
                if code > 60 {
                    ok = false;
                    break;
                }
                strategy.bwcod[ch] = code;
            }
        }
        if !ok {
            self.core.strategy = strategy;
            return Ok(false);
        }
        say("sideinfo", blk, bits.pos);
        strategy.measure(header);
        if std::env::var("EAC3_TRACE").is_ok() {
            let p = &strategy.params;
            eprintln!(
                "      expstr={:?} start={:?} end={:?} cpl beg={} end={} sub={} bnd={} incpl={:?} ph={:?}",
                strategy.expstr, strategy.start, strategy.end, strategy.cplbegf,
                strategy.cplendf, strategy.subnd, strategy.bnd, strategy.incpl, strategy.phsflginu,
            );
            eprintln!(
                "      params baie={} snre={} cs={} fs={:?} floor={} sd={} fd={} sg={} db={} gains={} fg={:?} leak={:?} delt={}",
                p.baie, p.snre, p.csnroffst, p.fsnroffst, p.floorcod, p.sdcycod, p.fdcycod,
                p.sgaincod, p.dbpbcod, p.gains, p.fgaincod, p.cplleak, p.delt,
            );
            eprintln!(
                "      frame expstrategies {:?} snr={:?} lfe={:?}",
                frame.strategies(blk), frame.snr_offsets, frame.low_frequency_strategies,
            );
            eprintln!("      raw exponents {:?}", frame.exponents);
        }
        if self.spx.in_use {
            // Section E3.3.3: a channel that is one of the synthesized ones stops where
            // the region begins, and its coded run is measured from there rather than
            // from a bandwidth code it never sent.
            self.spx.reach(header, &mut strategy);
        }
        if strategy.expstr[CPL] != 0 {
            // Section 7.1.3: the coupling plane's absolute exponent is a reference
            // rather than a coefficient's, and arrives doubled.
            let absexp = bits.take(4) << 1;
            strategy.exponents(bits, CPL, absexp);
        }
        for ch in 0..nfchans {
            if strategy.expstr[ch] != 0 {
                let absexp = bits.take(4);
                strategy.exponents(bits, ch, absexp);
                // `gainrng` is Section 7.9.5's optional pre-scale for a decoder working
                // in integers; f32 output needs no headroom taken back.
                bits.skip(2);
            }
        }
        if header.lfeon && strategy.expstr[LFE] != 0 {
            let absexp = bits.take(4);
            strategy.exponents(bits, LFE, absexp);
        }
        if state.bit_allocation {
            strategy.params.baie = bits.flag();
            if strategy.params.baie {
                strategy.params.sdcycod = bits.take(2) as usize;
                strategy.params.fdcycod = bits.take(2) as usize;
                strategy.params.sgaincod = bits.take(2) as usize;
                strategy.params.dbpbcod = bits.take(2) as usize;
                strategy.params.floorcod = bits.take(3) as usize;
            } else if first {
                // Section 5.4.3.30: the first block states the allocation prototype.
                self.core.strategy = strategy;
                return Ok(false);
            }
        } else {
            // `bamode` = 0 is the frame saying every block of it mixes by one fixed
            // curve, which Table E1.4 names outright instead of reaching for AC-3's
            // Table 5.20.
            strategy.params.baie = false;
            strategy.params.sdcycod = 2;
            strategy.params.fdcycod = 1;
            strategy.params.sgaincod = 1;
            strategy.params.dbpbcod = 2;
            strategy.params.floorcod = 7;
        }
        // Signal-to-noise offsets. Strategy 0 is the frame handing the same pair to
        // every block; the two per-block strategies leave the block to state its own,
        // with `snroffste` implicit at block 0 so that the frame's first block always
        // pays for them.
        strategy.params.snre = false;
        if let Some(offsets) = frame.snr_offsets {
            strategy.params.snre = true;
            strategy.params.csnroffst = i32::from(offsets.coarse);
            let fine = i32::from(offsets.fast);
            for plane in strategy.params.fsnroffst.iter_mut() {
                *plane = fine;
            }
        } else if first || bits.flag() {
            strategy.params.snre = true;
            strategy.params.csnroffst = bits.take(6);
            if state.snr_offsets == 1 {
                // Strategy 1 is one block-wide offset for every plane of the block.
                let fine = bits.take(4);
                for plane in strategy.params.fsnroffst.iter_mut() {
                    *plane = fine;
                }
            } else {
                if strategy.cplinu {
                    strategy.params.fsnroffst[CPL] = bits.take(4);
                }
                for ch in 0..nfchans {
                    strategy.params.fsnroffst[ch] = bits.take(4);
                }
                if header.lfeon {
                    strategy.params.fsnroffst[LFE] = bits.take(4);
                }
            }
        }
        // Fast gain codes, which travel on their own flag rather than with the offsets
        // as they do in Table 5.3.
        strategy.params.gains = state.gain_codes && bits.flag();
        if strategy.params.gains {
            if strategy.cplinu {
                strategy.params.fgaincod[CPL] = bits.take(3) as usize;
            }
            for ch in 0..nfchans {
                strategy.params.fgaincod[ch] = bits.take(3) as usize;
            }
            if header.lfeon {
                strategy.params.fgaincod[LFE] = bits.take(3) as usize;
            }
        } else {
            // Table E1.4's own default, which is not AC-3's: a block that states no
            // gains mixes by `FASTGAIN[4]`, not by the last entry of Table 5.32.
            for plane in strategy.params.fgaincod.iter_mut() {
                *plane = 4;
            }
        }
        // The offsets of the AC-3 syncframes this stream converts to, which no player
        // that sounds E-AC-3 out as itself reads. They are there for every independent
        // stream, which is every stream this decoder opens.
        if bits.flag() {
            bits.skip(10); // convsnroffst
        }
        strategy.params.leaks = false;
        if strategy.cplinu {
            // A block that begins coupling states its leaks with no flag saying so,
            // which is all `firstcplleak` is for.
            let leak = if strategy.firstcplleak {
                strategy.firstcplleak = false;
                true
            } else {
                bits.flag()
            };
            if leak {
                let fast = bits.take(3);
                let slow = bits.take(3);
                strategy.params.cplleak = [fast, slow];
                strategy.params.leaks = true;
            }
        }
        strategy.params.delt = false;
        if state.dynamic_parameters && !strategy.delta_ba(header, bits) {
            // The reserved delta bit allocation mode, which mutes the frame.
            self.core.strategy = strategy;
            return Ok(false);
        }
        if state.skip_fields && bits.flag() {
            let skip = bits.take(9) as usize;
            bits.skip(skip * 8);
        }
        // From here the block is an AC-3 block: the mantissas Table E1.4 writes in
        // Table 5.3's own order, the coupling and rematrixing that rebuild the planes,
        // the transform, and the fold into the channels the track plays.
        self.core.strategy = strategy;
        if self.core.strategy.needs_allocation(header) {
            self.core.allocate_all(header);
        }
        say("mantissas", blk, bits.pos);
        self.core.mantissas(header, bits);
        say("after mantissa", blk, bits.pos);
        self.core.decouple(header);
        if self.spx.in_use {
            self.synthesise(header);
        }
        self.core.rematrix_restore(header);
        self.core.spectrum_to_sound(header, folding, out);
        Ok(true)
    }

    /// Section E3.6.4: fill the region this block synthesises by copying the bands below
    /// it up, blending the copies with noise, and scaling the blend to the envelope the
    /// block's coordinates describe. It runs after decoupling because the copy region is
    /// the whole coded band - what the channel coded itself and what coupling restored
    /// for it alike - and before the transform, which sees the result as coefficients the
    /// stream had sent.
    ///
    /// The scaling is what makes the region right: the noise is the standard's own
    /// freedom, so a decoder's high bands match a reference decode band for band rather
    /// than sample for sample.
    fn synthesise(&mut self, header: Header) {
        let spx = self.spx;
        for ch in 0..header.nfchans {
            if !spx.channels[ch] {
                continue;
            }
            let coefficients = self.core.coefficients(ch);
            // Section E3.6.4.1: the copy walks up from `spxstrtf` while the insert walks
            // up from the begin frequency, and a band that would copy past the coded
            // region restarts the copy at its bottom.
            let mut copy = spx.copy_first;
            let mut insert = spx.first;
            for bnd in 0..spx.count {
                let size = spx.bands[bnd];
                if copy + size > spx.first {
                    copy = spx.copy_first;
                }
                for _ in 0..size {
                    if copy == spx.first {
                        copy = spx.copy_first;
                    }
                    let value = coefficients[copy];
                    coefficients[insert] = value;
                    insert += 1;
                    copy += 1;
                }
            }
            let mut noise_state = self.noise_state;
            let mut start = spx.first;
            for bnd in 0..spx.count {
                let size = spx.bands[bnd];
                // Section E3.6.4.2.2: the band's own energy is what the noise is scaled
                // to, so that blending it in changes the band's character and not its
                // level.
                let mut accum = 0.0f64;
                for bin in start..start + size {
                    let value = f64::from(coefficients[bin]);
                    accum += value * value;
                }
                let energy = (accum / size as f64).sqrt() as f32 * spx.noise[ch][bnd];
                let signal = spx.signal[ch][bnd];
                let scale = spx.scale[ch][bnd];
                for bin in start..start + size {
                    let copied = coefficients[bin];
                    coefficients[bin] =
                        (copied * signal + dice_noise(&mut noise_state) * energy) * scale;
                }
                start += size;
            }
            self.noise_state = noise_state;
        }
    }
}

impl AudioDecode for Eac3Decoder {
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

    /// A seek starts a new frame, and every frame states its own strategies, so the
    /// state to undo is the reuse history and the overlap tail. The dither sequence
    /// restarts from its seed with them, which is what makes a packet decoded twice
    /// after a seek the same packet rather than the same signal plus different noise.
    fn reset(&mut self) {
        self.core = Core::new();
        self.lead = ENCODER_DELAY;
        self.spx.begin_frame();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2/0 stream at 48 kHz and 192 kbps whose channels each carry two tones, one
    /// frame per 32 ms.
    ///
    /// ```text
    /// ffmpeg -hide_banner -loglevel error -y -f lavfi \
    ///   -i 'aevalsrc=0.3*sin(2*PI*440*t)+0.1*sin(2*PI*1500*t)|0.2*sin(2*PI*220*t)+0.1*sin(2*PI*300*t):s=48000:d=0.25' \
    ///   -c:a eac3 -b:a 192k tests/fixtures/audio/eac3-stereo.ec3
    /// ```
    const STEREO: &[u8] = include_bytes!("../../tests/fixtures/audio/eac3-stereo.ec3");

    /// A 5.1 stream at 448 kbps with one tone per channel, so the channel count the
    /// header reports is the count the blocks will hold and the LFE is the difference
    /// between the two.
    ///
    /// ```text
    /// ffmpeg -hide_banner -loglevel error -y -f lavfi \
    ///   -i 'aevalsrc=0.3*sin(2*PI*440*t)|0.2*sin(2*PI*220*t)|0.15*sin(2*PI*1000*t)|0.1*sin(2*PI*3000*t)|0.05*sin(2*PI*5500*t)|0.4*sin(2*PI*60*t):s=48000:d=0.25' \
    ///   -ch_layout 5.1 -c:a eac3 -b:a 448k tests/fixtures/audio/eac3-51.ec3
    /// ```
    const SURROUND: &[u8] = include_bytes!("../../tests/fixtures/audio/eac3-51.ec3");

    /// AC-3's own 2/0 stream, which shares the syncword and nothing else: reading it
    /// as Annex E must say so rather than invent a geometry.
    const AC3: &[u8] = include_bytes!("../../tests/fixtures/audio/ac3-stereo.ac3");

    /// Walk a whole elementary stream frame by frame, the way a bare-file reader will.
    fn walk(bytes: &[u8]) -> Vec<Frame> {
        let mut frames = Vec::new();
        let mut rest = bytes;
        while let Some(read) = frame(rest) {
            rest = &rest[read.frame_bytes..];
            frames.push(read);
        }
        assert!(rest.is_empty(), "the walk stopped {} bytes early", rest.len());
        frames
    }

    /// What every frame of one stream agrees on, as against the strategy codes, which
    /// an encoder is free to change from frame to frame.
    fn geometry(read: &Frame) -> (u32, u16, bool, usize, usize, usize, usize) {
        (
            read.sample_rate,
            read.channels,
            read.low_frequency,
            read.frame_bytes,
            read.blocks,
            read.audio_bit,
            read.coupling.iter().filter(|&&flag| flag).count(),
        )
    }

    /// The bits planted behind the metadata of a crafted frame, which no field of
    /// Table E1.2 spells this way: a walk that counts one bit too many or too few
    /// finds something else there.
    const MARKER: u32 = 0x2DD3;

    /// A frame's own bits, written one field at a time, for the syntax the encoders
    /// on this machine do not produce.
    #[derive(Default)]
    struct Sink {
        bytes: Vec<u8>,
        spare: u8,
        width: usize,
    }

    impl Sink {
        fn put(&mut self, value: u32, count: usize) -> &mut Self {
            for step in (0..count).rev() {
                let bit = if step < usize::BITS as usize {
                    ((value >> step) & 1) as u8
                } else {
                    0
                };
                self.push(bit);
            }
            self
        }

        /// `count` bits of nothing, for the slack a walk has to run into before a
        /// test can see that it ran too far.
        fn zeros(&mut self, count: usize) -> &mut Self {
            for _ in 0..count {
                self.push(0);
            }
            self
        }

        fn push(&mut self, bit: u8) {
            self.spare = (self.spare << 1) | bit;
            self.width += 1;
            if self.width == 8 {
                self.bytes.push(self.spare);
                self.spare = 0;
                self.width = 0;
            }
        }

        /// Pad out to the next byte, so the frame is a whole number of them.
        fn finish(&mut self) -> &mut Self {
            if self.width != 0 {
                let rest = 8 - self.width;
                self.zeros(rest);
            }
            self
        }
    }

    /// A six-block frame at 48 kHz in `acmod`, with `compre`'s compression word when
    /// asked for, then whatever `metadata` writes from the `mixmdate` flag on, then
    /// the marker and enough slack behind it that a walk that runs on is seen to.
    fn crafted(acmod: usize, lfeon: bool, compre: bool, metadata: impl FnOnce(&mut Sink)) -> Vec<u8> {
        let mut sink = Sink::default();
        sink.put(0x0B77, 16)
            .put(0, 2) // strmtyp: independent
            .put(0, 3) // substreamid: the only one a frame of this kind has
            .put(200, 11) // frmsiz: the frame claims 402 bytes
            .put(0, 2) // fscod: 48 kHz
            .put(3, 2) // numblkscod: six blocks
            .put(acmod as u32, 3)
            .put(u32::from(lfeon), 1)
            .put(EAC3_BSID, 5)
            .put(31, 5) // dialnorm
            .put(u32::from(compre), 1);
        if compre {
            sink.put(1, 8); // compr
        }
        if acmod == 0 {
            sink.put(31, 5).put(0, 1); // dialnorm2, compr2e
        }
        metadata(&mut sink);
        sink.put(MARKER, 16).zeros(64).finish();
        sink.bytes
    }

    /// Where the `bsi` of a crafted frame ends, which is where its marker sits: a
    /// metadata field read at the wrong width leaves a walk that never reaches it.
    /// `audfrm` goes on past the marker, which is what [`walked`] reads.
    fn bsi_end(bytes: &[u8]) -> usize {
        let mut bits = BitReader::new(bytes);
        assert_eq!(bits.read(16).unwrap(), SYNCWORD, "not a frame the helper wrote");
        assert_eq!(bits.read(2).unwrap(), 0, "strmtyp");
        assert_eq!(bits.read(3).unwrap(), 0, "substreamid");
        bits.skip(11).unwrap(); // frmsiz
        assert_eq!(bits.read(2).unwrap(), 0, "fscod");
        assert_eq!(bits.read(2).unwrap(), 3, "numblkscod");
        let acmod = bits.read(3).unwrap() as usize;
        let lfeon = bits.bit().unwrap();
        assert_eq!(bits.read(5).unwrap(), EAC3_BSID, "bsid");
        bsi(&mut bits, acmod, lfeon, BLOCKS).unwrap();
        let position = bits.position();
        assert_eq!(
            bits.read(16).unwrap(),
            MARKER,
            "the metadata walk left the reader inside the frame's own bits"
        );
        position
    }

    /// Read a crafted frame all the way to the bit it says its first block starts at,
    /// and insist the marker is what lies behind that bit: the walk's own claim about
    /// where the audio begins, checked against the bits the frame was written from.
    fn walked(bytes: &[u8]) -> Frame {
        let read = frame(bytes).expect("a crafted frame this reader should accept");
        let mut bits = BitReader::new(bytes);
        bits.skip(read.audio_bit).unwrap();
        assert_eq!(
            bits.read(16).unwrap(),
            MARKER,
            "the walk stopped {} bits away from the marker",
            read.audio_bit
        );
        read
    }

    /// The flags of Table E1.3 a crafted frame writes, every one of them off unless a
    /// test names it. The strategy data behind the flags is written as zeros, which is
    /// a legal code of every width, so what a test measures is the count of bits each
    /// flag brings with it.
    #[derive(Default)]
    struct Written {
        expstre: bool,
        ahte: bool,
        snroffststr: u8,
        transproce: bool,
        blkswe: bool,
        dithflage: bool,
        bamode: bool,
        gain_codes: bool,
        dbaflde: bool,
        skipflde: bool,
        spxattene: bool,
        couple: bool,
        block_starts: bool,
        starts: [u32; BLOCKS - 1],
    }

    impl Written {
        /// The frame's own flags, then whatever they ask for, then the block starts:
        /// in the order the table writes them, so a frame built this way ends at its
        /// marker whatever combination of flags is on.
        fn write(&self, sink: &mut Sink, acmod: usize, lfeon: bool) {
            let channels = NFCHANS[acmod];
            let coupling = self.couple && acmod > 1;
            sink.put(u32::from(self.expstre), 1)
                .put(u32::from(self.ahte), 1)
                .put(self.snroffststr as u32, 2)
                .put(u32::from(self.transproce), 1)
                .put(u32::from(self.blkswe), 1)
                .put(u32::from(self.dithflage), 1)
                .put(u32::from(self.bamode), 1)
                .put(u32::from(self.gain_codes), 1)
                .put(u32::from(self.dbaflde), 1)
                .put(u32::from(self.skipflde), 1)
                .put(u32::from(self.spxattene), 1);
            if acmod > 1 {
                // The first block states its own flag; the five behind it state no
                // flag of their own and so keep the one before them.
                sink.put(u32::from(self.couple), 1).zeros(5);
            }
            if self.expstre {
                for _ in 0..BLOCKS {
                    if coupling {
                        sink.zeros(2); // cplexpstr of the block
                    }
                    sink.zeros(2 * channels); // chexpstr of the block
                }
            } else {
                if coupling {
                    sink.zeros(5); // frmcplexpstr
                }
                sink.zeros(5 * channels); // frmchexpstr
            }
            if lfeon {
                sink.zeros(BLOCKS); // lfeexpstr
            }
            sink.zeros(5 * channels); // convexpstr
            if self.snroffststr == 0 {
                sink.zeros(10); // frmcsnroffst and frmfsnroffst
            }
            if self.transproce {
                for _ in 0..channels {
                    sink.put(1, 1).zeros(18); // a window, with its place and length
                }
            }
            sink.put(u32::from(self.block_starts), 1);
            if self.block_starts {
                // A frame of 402 bytes is 3216 bits, which twelve bits address.
                for &start in &self.starts {
                    sink.put(start, 12);
                }
            }
        }
    }

    /// A crafted frame with no bit stream metadata at all and the audio frame `audio`
    /// asks for, which is how a flag of Table E1.3 comes to be read on its own.
    fn audio_frame(acmod: usize, lfeon: bool, audio: impl FnOnce(&mut Written)) -> Vec<u8> {
        let mut written = Written {
            starts: [400, 800, 1_200, 1_600, 2_000],
            ..Written::default()
        };
        audio(&mut written);
        crafted(acmod, lfeon, false, |sink| {
            sink.put(0, 1) // mixmdate
                .put(0, 1) // infomdate
                .put(0, 1); // addbsie
            written.write(sink, acmod, lfeon);
        })
    }

    #[test]
    fn every_frame_of_a_stream_states_the_same_geometry() {
        let frames = walk(STEREO);
        assert_eq!(
            frames[0],
            Frame {
                sample_rate: 48_000,
                rate_code: 0,
                layout: 2,
                channels: 2,
                low_frequency: false,
                frame_bytes: 768,
                blocks: 6,
                audio_bit: 108,
                coupling: [true; BLOCKS],
                coupling_stated: [true, false, false, false, false, false],
                downmix: None,
                exponents: Exponents::PerFrame {
                    coupling: Some(16),
                    channels: [16, 16, 0, 0, 0],
                },
                low_frequency_strategies: [false; BLOCKS],
                converter_strategies: [16, 16, 0, 0, 0],
                snr_offsets: Some(SnrOffsets { coarse: 26, fast: 2 }),
                blocks_state: BlocksState {
                    dither_flags: false,
                    bit_allocation: false,
                    gain_codes: false,
                    dynamic_parameters: false,
                    skip_fields: false,
                    transient_processing: false,
                    snr_offsets: 0,
                },
                block_starts: None,
            }
        );
        assert_eq!(frames.len(), STEREO.len() / 768);
        assert!(
            frames.windows(2).all(|pair| geometry(&pair[0]) == geometry(&pair[1])),
            "a stream that changed its geometry mid-file would be a different stream"
        );
    }

    #[test]
    fn the_low_frequency_channel_is_one_of_the_ones_a_frame_counts() {
        let frames = walk(SURROUND);
        assert_eq!(
            frames[0],
            Frame {
                sample_rate: 48_000,
                rate_code: 0,
                layout: 7,
                channels: 6,
                low_frequency: true,
                frame_bytes: 1792,
                blocks: 6,
                audio_bit: 144,
                coupling: [true; BLOCKS],
                coupling_stated: [true, false, false, false, false, false],
                downmix: None,
                exponents: Exponents::PerFrame {
                    coupling: Some(22),
                    channels: [16, 16, 16, 16, 16],
                },
                // The first block states a strategy of its own; the five behind it
                // ask to reuse the block before them.
                low_frequency_strategies: [true, false, false, false, false, false],
                converter_strategies: [16, 16, 16, 16, 16],
                snr_offsets: Some(SnrOffsets { coarse: 24, fast: 4 }),
                blocks_state: BlocksState {
                    dither_flags: false,
                    bit_allocation: false,
                    gain_codes: false,
                    dynamic_parameters: false,
                    skip_fields: false,
                    transient_processing: false,
                    snr_offsets: 0,
                },
                block_starts: None,
            }
        );
        assert_eq!(frames.len(), SURROUND.len() / 1792);
    }

    /// The bit each real frame leaves for its first block, which is what a block
    /// reader starts from and what nothing but a measured walk can pin: an encoder
    /// that wrote one bit of metadata more would leave the same six blocks standing
    /// and shift every one of them off the screen.
    #[test]
    fn a_frame_states_the_bit_its_first_block_starts_at() {
        for (bytes, size, audio_bit) in [(STEREO, 768, 108), (SURROUND, 1_792, 144)] {
            for (number, read) in walk(bytes).iter().enumerate() {
                assert_eq!(
                    (read.frame_bytes, read.audio_bit),
                    (size, audio_bit),
                    "frame {number} of a {}-channel stream",
                    read.channels
                );
                assert_eq!(read.blocks, 6);
                assert!(read.coupling.iter().all(|&flag| flag));
                assert_eq!(read.block_starts, None, "an encoder that divides the frame itself");
            }
        }
    }

    /// The two syntaxes share their first two bytes and disagree on every bit after
    /// them, and AC-3 packs them the other way round, so an AC-3 frame walked as
    /// Annex E falls over on the first field past the syncword. Either of the two
    /// refusals is a good outcome; what matters is that no geometry is invented.
    #[test]
    fn an_ac_three_frame_is_refused_before_it_is_misread() {
        let error = header(&mut BitReader::new(AC3)).unwrap_err();
        assert!(matches!(error, crate::Error::Unsupported(_)), "{error}");
        let said = error.to_string();
        assert!(
            said.contains("stream type") || said.contains("bsid"),
            "{said} names neither of the two marks that tell the syntaxes apart"
        );
    }

    #[test]
    fn a_stream_that_is_not_the_independent_one_is_named_not_guessed_at() {
        for (kind, words) in [(1u32, "type 1"), (2, "type 2")] {
            let mut bytes = STEREO.to_vec();
            // The stream type is the frame's first two bits past the syncword.
            bytes[2] = (bytes[2] & 0x3F) | u8::try_from(kind << 6).unwrap();
            let error = header(&mut BitReader::new(&bytes)).unwrap_err();
            assert!(error.to_string().contains(words), "{error}");
        }
        let mut bytes = STEREO.to_vec();
        bytes[2] |= 0x20; // the top bit of the three that name the substream
        let error = header(&mut BitReader::new(&bytes)).unwrap_err();
        assert!(error.to_string().contains("second"), "{error}");
    }

    #[test]
    fn the_shapes_this_reader_will_not_unpack_yet_are_refused_by_name() {
        // One, two or three blocks per frame is the shape that carries AC-3's
        // conversion layer; six is what the encoders here write.
        let mut bytes = STEREO.to_vec();
        bytes[4] &= 0xEF; // the low bit of the two that count the blocks
        let error = header(&mut BitReader::new(&bytes)).unwrap_err();
        assert!(error.to_string().contains("3 blocks"), "{error}");

        // The reduced rates are the fourth `fscod`, which moves the band edges the
        // reused AC-3 tables are keyed to.
        let mut bytes = STEREO.to_vec();
        bytes[4] |= 0xC0;
        let error = header(&mut BitReader::new(&bytes)).unwrap_err();
        assert!(error.to_string().contains("reduced"), "{error}");
    }

    #[test]
    fn a_header_longer_than_the_frame_that_carries_it_is_not_read() {
        let mut bytes = STEREO.to_vec();
        // The eleven bits of `frmsiz` start at the frame's 22nd bit; one whole frame
        // is four bytes, which cannot hold a header of 54 bits.
        bytes[2] &= 0xF8;
        bytes[3] = 0x01;
        let error = header(&mut BitReader::new(&bytes)).unwrap_err();
        assert!(matches!(error, crate::Error::Invalid(_)), "{error}");
        assert!(error.to_string().contains("longer"), "{error}");
    }

    /// A frame that says it carries no bit stream metadata at all: 45 bits of
    /// geometry, then `dialnorm`, `compre`, and the three flags that end the bit stream
    /// information, then the 54 bits of an audio frame that states nothing beyond its
    /// two channels' strategies. Every syncframe of the stereo fixture ends there, and
    /// a frame written field by field ends at the same bit, which is the two walks
    /// agreeing on the same stream from opposite ends.
    #[test]
    fn a_frame_without_metadata_ends_where_the_real_ones_do() {
        let stopped = bsi_end(&crafted(2, false, false, |sink| {
            sink.put(0, 1) // mixmdate
                .put(0, 1) // infomdate
                .put(0, 1); // addbsie
        }));
        assert_eq!(stopped, 54);
        let audio = walked(&audio_frame(2, false, |written| written.couple = true));
        assert_eq!(stopped + 54, audio.audio_bit);
        assert_eq!(audio.audio_bit, frame(STEREO).unwrap().audio_bit);
    }

    /// The 5.1 broadcast shape: a preferred downmix mode, both pairs of folding
    /// levels, a stated LFE level and a program scale, then the informational group
    /// with its own mix level and room type. Its header was measured to end 104 bits
    /// into a real broadcast stream, which is the number this asserts.
    #[test]
    fn the_mixing_metadata_of_a_broadcast_stream_leaves_the_audio_where_it_starts() {
        let stopped = bsi_end(&crafted(7, true, true, |sink| {
            sink.put(1, 1) // mixmdate
                .put(1, 2) // dmixmod
                .put(1, 3) // ltrtcmixlev
                .put(2, 3) // lorocmixlev
                .put(3, 3) // ltrtsurmixlev
                .put(0, 3) // lorosurmixlev
                .put(1, 1) // lfemixlevcode
                .put(7, 5) // lfemixlevcod
                .put(0, 1) // pgmscle
                .put(0, 1) // extpgmscle
                .put(0, 2) // mixdef
                .put(0, 1) // frmmixcfginfoe
                .put(1, 1) // infomdate
                .put(2, 3) // bsmod
                .put(1, 1) // copyrightb
                .put(1, 1) // origbs
                .put(1, 2) // dsurexmod
                .put(1, 1) // audprodie
                .put(20, 5) // mixlevel
                .put(1, 2) // roomtyp
                .put(0, 1) // adconvtyp
                .put(0, 1) // sourcefscod
                .put(0, 1); // addbsie
        }));
        assert_eq!(stopped, 104);
    }

    /// The most flexible mixing definition states its own length in bytes, and every
    /// scale factor and speech parameter in it lives inside that: a walk that reads
    /// the group as a total stays in step whatever the flags inside it say.
    #[test]
    fn an_external_mixing_group_fills_the_bytes_it_declares() {
        let stopped = bsi_end(&crafted(2, false, false, |sink| {
            sink.put(1, 1) // mixmdate
                .put(0, 1) // pgmscle
                .put(0, 1) // extpgmscle
                .put(3, 2) // mixdef: the flexible one
                .put(5, 5) // mixdeflen: seven bytes from here
                .put(1, 1) // mixdata2e
                .put(1, 1) // premixcmpsel
                .put(0, 1) // drcsrc
                .put(3, 3) // premixcmpscl
                .put(1, 1) // extpgmlscle
                .put(6, 4) // extpgmlscl
                .put(0, 1) // extpgmcscle
                .put(1, 1) // extpgmrscle
                .put(7, 4) // extpgmrscl
                .put(0, 1) // extpgmlsscle
                .put(0, 1) // extpgmrsscle
                .put(0, 1) // extpgmlfescle
                .put(0, 1) // dmixscle
                .put(1, 1) // addche
                .put(1, 1) // extpgmaux1scle
                .put(4, 4) // extpgmaux1scl
                .put(0, 1) // extpgmaux2scle
                .put(1, 1) // mixdata3e
                .put(3, 5) // spchdat
                .put(1, 1) // addspchdate
                .put(4, 5) // spchdat1
                .put(2, 2) // spchan1att
                .put(1, 1) // addspchdat1e
                .put(5, 5) // spchdat2
                .put(3, 3) // spchan2att
                .put(0, 1) // frmmixcfginfoe
                .put(0, 1) // infomdate
                .put(0, 1); // addbsie
        }));
        // The parameters filled the seven declared bytes to the last bit, and the
        // three flags behind them are the whole of what is left to read.
        assert_eq!(stopped, 115);
    }

    /// A six-block frame states its mixing configuration once per block, which is
    /// what the frame's block count - and not a fixed number - has to answer for.
    #[test]
    fn per_block_mixing_configurations_are_counted_per_block() {
        let stopped = bsi_end(&crafted(2, false, false, |sink| {
            sink.put(1, 1) // mixmdate
                .put(0, 1) // pgmscle
                .put(0, 1) // extpgmscle
                .put(0, 2) // mixdef
                .put(1, 1) // frmmixcfginfoe
                .put(1, 1) // blkmixcfginfoe of block 0
                .put(3, 5) // blkmixcfginfo
                .put(1, 1) // blkmixcfginfoe of block 1
                .put(20, 5)
                .put(0, 1) // the four blocks behind them state nothing
                .put(0, 1)
                .put(0, 1)
                .put(0, 1)
                .put(0, 1); // infomdate
            sink.put(0, 1); // addbsie
        }));
        assert_eq!(stopped, 75);
    }

    /// The three shapes of Table E1.3 that reach into every block of a frame rather
    /// than sit in front of them: the adaptive hybrid transform, switching a block
    /// over to AC-3's short transform, and the attenuation data of a reconstructed
    /// high band. A stream that needs one is a missing decoder rather than a broken
    /// file, and the two cannot be told apart unless the refusal names the syntax.
    #[test]
    fn an_audio_frame_beyond_this_walk_is_refused_by_the_syntax_it_needs() {
        let shapes: [(&str, fn(&mut Written)); 3] = [
            ("hybrid", |written| written.ahte = true),
            ("short transform", |written| written.blkswe = true),
            ("high band", |written| written.spxattene = true),
        ];
        for (said, write) in shapes {
            let bytes = audio_frame(7, true, write);
            let error = header(&mut BitReader::new(&bytes)).unwrap_err();
            assert!(matches!(error, crate::Error::Unsupported(_)), "{error}");
            assert!(error.to_string().contains(said), "{error} does not name {said}");
        }
    }

    /// `expstre`: the frame hands every block a 2-bit strategy of its own instead of
    /// one 5-bit code for all of them, which is the form an encoder chooses when the
    /// blocks of a frame really do want different band structures.
    #[test]
    fn a_frame_that_states_a_strategy_per_block_is_read_that_way() {
        let read = walked(&audio_frame(2, false, |written| {
            written.couple = true;
            written.expstre = true;
        }));
        assert_eq!(
            read.exponents,
            Exponents::PerBlock {
                coupling: [0; BLOCKS],
                channels: [[0; CHANS]; BLOCKS],
            }
        );
        let per_frame = walked(&audio_frame(2, false, |written| written.couple = true));
        assert_eq!(
            read.audio_bit,
            per_frame.audio_bit + 6 * (2 + 2 * 2) - (5 + 5 * 2),
            "six blocks of two bits for the coupling channel and two apiece for the \
             others, in place of the frame-wide five-bit code per channel and its own"
        );
    }

    /// A single-channel frame states nothing at all for coupling - not even the flag
    /// that would say it does not - so what sits ahead of the audio turns on `acmod`,
    /// and not on the block count alone.
    #[test]
    fn a_frame_of_one_channel_states_nothing_for_coupling() {
        let read = walked(&audio_frame(1, false, |written| written.couple = true));
        assert_eq!(read.channels, 1);
        assert_eq!(read.coupling, [false; BLOCKS]);
        assert_eq!(
            read.exponents,
            Exponents::PerFrame {
                coupling: None,
                channels: [0; CHANS],
            }
        );
        assert_eq!(read.audio_bit, 87);
    }

    /// The frame-wide form of the coupling strategy, which is what both fixtures
    /// write: one 5-bit code for the whole frame, with every block inheriting both
    /// that code and the decision to couple.
    #[test]
    fn a_frame_of_coupling_blocks_states_its_strategy_once() {
        let read = walked(&audio_frame(2, false, |written| written.couple = true));
        assert_eq!(read.coupling, [true; BLOCKS]);
        assert_eq!(
            read.exponents,
            Exponents::PerFrame {
                coupling: Some(0),
                channels: [0; CHANS],
            }
        );
        assert_eq!(read.audio_bit, 108, "which is where the encoder measured it");
    }

    /// A frame that does not couple states no coupling strategy at all, and the five
    /// bits it leaves out are five bits a walk that asked for them anyway would
    /// misread as the channels' own codes.
    #[test]
    fn a_frame_that_does_not_couple_states_nothing_for_it() {
        let read = walked(&audio_frame(2, false, |_| {}));
        assert_eq!(read.coupling, [false; BLOCKS]);
        assert_eq!(
            read.exponents,
            Exponents::PerFrame {
                coupling: None,
                channels: [0; CHANS],
            }
        );
        assert_eq!(read.audio_bit, 103);
    }

    /// The block starts are a frame's own index into itself: one entry per block after
    /// the first, each as wide as the frame has bits to address, and each naming the
    /// bit a block begins at rather than a distance to walk from the block before it.
    #[test]
    fn a_frame_that_states_where_its_blocks_begin_is_read_to_the_last_of_them() {
        let read = walked(&audio_frame(2, false, |written| {
            written.block_starts = true;
        }));
        assert_eq!(
            read.audio_bit,
            103 + 5 * 12,
            "the starts come after the bit that says they are there"
        );
        assert_eq!(read.block_starts, Some([400, 800, 1_200, 1_600, 2_000]));
    }

    /// A start that cannot frame a block is a broken file rather than a missing
    /// feature: a block that begins inside the header, or behind the block before it,
    /// or past the last bit of the frame that states it, leaves a decoder nowhere to
    /// put samples. No encoder on this machine writes these fields, so this is as much
    /// of the check as can be made without a file to make it against.
    #[test]
    fn block_starts_that_cannot_frame_a_block_are_refused() {
        for (starts, said) in [
            ([40, 800, 1_200, 1_600, 2_000], "before the one before it"),
            ([400, 300, 1_200, 1_600, 2_000], "before the one before it"),
            ([400, 800, 1_200, 1_600, 3_216], "past the end"),
        ] {
            let bytes = audio_frame(2, false, |written| {
                written.block_starts = true;
                written.starts = starts;
            });
            let error = header(&mut BitReader::new(&bytes)).unwrap_err();
            assert!(matches!(error, crate::Error::Invalid(_)), "{error}");
            assert!(error.to_string().contains(said), "{error} does not name {said}");
        }
    }

    /// `snroffststr` = 0 is the frame saying it carries the offsets its blocks mix by;
    /// the two it replaces that with leave each block to state its own, and there is
    /// nothing at the frame level left to read for them then.
    #[test]
    fn a_frame_carries_its_blocks_offsets_only_when_the_strategy_says_so() {
        let read = walked(&audio_frame(2, false, |written| {
            written.couple = true;
            written.snroffststr = 2;
        }));
        assert_eq!(read.snr_offsets, None);
        assert_eq!(read.blocks_state.snr_offsets, 2);
        assert_eq!(read.audio_bit, 98, "ten bits of offsets the frame does not write");

        let read = walked(&audio_frame(2, false, |written| written.couple = true));
        assert_eq!(read.snr_offsets, Some(SnrOffsets { coarse: 0, fast: 0 }));
        assert_eq!(read.blocks_state.snr_offsets, 0);
    }

    /// The fourth `snroffststr` is the one Table E2.9 leaves reserved, so a frame that
    /// names it promises offsets no block of it knows how to spell.
    #[test]
    fn a_frame_that_names_the_reserved_offset_strategy_is_refused_by_name() {
        let error = header(&mut BitReader::new(&audio_frame(2, false, |written| {
            written.couple = true;
            written.snroffststr = 3;
        })))
        .unwrap_err();
        assert!(error.to_string().contains("reserved"), "{error}");
    }

    /// The flags that say what a block may state for itself move nothing in the frame:
    /// they are read once here and obeyed by every block after them, which is why a
    /// frame full of them ends on the same bit as a quiet one.
    #[test]
    fn the_flags_a_block_reads_are_handed_to_it_by_its_frame() {
        let read = walked(&audio_frame(2, false, |written| {
            written.couple = true;
            written.dithflage = true;
            written.bamode = true;
            written.gain_codes = true;
            written.dbaflde = true;
            written.skipflde = true;
        }));
        assert_eq!(
            read.blocks_state,
            BlocksState {
                dither_flags: true,
                bit_allocation: true,
                gain_codes: true,
                dynamic_parameters: true,
                skip_fields: true,
                transient_processing: false,
                snr_offsets: 0,
            }
        );
        assert_eq!(read.audio_bit, 108, "the flags themselves cost nothing");
    }

    /// `transproce` says some channel of the frame carries a transient pre-noise
    /// processing window, and then every channel says whether it does. This reader
    /// applies no such processing, so the windows are stepped over - but stepped over
    /// at their right width, since the audio begins where the last of them ends.
    #[test]
    fn transient_processing_windows_are_stepped_over_not_kept() {
        let read = walked(&audio_frame(2, false, |written| {
            written.couple = true;
            written.transproce = true;
        }));
        assert!(read.blocks_state.transient_processing);
        assert_eq!(
            read.audio_bit,
            108 + 2 * 19,
            "a place and a length for each of the frame's two channels"
        );
    }

    /// A frame with a low frequency channel states one more thing per block, and those
    /// six bits are the difference between a decoder that folds the frame by five
    /// channels and one that folds it by six.
    #[test]
    fn a_frame_with_the_low_frequency_channel_states_its_strategy_for_every_block() {
        let read = walked(&audio_frame(7, true, |written| written.couple = true));
        assert_eq!(read.channels, 6);
        assert!(read.low_frequency);
        assert_eq!(read.low_frequency_strategies, [false; BLOCKS]);
        assert_eq!(read.converter_strategies, [0; CHANS]);
        // The flags, the block coupling, the frame's code and the channels' five
        // apiece, the low frequency channel's per block, and the converter's own.
        assert_eq!(read.audio_bit, 54 + 12 + 6 + 5 + 25 + 6 + 25 + 10 + 1);
    }

    /// One frame-wide code is six per-block ones, and most rows of Table E2.10 spend
    /// only the first block or two on strategies and let the rest reuse them: the
    /// stereo fixture codes exponents in blocks 0 and 1 and lets the four behind stand,
    /// both channels alike because the frame states one code for each and the same one.
    #[test]
    fn a_stereo_frame_codes_exponents_in_two_of_its_six_blocks() {
        let read = frame(STEREO).expect("the committed stereo stream");
        assert_eq!(read.full_bandwidth(), 2);
        let stated: Vec<_> = (0..read.blocks)
            .map(|block| read.strategies(block).expect("a block the frame has"))
            .collect();
        for block in &stated {
            assert_eq!(
                block.channels[..2],
                [block.channels[0]; 2],
                "the frame names one code per channel, and both are the same"
            );
            assert_eq!(
                block.channels[2..],
                [Strategy::Reuse; CHANS - 2],
                "the slots the frame never states stay padding"
            );
        }
        let resolved: Vec<_> = stated
            .iter()
            .map(|block| (block.coupling, block.channels[0], block.low_frequency))
            .collect();
        assert_eq!(
            resolved,
            vec![
                (Some(Strategy::D45), Strategy::D45, None),
                (Some(Strategy::D15), Strategy::D15, None),
                (Some(Strategy::Reuse), Strategy::Reuse, None),
                (Some(Strategy::Reuse), Strategy::Reuse, None),
                (Some(Strategy::Reuse), Strategy::Reuse, None),
                (Some(Strategy::Reuse), Strategy::Reuse, None),
            ],
            "row 16 of Table E2.10, which is the code every channel of this frame states"
        );
    }

    /// The 5.1 fixture states row 22 for its coupling channel and all five of its
    /// channels, and Table 7.5 for the low frequency one, where a single flag of block 0
    /// is the difference between coding exponents and letting the previous block's stand.
    #[test]
    fn a_surround_frame_reuses_the_exponents_its_earlier_blocks_coded() {
        let read = frame(SURROUND).expect("the committed 5.1 stream");
        assert_eq!(read.full_bandwidth(), 5);
        let stated: Vec<_> = (0..read.blocks)
            .map(|block| read.strategies(block).expect("a block the frame has"))
            .collect();
        assert_eq!(
            stated
                .iter()
                .map(|block| block.coupling)
                .collect::<Vec<_>>(),
            vec![
                Some(Strategy::D45),
                Some(Strategy::D25),
                Some(Strategy::Reuse),
                Some(Strategy::D45),
                Some(Strategy::D25),
                Some(Strategy::Reuse),
            ],
            "row 22 of Table E2.10"
        );
        for block in &stated {
            assert_eq!(
                block.channels[..5],
                [block.channels[0]; 5],
                "every channel of the frame states the same code"
            );
        }
        assert_eq!(
            stated
                .iter()
                .map(|block| block.low_frequency)
                .collect::<Vec<_>>(),
            vec![
                Some(Strategy::D15),
                Some(Strategy::Reuse),
                Some(Strategy::Reuse),
                Some(Strategy::Reuse),
                Some(Strategy::Reuse),
                Some(Strategy::Reuse),
            ],
            "Table 7.5, read off the one flag of block 0 the walk recorded"
        );
    }

    /// A frame that hands each block its own 2-bit code needs no row of Table E2.10 to
    /// resolve: the codes of Table 7.4 are the strategies, and the low frequency
    /// channel's bit is already one of the two Table 7.5 offers it.
    #[test]
    fn a_frame_that_states_a_strategy_per_block_resolves_it_as_written() {
        let read = walked(&audio_frame(2, true, |written| {
            written.couple = true;
            written.expstre = true;
        }));
        assert_eq!(Strategy::per_block(0), Strategy::Reuse);
        assert_eq!(Strategy::per_block(1), Strategy::D15);
        assert_eq!(Strategy::per_block(2), Strategy::D25);
        assert_eq!(Strategy::per_block(3), Strategy::D45);
        assert_eq!(Strategy::low_frequency(true), Strategy::D15);
        let stated = read.strategies(0).expect("the frame's first block");
        assert_eq!(stated.coupling, Some(Strategy::Reuse));
        assert_eq!(stated.channels[..2], [Strategy::Reuse; 2]);
        assert_eq!(stated.low_frequency, Some(Strategy::Reuse));
        assert_eq!(read.strategies(read.blocks), None, "past its last block");
    }

    /// A block that does not couple codes no coupling channel, so a walk that resolved
    /// the frame's coupling code for it anyway would hand a decoder exponents to read
    /// out of a channel the block never folded anything into.
    #[test]
    fn a_block_that_does_not_couple_resolves_nothing_for_coupling() {
        let read = walked(&audio_frame(2, false, |written| written.expstre = true));
        assert_eq!(read.coupling, [false; BLOCKS]);
        for block in 0..read.blocks {
            assert_eq!(
                read.strategies(block).expect("a block the frame has").coupling,
                None
            );
        }
    }

    /// The standard's own promise about Table E2.10: every row names a strategy for
    /// block 0, because exponent information never crosses a syncframe boundary. A row
    /// that began with reuse would leave a block decoder with nothing to reuse, and it
    /// is the table that says so, not the encoder.
    #[test]
    fn no_frame_code_leaves_its_first_block_without_a_strategy() {
        for (code, row) in FRAME_STRATEGIES.iter().enumerate() {
            assert_ne!(
                row[0],
                Strategy::Reuse,
                "code {code} would leave the frame's first block with nothing to reuse"
            );
        }
    }
}


fn say(name: &str, blk: usize, pos: usize) {
    if std::env::var("EAC3_TRACE").is_ok() {
        eprintln!("  blk {blk} {pos:6} {name}");
    }
}

#[cfg(test)]
mod walker_trace {
    use super::*;

    /// Walk one file named by `FVID_EAC3`, printing the position every block's sections
    /// end at alongside the frame's own geometry, so an independent walk of the same
    /// tables can be diffed against it field for field.
    #[test]
    fn trace_one_file() {
        let Ok(path) = std::env::var("FVID_EAC3") else {
            return;
        };
        let data = std::fs::read(&path).expect("readable");
        let frame = {
            let mut reader = BitReader::new(&data);
            header(&mut reader).expect("header")
        };
        eprintln!(
            "== {path}: {} bytes, {} Hz, {} ch",
            data.len(),
            frame.sample_rate,
            frame.channels
        );
        let mut decoder =
            Eac3Decoder::new(&[], frame.sample_rate, frame.channels).expect("opens");
        let raw = decoder
            .decode_encoded(&data, 0, 0)
            .expect("decodes")
            .expect("audio");
        let floats: Vec<f32> = raw
            .data
            .chunks_exact(4)
            .map(|w| f32::from_le_bytes(w.try_into().unwrap()))
            .collect();
        let mut squared = 0.0f64;
        let mut peak = 0.0f32;
        for x in &floats {
            squared += f64::from(*x) * f64::from(*x);
            if x.abs() > peak {
                peak = x.abs();
            }
        }
        eprintln!(
            "   {} words, rms {:.6}, peak {peak:.6}, nonzero {}",
            floats.len(),
            (squared / floats.len().max(1) as f64).sqrt() as f32,
            floats.iter().filter(|x| **x != 0.0).count(),
        );
    }
}
