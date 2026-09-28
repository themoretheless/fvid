//! Audio playback of uncompressed PCM out of the wrapper that states nothing of
//! its own.
//!
//! A Wave file carries the geometry the player asks a container for - width, byte
//! order, rate, channel count - and a run of samples in exactly the form
//! [`crate::codec::pcm_decoder`] consumes. So there is no codec setup to hand
//! over and nothing to render: the reader's whole job is to cut the sample run
//! into the windows the decode thread paces itself by, and to say which tag those
//! windows belong to.
//!
//! The same envelope can also state one of G.711's two companding tables, and then
//! the run is one byte per sample rather than a width times a byte order. Nothing
//! about the windowing changes: the table is the decoder's, the geometry is the
//! header's, and the two derived fields check out against either.
//!
//! G.722 states a third shape: four bits a sample, so a whole frame only lands in a
//! block when two of them share a byte, and the run therefore carries twice as many
//! frames as its blocks. Its own `dwAvgBytesPerSec` is the one field a reader cannot
//! take as a statement of geometry, which is measured below; `fact`, which the same
//! writer gets right, is checked in its place.
//!
//! The header is read strictly, and every field of it is checked against the
//! others: `wBlockAlign` against the width and the channel count, `dwAvgBytesPerSec`
//! against the block and the rate. A Wave file is a handful of redundant ways of
//! stating one geometry, and the redundancy is the only way a reader has of telling
//! a header from a mistake.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::codec::pcm_decoder::PcmFormat;
use crate::{Result, invalid, unsupported};
use std::io::Read;
use std::time::Duration;

/// What a limit guards: how much the reader may take in, and how wide a track it
/// may agree to lay out.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest file accepted.
    pub file_bytes: usize,
    /// Largest sample run accepted, whether or not the header declared it.
    pub data_bytes: usize,
    /// Most channels one track may name.
    pub channels: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // A minute of 192 kHz 32-bit stereo is a little under 55 MB.
            file_bytes: 128 << 20,
            data_bytes: 128 << 20,
            channels: 32,
        }
    }
}

/// Frames per packet. The format says nothing about how often to be called, so
/// this is the player's own grain: at 44.1 kHz a window is 46 ms, and at 8 kHz one
/// second of audio still comes in 39 of them.
pub const PACKET_FRAMES: usize = 2_048;

const PCM: u16 = 0x0001;
const IEEE_FLOAT: u16 = 0x0003;
/// ITU-T G.711's two companding tables, as the Wave registry numbers them.
const ALAW: u16 = 0x0006;
const MULAW: u16 = 0x0007;
/// ITU-T G.722's ADPCM, which the registry numbers rather than fourccs.
const G722_ADPCM: u16 = 0x028F;
/// ITU-T G.726's ADPCM, at whichever of its four rates the track's byte rate says it
/// runs at.
const G726_ADPCM: u16 = 0x0045;
/// GSM 06.10 as Microsoft ships it: registry number 49, one channel, and a rate stated
/// by the block's width rather than by any depth field.
const GSM_FULL_RATE: u16 = 0x0031;
/// Creative Technology's ADPCM, registry number 512: four-bit codes, two of them to a
/// byte, and no block for the track to state a width in.
const ADPCM_CT: u16 = 0x0200;
/// DSP Group TrueSpeech, registry number 34: a whole 32-byte block per 240 samples, which
/// the format states in no field of its own - the depth field holds a 1.
const TRUE_SPEECH: u16 = 0x0022;
const EXTENSIBLE: u16 = 0xFFFE;

/// How the sample bytes are to be read. A Wave file states this as a number, and
/// only two of the numbers it can state describe something wider than one byte per
/// sample: the rest of the registry is either uncompressed PCM of some width or one
/// of G.711's two tables, both of which are a byte wide. G.722, G.726, GSM, Creative
/// ADPCM, TrueSpeech and MACE's two codings all code several frames per unit of their run
/// - a code narrower than a byte for the first four, a whole 32-byte block for the fifth,
/// and a block of one or two bytes a channel for the last - which is why they are the
/// codings this enum has to speak of separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coding {
    Pcm(PcmFormat),
    ALaw,
    MuLaw,
    G722,
    /// G.726, with the width of its code in bits - the one thing a G.726 track states in
    /// its byte rate rather than anywhere a reader would look for a bit depth.
    G726(u8),
    /// GSM 06.10 at its full rate, which is the only rate the reader lets through: a
    /// track states a lower one by shortening its block, and the codes that pay for the
    /// difference are not decoded here.
    Gsm,
    /// Creative Technology's ADPCM: four bits a code, two codes a byte, and a run with no
    /// block in it at all - which is why this coding is the one whose header fields the
    /// reader reads the least of.
    AdpcmCt,
    /// DSP Group TrueSpeech: a 32-byte block that codes 240 samples and states neither its
    /// rate nor its depth anywhere else.
    Truespeech,
    /// Apple MACE 3-to-1: a byte's three codes give one sample each, so a channel's block
    /// is two bytes for six samples. It reaches this enum from a container's fourcc, not
    /// from a Wave number - this build's muxer refuses to write the coding into a Wave.
    Mace3,
    /// Apple MACE 6-to-1: the same three codes interpolated into two samples each, so a
    /// channel's block is one byte and still codes six samples.
    Mace6,
}

/// A TrueSpeech block: 256 bits of filter, offsets and pulses, which the reconstruction
/// turns into four subframes of 60 samples. The width is the coding's own and not the
/// header's to state - measured, this build's reference decoded the same run from a track
/// stating an alignment of 16 and of 64 alike.
const TRUE_SPEECH_BLOCK_BYTES: usize = 32;
const TRUE_SPEECH_SAMPLES_PER_BLOCK: usize = 240;

/// Samples one MACE block codes, either coding: 3-to-1 spends two bytes per channel on
/// them and 6-to-1 one, because the second interpolates each of a byte's three codes into
/// a pair. Both widths are the coding's own, which is why they are not read from a header:
/// this build's reference derives them from the fourcc, over a sample-width field that
/// states 8 for both codings and, measured, for neither.
const MACE_SAMPLES_PER_BLOCK: usize = 6;

/// GSM 06.10's block: two frames of 260 bits, which is 65 bytes with no padding, coding
/// 320 samples at the format's own 20 ms a frame. The narrower blocks the format also
/// states - 62, 59, down to 41 bytes - trade RPE bits for rate and are refused by name.
const GSM_BLOCK_BYTES: usize = 65;
const GSM_SAMPLES_PER_BLOCK: usize = 320;

/// Bytes one G.726 channel's block takes, and the samples it codes. Measured on this
/// build's muxer over all four rates: where a code's width does not divide a byte - the
/// three- and five-bit rates - a block is three or five bytes wide and holds eight
/// samples, so the widest blocks the format states are still the smallest whole ones.
const fn g726_block(code_size: u8) -> (usize, usize) {
    match code_size {
        2 => (1, 4),
        3 => (3, 8),
        4 => (1, 2),
        _ => (5, 8),
    }
}

/// A file's sample geometry, in the words the player's decoders speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub coding: Coding,
    pub sample_rate: u32,
    pub channels: u16,
    /// Bits as the header declares them, which is what the decoder is built with.
    pub bits_per_sample: u16,
}

impl Pcm {
    /// Bytes of one block: every channel's contribution to the run, packed with no
    /// padding. Only a coding narrower than a byte can state a width that does not
    /// divide into this product, which is what makes a G.722 block a byte per channel
    /// holding two samples rather than one, and a G.726 one at the 24 and 40 kbit/s
    /// rates several bytes wide.
    pub fn block_align(&self) -> usize {
        match self.coding {
            Coding::G722 => usize::from(self.channels),
            Coding::G726(code_size) => usize::from(self.channels) * g726_block(code_size).0,
            Coding::Gsm => GSM_BLOCK_BYTES,
            Coding::AdpcmCt => usize::from(self.channels),
            Coding::Truespeech => TRUE_SPEECH_BLOCK_BYTES,
            // Two bytes of 3-to-1 or one of 6-to-1 per channel, both coding six samples.
            Coding::Mace3 => 2 * usize::from(self.channels),
            Coding::Mace6 => usize::from(self.channels),
            _ => usize::from(self.channels) * usize::from(self.bits_per_sample) / 8,
        }
    }

    /// Frames of sound one block codes. For everything that stores a whole sample per
    /// channel that is one; G.722's code is four bits, so two of them share a byte,
    /// G.726 packs from two to eight frames into its block depending on its rate, a
    /// GSM block is two frames of the phone network's 20 ms each, a Creative byte is
    /// simply its two codes, and either MACE block codes six - which is why 3-to-1's two
    /// bytes hold three samples apiece while 6-to-1's one byte interpolates to six.
    pub fn frames_per_block(&self) -> usize {
        match self.coding {
            Coding::G722 => 2,
            Coding::G726(code_size) => g726_block(code_size).1,
            Coding::Gsm => GSM_SAMPLES_PER_BLOCK,
            Coding::AdpcmCt => 2,
            Coding::Truespeech => TRUE_SPEECH_SAMPLES_PER_BLOCK,
            Coding::Mace3 | Coding::Mace6 => MACE_SAMPLES_PER_BLOCK,
            _ => 1,
        }
    }

    /// Bytes the coded run takes per second, which is the geometry's own statement of
    /// how fast the stream goes.
    pub fn coded_bytes_per_second(&self) -> usize {
        self.block_align() * self.sample_rate as usize / self.frames_per_block()
    }

