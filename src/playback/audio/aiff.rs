//! Audio playback of PCM and Apple MACE out of the wrapper the Mac wrote.
//!
//! An AIFF file states the geometry the player asks a container for - channel
//! count, frame count, width, rate - and holds the samples in exactly the form
//! [`crate::codec::pcm_decoder`] consumes, so as with [`crate::playback_wav`]
//! there is no codec setup to hand over and nothing to render. Two things make
//! this reader more than the Wave one spelled with different constants: every
//! number of the header is big-endian, and the sample rate is an 80-bit IEEE
//! extended value rather than an integer, a field no other container of this
//! player uses.
//!
//! Where a Wave file states its geometry several redundant ways and checks them
//! against each other, an AIFF file states each number once. So what this reader
//! holds the header to instead is the sample run: the bytes the `SSND` chunk
//! actually carries decide how much plays, and a `COMM` frame count that promises
//! more is kept as the file's own claim rather than played as silence.
//!
//! MACE is the one coding whose geometry the header does not state at all. Its two
//! compression types name their own block widths - two bytes a channel for 3-to-1,
//! one for 6-to-1, six samples for both - so `MAC3` and `MAC6` are read as the whole
//! of what a file says about the coding, and the width field, which every writer of
//! this build's reference sets to 8 for both, is not consulted. It is also the one
//! coding whose `COMM` frame count does not count samples: measured, a 85 675-byte
//! mono 3-to-1 run states 42 837 there, which is its number of blocks. The claim is
//! kept as the header spells it, so the disagreement stays visible.
//!
//! Samples are read as two's complement, which is what a writer produces: the
//! `pcm_s16be` AIFF FFmpeg muxes steps smoothly through its zero crossing -
//! `0x0081`, `0xff95`, `0xfeaa` - where the sign-magnitude encoding Apple's
//! original document describes reads those same bytes as -32 661 and -33 110, a
//! full-scale swing one sample into a gentle sine.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::codec::pcm_decoder::PcmFormat;
use crate::playback_wav::{Coding, PACKET_FRAMES, Pcm};
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
            file_bytes: 128 << 20,
            data_bytes: 128 << 20,
            channels: 32,
        }
    }
}

/// The compression types an `AIFC` header may name for samples this decoder reads.
/// Plain `AIFF` has no such field and always means the first of these.
const NONE: &[u8] = b"NONE";
const SOWT: &[u8] = b"sowt";
/// Apple's two MACE codings, which an AIFC header names by fourcc and describes in no
/// other field: neither its width nor its frame count says anything a reader needs.
const MAC3: &[u8] = b"MAC3";
const MAC6: &[u8] = b"MAC6";

/// An AIFF file: its geometry and the bytes of its sample run.
#[derive(Clone, Debug)]
pub struct Aiff {
    pub pcm: Pcm,
    /// What the `COMM` chunk's frame count claims, which may be more or fewer
    /// frames than the run the file carries. A MACE file states blocks in this field
    /// rather than samples - measured, this build's reference's own AIFF muxer wrote
    /// 42 837 for an 85 675-byte mono 3-to-1 run and 105 105 for a 420 420-byte stereo
    /// one - so for those two codings it is the header's claim in the header's units.
    pub declared_frames: usize,
    /// Frames the reader will hand over, a short tail included.
    pub frames: usize,
    data: Vec<u8>,
}

impl Aiff {
    /// Read a whole file: the `FORM` envelope, the `COMM` chunk that states the
    /// geometry, and the `SSND` chunk that carries the samples.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        if bytes.len() < 12 || &bytes[..4] != b"FORM" {
            return Err(invalid("not a FORM file"));
        }
        let form = &bytes[8..12];
        let aifc = match form {
            b"AIFF" => false,
            b"AIFC" => true,
            other => {
                return Err(invalid(&format!(
                    "a FORM envelope of {}, which is not an audio interchange file",
                    String::from_utf8_lossy(other)
                )));
            }
        };
        let mut comm = None;
        let mut ssnd = None;
        // As in the Wave reader, the walk runs to the end of the buffer rather than to
        // the envelope's declared size: a writer that got that number wrong still has
        // readable chunks in it.
        let mut at = 12usize;
        while at + 8 <= bytes.len() {
            let tag = &bytes[at..at + 4];
            let declared = usize::try_from(u32::from_be_bytes(
                bytes[at + 4..at + 8].try_into().unwrap_or([0xff; 4]),
            ))
            .unwrap_or(usize::MAX);
            let body = at + 8;
            let end = body.saturating_add(declared).min(bytes.len());
            // Chunks are word aligned, so a writer adds a byte after an odd-length
            // one, and the next chunk starts past it.
            at = body.saturating_add(declared.saturating_add(declared & 1));
            if tag == b"COMM" && comm.is_none() {
                comm = Some(parse_comm(&bytes[body..end], aifc, limits)?);
            } else if tag == b"SSND" && ssnd.is_none() {
                ssnd = Some((body, end));
            }
            // `FVER`, `MARK`, `INST`, `AUTH` and whatever else an authoring tool
            // leaves behind are skipped: none of them changes how the samples lie.
        }
        let (pcm, declared_frames) =
            comm.ok_or_else(|| invalid("an AIFF file with no COMM chunk"))?;
        let (start, end) = ssnd.ok_or_else(|| invalid("an AIFF file with no SSND chunk"))?;
        if end - start < 8 {
            return Err(invalid("SSND chunk too short for its own offset"));
        }
        // The chunk says how far past its own header the samples start, which is how a
        // writer that aligns each frame to a long word says so.
        let offset = usize::try_from(u32::from_be_bytes(
            bytes[start..start + 4].try_into().unwrap_or([0xff; 4]),
        ))
        .unwrap_or(usize::MAX);
        let begin = start.saturating_add(8).saturating_add(offset);
        if begin > end {
            return Err(invalid("SSND's own offset points past its data"));
        }
        let samples = &bytes[begin..end];
        if samples.is_empty() {
            return Err(invalid("an AIFF file with no samples in it"));
        }
        if samples.len() > limits.data_bytes {
            return Err(invalid(&format!(
                "sample run is over the {} byte limit",
                limits.data_bytes
            )));
        }
        Ok(Self {
            pcm,
            declared_frames,
            frames: samples.len() / pcm.block_align() * pcm.frames_per_block(),
            data: samples.to_vec(),
        })
    }

    /// Packets the sample run is read out as, the last a short one when the run does
    /// not divide evenly.
    pub fn packets(&self) -> usize {
        self.data.len().div_ceil(self.window_bytes())
    }

    /// One packet's bytes, aligned to a block. A tail shorter than a block holds no
    /// complete frame - not one sample for every channel, and for MACE 3-to-1 not one
    /// of the six its two bytes code - and is not handed over at all.
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

fn parse_comm(body: &[u8], aifc: bool, limits: &Limits) -> Result<(Pcm, usize)> {
    if body.len() < 18 {
        return Err(invalid("COMM chunk is shorter than its four fields"));
    }
    let u16_at = |at: usize| u16::from_be_bytes(body[at..at + 2].try_into().unwrap_or([0xff; 2]));
    let u32_at = |at: usize| u32::from_be_bytes(body[at..at + 4].try_into().unwrap_or([0xff; 4]));
    let channels = u16_at(0);
    let frames = usize::try_from(u32_at(2)).unwrap_or(usize::MAX);
    let bits = u16_at(6);
    let sample_rate = read_extended_rate(&body[8..18])?;
    // Only an AIFC header has a compression type to state, and only a COMM chunk long
    // enough to hold one states it; a short AIFC chunk is the plain geometry.
    let kind = if aifc && body.len() >= 22 {
        &body[18..22]
    } else {
        NONE
    };
    if channels == 0 || usize::from(channels) > limits.channels {
        return Err(invalid("channel count is out of range"));
    }
    let coding = if kind == MAC3 {
        Coding::Mace3
    } else if kind == MAC6 {
        Coding::Mace6
    } else if kind == NONE {
        Coding::Pcm(PcmFormat::Int {
            bits: bits as u8,
            big_endian: true,
        })
    } else if kind == SOWT {
        // Byte-swapped PCM, which is how an AIFC file says it holds the little-endian
        // widths its successors took from the PC.
        Coding::Pcm(PcmFormat::Int {
            bits: bits as u8,
            big_endian: false,
        })
    } else {
        return Err(unsupported(&format!(
            "AIFF compression {} is not PCM or MACE this decoder reads",
            String::from_utf8_lossy(kind)
        )));
    };
    // MACE is the coding whose width field describes no layout anyone reads: what the
    // fourcc alone decides is the block - two bytes a channel for 3-to-1, one for 6-to-1,
    // six samples for both - so the 8 a header states for those files is a number the
    // coding has already made pointless. Measured, this build's reference decoded the same
    // run with that field stating 1, 8 and 16 bits alike.
    //
    // PCM's width is the opposite case: it is the only thing that says how the bytes lie,
    // so a width this player has no decoder for is refused here rather than at the
    // decoder. 8-bit is the one width whose meaning is not obvious from these headers, so
    // it was measured rather than guessed: the 8-bit AIFF FFmpeg muxes starts its sine with
    // `00 00 01 02 03`, and its own decode of those bytes is 0.0, 0.0, 1/128, 2/128,
    // 3/128 - a signed row, not the offset-by-half-scale row a Wave file of the same
    // width holds. `pcm_decoder` reads it that way already.
    if !matches!(coding, Coding::Mace3 | Coding::Mace6) && !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(unsupported(&format!(
            "AIFF PCM of {bits} bits a sample is a width this player has no decoder for"
        )));
    }
    Ok((
        Pcm {
            coding,
            sample_rate,
            channels,
            bits_per_sample: bits,
        },
        frames,
    ))
}