    /// The byte rate a header states truthfully for this coding, or none where the
    /// field is known to carry something else. Measured on G.722: the Wave muxers
    /// write 16000 in that field whatever the track's rate, over runs that really go
    /// at 4000, 8000 and 22050 bytes a second, so believing it would refuse every file
    /// the format's own writers produce. G.726 is the other way round: its rate is not
    /// stated anywhere else, and the reference takes it from this field alone - a header
    /// whose byte rate does not come out at two to five bits a code decodes as nothing -
    /// so the field is read here and then held to the geometry it has to agree with.
    /// Creative ADPCM is the third case again: nothing in its header frames its run, so
    /// the reference reads none of it - measured over the shipped take, the same bytes
    /// give the same 524 192 samples with `wBlockAlign` 1, 258 and 516 and with
    /// `dwAvgBytesPerSec` stating double what the run costs, because the only thing that
    /// decides a Creative run's length is how many bytes it has. Believing the field would
    /// refuse every file a Creative writer produces. TrueSpeech is the fourth case: its
    /// block is a whole 32 bytes coding 240 samples, so the honest value of the field is
    /// 32 × 8 000 ÷ 240 = 1 066.6̄ - not a whole number of bytes, and so not a claim a
    /// reader could hold anyone to. Measured over the shipped take: the same 692 160 output
    /// bytes came out with `dwAvgBytesPerSec` stating 1 066 and 8 000 alike.
    pub fn derived_byte_rate(&self) -> Option<usize> {
        match self.coding {
            Coding::G722 | Coding::AdpcmCt | Coding::Truespeech => None,
            _ => Some(self.coded_bytes_per_second()),
        }
    }

    /// The tag [`crate::codec::make_audio_decoder`] matches on. A Wave file is the
    /// third container naming these streams - after QuickTime's fourcc and
    /// Matroska's codec ID - and it borrows Matroska's names for the uncompressed
    /// ones, which state the byte order the header picked. The two G.711 tables are
    /// named as this build's reference names them instead, because Matroska has no
    /// separate ID for them: it files both under `A_MS/ACM` and puts the Wave number
    /// in the setup block.
    pub fn codec(&self) -> &'static str {
        match self.coding {
            Coding::Pcm(PcmFormat::Float { .. }) => "A_PCM/FLOAT/IEEE",
            Coding::Pcm(PcmFormat::Int {
                big_endian: false, ..
            }) => "A_PCM/INT/LIT",
            Coding::Pcm(PcmFormat::Int {
                big_endian: true, ..
            }) => "A_PCM/INT/BIG",
            Coding::ALaw => "pcm_alaw",
            Coding::MuLaw => "pcm_mulaw",
            Coding::G722 => "adpcm_g722",
            Coding::G726(_) => "adpcm_g726",
            Coding::Gsm => "gsm_ms",
            Coding::AdpcmCt => "adpcm_ct",
            Coding::Truespeech => "truespeech",
            Coding::Mace3 => "mace3",
            Coding::Mace6 => "mace6",
        }
    }
}

/// A Wave file: its geometry and the bytes of its sample run.
#[derive(Clone, Debug)]
pub struct Wav {
    pub pcm: Pcm,
    /// What the `data` chunk's own length field claims, which may be more than the
    /// file carries.
    pub declared_bytes: usize,
    /// What a `fact` chunk states about the run, when the file has one.
    pub declared_frames: Option<usize>,
    /// Frames the reader will hand over, a short tail included.
    pub frames: usize,
    data: Vec<u8>,
}

impl Wav {
    /// Read a whole file: the `RIFF`/`WAVE` envelope, the `fmt ` chunk that states
    /// the geometry, and the `data` chunk that follows it.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(invalid("not a RIFF/WAVE file"));
        }
        let mut fmt = None;
        let mut data = None;
        let mut fact = None;
        // The walk stops at the end of the buffer rather than at the envelope's
        // declared size: a writer that got that size wrong still has readable chunks
        // in it, and the only claim a reader can check against samples is the one the
        // header's redundant fields make against each other.
        let mut at = 12usize;
        while at + 8 <= bytes.len() {
            let tag = &bytes[at..at + 4];
            let declared =
                u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap_or([0xff; 4]));
            let declared = usize::try_from(declared).unwrap_or(usize::MAX);
            let body = at + 8;
            // The claim is the writer's, the bytes are the reader's: a chunk that
            // promises more than the file holds is taken as far as the file goes, and
            // a chunk that ends early leaves whatever follows it to be read as the
            // next chunk rather than played as audio.
            let end = body.saturating_add(declared).min(bytes.len());
            // Chunks are word aligned, so a writer adds a byte after an odd-length
            // one, and the next chunk starts past it.
            at = body.saturating_add(declared.saturating_add(declared & 1));
            let first_unseen = match tag {
                b"fmt " => fmt.is_none(),
                b"data" => data.is_none(),
                // `fact` states the run's frame count, which is one of the header's
                // redundant ways of saying how long the track is, so it is kept for the
                // codings whose own byte rate says nothing - see `parse`. `LIST` and
                // whatever else a writer adds are skipped: none of them can change how
                // the samples are laid out.
                b"fact" => fact.is_none(),
                _ => false,
            };
            if first_unseen {
                if tag == b"fmt " {
                    fmt = Some(parse_fmt(&bytes[body..end], limits)?);
                } else if tag == b"fact" {
                    // A `fact` chunk that stops before its one field states nothing,
                    // which is not the same as a statement the rest of the file
                    // contradicts.
                    fact = bytes.get(body..end).and_then(|chunk| {
                        let stated = u32::from_le_bytes(chunk.get(..4)?.try_into().ok()?);
                        Some(usize::try_from(stated).unwrap_or(usize::MAX))
                    });
                } else {
                    data = Some((body, end, declared));
                }
            }
        }
        let pcm = fmt.ok_or_else(|| invalid("a Wave file with no fmt chunk"))?;
        let (start, end, declared_bytes) =
            data.ok_or_else(|| invalid("a Wave file with no data chunk"))?;
        let samples = &bytes[start..end];
        if samples.is_empty() {
            return Err(invalid("a Wave file with no samples in it"));
        }
        if samples.len() > limits.data_bytes {
            return Err(invalid(&format!(
                "sample run is over the {} byte limit",
                limits.data_bytes
            )));
        }
        let blocks = samples.len() / pcm.block_align();
        let frames = blocks * pcm.frames_per_block();
        // Where the byte rate states nothing, the `fact` chunk is the run's remaining
        // redundant statement, and the one the same writers get right. A run shorter
        // than the file claims has no claim to check against: the reader plays the
        // bytes it has, as it does for every coding. TrueSpeech is left out of the check
        // because its own writer contradicts itself: measured over the shipped take, its
        // `fact` states 345 972 frames where its 1 442 whole blocks code 346 080, and the
        // reference plays all of them - the field counts something other than the samples
        // this coding decodes. The reader still carries the claim, so the disagreement
        // stays visible as `declared_frames` beside `frames`.
        if pcm.derived_byte_rate().is_none()
            && pcm.coding != Coding::Truespeech
            && declared_bytes <= samples.len()
            && let Some(stated) = fact
            && stated != frames
        {
            return Err(invalid(&format!(
                "the fact chunk states {stated} frames where the run codes {frames}"
            )));
        }
        Ok(Self {
            pcm,
            declared_bytes,
            declared_frames: fact,
            frames,
            data: samples.to_vec(),
        })
    }

    /// Packets the sample run is read out as, the last a short one when the run does
    /// not divide evenly.
    pub fn packets(&self) -> usize {
        self.data.len().div_ceil(self.window_bytes())
    }

    /// One packet's bytes, aligned to a block. A tail shorter than a block holds no
    /// complete frame - not one sample for every channel, and for a coding that packs
    /// two frames into a byte neither of them - and is not handed over at all.
    pub fn packet(&self, index: usize) -> &[u8] {
        let width = self.window_bytes();
        let start = index.min(self.packets().saturating_sub(1)) * width;
        let end = start.saturating_add(width).min(self.data.len());
        let whole = (end - start) / self.pcm.block_align() * self.pcm.block_align();
        &self.data[start..start + whole]
    }

    /// Bytes of one window: the player's own grain of frames, counted in the blocks
    /// this coding packs those frames into.
    fn window_bytes(&self) -> usize {
        PACKET_FRAMES / self.pcm.frames_per_block() * self.pcm.block_align()
    }
}