/// The `COMM` sample rate: an 80-bit IEEE extended number, a sign and a 15-bit
/// exponent in the first two bytes and then a 64-bit significand whose leading bit
/// is stated rather than implied. The rate is worked out in integers, so a number
/// with a fraction in it is a refusal and not a rounding.
fn read_extended_rate(bytes: &[u8]) -> Result<u32> {
    let word = u16::from_be_bytes(bytes[..2].try_into().unwrap_or([0xff; 2]));
    if word >> 15 != 0 {
        return Err(invalid("a negative sample rate"));
    }
    let shift = i32::from(word & 0x7fff) - 16383 - 63;
    if !(-128..128).contains(&shift) {
        return Err(invalid("sample rate exponent is out of any audio range"));
    }
    let significand = u64::from_be_bytes(bytes[2..10].try_into().unwrap_or([0xff; 8]));
    let rate = if shift >= 0 {
        let widened = u128::from(significand)
            .checked_shl(shift as u32)
            .ok_or_else(|| invalid("sample rate is out of range"))?;
        u32::try_from(widened).map_err(|_| invalid("sample rate is out of range"))?
    } else {
        let drop = u32::try_from(-shift).unwrap_or(u32::MAX);
        if drop >= 64 || significand % (1u64 << drop) != 0 {
            return Err(invalid("a fractional sample rate"));
        }
        u32::try_from(significand >> drop).map_err(|_| invalid("sample rate is out of range"))?
    };
    if rate == 0 {
        return Err(invalid("sample rate of zero"));
    }
    Ok(rate)
}

/// An AIFF file read as an audio track.
pub struct AiffAudioReader {
    aiff: Aiff,
    packet: usize,
}

impl AiffAudioReader {
    /// Read a whole file. The sample run is one contiguous block and the packets are
    /// slices of it, so there is nothing here to produce incrementally.
    pub fn open<R: Read>(mut reader: R, limits: Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        Ok(Self {
            aiff: Aiff::parse(&bytes, &limits)?,
            packet: 0,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn aiff(&self) -> &Aiff {
        &self.aiff
    }
}

impl AudioStream for AiffAudioReader {
    fn codec(&self) -> &str {
        self.aiff.pcm.codec()
    }

    fn timescale(&self) -> u32 {
        self.aiff.pcm.sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.aiff.pcm.sample_rate
    }

    fn channels(&self) -> u16 {
        self.aiff.pcm.channels
    }

    fn bits_per_sample(&self) -> u16 {
        self.aiff.pcm.bits_per_sample
    }

    /// How long the run plays, from the frames the reader hands over rather than from the
    /// bytes it holds: a MACE block costs one to four bytes and codes six frames, so
    /// dividing a run by its byte rate states a sixth of the seconds its samples play for.
    /// Every other coding's block is one frame wide, where the two agree.
    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.aiff.frames as f64 / f64::from(self.aiff.pcm.sample_rate),
        ))
    }

    /// PCM needs no setup: the header's fields have already gone into the decoder's
    /// construction.
    fn extra_data(&self) -> &[u8] {
        &[]
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: self.aiff.pcm.sample_rate,
            channels: self.aiff.pcm.channels,
            name: String::new(),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        if self.packet >= self.aiff.packets() {
            return Ok(None);
        }
        let data = self.aiff.packet(self.packet).to_vec();
        let packet = EncodedPacket {
            duration: ((data.len() / self.aiff.pcm.block_align())
                * self.aiff.pcm.frames_per_block()) as i64,
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
        let last = self.aiff.packets().saturating_sub(1);
        self.packet = (pts.max(0) as usize / PACKET_FRAMES).min(last);
        (self.packet * PACKET_FRAMES) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{Aiff, AiffAudioReader, Limits, PACKET_FRAMES};
    use crate::audio::{AudioDecode as _, AudioStream};
    use crate::codec::pcm_decoder::PcmDecoder;
    use std::time::Duration;

    /// The 80-bit spellings of four rates, read off files FFmpeg's AIFF muxer wrote
    /// rather than computed here - a helper that filled in this field the way the
    /// reader reads it would prove nothing about either.
    const RATES: &[(u32, [u8; 10])] = &[
        (
            8_000,
            [0x40, 0x0b, 0xfa, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            44_100,
            [0x40, 0x0e, 0xac, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            48_000,
            [0x40, 0x0e, 0xbb, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            192_000,
            [0x40, 0x10, 0xbb, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
    ];
    const RATE_48_000: [u8; 10] = RATES[2].1;

    fn comm(channels: u16, frames: u32, bits: u16, rate: [u8; 10]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&channels.to_be_bytes());
        out.extend_from_slice(&frames.to_be_bytes());
        out.extend_from_slice(&bits.to_be_bytes());
        out.extend_from_slice(&rate);
        out
    }

    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn file(form: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: usize = chunks.iter().map(|chunk| chunk.len()).sum();
        let mut out = Vec::new();
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&(4 + body as u32).to_be_bytes());
        out.extend_from_slice(form);
        for chunk in chunks {
            out.extend_from_slice(chunk);
        }
        out
    }

    fn ssnd(samples: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(samples);
        chunk(b"SSND", &body)
    }

    /// A plain 16-bit big-endian mono or stereo AIFF of the samples its caller names.
    fn aiff(channels: usize, samples: &[u8]) -> Vec<u8> {
        file(
            b"AIFF",
            &[
                chunk(
                    b"COMM",
                    &comm(
                        channels as u16,
                        (samples.len() / (channels * 2)) as u32,
                        16,
                        RATE_48_000,
                    ),
                ),
                ssnd(samples),
            ],
        )
    }

    fn be16(frames: &[i16]) -> Vec<u8> {
        frames.iter().flat_map(|s| s.to_be_bytes()).collect()
    }

    fn open(bytes: &[u8]) -> AiffAudioReader {
        AiffAudioReader::open(bytes, Limits::default()).expect("opens")
    }

    #[test]
    fn a_real_headers_numbers_are_read_as_the_muxer_meant_them() {
        // The first 54 bytes of the AIFF file FFmpeg wrote for the coverage gate's own
        // fixture: a `COMM` of 18 bytes, then an `SSND` whose header claims 19 208
        // bytes, followed by four samples of the ramp it starts with. Every constant
        // in it came from a writer.
        let mut bytes = vec![
            0x46, 0x4f, 0x52, 0x4d, 0x00, 0x00, 0x4b, 0x2e, 0x41, 0x49, 0x46, 0x46, 0x43, 0x4f,
            0x4d, 0x4d, 0x00, 0x00, 0x00, 0x12, 0x00, 0x01, 0x00, 0x00, 0x25, 0x80, 0x00, 0x10,
            0x40, 0x0e, 0xbb, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x53, 0x53, 0x4e, 0x44,
            0x00, 0x00, 0x4b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        bytes.extend_from_slice(&be16(&[0, 0x00eb, 0x01d6, 0x02c0]));
        let reader = open(&bytes);
        assert_eq!(reader.codec(), "A_PCM/INT/BIG");
        assert_eq!((reader.sample_rate(), reader.channels()), (48_000, 1));
        assert_eq!(reader.bits_per_sample(), 16);
        assert!(reader.extra_data().is_empty());
        assert_eq!(reader.audio_tracks()[0].label(), "1 ch 48000 Hz");
        let aiff = reader.aiff();
        // The header promises 9600 frames and the file carries four; four is what
        // plays, and the promise stays visible to a caller that wants to say so.
        assert_eq!((aiff.declared_frames, aiff.frames), (9_600, 4));
        assert_eq!(
            reader.duration(),
            Some(Duration::from_secs_f64(4.0 / 48_000.0))
        );
    }

    #[test]
    fn every_rate_a_muxer_writes_in_extended_form_is_read_exactly() {
        for (rate, bytes) in RATES {
            let aiff = file(
                b"AIFF",
                &[
                    chunk(b"COMM", &comm(1, 8, 16, *bytes)),
                    ssnd(&be16(&[0i16; 8])),
                ],
            );
            assert_eq!(
                Aiff::parse(&aiff, &Limits::default())
                    .expect("parses")
                    .pcm
                    .sample_rate,
                *rate,
                "the extended spelling of {rate}"
            );
        }
        // A significand with a bit set below where the rate's own integer ends is a
        // fractional rate, and this reader refuses to round one.
        let mut fractional = RATE_48_000;
        fractional[6] = 0x80;
        let bytes = file(
            b"AIFF",
            &[
                chunk(b"COMM", &comm(1, 8, 16, fractional)),
                ssnd(&be16(&[0i16; 8])),
            ],
        );
        assert_eq!(
            Aiff::parse(&bytes, &Limits::default())
                .err()
                .map(|error| error.to_string()),
            Some("a fractional sample rate".to_string())
        );
        // So is an exponent that puts the number anywhere near an audio rate.
        let bytes = file(
            b"AIFF",
            &[
                chunk(
                    b"COMM",
                    &comm(
                        1,
                        8,
                        16,
                        [0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
                    ),
                ),
                ssnd(&be16(&[0i16; 8])),
            ],
        );
        assert!(Aiff::parse(&bytes, &Limits::default()).is_err());
    }

    #[test]
    fn the_samples_are_twos_complement_big_endian() {
        // The bytes across the zero crossing of the AIFF FFmpeg writes. Read as
        // sign-magnitude, the third and fourth of them would be -32 661 and -33 110
        // rather than -107 and -342, and a 440 Hz tone would click at every crossing.
        let raw = [0x00, 0x00, 0x00, 0x81, 0xff, 0x95, 0xfe, 0xaa];
        let mut reader = open(&aiff(1, &raw));
        let mut decoder = PcmDecoder::int(
            reader.bits_per_sample(),
            true,
            reader.sample_rate(),
            reader.channels(),
        )
        .expect("the header's own geometry");
        let packet = reader.next_packet().expect("packet").expect("one packet");
        let audio = decoder
            .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
            .expect("convert")
            .expect("PCM always yields a packet");
        let scale = 32_768.0f32;
        assert_eq!(
            audio
                .data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                .collect::<Vec<_>>(),
            vec![0.0, 129.0 / scale, -107.0 / scale, -342.0 / scale]
        );
    }

    #[test]
    fn an_eight_bit_run_is_signed_which_is_not_what_a_wave_file_of_the_same_width_holds() {
        // The 54-byte head of the 8-bit mono AIFF FFmpeg wrote, followed by the first
        // twelve samples of its own run: a 440 Hz sine leaving zero upwards. Read the
        // way Wave 8-bit is read - unsigned, offset by half a scale - the same bytes
        // start at full negative deflection, which is a click rather than a note.
        let mut bytes = vec![
            0x46, 0x4f, 0x52, 0x4d, 0x00, 0x00, 0x03, 0xee, 0x41, 0x49, 0x46, 0x46, 0x43, 0x4f,
            0x4d, 0x4d, 0x00, 0x00, 0x00, 0x12, 0x00, 0x01, 0x00, 0x00, 0x03, 0xc0, 0x00, 0x08,
            0x40, 0x0e, 0xbb, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x53, 0x53, 0x4e, 0x44,
            0x00, 0x00, 0x03, 0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        bytes.extend_from_slice(&[
            0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x07, 0x08, 0x09,
        ]);
        let mut reader = open(&bytes);
        assert_eq!(reader.codec(), "A_PCM/INT/BIG");
        assert_eq!(
            (reader.sample_rate(), reader.bits_per_sample()),
            (48_000, 8)
        );
        assert_eq!(
            (reader.aiff().declared_frames, reader.aiff().frames),
            (960, 12)
        );
        let mut decoder =
            PcmDecoder::int(8, true, reader.sample_rate(), reader.channels()).expect("geometry");
        let packet = reader.next_packet().expect("packet").expect("one packet");
        let audio = decoder
            .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
            .expect("convert")
            .expect("PCM always yields a packet");
        // The values FFmpeg's decoder prints for these same bytes.
        let expected = [0i8, 0, 1, 2, 3, 4, 5, 6, 7, 7, 8, 9]
            .iter()
            .map(|&sample| f32::from(sample) / 128.0)
            .collect::<Vec<_>>();
        assert_eq!(
            audio
                .data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn an_aifc_header_names_its_byte_order_and_its_rejections() {
        // The 72-byte head of the AIFC file FFmpeg writes for little-endian 16-bit
        // PCM: `FVER` first, then a `COMM` of 24 bytes whose compression type is
        // `sowt`, then `SSND`.
        let head = vec![
            0x46, 0x4f, 0x52, 0x4d, 0x00, 0x00, 0x13, 0x00, 0x41, 0x49, 0x46, 0x43, 0x46, 0x56,
            0x45, 0x52, 0x00, 0x00, 0x00, 0x04, 0xa2, 0x80, 0x51, 0x40, 0x43, 0x4f, 0x4d, 0x4d,
            0x00, 0x00, 0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x09, 0x60, 0x00, 0x10, 0x40, 0x0e,
            0xbb, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x73, 0x6f, 0x77, 0x74, 0x00, 0x00,
            0x53, 0x53, 0x4e, 0x44, 0x00, 0x00, 0x12, 0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        let mut bytes = head.clone();
        bytes.extend_from_slice(&[0xeb, 0x00, 0xd6, 0x01]);
        let reader = open(&bytes);
        assert_eq!(reader.codec(), "A_PCM/INT/LIT");
        assert_eq!(
            (reader.sample_rate(), reader.bits_per_sample()),
            (48_000, 16)
        );
        assert_eq!(reader.aiff().frames, 2);

        // The same envelope naming `fl32` holds a big-endian float, and the player's
        // decoder reads only little-endian ones, so the file is refused rather than
        // played as denormals.
        let mut bytes = head.clone();
        bytes[50..54].copy_from_slice(b"fl32");
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x3b, 0xeb, 0x00, 0x00]);
        assert!(Aiff::parse(&bytes, &Limits::default()).is_err());
        // So is a width this decoder has no sample layout for, such as the 12-bit row
        // that some writers name in a header.
        let mut bytes = head.clone();
        bytes[39] = 0x0c;
        assert!(Aiff::parse(&bytes, &Limits::default()).is_err());
        // And a channel count of zero, which no header of either form may name.
        let bytes = file(
            b"AIFF",
            &[
                chunk(b"COMM", &comm(0, 8, 16, RATE_48_000)),
                ssnd(&be16(&[0i16; 8])),
            ],
        );
        assert!(Aiff::parse(&bytes, &Limits::default()).is_err());
    }

    #[test]
    fn the_sample_run_is_cut_into_windows_and_the_cursor_lands_on_one() {
        let raw = vec![0u8; (PACKET_FRAMES * 2 + 2) * 4];
        let mut reader = open(&aiff(2, &raw));
        assert_eq!(reader.aiff().pcm.block_align(), 4);
        let mut stamps = Vec::new();
        while let Some(packet) = reader.next_packet().expect("packet") {
            stamps.push((packet.pts, packet.duration, packet.data.len()));
        }
        assert_eq!(
            stamps,
            vec![(0, 2_048, 8_192), (2_048, 2_048, 8_192), (4_096, 2, 8)]
        );
        assert_eq!(reader.seek_to(2_500), 2_048);
        assert_eq!(reader.seek_to(-1), 0);
        assert_eq!(reader.seek_to(999_999), 4_096);
        assert!(reader.next_packet().expect("packet").is_some());
        assert!(reader.next_packet().expect("no more").is_none());
        reader.rewind();
        assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
    }

    #[test]
    fn a_header_that_is_not_an_audio_interchange_file_is_refused() {
        // A Wave file, whose envelope names a form this reader does not know.
        assert!(Aiff::parse(b"RIFF\0\0\0\0WAVEfmt ", &Limits::default()).is_err());
        // A `FORM` of another kind entirely: the same envelope carries ANIM pictures.
        assert!(Aiff::parse(&file(b"ANIM", &[]), &Limits::default()).is_err());
        // The geometry with no samples, and samples with no geometry.
        assert!(
            Aiff::parse(
                &file(b"AIFF", &[chunk(b"COMM", &comm(1, 4, 16, RATE_48_000))]),
                &Limits::default()
            )
            .is_err()
        );
        assert!(
            Aiff::parse(
                &file(b"AIFF", &[ssnd(&be16(&[1i16; 4]))]),
                &Limits::default()
            )
            .is_err()
        );
        // An `SSND` too short to hold its own offset, and one whose offset points past
        // the data it claims.
        assert!(
            Aiff::parse(
                &file(
                    b"AIFF",
                    &[
                        chunk(b"COMM", &comm(1, 4, 16, RATE_48_000)),
                        chunk(b"SSND", &[0, 0, 0, 0])
                    ]
                ),
                &Limits::default()
            )
            .is_err()
        );
        let mut body = (0x3000u32).to_be_bytes().to_vec();
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&be16(&[1i16; 4]));
        assert!(
            Aiff::parse(
                &file(
                    b"AIFF",
                    &[
                        chunk(b"COMM", &comm(1, 4, 16, RATE_48_000)),
                        chunk(b"SSND", &body)
                    ]
                ),
                &Limits::default()
            )
            .is_err()
        );
    }

    #[test]
    fn authoring_chunks_are_walked_past_and_odd_sizes_are_aligned() {
        // `MARK` and `INST` between the two chunks this reader needs, the first with an
        // odd length, so the walk has to step over its pad byte to find the next one.
        let bytes = file(
            b"AIFF",
            &[
                chunk(b"COMM", &comm(1, 4, 16, RATE_48_000)),
                chunk(b"MARK", b"some marker"),
                chunk(b"INST", b"cycle"),
                ssnd(&be16(&[1i16, 2, 3, 4])),
            ],
        );
        let mut reader = open(&bytes);
        assert_eq!(reader.aiff().frames, 4);
        assert_eq!(
            reader
                .next_packet()
                .expect("packet")
                .expect("one packet")
                .data
                .len(),
            8
        );
    }

    #[test]
    fn a_stereo_run_of_the_width_the_header_names_reaches_the_decoder() {
        // 24-bit stereo is what an AIFF file from a mastering tool holds, and the width
        // is packed three bytes to a sample rather than padded into a long word.
        let packed: Vec<u8> = [0x00eb00i32, -0x00eb00, 0x01d600, -0x01d600]
            .into_iter()
            .flat_map(|sample| sample.to_be_bytes()[1..].to_vec())
            .collect();
        let bytes = file(
            b"AIFF",
            &[chunk(b"COMM", &comm(2, 2, 24, RATE_48_000)), ssnd(&packed)],
        );
        let reader = open(&bytes);
        assert_eq!(reader.aiff().pcm.block_align(), 6);
        assert_eq!(reader.codec(), "A_PCM/INT/BIG");
        assert_eq!(reader.bits_per_sample(), 24);
    }

    /// MACE runs written for these tests, two of each coding. Their bytes are the Mac's own
    /// material - the 3-to-1 pair is the first 512 blocks of the shipped `mac3audio.mov` take,
    /// the 6-to-1 pair the first 512 bytes of `mjpega.mov` - and the `.s16` beside each is what
    /// this build's reference reads out of it.
    const MAC3_MONO: &[u8] = include_bytes!("../../../tests/fixtures/mace/mac3-mono.aiff");
    const MAC3_MONO_S16: &[u8] = include_bytes!("../../../tests/fixtures/mace/mac3-mono.s16");
    const MAC3_STEREO: &[u8] = include_bytes!("../../../tests/fixtures/mace/mac3-stereo.aiff");
    const MAC6_MONO: &[u8] = include_bytes!("../../../tests/fixtures/mace/mac6-mono.aiff");
    const MAC6_MONO_S16: &[u8] = include_bytes!("../../../tests/fixtures/mace/mac6-mono.s16");
    const MAC6_STEREO: &[u8] = include_bytes!("../../../tests/fixtures/mace/mac6-stereo.aiff");
    /// The 3-to-1 mono run with one byte standing over its last block.
    const MAC3_TAIL: &[u8] = include_bytes!("../../../tests/fixtures/mace/mac3-mono-tail.aiff");

    /// The `COMM` chunk's own fields stand where the muxer put them: the envelope's 12 bytes,
    /// the chunk's 8-byte header, then channels, frames, width and the 80-bit rate.
    fn comm_field(bytes: &[u8], bits: u16, frames: u32, rate: Option<[u8; 10]>) -> Vec<u8> {
        let mut out = bytes.to_vec();
        out[26..28].copy_from_slice(&bits.to_be_bytes());
        out[22..26].copy_from_slice(&frames.to_be_bytes());
        if let Some(rate) = rate {
            out[28..38].copy_from_slice(&rate);
        }
        out
    }

    #[test]
    fn the_fourcc_states_a_block_the_header_does_not() {
        // Two bytes a channel of 3-to-1 and one of 6-to-1, six samples for both: the width
        // field says neither, and the frames field - which the Mac's writer fills with blocks -
        // says the run is a sixth as long as it is. Measured over the four files.
        for (bytes, codec, channels, block, frames, claimed, rate) in [
            (
                MAC3_MONO, "mace3", 1u16, 2usize, 3_072usize, 512usize, 22_050,
            ),
            (MAC3_STEREO, "mace3", 2, 4, 3_072, 512, 22_050),
            (MAC6_MONO, "mace6", 1, 1, 3_072, 512, 8_000),
            (MAC6_STEREO, "mace6", 2, 2, 1_152, 192, 8_000),
        ] {
            let reader = open(bytes);
            let aiff = reader.aiff();
            assert_eq!(reader.codec(), codec);
            assert_eq!((reader.channels(), reader.sample_rate()), (channels, rate));
            assert_eq!(aiff.pcm.block_align(), block);
            assert_eq!(aiff.pcm.frames_per_block(), 6);
            assert_eq!(aiff.frames, frames, "{codec} {channels} ch");
            assert_eq!(
                aiff.declared_frames, claimed,
                "the claim survives as stated"
            );
            assert_eq!(
                reader.duration(),
                Some(Duration::from_secs_f64(
                    f64::from(frames as u32) / f64::from(rate as u32)
                ))
            );
        }
        // A 6-to-1 byte codes the same six samples a 3-to-1 pair does, so the two files of the
        // same length play at the same width and the 3-to-1 run of twice the bytes plays the
        // same number of frames.
        assert_eq!(open(MAC6_MONO).aiff().frames, open(MAC3_MONO).aiff().frames);
        assert_eq!(
            open(MAC6_MONO).aiff().data.len() * 2,
            open(MAC3_MONO).aiff().data.len()
        );
    }

    #[test]
    fn what_a_mace_header_lying_about_its_own_fields_changes_is_only_its_claim() {
        // Measured against this build's reference over a whole take: the width and the frame
        // count move nothing, because the coding's block comes from its fourcc and its length
        // from the payload. The claim is still carried, so a caller can say the file promised
        // something else.
        for bits in [1u16, 8, 16] {
            for frames in [4u32, 42_838, 999_999] {
                let reader = open(&comm_field(MAC3_MONO, bits, frames, None));
                assert_eq!(reader.aiff().frames, 3_072, "{bits}/{frames}");
                assert_eq!(reader.aiff().declared_frames, frames as usize);
                assert_eq!(reader.bits_per_sample(), bits);
            }
        }
    }

    #[test]
    fn the_run_is_cut_at_the_player_s_own_grain_and_a_byte_over_a_block_is_not_handed_over() {
        // 2 048 frames a window costs 341 blocks, which is 682 bytes of 3-to-1 mono and 341 of
        // 6-to-1: the pts steps by the window's own grain while the duration says what the
        // bytes really code - the same two numbers a GSM run is read at.
        let reader = open(MAC3_MONO);
        assert_eq!(reader.aiff().packets(), 2);
        assert_eq!(reader.aiff().packet(0).len(), 682);
        assert_eq!(reader.aiff().packet(1).len(), 342);
        let reader = open(MAC6_MONO);
        assert_eq!(reader.aiff().packets(), 2);
        assert_eq!(reader.aiff().packet(1).len(), 171);

        let mut reader = open(MAC3_MONO);
        let first = reader.next_packet().expect("packet").expect("first");
        assert_eq!((first.pts, first.duration), (0, 2_046));
        let second = reader.next_packet().expect("packet").expect("second");
        assert_eq!((second.pts, second.duration), (2_048, 1_026));
        assert!(reader.next_packet().expect("end").is_none());

        // The standing-over byte is the strictness the decoder would otherwise pay for with an
        // error: 1 025 bytes of the same run hand over exactly the 1 024 that are blocks, and
        // the decoder never sees a half block.
        let mut tail = open(MAC3_TAIL);
        let mut bytes = 0;
        while let Some(packet) = tail.next_packet().expect("packet") {
            assert_eq!(packet.data.len() % 2, 0);
            bytes += packet.data.len();
        }
        assert_eq!(bytes, 1_024);
    }

    #[test]
    fn the_windows_reach_the_decoder_as_one_continuous_run() {
        // Nothing about a packet start is state, so cutting the run at the player's grain has
        // to change no sample: what the two windows of either coding decode to is the
        // reference's whole answer for the file.
        for (bytes, oracle) in [(MAC6_MONO, MAC6_MONO_S16), (MAC3_MONO, MAC3_MONO_S16)] {
            let mut reader = open(bytes);
            let mut decoder = crate::codec::make_audio_decoder(
                reader.codec(),
                reader.extra_data(),
                reader.sample_rate(),
                reader.channels(),
                reader.bits_per_sample(),
            )
            .expect("MACE dispatch");
            let mut heard = Vec::new();
            while let Some(packet) = reader.next_packet().expect("packet") {
                let pcm = decoder
                    .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                    .expect("decodes")
                    .expect("samples");
                heard.extend(
                    pcm.data
                        .chunks_exact(4)
                        .map(|chunk| {
                            (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0)
                                .round() as i16
                        })
                        .collect::<Vec<i16>>(),
                );
            }
            let wanted: Vec<i16> = oracle
                .chunks_exact(2)
                .map(|chunk| i16::from_le_bytes(chunk.try_into().expect("two bytes")))
                .collect();
            assert_eq!(heard.len(), wanted.len());
            assert_eq!(
                heard,
                wanted,
                "{} windows must read as the whole take",
                reader.codec()
            );
        }
    }

    #[test]
    fn a_take_at_the_rate_the_mac_really_writes_is_a_measured_barrier_to_opening_it() {
        // Every MACE AIFF this build's reference ships states 22 254.545455932617 Hz in its
        // 80-bit field - the bytes below, read off `Bach6-1.aiff` - which is a 44 100 Hz track
        // at 8-to-6 sample-rate arithmetic rather than a rounding error, and ffprobe reports it
        // as 22 255. This reader works its rates out in integers and refuses a fractional one,
        // so those takes stay unopenable here; the coded runs the same files hold are the ones
        // the decoder is proved against in `codec::mace_decoder`.
        let bytes = comm_field(
            MAC3_MONO,
            8,
            512,
            Some([0x40, 0x0d, 0xad, 0xdd, 0x17, 0x46, 0x00, 0x00, 0x00, 0x00]),
        );
        let error = Aiff::parse(&bytes, &Limits::default())
            .err()
            .expect("a fractional rate is refused");
        assert!(
            error.to_string().contains("fractional sample rate"),
            "{error}"
        );
    }
}