fn parse_fmt(body: &[u8], limits: &Limits) -> Result<Pcm> {
    if body.len() < 16 {
        return Err(invalid("fmt chunk is shorter than its seven fields"));
    }
    let u16_at = |at: usize| u16::from_le_bytes(body[at..at + 2].try_into().expect("two bytes"));
    let u32_at = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().expect("four bytes"));
    let mut format = u16_at(0);
    let channels = u16_at(2);
    let sample_rate = u32_at(4);
    let byte_rate = u32_at(8);
    let block_align = u16_at(12);
    let mut bits = u16_at(14);
    if format == EXTENSIBLE {
        // WAVE_FORMAT_EXTENSIBLE repeats the geometry above and adds a GUID saying
        // what the samples actually are; the low half of the GUID's first word is
        // the plain format number.
        if body.len() < 40 || u16_at(16) < 22 {
            return Err(invalid("extensible fmt chunk has no sub-format to read"));
        }
        let guid = &body[24..40];
        let data_format = u32::from_le_bytes(guid[..4].try_into().expect("four bytes"));
        if &guid[4..]
            != &[
                0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
            ]
        {
            return Err(unsupported(&format!(
                "Wave sub-format {data_format:#06x} is outside the RIFF PCM GUIDs"
            )));
        }
        if data_format > u32::from(u16::MAX) {
            return Err(unsupported(&format!(
                "Wave sub-format {data_format} is beyond any Wave format"
            )));
        }
        format = data_format as u16;
        // The width the samples are held in, which is what the decoder reads, while
        // the field above is the width they are stored in.
        bits = u16_at(18);
    }
    if channels == 0 || channels as usize > limits.channels {
        return Err(invalid("channel count is out of range"));
    }
    if sample_rate == 0 {
        return Err(invalid("sample rate of zero"));
    }
    let kind = match (format, bits) {
        // RIFF's 8-bit PCM is offset by half its scale, while the player's decoder
        // reads an 8-bit integer tag as signed - the width QuickTime's fourcc means.
        // Re-shifting those bytes is the decoder's to offer, not something a header
        // reader should do quietly, so this stays refused.
        (PCM, 16 | 24 | 32) => Coding::Pcm(PcmFormat::Int {
            bits: bits as u8,
            big_endian: false,
        }),
        (IEEE_FLOAT, 32 | 64) => Coding::Pcm(PcmFormat::Float { bits: bits as u8 }),
        (PCM | IEEE_FLOAT, _) => {
            return Err(unsupported(&format!(
                "Wave PCM of {bits} bits a sample is a width this player has no decoder for"
            )));
        }
        // Both tables code a sample as one byte, so a track that states another width
        // for one of them has said something the number cannot mean.
        (ALAW, 8) => Coding::ALaw,
        (MULAW, 8) => Coding::MuLaw,
        (ALAW | MULAW, bits) => {
            return Err(invalid(&format!(
                "a G.711 table codes one byte per sample, this track states {bits}"
            )));
        }
        // G.722's code is four bits, so a track stating any other width for it has
        // said something the format number cannot mean. The block is still a byte wide
        // per channel, which is the smallest run holding a whole frame.
        (G722_ADPCM, 4) => Coding::G722,
        (G722_ADPCM, bits) => {
            return Err(invalid(&format!(
                "G.722 codes four bits per sample, this track states {bits}"
            )));
        }
        // G.726 states its code width nowhere a reader would look for a bit depth: the
        // reference takes it from the byte rate alone, over runs whose width field says
        // anything at all - measured, this build's decoder read four-bit codes from a
        // header stating 32, 64 and no bits at all alike. So the rate is what is asked
        // here, and only a rate that comes out at one of the four widths whole is
        // accepted: the reference rounds a rate between two of them down to the nearer
        // and decodes, which is a guess about a header that has contradicted itself.
        (G726_ADPCM, _) => {
            // Per channel, since the rate is the whole track's; and divided exactly,
            // because a rate that lands between two of the four widths has said something
            // the coding cannot mean - see the note above.
            let bits_per_second = u64::from(byte_rate) * 8;
            let frame_rate = u64::from(sample_rate) * u64::from(channels);
            match bits_per_second.checked_div(frame_rate) {
                Some(code_size)
                    if bits_per_second % frame_rate == 0 && (2..=5u64).contains(&code_size) =>
                {
                    Coding::G726(code_size as u8)
                }
                _ => {
                    return Err(invalid(&format!(
                        "G.726 codes two to five bits, {byte_rate} bytes a second at \
                         {sample_rate} Hz over {channels} channels is neither"
                    )));
                }
            }
        }
        // GSM 06.10 states its rate in its block alignment and nowhere else: measured on
        // this build's reference, the same 89 blocks read out the same 28 480 samples at
        // `wBitsPerSample` 0, 8, 16 and 64, and a shortened alignment is taken as one of
        // the trimmed rates the coding carries rather than as a contradiction - 62 bytes
        // gives 29 760 samples, 41 gives 45 120, and 64, which is no alignment the format
        // can mean, is the one value refused outright.
        (GSM_FULL_RATE, _) if channels != 1 => {
            return Err(invalid(&format!(
                "GSM 06.10 codes one channel, this track names {channels}"
            )));
        }
        (GSM_FULL_RATE, _) if block_align as usize == GSM_BLOCK_BYTES => Coding::Gsm,
        (GSM_FULL_RATE, _) => {
            if !(41..=65u16).contains(&block_align) || (block_align - 41) % 3 != 0 {
                return Err(invalid(&format!(
                    "a GSM block is 41 to 65 bytes wide in three-byte steps, this track states {block_align}"
                )));
            }
            return Err(unsupported(&format!(
                "GSM at a {block_align}-byte block takes its codes off the rate this decoder reads"
            )));
        }
        // Creative ADPCM codes four bits a sample and packs two codes to a byte, with no
        // block to state anything else in. Measured over this build's reference and the
        // shipped take: the same bytes give the same samples whatever `wBlockAlign`,
        // `dwAvgBytesPerSec` and `fact` say, and the only field with an effect on the run
        // besides the format number is the channel count - which has one because stereo
        // hands its alternate nibbles to the other channel's predictor. So the width is
        // asked and the channels are, and the rest of the header is left to itself.
        (ADPCM_CT, _) if channels != 1 => {
            return Err(invalid(&format!(
                "Creative ADPCM reads one channel as a flat nibble run, this track names {channels}"
            )));
        }
        (ADPCM_CT, 4) => Coding::AdpcmCt,
        (ADPCM_CT, bits) => {
            return Err(invalid(&format!(
                "Creative ADPCM codes four bits per sample, this track states {bits}"
            )));
        }
        // TrueSpeech codes one channel: the reference's decoder is handed a mono run and
        // its block carries no channel interleaving at all. Measured over the shipped take,
        // a track naming two channels makes that decoder fail outright, so the count is
        // asked rather than ignored. Everything else in the header is left to itself: the
        // same 692 160 output bytes came out with `wBlockAlign` 16 and 64, with
        // `dwAvgBytesPerSec` 1 066 and 8 000, and with `wBitsPerSample` 0, 1 and 16 - so
        // the format number alone carries the geometry, which is why the depth is not asked.
        (TRUE_SPEECH, _) if channels != 1 => {
            return Err(invalid(&format!(
                "TrueSpeech codes one channel, this track names {channels}"
            )));
        }
        (TRUE_SPEECH, _) => Coding::Truespeech,
        (other, _) => {
            return Err(unsupported(&format!(
                "Wave format {other:#06x} is neither uncompressed PCM, a G.711 table, G.722, \
                 G.726, GSM 06.10, Creative ADPCM nor TrueSpeech"
            )));
        }
    };
    // A coding narrower than a byte states its width in the rate rather than in the
    // field named for it, and that is the width the decoder is built to read.
    let width = match kind {
        Coding::G726(code_size) => u16::from(code_size),
        _ => bits,
    };
    let pcm = Pcm {
        coding: kind,
        sample_rate,
        channels,
        bits_per_sample: width,
    };
    // Every coding but Creative's frames its run in whole blocks, so the alignment the
    // header states has to be the one the coding implies. A Creative track states a block
    // its coding does not have, and the field is unread by the reference: 1, 258 and 516
    // all read out the same run here, because the run is bytes and not blocks. TrueSpeech
    // does have a block, and so this check is held over it: a track stating an alignment
    // other than 32 would have its run cut into the wrong units, and measured this build's
    // reference only reads a 32-byte alignment - the 16 it tolerates is the truncation of
    // the same run to half its blocks, which it reports as an error.
    if pcm.coding != Coding::AdpcmCt && block_align as usize != pcm.block_align() {
        return Err(invalid(
            "block alignment does not match the width and channel count",
        ));
    }
    if let Some(rate) = pcm.derived_byte_rate() {
        if byte_rate as usize != rate {
            return Err(invalid("byte rate does not match the geometry"));
        }
    }
    Ok(pcm)
}

/// A Wave file read as an audio track.
pub struct WavAudioReader {
    wav: Wav,
    packet: usize,
}

impl WavAudioReader {
    /// Read a whole file. The sample run is one contiguous block and the packets are
    /// slices of it, so there is nothing here to produce incrementally.
    pub fn open<R: Read>(mut reader: R, limits: Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        Ok(Self {
            wav: Wav::parse(&bytes, &limits)?,
            packet: 0,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn wav(&self) -> &Wav {
        &self.wav
    }
}

impl AudioStream for WavAudioReader {
    fn codec(&self) -> &str {
        self.wav.pcm.codec()
    }

    fn timescale(&self) -> u32 {
        self.wav.pcm.sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.wav.pcm.sample_rate
    }

    fn channels(&self) -> u16 {
        self.wav.pcm.channels
    }

    fn bits_per_sample(&self) -> u16 {
        self.wav.pcm.bits_per_sample
    }

    /// How long the run plays, from the frames the reader hands over rather than from the
    /// bytes it holds: a TrueSpeech block costs 1 066.6̄ bytes a second, so the byte
    /// division truncates away a third of a frame's worth of rate and states a run longer
    /// than the samples in it. Every other coding's rate is a whole number of bytes, so
    /// this is the same duration those files' headers claim.
    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.wav.frames as f64 / f64::from(self.wav.pcm.sample_rate),
        ))
    }

    /// PCM needs no setup: the header's fields have already gone into the decoder's
    /// construction.
    fn extra_data(&self) -> &[u8] {
        &[]
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: self.wav.pcm.sample_rate,
            channels: self.wav.pcm.channels,
            name: String::new(),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        if self.packet >= self.wav.packets() {
            return Ok(None);
        }
        let data = self.wav.packet(self.packet).to_vec();
        let packet = EncodedPacket {
            duration: ((data.len() / self.wav.pcm.block_align()) * self.wav.pcm.frames_per_block())
                as i64,
            pts: (self.packet * PACKET_FRAMES) as i64,
            data,
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let last = self.wav.packets().saturating_sub(1);
        self.packet = (pts.max(0) as usize / PACKET_FRAMES).min(last);
        (self.packet * PACKET_FRAMES) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ADPCM_CT, ALAW, Coding, EXTENSIBLE, G722_ADPCM, G726_ADPCM, GSM_BLOCK_BYTES, GSM_FULL_RATE,
        IEEE_FLOAT, Limits, MULAW, PACKET_FRAMES, PCM, PcmFormat, Wav, WavAudioReader,
    };
    use crate::audio::{AudioDecode, AudioStream};
    use crate::codec::pcm_decoder::PcmDecoder;
    use std::time::Duration;

    /// A `fmt ` chunk's body for the geometry its caller names, with the two
    /// derived fields computed the way a writer computes them.
    fn fmt(format: u16, channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
        let block_align = channels * bits / 8;
        let mut out = Vec::new();
        out.extend_from_slice(&format.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(u32::from(block_align) * sample_rate).to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out
    }

    /// A Wave envelope around a list of chunks, in the order given.
    fn wave(parts: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (tag, chunk) in parts {
            body.extend_from_slice(tag);
            body.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            body.extend_from_slice(chunk);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(4 + body.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(&body);
        out
    }

    /// A whole file from a `fmt ` body and a sample run.
    fn file(fmt_body: &[u8], data: &[u8]) -> Vec<u8> {
        wave(&[(b"fmt ", fmt_body), (b"data", data)])
    }

    /// A G.722 `fmt ` with both of its derived fields stated rather than computed: at
    /// four bits a sample the product the helper above forms is zero, and the writers
    /// measured here put the coding's byte rate in the header, not the track's.
    fn g722_file(
        channels: u16,
        sample_rate: u32,
        bits: u16,
        block: u16,
        byte_rate: u32,
        fact: Option<u32>,
        data: &[u8],
    ) -> Vec<u8> {
        let mut fmt_body = Vec::new();
        fmt_body.extend_from_slice(&G722_ADPCM.to_le_bytes());
        fmt_body.extend_from_slice(&channels.to_le_bytes());
        fmt_body.extend_from_slice(&sample_rate.to_le_bytes());
        fmt_body.extend_from_slice(&byte_rate.to_le_bytes());
        fmt_body.extend_from_slice(&block.to_le_bytes());
        fmt_body.extend_from_slice(&bits.to_le_bytes());
        let mut parts: Vec<(&[u8], &[u8])> = vec![(b"fmt ", fmt_body.as_slice())];
        let stated = fact.map(|frames| frames.to_le_bytes());
        if let Some(body) = &stated {
            parts.push((b"fact", body.as_slice()));
        }
        parts.push((b"data", data));
        wave(&parts)
    }

    /// State a `data` length other than the run's, the way a writer that was interrupted
    /// or appended to does.
    fn claim_more(bytes: &mut [u8], len: u32) {
        let at = bytes
            .windows(4)
            .position(|w| w == b"data")
            .expect("a data chunk");
        bytes[at + 4..at + 8].copy_from_slice(&len.to_le_bytes());
    }

    fn samples(frames: &[i16]) -> Vec<u8> {
        frames.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn wav(frames: &[i16]) -> Vec<u8> {
        file(&fmt(PCM, 1, 8_000, 16), &samples(frames))
    }

    fn open(bytes: &[u8]) -> WavAudioReader {
        WavAudioReader::open(bytes, Limits::default()).expect("opens")
    }

    #[test]
    fn the_header_states_what_the_decoder_needs() {
        let reader = open(&wav(&[0, 1, 2, 3]));
        assert_eq!(reader.codec(), "A_PCM/INT/LIT");
        assert_eq!(reader.timescale(), 8_000);
        assert_eq!((reader.sample_rate(), reader.channels()), (8_000, 1));
        assert_eq!(reader.bits_per_sample(), 16);
        assert!(reader.extra_data().is_empty());
        assert_eq!(reader.audio_tracks()[0].label(), "1 ch 8000 Hz");
        // Four frames at 8 kHz is half a millisecond.
        assert_eq!(
            reader.duration(),
            Some(Duration::from_secs_f64(4.0 / 8_000.0))
        );
        let wav = reader.wav();
        assert_eq!(wav.pcm.block_align(), 2);
        assert_eq!((wav.frames, wav.declared_bytes), (4, 8));
    }

    #[test]
    fn packets_are_windows_of_frames_stamped_in_samples() {
        let frames = vec![7i16; PACKET_FRAMES * 2 + 5];
        let mut reader = open(&wav(&frames));
        let mut stamps = Vec::new();
        let mut sizes = Vec::new();
        while let Some(packet) = reader.next_packet().expect("packet") {
            stamps.push((packet.pts, packet.duration));
            sizes.push(packet.data.len());
        }
        assert_eq!(stamps, vec![(0, 2_048), (2_048, 2_048), (4_096, 5)]);
        assert_eq!(sizes, vec![4_096, 4_096, 10]);
        // The stamps add up to the run the header describes, so the track is as long
        // as its own timeline claims, and the timescale is the rate the packets are
        // counted in.
        assert_eq!(
            stamps.iter().map(|(_, d)| d).sum::<i64>() as usize,
            frames.len()
        );
        assert_eq!(
            reader.time_of(4_096 + 5),
            Duration::from_secs_f64(4_101.0 / 8_000.0)
        );
    }

    #[test]
    fn the_cursor_rewinds_and_lands_on_a_window() {
        let mut reader = open(&wav(&vec![0i16; PACKET_FRAMES * 3]));
        assert_eq!(reader.seek_to(2_500), 2_048);
        assert_eq!(
            reader.next_packet().expect("packet").expect("third").pts,
            2_048
        );
        // Before the start, and past the file's end, which is the last window the run
        // actually has.
        assert_eq!(reader.seek_to(-1), 0);
        assert_eq!(reader.seek_to(999_999), 4_096);
        assert!(reader.next_packet().expect("packet").is_some());
        assert!(reader.next_packet().expect("no more").is_none());
        reader.rewind();
        assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
    }

    #[test]
    fn the_samples_arrive_as_the_pcm_decoder_writes_them() {
        let mut reader = open(&wav(&[-32_768i16, -1, 0, 32_767]));
        let mut decoder = PcmDecoder::int(
            reader.bits_per_sample(),
            false,
            reader.sample_rate(),
            reader.channels(),
        )
        .expect("the header's own geometry");
        let packet = reader.next_packet().expect("packet").expect("one packet");
        let audio = decoder
            .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
            .expect("convert")
            .expect("PCM always yields a packet");
        assert_eq!((audio.timebase_num, audio.timebase_den), (1, 8_000));
        assert_eq!(
            audio
                .data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                .collect::<Vec<_>>(),
            vec![-1.0, -1.0 / 32_768.0, 0.0, 32_767.0 / 32_768.0]
        );
    }

    /// G.711 in the only form a Wave file can state it: a format number and eight bits
    /// a sample. The expected samples are not this file's own arithmetic - they are
    /// what this machine's FFmpeg returned for the same codes, and the whole 256-code
    /// run of each table was compared that way, every code agreeing.
    #[test]
    fn a_g_711_wave_names_its_table_and_codes_one_byte_per_sample() {
        let codes = [0x00u8, 0x01, 0x02, 0x03];
        for (format, coding, tag, expected) in [
            (
                ALAW,
                Coding::ALaw,
                "pcm_alaw",
                [-5_504i16, -5_248, -6_016, -5_760],
            ),
            (
                MULAW,
                Coding::MuLaw,
                "pcm_mulaw",
                [-32_124i16, -31_100, -30_076, -29_052],
            ),
        ] {
            let mut reader = open(&file(&fmt(format, 1, 8_000, 8), &codes));
            assert_eq!(reader.wav().pcm.coding, coding);
            assert_eq!(reader.codec(), tag);
            assert_eq!(reader.bits_per_sample(), 8);
            // One byte a sample at any rate, which is what makes these two tables
            // readable without the setup block a compressed codec would need.
            assert_eq!(reader.wav().pcm.block_align(), 1);
            assert_eq!(reader.wav().frames, 4);
            let mut decoder = crate::codec::make_audio_decoder(
                tag,
                reader.extra_data(),
                reader.sample_rate(),
                reader.channels(),
                reader.bits_per_sample(),
            )
            .unwrap_or_else(|error| panic!("{tag}: {error}"));
            let packet = reader.next_packet().expect("packet").expect("one window");
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("expand")
                .expect("a code always expands to a sample");
            let heard: Vec<f32> = audio
                .data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four bytes")))
                .collect();
            assert_eq!(heard.len(), 4, "{tag}");
            for (index, (code, got)) in codes.iter().zip(&heard).enumerate() {
                let want = f32::from(expected[index]) / 32_768.0;
                assert!(
                    (got - want).abs() < f32::EPSILON,
                    "{tag} code {code:#04x} gives {got}, the reference gives {want}"
                );
            }
        }
    }

    /// G.722 in the form the registry states it: the format number, four bits a sample,
    /// and a block one byte wide per channel because two samples share it. The byte rate
    /// is the field these writers do not fill in truthfully - measured at 8, 16 and
    /// 44.1 kHz, all three headers say 16000 while the runs go at 4000, 8000 and 22050
    /// bytes a second - so it is the one claim here the reader refuses to consult, and
    /// the frame count is taken from the `fact` chunk and the run instead.
    #[test]
    fn a_g_722_wave_codes_two_samples_a_byte() {
        let codes: Vec<u8> = (0..8).collect();
        for rate in [8_000u32, 16_000, 44_100] {
            let reader = open(&g722_file(1, rate, 4, 1, 16_000, Some(16), &codes));
            assert_eq!(reader.wav().pcm.coding, Coding::G722, "{rate} Hz");
            assert_eq!(reader.codec(), "adpcm_g722");
            assert_eq!(reader.bits_per_sample(), 4, "{rate} Hz");
            assert_eq!(reader.wav().pcm.block_align(), 1);
            assert_eq!(reader.wav().pcm.frames_per_block(), 2);
            // Eight bytes code eight frames of one channel: two samples to the byte.
            assert_eq!(reader.wav().frames, 16, "{rate} Hz");
            assert_eq!(reader.wav().pcm.coded_bytes_per_second(), rate as usize / 2);
            assert_eq!(reader.wav().pcm.derived_byte_rate(), None);
            assert_eq!(reader.wav().declared_frames, Some(16));
            assert_eq!(
                reader.duration(),
                Some(Duration::from_secs_f64(16.0 / f64::from(rate)))
            );
        }
    }

    /// The container and the coding meet: four bytes in, the reference's own eight
    /// samples out, stamped in frames. The numbers are `ffmpeg -i`'s for the same bytes;
    /// the whole 256-code run was compared that way in [`crate::codec::g722_decoder`].
    #[test]
    fn a_g_722_wave_reaches_the_decoder_as_the_run_it_claims() {
        let mut reader = open(&g722_file(1, 16_000, 4, 1, 16_000, Some(8), &[0, 1, 2, 3]));
        let mut decoder = crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("a G.722 arm");
        let packet = reader.next_packet().expect("packet").expect("one window");
        assert_eq!((packet.pts, packet.duration, packet.data.len()), (0, 8, 4));
        let audio = decoder
            .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
            .expect("expand")
            .expect("codes expand");
        assert_eq!((audio.timebase_num, audio.timebase_den), (1, 16_000));
        let heard: Vec<i16> = audio
            .data
            .chunks_exact(4)
            .map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0) as i16
            })
            .collect();
        assert_eq!(heard, [0, 0, -1, -1, 0, 0, 0, -1]);
    }

    /// The player's grain is frames, so a G.722 window is half the bytes a PCM one is
    /// and the stamps still advance by 2048 frames a window.
    #[test]
    fn a_g_722_run_is_cut_into_windows_of_frames() {
        let codes: Vec<u8> = (0..3_000).map(|index| index as u8).collect();
        let mut reader = open(&g722_file(1, 16_000, 4, 1, 16_000, Some(6_000), &codes));
        assert_eq!(reader.wav().packets(), 3);
        let mut seen = Vec::new();
        while let Some(packet) = reader.next_packet().expect("packet") {
            seen.push((packet.pts, packet.duration, packet.data.len()));
        }
        assert_eq!(
            seen,
            vec![
                (0, 2_048, 1_024),
                (2_048, 2_048, 1_024),
                (4_096, 1_904, 952)
            ]
        );
        // A seek lands on a window boundary in frames, whatever the coding's width.
        assert_eq!(reader.seek_to(4_500), 4_096);
        let tail = reader
            .next_packet()
            .expect("packet")
            .expect("the short window");
        assert_eq!((tail.pts, tail.duration), (4_096, 1_904));
    }

    /// A G.726 `fmt ` whose two derived fields are stated by the caller: the byte rate is
    /// where this coding keeps its code width, and the block is one to five bytes wide
    /// rather than a byte per channel.
    fn g726_file(
        channels: u16,
        sample_rate: u32,
        bits: u16,
        block: u16,
        byte_rate: u32,
        fact: Option<u32>,
        data: &[u8],
    ) -> Vec<u8> {
        let mut fmt_body = Vec::new();
        fmt_body.extend_from_slice(&G726_ADPCM.to_le_bytes());
        fmt_body.extend_from_slice(&channels.to_le_bytes());
        fmt_body.extend_from_slice(&sample_rate.to_le_bytes());
        fmt_body.extend_from_slice(&byte_rate.to_le_bytes());
        fmt_body.extend_from_slice(&block.to_le_bytes());
        fmt_body.extend_from_slice(&bits.to_le_bytes());
        let mut parts: Vec<(&[u8], &[u8])> = vec![(b"fmt ", fmt_body.as_slice())];
        let stated = fact.map(|frames| frames.to_le_bytes());
        if let Some(body) = &stated {
            parts.push((b"fact", body.as_slice()));
        }
        parts.push((b"data", data));
        wave(&parts)
    }

    /// G.726's four widths, in the geometry this build's muxer writes for each: the block
    /// is the narrowest run holding whole codes, so a byte holds four frames at 16 kbit/s
    /// and three bytes hold eight at 24. The width itself comes out of the byte rate,
    /// which is the only header field that states it - measured, patching the bit-depth
    /// field to anything from 0 to 64 leaves the reference's samples alone.
    #[test]
    fn a_g_726_wave_states_its_code_width_in_its_byte_rate() {
        for (code_size, block, byte_rate) in [
            (2u8, 1u16, 2_000u32),
            (3, 3, 3_000),
            (4, 1, 4_000),
            (5, 5, 5_000),
        ] {
            // A run of four blocks, with the bit-depth field set to something the width
            // is not, so the header is read for the rate rather than the field.
            let codes = vec![0x5au8; usize::from(block) * 4];
            let reader = open(&g726_file(1, 8_000, 32, block, byte_rate, None, &codes));
            assert_eq!(
                reader.wav().pcm.coding,
                Coding::G726(code_size),
                "{code_size} bits"
            );
            assert_eq!(reader.codec(), "adpcm_g726");
            assert_eq!(
                reader.bits_per_sample(),
                u16::from(code_size),
                "{code_size} bits"
            );
            assert_eq!(reader.wav().pcm.block_align(), usize::from(block));
            assert_eq!(
                reader.wav().pcm.frames_per_block(),
                8 * usize::from(block) / usize::from(code_size)
            );
            assert_eq!(
                reader.wav().pcm.coded_bytes_per_second(),
                usize::try_from(byte_rate).expect("a rate"),
                "{code_size} bits"
            );
            assert_eq!(
                reader.wav().pcm.derived_byte_rate(),
                Some(usize::try_from(byte_rate).expect("a rate"))
            );
            assert_eq!(reader.wav().frames, 4 * reader.wav().pcm.frames_per_block());
            assert_eq!(reader.wav().declared_frames, None);
        }
        // Where the header does state a `fact`, it is cross-checked like any other claim.
        let reader = open(&g726_file(1, 8_000, 4, 1, 4_000, Some(16), &[0; 8]));
        assert_eq!(reader.wav().declared_frames, Some(16));
        assert_eq!(reader.wav().frames, 16);
    }

    /// The container and the coding meet: one byte in, the reference's own four samples
    /// out. The numbers are `ffmpeg -i`'s for the same bytes; the whole codebook run was
    /// compared that way in [`crate::codec::g726_decoder`].
    #[test]
    fn a_g_726_wave_reaches_the_decoder_as_the_run_it_claims() {
        // 0b00_01_10_11: the four two-bit codes, ascending, on a cold receiver.
        let mut reader = open(&g726_file(1, 8_000, 2, 1, 2_000, Some(4), &[0x1b]));
        let mut decoder = crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("a G.726 arm");
        let packet = reader.next_packet().expect("packet").expect("one window");
        assert_eq!((packet.pts, packet.duration, packet.data.len()), (0, 4, 1));
        let audio = decoder
            .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
            .expect("expand")
            .expect("codes expand");
        assert_eq!((audio.timebase_num, audio.timebase_den), (1, 8_000));
        let heard: Vec<i16> = audio
            .data
            .chunks_exact(4)
            .map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0) as i16
            })
            .collect();
        assert_eq!(heard, [12, 60, -68, -20]);
    }

    /// The player's grain is frames, so a window holds however many frames the width
    /// packs into its bytes - 1 024 of them at 32 kbit/s, 256 blocks at 24 - and the
    /// stamps advance by 2 048 frames a window whatever the coding.
    #[test]
    fn a_g_726_run_is_cut_into_windows_of_frames() {
        for (code_size, block, byte_rate) in [
            (2u16, 1u16, 2_000u32),
            (3, 3, 3_000),
            (4, 1, 4_000),
            (5, 5, 5_000),
        ] {
            let frames_of = usize::from(block) * 8 / usize::from(code_size);
            let window = 2_048 / frames_of * usize::from(block);
            // Two whole windows and three more blocks, so the split is the reader's and
            // not the file's.
            let codes = vec![0x5au8; window * 2 + usize::from(block) * 3];
            let mut reader = open(&g726_file(
                1,
                8_000,
                code_size,
                block,
                byte_rate,
                Some(((codes.len() / usize::from(block)) * frames_of) as u32),
                &codes,
            ));
            let mut seen = Vec::new();
            while let Some(packet) = reader.next_packet().expect("packet") {
                seen.push((packet.pts, packet.duration, packet.data.len()));
            }
            assert_eq!(
                seen,
                vec![
                    (0, 2_048, window),
                    (2_048, 2_048, window),
                    (4_096, (frames_of * 3) as i64, usize::from(block) * 3)
                ],
                "{code_size} bits"
            );
            assert_eq!(
                reader.seek_to(4_500),
                4_096,
                "{code_size} bits: a seek lands on a window boundary"
            );
        }
    }

    /// The ways this header can be wrong about itself are refused rather than decoded
    /// anyway: a byte rate that codes no whole width, a width outside the four the format
    /// has, and an alignment that contradicts the geometry. A stale `fact` is the one
    /// claim read and not obeyed, because here it contradicts nothing the reader needs.
    #[test]
    fn a_g_726_header_that_contradicts_itself_is_refused() {
        let refused = |bytes: &[u8]| match Wav::parse(bytes, &Limits::default()) {
            Ok(wav) => panic!("{} frames read from {:?}", wav.frames, bytes),
            Err(error) => error.to_string(),
        };
        // 3 999 bytes a second at 8 kHz is 3.999 bits a frame: no width the format holds.
        assert!(
            refused(&g726_file(1, 8_000, 4, 1, 3_999, None, &[0; 8])).contains("two to five"),
            "fractional width"
        );
        // Eight bits a frame is a width this coding does not have, whatever the field named
        // for bit depth says.
        assert!(
            refused(&g726_file(1, 8_000, 8, 1, 8_000, None, &[0; 8])).contains("two to five"),
            "too wide"
        );
        // A block that holds no whole number of codes at the width the rate states.
        assert!(
            refused(&g726_file(1, 8_000, 3, 2, 3_000, None, &[0; 8])).contains("alignment"),
            "block"
        );
        // The coding's own rate is 8 kHz, and this build's encoder refuses anything else
        // outright, so a file at 44.1 kHz can only be a relabelled one. An honest relabel
        // - the byte rate scaled with the sample rate - still states a whole width and is
        // read; a file that kept the 8 kHz byte rate states 0.72 bits a frame and is
        // refused.
        let honest = open(&g726_file(1, 44_100, 4, 1, 22_050, None, &[0; 8]));
        assert_eq!(honest.bits_per_sample(), 4);
        assert_eq!(honest.sample_rate(), 44_100);
        assert!(
            refused(&g726_file(1, 44_100, 4, 1, 4_000, None, &[0; 8])).contains("two to five"),
            "unscaled relabel"
        );
        // A frame count the run does not code is kept as the file's claim while the run
        // plays. The G.722 header is checked because its byte rate states nothing about the
        // coding, so `fact` is its only statement about the run; this one's byte rate is
        // the load-bearing field and the reference ignores a `fact` that disagrees. The
        // gate's own fixtures are still counted without the reader, which is where a
        // contradicting fixture would raise.
        let stale = open(&g726_file(1, 8_000, 4, 1, 4_000, Some(15), &[0; 8]));
        assert_eq!(stale.wav().declared_frames, Some(15));
        assert_eq!(stale.wav().frames, 16);
    }

    /// A run that ends inside a block is the one place this reader and the reference part
    /// company, and the reader is the stricter of the two: the reference reads a packet
    /// wherever its bytes begin and decodes the seven bytes below as eighteen codes,
    /// while this reader frames a Wave track by its blocks and stops at the sixteenth
    /// frame. Both numbers are `ffmpeg -i`'s for these bytes.
    #[test]
    fn a_g_726_run_ends_where_its_blocks_do() {
        let reader = open(&g726_file(1, 8_000, 3, 3, 3_000, Some(16), &[0x55; 7]));
        assert_eq!(reader.wav().frames, 16);
        let mut reader = open(&g726_file(1, 8_000, 3, 3, 3_000, Some(16), &[0x55; 7]));
        let packet = reader.next_packet().expect("packet").expect("one window");
        assert_eq!((packet.duration, packet.data.len()), (16, 6));
        assert!(reader.next_packet().expect("packet").is_none());
    }

    /// Two channels is the shape the header can state and the coding cannot: the byte rate
    /// then reads as a narrower code, the block is honest about it, and the refusal
    /// happens where the track meets the decoder, as the reference's own asks for a patch
    /// for more than one channel.
    #[test]
    fn a_stereo_g_726_wave_is_read_and_then_refused_by_the_coding() {
        let reader = open(&g726_file(2, 8_000, 4, 2, 4_000, Some(16), &[0; 8]));
        assert_eq!(reader.channels(), 2);
        assert_eq!(reader.bits_per_sample(), 2, "two channels halve the width");
        assert_eq!(reader.wav().pcm.block_align(), 2);
        assert_eq!(reader.wav().frames, 16);
        let error = match crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        ) {
            Ok(_) => panic!("a stereo G.726 track read as mono"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("one channel"), "{error}");
    }

    /// The ways this header can be wrong about itself are refused rather than decoded
    /// anyway: a width the format number cannot carry, an alignment that contradicts
    /// the channel count, and a `fact` the run contradicts.
    #[test]
    fn a_g_722_header_that_contradicts_itself_is_refused() {
        let refused = |bytes: &[u8]| match Wav::parse(bytes, &Limits::default()) {
            Ok(wav) => panic!("{} frames read from {:?}", wav.frames, bytes),
            Err(error) => error.to_string(),
        };
        // Eight bits a sample is not what this format number means.
        assert!(
            refused(&g722_file(1, 16_000, 8, 2, 16_000, None, &[0; 8])).contains("four bits"),
            "width"
        );
        // Two channels in a byte-wide block: one of the two fields is wrong.
        assert!(
            refused(&g722_file(2, 16_000, 4, 1, 16_000, None, &[0; 8])).contains("alignment"),
            "block"
        );
        // A frame count the run does not code, which is checkable only because the byte
        // rate is not.
        assert!(
            refused(&g722_file(1, 16_000, 4, 1, 16_000, Some(15), &[0; 8])).contains("fact chunk"),
            "fact"
        );
    }

    /// The `fact` cross-check needs an intact run to check against. Where the `data`
    /// length lies, the bytes the file holds are the evidence and the claim has nothing
    /// to be contradicted by, so the reader plays what it has.
    #[test]
    fn a_fact_claim_is_only_checked_against_a_run_that_is_all_there() {
        let mut bytes = g722_file(1, 16_000, 4, 1, 16_000, Some(15), &[0; 8]);
        claim_more(&mut bytes, 16);
        let reader = WavAudioReader::open(&bytes[..], Limits::default()).expect("opens");
        assert_eq!(reader.wav().declared_bytes, 16);
        assert_eq!(reader.wav().frames, 16);
        assert_eq!(reader.wav().declared_frames, Some(15));
    }

    /// Stereo is the one case where the container and the coding disagree by design: a
    /// Wave header can state two channels with a two-byte block, and no writer measured
    /// here says how two G.722 streams would lie in those bytes. So the header is read
    /// and the refusal happens where the track meets the decoder.
    #[test]
    fn a_stereo_g_722_wave_is_read_and_then_refused_by_the_coding() {
        let reader = open(&g722_file(2, 16_000, 4, 2, 16_000, Some(8), &[0; 8]));
        assert_eq!(reader.channels(), 2);
        assert_eq!(reader.wav().pcm.block_align(), 2);
        assert_eq!(reader.wav().frames, 8);
        let error = match crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        ) {
            Ok(_) => panic!("two interleaved G.722 streams have no convention here"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("one channel"), "{error}");
    }

    /// The 40-byte `fmt ` of WAVE_FORMAT_EXTENSIBLE: the same geometry, then the
    /// extension the GUID repeats it in.
    fn extensible(data_format: u32, channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
        let mut body = fmt(EXTENSIBLE, channels, sample_rate, bits);
        body.extend_from_slice(&22u16.to_le_bytes());
        body.extend_from_slice(&bits.to_le_bytes());
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&data_format.to_le_bytes());
        body.extend_from_slice(&[0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA]);
        body.extend_from_slice(&[0x00, 0x38, 0x9B, 0x71]);
        body
    }

    #[test]
    fn an_extensible_header_is_read_by_its_sub_format() {
        let bytes = file(
            &extensible(u32::from(IEEE_FLOAT), 2, 96_000, 32),
            &vec![0u8; 8],
        );
        let pcm = Wav::parse(&bytes, &Limits::default())
            .expect("extensible")
            .pcm;
        assert_eq!(pcm.coding, Coding::Pcm(PcmFormat::Float { bits: 32 }));
        assert_eq!(pcm.channels, 2);
        // Stereo 32-bit float: eight bytes a frame, and the Matroska tag the
        // decoder is built from.
        assert_eq!(pcm.block_align(), 8);
        assert_eq!(pcm.codec(), "A_PCM/FLOAT/IEEE");
        // A GUID naming neither PCM shape is refused rather than read as int.
        let bytes = file(&extensible(0x0011, 2, 96_000, 32), &vec![0u8; 8]);
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
        // So is a GUID from another family entirely: the file offset is the GUID's
        // own start (44) plus the four bytes of its data-format word.
        let mut bytes = file(&extensible(u32::from(PCM), 2, 96_000, 32), &vec![0u8; 8]);
        bytes[48] = 0x01;
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
    }

    /// A `fmt ` chunk as a real muxer writes it for 24-bit PCM: the extensible form
    /// FFmpeg's Wave writer emits, byte for byte, so the GUID this reader compares
    /// against is the one the format uses rather than one a test agreed with itself.
    #[test]
    fn the_extensible_header_a_muxer_actually_writes_is_read() {
        let mut bytes = vec![
            0x52, 0x49, 0x46, 0x46, 0xde, 0x70, 0x00, 0x00, 0x57, 0x41, 0x56, 0x45, 0x66, 0x6d,
            0x74, 0x20, 0x28, 0x00, 0x00, 0x00, 0xfe, 0xff, 0x01, 0x00, 0x80, 0xbb, 0x00, 0x00,
            0x80, 0x32, 0x02, 0x00, 0x03, 0x00, 0x18, 0x00, 0x16, 0x00, 0x18, 0x00, 0x04, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa,
            0x00, 0x38, 0x9b, 0x71,
        ];
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&6u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let wav = Wav::parse(&bytes, &Limits::default()).expect("the header a muxer writes");
        assert_eq!(
            (
                wav.pcm.sample_rate,
                wav.pcm.channels,
                wav.pcm.bits_per_sample
            ),
            (48_000, 1, 24)
        );
        assert_eq!(
            wav.pcm.coding,
            Coding::Pcm(PcmFormat::Int {
                bits: 24,
                big_endian: false
            })
        );
        // Two frames of three bytes, and the stale envelope size is not the run.
        assert_eq!((wav.frames, wav.data.len()), (2, 6));
    }

    #[test]
    fn every_field_of_the_header_has_to_agree_with_the_others() {
        // A float header at a width the decoder does not read.
        let bytes = file(&fmt(IEEE_FLOAT, 1, 8_000, 16), &vec![0u8; 4]);
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
        // RIFF's unsigned 8-bit PCM, which the player's signed 8-bit tag inverts.
        let bytes = file(&fmt(PCM, 1, 8_000, 8), &vec![0u8; 4]);
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
        // A codec other than PCM or G.711: `data` would be a bitstream, not samples.
        let bytes = file(&fmt(0x0055, 1, 8_000, 16), &vec![0u8; 4]);
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
        // A G.711 header at a width its table cannot have.
        let bytes = file(&fmt(ALAW, 1, 8_000, 16), &vec![0u8; 4]);
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
        // The rate contradicted in the field derived from it: the block alignment
        // and the byte rate are both stated, so one of them has to be wrong.
        let mut bytes = wav(&[0, 1, 2, 3]);
        let at = 20 + 8;
        bytes[at..at + 4].copy_from_slice(&44_100u32.to_le_bytes());
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
        // Over the caller's budget.
        let bytes = wav(&[0; 64]);
        let limits = Limits {
            file_bytes: 32,
            ..Limits::default()
        };
        assert!(Wav::parse(&bytes, &limits).is_err());
        let limits = Limits {
            data_bytes: 4,
            ..Limits::default()
        };
        assert!(Wav::parse(&bytes, &limits).is_err());
    }

    #[test]
    fn a_missing_or_short_chunk_is_refused() {
        // Not a Wave file at all, or a Wave envelope with nothing in it.
        assert!(Wav::parse(b"RIFF....WAV", &Limits::default()).is_err());
        assert!(
            Wav::parse(
                b"RIFF\x12\x00\x00\x00WAVELIST\x04\x00\x00\x00",
                &Limits::default()
            )
            .is_err()
        );
        // A `fmt ` chunk that stops before its fields do.
        let mut body = fmt(PCM, 1, 8_000, 16);
        body.truncate(8);
        let bytes = file(&body, &[0; 4]);
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
        // A `data` chunk that holds nothing, and one that never arrives: neither is
        // a track the player can run.
        let bytes = file(&fmt(PCM, 1, 8_000, 16), &[]);
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
        let mut without_data = Vec::new();
        without_data.extend_from_slice(b"RIFF");
        without_data.extend_from_slice(&(24u32).to_le_bytes());
        without_data.extend_from_slice(b"WAVE");
        without_data.extend_from_slice(b"fmt ");
        without_data.extend_from_slice(&16u32.to_le_bytes());
        without_data.extend_from_slice(&fmt(PCM, 1, 8_000, 16));
        assert!(Wav::parse(&without_data, &Limits::default()).is_err());
        // An extensible header whose extension is cut off mid-GUID.
        let mut body = extensible(u32::from(PCM), 2, 96_000, 32);
        body.truncate(32);
        let bytes = file(&body, &[0; 8]);
        assert!(Wav::parse(&bytes, &Limits::default()).is_err());
    }

    /// The envelope's own size field is wrong in the way writers get it wrong - a
    /// file appended to, a header never rewritten - and the chunks inside it are
    /// still readable, because the reader walks to the end of what it has.
    #[test]
    fn a_stale_envelope_size_still_leaves_the_sample_run_readable() {
        let mut bytes = wav(&[5i16, 6, 7, 8]);
        bytes[4..8].copy_from_slice(&8u32.to_le_bytes());
        let wav = Wav::parse(&bytes, &Limits::default()).expect("the chunks, not the size");
        assert_eq!(wav.frames, 4);
        assert_eq!(wav.data.len(), 8);
    }

    /// A `data` chunk claiming more than the file holds is read as what is there:
    /// the claim is the writer's, the bytes are the reader's, and a run that stops
    /// mid-frame loses that fragment rather than playing half a sample.
    #[test]
    fn a_data_chunk_longer_than_the_file_is_read_as_the_file() {
        let mut bytes = wav(&[1i16, 2, 3]);
        let at = 12 + 8 + 16 + 4;
        bytes[at..at + 4].copy_from_slice(&16u32.to_le_bytes());
        let mut reader = WavAudioReader::open(&bytes[..], Limits::default()).expect("opens");
        assert_eq!(reader.wav().declared_bytes, 16);
        assert_eq!(reader.wav().frames, 3);
        let packet = reader.next_packet().expect("packet").expect("one packet");
        assert_eq!((packet.pts, packet.duration), (0, 3));
        assert_eq!(packet.data.len(), 6);
    }

    /// A GSM `fmt ` in the shape this format's writers use. The block is the whole 65-byte
    /// frame pair rather than a width times a channel count, the byte rate follows from
    /// that block alone, and the depth field states nothing the coding reads. The
    /// extension, when given, is the two bytes a writer adds for `wSamplesPerBlock`.
    fn gsm_file(
        channels: u16,
        sample_rate: u32,
        bits: u16,
        block: u16,
        byte_rate: u32,
        samples_per_block: Option<u16>,
        fact: Option<u32>,
        data: &[u8],
    ) -> Vec<u8> {
        let mut fmt_body = Vec::new();
        fmt_body.extend_from_slice(&GSM_FULL_RATE.to_le_bytes());
        fmt_body.extend_from_slice(&channels.to_le_bytes());
        fmt_body.extend_from_slice(&sample_rate.to_le_bytes());
        fmt_body.extend_from_slice(&byte_rate.to_le_bytes());
        fmt_body.extend_from_slice(&block.to_le_bytes());
        fmt_body.extend_from_slice(&bits.to_le_bytes());
        if let Some(samples) = samples_per_block {
            fmt_body.extend_from_slice(&2u16.to_le_bytes());
            fmt_body.extend_from_slice(&samples.to_le_bytes());
        }
        let mut parts: Vec<(&[u8], &[u8])> = vec![(b"fmt ", fmt_body.as_slice())];
        let stated = fact.map(|frames| frames.to_le_bytes());
        if let Some(body) = &stated {
            parts.push((b"fact", body.as_slice()));
        }
        parts.push((b"data", data));
        wave(&parts)
    }

    /// GSM 06.10 states its rate in the width of its block and in no other field, so that
    /// is the only geometry the reader asks. Measured on this build's reference: the same
    /// 89 blocks of the take below decode the same 28 480 samples whether the depth field
    /// says 0, 8, 16 or 64, and whether the block's sample count is stated at all.
    #[test]
    fn a_gsm_wave_states_its_rate_in_its_block_alignment() {
        for bits in [0u16, 8, 16, 64] {
            let reader = open(&gsm_file(
                1,
                8_000,
                bits,
                65,
                1_625,
                Some(320),
                None,
                &[0; 2 * GSM_BLOCK_BYTES],
            ));
            let wav = reader.wav();
            assert_eq!(wav.pcm.coding, Coding::Gsm, "{bits} bits a sample");
            assert_eq!(reader.codec(), "gsm_ms");
            assert_eq!(wav.pcm.block_align(), GSM_BLOCK_BYTES);
            assert_eq!(wav.pcm.frames_per_block(), 320);
            assert_eq!(wav.pcm.derived_byte_rate(), Some(1_625));
            assert_eq!(
                reader.bits_per_sample(),
                bits,
                "the field is kept as stated"
            );
            assert_eq!(wav.frames, 640, "two blocks of 320 samples");
        }
        // The same track with nothing past the seven fields: the coding is read from the
        // fourteen bytes that were always there.
        let bare = open(&gsm_file(1, 8_000, 0, 65, 1_625, None, None, &[0; 130]));
        assert_eq!(bare.wav().pcm.coding, Coding::Gsm);
        assert_eq!(bare.wav().frames, 640);
    }

    /// The narrower blocks are the coding's trimmed rates, refused here by name rather than
    /// read as full-rate codes: every three bytes a block loses stands for twelve bits of a
    /// frame's RPE codes, and which twelve the reference picks by a mode the header states
    /// only as that width - the tables this decoder carries are the full rate's alone. A
    /// width that is no alignment the format can state at all is a contradiction instead,
    /// and this build's reference refuses that one outright too.
    #[test]
    fn a_gsm_block_narrower_than_the_full_rate_is_refused_by_name() {
        let refused = |block: u16| match Wav::parse(
            &gsm_file(1, 8_000, 0, block, 1_625, None, None, &[0; 130]),
            &Limits::default(),
        ) {
            Ok(wav) => panic!("a {block}-byte block read as {:?}", wav.pcm.coding),
            Err(error) => error.to_string(),
        };
        for block in [62u16, 59, 56, 53, 50, 47, 44, 41] {
            let error = refused(block);
            assert!(
                error.contains(&format!("{block}-byte block")),
                "{block}: {error}"
            );
        }
        for block in [64u16, 68, 40, 130] {
            assert!(refused(block).contains("three-byte steps"), "{block}");
        }
    }

    /// The two ways this header can contradict the coding it names. Both are refused
    /// rather than decoded anyway, and both are cases this build's reference reads: it
    /// believes neither field, since a GSM stream is fully framed by its block width.
    #[test]
    fn a_gsm_header_that_contradicts_itself_is_refused() {
        let refused = |bytes: &[u8]| match Wav::parse(bytes, &Limits::default()) {
            Ok(wav) => panic!("{} frames read from {:?}", wav.frames, wav.pcm.coding),
            Err(error) => error.to_string(),
        };
        // 65 bytes every 320 samples at 8 kHz is 1 625 a second, and no other number is.
        assert!(
            refused(&gsm_file(1, 8_000, 0, 65, 1_624, None, None, &[0; 130])).contains("byte rate"),
            "rate"
        );
        // A rate the block does not divide into whole bytes a second cannot state its own
        // byte rate truthfully: at 44.1 kHz the geometry asks for 8 957.8125, so whichever
        // number a writer puts there is wrong, and this one is wrong for an 8 kHz run
        // whose rate field was patched afterwards.
        assert!(
            refused(&gsm_file(1, 44_100, 0, 65, 1_625, None, None, &[0; 130]))
                .contains("byte rate"),
            "a rate the geometry cannot be paid for"
        );
        // A second channel is not a width or a rate this coding trades in: the block is
        // one channel's two frames, so a track naming two has said something the format
        // number cannot mean. The reference decodes the same bytes as mono.
        assert!(
            refused(&gsm_file(2, 8_000, 0, 65, 1_625, None, None, &[0; 130]))
                .contains("one channel"),
            "channels"
        );
    }

    /// The rate this coding is held to is the block's own division of the sample rate,
    /// done in whole bytes, which is exactly the number a writer puts in the field. So a
    /// run whose rate the block does not divide whole is still read at the floored claim
    /// and refused at the rounded one - measured against this build's reference, which
    /// plays both: the take below at 44.1 kHz gives 8 957.8125 bytes a second, and the
    /// 8 957 a muxer writes decodes here into the same 28 480 samples the reference gives,
    /// while 8 958 does not decode at all. The rate is a timestamp and nothing else to
    /// this coding: 65 bytes stay 320 samples whatever it says.
    #[test]
    fn a_gsm_byte_rate_is_the_blocks_own_division_of_the_rate() {
        let bytes = gsm_file(
            1,
            44_100,
            0,
            65,
            8_957,
            None,
            None,
            &[0; 2 * GSM_BLOCK_BYTES],
        );
        let wav = Wav::parse(&bytes, &Limits::default())
            .expect("the floored byte rate is what a writer states");
        assert_eq!(wav.pcm.coding, Coding::Gsm);
        assert_eq!(wav.pcm.derived_byte_rate(), Some(8_957));
        assert_eq!((wav.pcm.sample_rate, wav.frames), (44_100, 640));
        let rounded = gsm_file(
            1,
            44_100,
            0,
            65,
            8_958,
            None,
            None,
            &[0; 2 * GSM_BLOCK_BYTES],
        );
        let error = match Wav::parse(&rounded, &Limits::default()) {
            Ok(wav) => panic!("8 958 read as {:?}", wav.pcm.coding),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("byte rate"), "{error}");
    }

    /// A `fact` is the run's third claim on its length, and for GSM the one nobody checks:
    /// the byte rate already ties the block width to the rate, so this coding cannot have
    /// a run that disagrees with its header without one of those two fields lying first.
    /// The claim is carried as the file states it, as G.726's is, and the run plays as its
    /// blocks say it is.
    #[test]
    fn a_gsm_fact_is_carried_rather_than_cross_checked() {
        let reader = open(&gsm_file(
            1,
            8_000,
            0,
            65,
            1_625,
            Some(320),
            Some(15),
            &[0; 2 * GSM_BLOCK_BYTES],
        ));
        assert_eq!(reader.wav().declared_frames, Some(15));
        assert_eq!(reader.wav().frames, 640);
    }

    /// The take the decoder is proved sample-for-sample against, read for its geometry:
    /// 89 whole blocks with a three-byte tail nowhere in them, a depth field of 0, an
    /// extension stating the block's 320 samples and a `fact` stating the whole run.
    #[test]
    fn the_reference_gsm_take_reads_as_its_own_blocks() {
        const TAKE: &[u8] = include_bytes!("../tests/fixtures/gsm/ciao.wav");
        let reader = open(TAKE);
        let wav = reader.wav();
        assert_eq!(wav.pcm.coding, Coding::Gsm);
        assert_eq!(
            (
                reader.sample_rate(),
                reader.channels(),
                reader.bits_per_sample()
            ),
            (8_000, 1, 0)
        );
        assert_eq!(wav.declared_bytes, 5_785);
        assert_eq!(wav.frames, 28_480);
        assert_eq!(wav.declared_frames, Some(28_480));
        assert_eq!(
            reader.duration(),
            Some(Duration::from_secs_f64(28_480.0 / 8_000.0))
        );
        // The player's window is 2 048 frames, which the block's 320 do not fill: the
        // grain this reader cuts is six whole blocks - 1 920 frames, 390 bytes - so the
        // run arrives as 14 full windows and a last one of five blocks.
        assert_eq!(wav.packets(), 15);
        assert_eq!(wav.packet(0).len(), 6 * GSM_BLOCK_BYTES);
        assert_eq!(wav.packet(14).len(), 5 * GSM_BLOCK_BYTES);
        for index in 0..wav.packets() {
            assert_eq!(
                wav.packet(index).len() % GSM_BLOCK_BYTES,
                0,
                "window {index}"
            );
        }
    }

    /// A Creative ADPCM `fmt ` in the shape its writers use: the format number, a depth
    /// field of four bits, and then whatever the muxer felt like putting in the two
    /// derived fields. The block states a byte per channel because the coding has no
    /// block at all, and the byte rate follows from that byte and the rate alone.
    fn ct_file(
        channels: u16,
        sample_rate: u32,
        bits: u16,
        block: u16,
        byte_rate: u32,
        fact: Option<u32>,
        data: &[u8],
    ) -> Vec<u8> {
        let mut fmt_body = Vec::new();
        fmt_body.extend_from_slice(&ADPCM_CT.to_le_bytes());
        fmt_body.extend_from_slice(&channels.to_le_bytes());
        fmt_body.extend_from_slice(&sample_rate.to_le_bytes());
        fmt_body.extend_from_slice(&byte_rate.to_le_bytes());
        fmt_body.extend_from_slice(&block.to_le_bytes());
        fmt_body.extend_from_slice(&bits.to_le_bytes());
        let mut parts: Vec<(&[u8], &[u8])> = vec![(b"fmt ", fmt_body.as_slice())];
        let stated = fact.map(|frames| frames.to_le_bytes());
        if let Some(body) = &stated {
            parts.push((b"fact", body.as_slice()));
        }
        parts.push((b"data", data));
        wave(&parts)
    }

    /// Creative's coding frames its run by nothing but its own bytes, so the reader asks
    /// two fields of the header and leaves the rest. Measured on this build's reference
    /// over the shipped take below: the same 262 096 bytes decode to the same 524 192
    /// samples with `wBlockAlign` 1, 258 and 516, with `dwAvgBytesPerSec` stating 22 050
    /// and 44 100, and with `wBitsPerSample` 4, 8 or 16 - the last of those is why the
    /// depth is asked here and the other two are not.
    #[test]
    fn a_creative_wave_states_nothing_but_its_code_width() {
        for (block, byte_rate) in [(1u16, 22_050u32), (258, 22_050), (516, 44_100), (1, 8_000)] {
            let reader = open(&ct_file(1, 44_100, 4, block, byte_rate, None, &[0; 8]));
            let wav = reader.wav();
            assert_eq!(wav.pcm.coding, Coding::AdpcmCt, "{block}/{byte_rate}");
            assert_eq!(reader.codec(), "adpcm_ct");
            assert_eq!(wav.pcm.block_align(), 1, "a byte per channel");
            assert_eq!(wav.pcm.frames_per_block(), 2, "two codes to a byte");
            assert_eq!(
                wav.pcm.derived_byte_rate(),
                None,
                "{block}/{byte_rate} states no rate the coding could mean"
            );
            assert_eq!(wav.frames, 16);
        }
        // Stereo is the one field with an effect the reader does not model: the reference
        // hands alternate nibbles to a second predictor, so a two-channel track is not a
        // flat run and is refused rather than played as one channel's worth of halves.
        for channels in [2u16, 4] {
            let error = match Wav::parse(
                &ct_file(channels, 44_100, 4, channels, 44_100, None, &[0; 8]),
                &Limits::default(),
            ) {
                Ok(wav) => panic!("{} channels read as {:?}", channels, wav.pcm.coding),
                Err(error) => error.to_string(),
            };
            assert!(error.contains("one channel"), "{channels}: {error}");
        }
    }

    /// The depth field is the one Creative header the reader does hold to, because a
    /// four-bit code is what the format number means: a track stating another width has
    /// named a coding this format does not have, and the reference would be guessing at
    /// nibbles. Measured, it does guess - the take decoded identically at 8 and 16 - and
    /// this reader does not.
    #[test]
    fn a_creative_track_that_states_another_width_is_refused() {
        for bits in [0u16, 8, 16] {
            let error = match Wav::parse(
                &ct_file(1, 44_100, bits, 1, 22_050, None, &[0; 8]),
                &Limits::default(),
            ) {
                Ok(wav) => panic!("{bits} bits read as {:?}", wav.pcm.coding),
                Err(error) => error.to_string(),
            };
            assert!(
                error.contains(&format!("four bits per sample, this track states {bits}")),
                "{bits}: {error}"
            );
        }
    }

    /// With no byte rate to tie the run to, the `fact` chunk is the one claim a Creative
    /// header can be checked against, and this reader checks it - the reference does not,
    /// measured over the take: a `fact` stating four frames plays all 524 192 samples of
    /// its bytes. So the divergence is deliberate and named: a run that contradicts its
    /// own stated length is a file two writers disagreed about, and playing it would pick
    /// a side silently.
    #[test]
    fn a_creative_fact_is_cross_checked_because_nothing_else_states_the_length() {
        let refused = |frames: u32| match Wav::parse(
            &ct_file(1, 44_100, 4, 1, 22_050, Some(frames), &[0; 8]),
            &Limits::default(),
        ) {
            Ok(wav) => panic!("{frames} frames agreed with {:?}", wav.frames),
            Err(error) => error.to_string(),
        };
        assert!(refused(4).contains("fact chunk states 4 frames"), "four");
        assert!(
            refused(17).contains("fact chunk states 17 frames"),
            "seventeen"
        );
        // The truth passes, and a run shorter than the `data` field claims is still the
        // bytes it has: no claim to check where the file never made one.
        let truth = open(&ct_file(1, 44_100, 4, 1, 22_050, Some(16), &[0; 8]));
        assert_eq!(
            (truth.wav().frames, truth.wav().declared_frames),
            (16, Some(16))
        );
        let lying_size = {
            let mut bytes = ct_file(1, 44_100, 4, 1, 22_050, Some(16), &[0; 8]);
            // The `data` chunk's own length field, four bytes before its eight samples.
            let at = bytes.len() - 12;
            bytes[at..at + 4].copy_from_slice(&8_192u32.to_le_bytes());
            bytes
        };
        let wav = Wav::parse(&lying_size, &Limits::default())
            .expect("a run truncated by the file itself is played as its bytes");
        assert_eq!((wav.frames, wav.declared_bytes), (16, 8_192));
    }

    /// The take the decoder is proved sample-for-sample against, read for its geometry:
    /// one channel at 44 100 Hz, four bits a sample, a byte a block - and a `data` chunk
    /// claiming 2 235 116 bytes over the 262 096 the file holds, with no `fact` to
    /// contradict it. The run arrives as the player's own windows of a thousand-odd
    /// bytes, since 2 048 frames of two codes each is 1 024 of them.
    #[test]
    fn the_reference_creative_take_reads_as_its_own_bytes() {
        const TAKE: &[u8] = include_bytes!("../tests/fixtures/adpcm_ct/intro-partial.wav");
        let reader = open(TAKE);
        let wav = reader.wav();
        assert_eq!(wav.pcm.coding, Coding::AdpcmCt);
        assert_eq!(
            (
                reader.sample_rate(),
                reader.channels(),
                reader.bits_per_sample()
            ),
            (44_100, 1, 4)
        );
        assert_eq!(wav.declared_bytes, 2_235_116);
        assert_eq!(wav.declared_frames, None);
        assert_eq!(wav.frames, 524_192);
        assert_eq!(wav.data.len(), 262_096);
        assert_eq!(
            reader.duration(),
            Some(Duration::from_secs_f64(524_192.0 / 44_100.0))
        );
        assert_eq!(wav.packets(), 256);
        assert_eq!(wav.packet(0).len(), 1_024);
        assert_eq!(wav.packet(255).len(), 976, "the tail is whole bytes");
        for index in 0..wav.packets() {
            assert_eq!(wav.packet(index).len() % 2, 0, "window {index}");
        }
    }
}
