//! Audio playback out of the AVI interleaved run.
//!
//! [`crate::container::avi`] reads a file's framing and stops at the edge of
//! meaning: it finds the stream groups, the two headers each one states its
//! geometry in, and the records of the `movi` run, and it cross-checks the index
//! against the walk. What it deliberately leaves open is what a record *is*, and
//! this file is where that gets answered, because the answer belongs to the
//! coding rather than to the container.
//!
//! The two headers of one stream state the same facts in different words, so a
//! stream is only readable once the coding says which word means what. For the two
//! Microsoft-spelled ADPCM codings the pair is settled by the block. `dwScale` over
//! `dwRate` is how long one record plays, and a record of these codings is one
//! block, so the pair is the block's sample count over the sample rate the `strf`
//! record states beside it - stated in reduced terms, which is why the two are
//! compared as a product and not as two numbers: a 48 kHz Microsoft mono stream
//! whose block holds 2 036 samples writes 509 over 12 000. And `dwLength` counts
//! *records*, measured on files this build's reference muxer writes, where a
//! five-record stream states a length of five. The same muxer counts *samples* for
//! an uncompressed stream of the same shape, so the counting is a property of the
//! coding, and this reader limits itself to the codings whose counting it has
//! measured rather than inferring one from the other.
//!
//! The block's sample count is not read from the setup record either: it is
//! computed from the block's byte length, the channel count and the coding, by the
//! same arithmetic the decoder divides its bytes by, so a file whose headers state
//! a different number contradicts itself and the reader says so.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::codec::adpcm_decoder::block_frames;
use crate::container::avi::{Audio, Avi, Limits};
use crate::{Error, Result, invalid, unsupported};
use std::io::Read;
use std::time::Duration;

/// Wave format numbers of the two codings this reader frames records for: the
/// Microsoft ADPCM block and IMA's in a Wave-shaped block. Both state their
/// geometry in the `strf` record and carry the block's own predictor, so one
/// record is one block and nothing between them.
const ADPCM_MS: u16 = 0x0002;
const ADPCM_IMA_WAV: u16 = 0x0011;

/// Wave format numbers of codings this build decodes out of another container:
/// uncompressed integers and floats and the two G.711 tables, all of which the Wave
/// reader hands to an arm that already exists. A stream of one of these is missing
/// this reader's record framing rather than a decoder, and says so.
const FRAMED_ELSEWHERE: [u16; 4] = [0x0001, 0x0003, 0x0006, 0x0007];

/// The dispatch name of a coding this reader frames, or `None` for one it leaves to
/// another file's reader.
fn coding(tag: u16) -> Option<&'static str> {
    match tag {
        ADPCM_MS => Some("adpcm_ms"),
        ADPCM_IMA_WAV => Some("adpcm_ima_wav"),
        _ => None,
    }
}

/// What a stream this reader leaves alone states about itself, in the half of the
/// pipeline that is missing: a coding with an arm and no framing here is a reader's
/// gap, and one with no arm at all is a decoder's.
fn refused(tag: u16) -> Error {
    if FRAMED_ELSEWHERE.contains(&tag) {
        return invalid(&format!(
            "AVI audio coded as Wave format {tag:#06x} is a coding this reader frames no records for"
        ));
    }
    unsupported(&format!(
        "AVI audio coded as Wave format {tag:#06x} has no decoder arm here"
    ))
}

/// One audio stream, laid out for playing: the geometry a decoder is built from and
/// the arithmetic of the timeline its records make.
#[derive(Clone, Debug)]
struct Program {
    codec: &'static str,
    sample_rate: u32,
    channels: u16,
    bits: u16,
    /// Samples one block holds, as the coding's arithmetic and the header's own
    /// timing agree on. Every record of the run is one block, so this is also how
    /// far the timeline advances per packet.
    frames: u64,
    /// The whole `strf` body: the `WAVEFORMATEX` a decoder reads its block length
    /// and, for Microsoft's coding, its prediction tables out of.
    setup: Vec<u8>,
    /// This stream's place in the container's own list, whose records it reads.
    stream: usize,
    /// How many records the run holds, which is what the header's `dwLength` states.
    records: usize,
}

/// An AVI file's playable audio, with its records still in the file's bytes.
pub struct AviAudioReader {
    file: Vec<u8>,
    avi: Avi,
    programs: Vec<Program>,
    chosen: usize,
    packet: usize,
}

impl AviAudioReader {
    /// Open the first audio track a decoder exists for.
    pub fn open<R: Read>(reader: R, limits: Limits) -> Result<Self> {
        Self::open_at(reader, limits, 0)
    }

    /// Open the `nth` audio track this reader frames, counting in file order. An AVI
    /// file can hold several streams of one kind, so the key is worth having. A key
    /// the file's own list does not reach says how far it goes; a file with nothing
    /// to frame says what its first audio stream is coded as, which is the
    /// difference between a container the player cannot frame and a coding it cannot
    /// decode.
    pub fn open_at<R: Read>(mut reader: R, limits: Limits, nth: usize) -> Result<Self> {
        // The records point into the file's bytes rather than copying them, so the
        // reader keeps the whole buffer: an AVI file's index is a flat array after
        // the run, and the run it indexes is what a reader of records walks.
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        let avi = Avi::parse(&bytes, &limits)?;
        let mut programs = Vec::new();
        for (index, stream) in avi.audio().iter().enumerate() {
            let Some(codec) = coding(stream.format.tag) else {
                continue;
            };
            programs.push(laid_out(codec, index, stream)?);
        }
        if programs.is_empty() {
            return Err(match avi.audio().first() {
                Some(stream) => refused(stream.format.tag),
                None => invalid("AVI has no audio stream"),
            });
        }
        if nth >= programs.len() {
            return Err(invalid(&format!(
                "AVI has no audio track {nth}: it frames {} of them",
                programs.len()
            )));
        }
        Ok(Self {
            file: bytes,
            avi,
            programs,
            chosen: nth,
            packet: 0,
        })
    }

    fn program(&self) -> &Program {
        &self.programs[self.chosen]
    }

    /// Bytes of one of the chosen stream's records: exactly one block, by the check
    /// that laid the stream out.
    fn record(&self, at: usize) -> Option<&[u8]> {
        let stream = self.programs[self.chosen].stream;
        self.avi.audio()[stream].record(&self.file, at)
    }
}

/// Lay a stream out as a decoder's timeline, or say where its two headers and its
/// records fail to agree. Every check here compares one claim of the file with
/// another; none of them fills in a number the file left out.
fn laid_out(codec: &'static str, index: usize, stream: &Audio) -> Result<Program> {
    let format = stream.format;
    let block_bytes = u64::from(format.block_align);
    let rate = u64::from(format.sample_rate);
    let frames =
        block_frames(codec, block_bytes, u64::from(format.channels), u64::from(format.bits))
            .ok_or_else(|| {
                invalid(&format!(
                    "AVI audio stream {} states a block of {block_bytes} bytes at {} bits a sample, which is not {codec} geometry",
                    stream.number, format.bits
                ))
            })?;

    if stream.records.is_empty() {
        // The header and the run agree that there is nothing here, which is a stream
        // with no audio in it rather than one this reader cannot frame.
        return Err(invalid(&format!(
            "AVI audio stream {} states no records to read audio out of",
            stream.number
        )));
    }
    // For these codings one record is one block, and the header's `dwLength` counts
    // the records - the counting this build's reference muxer writes them with.
    if usize::try_from(stream.declared) != Ok(stream.records.len()) {
        return Err(invalid(&format!(
            "AVI audio stream {} states {} records and the run holds {}",
            stream.number,
            stream.declared,
            stream.records.len()
        )));
    }
    for (at, record) in stream.records.iter().enumerate() {
        if usize::try_from(block_bytes) != Ok(record.len) {
            return Err(invalid(&format!(
                "AVI audio stream {} record {at} holds {} bytes where its coding's block is {block_bytes}",
                stream.number, record.len
            )));
        }
    }
    // The pair in the stream header is one block's duration, and the block's sample
    // count over the sample rate is the same duration, so the two products are one
    // claim in two spellings. A writer reduces the pair or does not; either way the
    // products match.
    if u64::from(stream.scale) * rate != u64::from(stream.rate) * frames {
        return Err(invalid(&format!(
            "AVI audio stream {} plays a record for {} over {} where its block of {frames} samples at {rate} Hz plays {frames} over {rate}",
            stream.number, stream.scale, stream.rate
        )));
    }
    if stream.start != 0 {
        return Err(invalid(&format!(
            "AVI audio stream {} starts at {} on the timeline, which this reader does not offset its packets for",
            stream.number, stream.start
        )));
    }
    // How fast the run plays, as the format record states it, is the block's bytes
    // over the time it holds: a writer's own arithmetic, checked rather than believed.
    let stated = u64::from(format.bytes_per_second);
    let counted = block_bytes * rate / frames;
    if stated != counted {
        return Err(invalid(&format!(
            "AVI audio stream {} plays at {stated} bytes a second where its block and rate give {counted}",
            stream.number
        )));
    }
    Ok(Program {
        codec,
        sample_rate: format.sample_rate,
        channels: format.channels,
        bits: format.bits,
        frames,
        setup: stream.setup.clone(),
        stream: index,
        records: stream.records.len(),
    })
}

/// Packets are timestamped in samples of the stream's own rate, which is the unit
/// the block's duration and the decoder's output are both counted in.
impl AudioStream for AviAudioReader {
    fn codec(&self) -> &str {
        self.program().codec
    }

    fn timescale(&self) -> u32 {
        self.program().sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.program().sample_rate
    }

    fn channels(&self) -> u16 {
        self.program().channels
    }

    fn bits_per_sample(&self) -> u16 {
        self.program().bits
    }

    fn duration(&self) -> Option<Duration> {
        let program = self.program();
        let frames = program.frames * program.records as u64;
        let frames = i64::try_from(frames).unwrap_or(i64::MAX);
        (frames > 0)
            .then(|| Duration::from_secs_f64(frames as f64 / f64::from(program.sample_rate.max(1))))
    }

    fn extra_data(&self) -> &[u8] {
        &self.program().setup
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.programs
            .iter()
            .map(|program| AudioTrack {
                sample_rate: program.sample_rate,
                channels: program.channels,
                // A stream header has no title and no language to give a track, so
                // the label is the layout and the rate on its own.
                name: String::new(),
                language: String::new(),
            })
            .collect()
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let program = self.program();
        if self.packet >= program.records {
            return Ok(None);
        }
        let at = self.packet;
        let data = self
            .record(at)
            .ok_or_else(|| invalid("an AVI record's bytes are not in the file it was read from"))?
            .to_vec();
        let frames = i64::try_from(program.frames).unwrap_or(i64::MAX);
        let packet = EncodedPacket {
            data,
            pts: at as i64 * frames,
            duration: frames,
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let program = self.program();
        if program.records == 0 {
            return 0;
        }
        // Records are equal length, so the one a timestamp lands in is a division;
        // the last of them is as far as a seek can go, past the end included.
        let frames = program.frames.max(1);
        let wanted = pts.max(0) as u64 / frames;
        let at = wanted.min(program.records as u64 - 1) as usize;
        self.packet = at;
        i64::try_from(at as u64 * frames).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::{AviAudioReader, Error, Limits};
    use crate::audio::AudioStream;

    /// Four files of the same 0.2 s, 440 Hz sine at 48 kHz, one per coding and
    /// channel count, as this build's reference muxer writes them:
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.2 \
    ///        -ac 1 -c:a adpcm_ima_wav -strict -2 adpcm-ima-mono.avi
    /// ```
    ///
    /// with `-ac` and the coder varied. The whole tone comes out as five records of
    /// one block each for a mono stream and ten for a stereo one, and the four files
    /// regenerate byte for byte from the command above.
    const IMA_MONO: &[u8] = include_bytes!("../../../tests/fixtures/avi/adpcm-ima-mono.avi");
    const IMA_STEREO: &[u8] = include_bytes!("../../../tests/fixtures/avi/adpcm-ima-stereo.avi");
    const MS_MONO: &[u8] = include_bytes!("../../../tests/fixtures/avi/adpcm-ms-mono.avi");
    const MS_STEREO: &[u8] = include_bytes!("../../../tests/fixtures/avi/adpcm-ms-stereo.avi");

    /// Where a header record's body starts, given the fourcc that introduces it: a
    /// chunk's fields are read from eight bytes past its id, which is where the real
    /// files put them.
    fn body(bytes: &[u8], id: &[u8; 4]) -> usize {
        let at = bytes
            .windows(4)
            .position(|w| w == id)
            .unwrap_or_else(|| panic!("the file has no {id:?} record"));
        if id == b"strh" {
            assert_eq!(
                &bytes[at + 8..at + 12],
                b"auds",
                "only these files' audio headers are poked at"
            );
        }
        at + 8
    }

    /// Rewrite one of a real file's own header words, so a check is tested against
    /// bytes a writer actually produced rather than a layout invented for the test.
    fn poke(bytes: &mut [u8], id: &[u8; 4], offset: usize, value: u32) {
        let at = body(bytes, id) + offset;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn poke16(bytes: &mut [u8], id: &[u8; 4], offset: usize, value: u16) {
        let at = body(bytes, id) + offset;
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn said(bytes: &[u8], nth: usize) -> (Option<String>, bool) {
        match AviAudioReader::open_at(bytes, Limits::default(), nth) {
            Ok(_) => (None, true),
            Err(Error::Unsupported(coding)) => (Some(coding), false),
            Err(error) => (Some(error.to_string()), false),
        }
    }

    /// Every packet of a stream, with the lengths and timestamps the reader hands
    /// out beside them.
    fn timeline(stream: &mut AviAudioReader) -> Vec<(usize, i64, i64)> {
        let mut out = Vec::new();
        while let Some(packet) = stream.next_packet().expect("packet") {
            out.push((packet.data.len(), packet.pts, packet.duration));
        }
        out
    }

    #[test]
    fn a_wav_shaped_ima_stream_reads_as_its_blocks_and_their_timing() {
        let mut stream = AviAudioReader::open(IMA_MONO, Limits::default()).expect("IMA mono");
        assert_eq!(stream.codec(), "adpcm_ima_wav");
        assert_eq!(
            (
                stream.sample_rate(),
                stream.channels(),
                stream.bits_per_sample()
            ),
            (48_000, 1, 4)
        );
        // The timeline unit is the sample, so a block's timestamp is its index in
        // blocks times the samples one block holds.
        assert_eq!(
            timeline(&mut stream),
            (0..5)
                .map(|at| (1_024, i64::from(at) * 2_041, 2_041))
                .collect::<Vec<_>>()
        );
        // Five blocks of 2 041 samples at 48 kHz, which is what the file's own
        // timing adds up to.
        let seconds = stream.duration().expect("duration").as_secs_f64();
        assert!(
            (seconds - 10_205.0 / 48_000.0).abs() < 1e-9,
            "the stream runs {seconds} s"
        );
        // The setup record is the whole `strf` body, the `WAVEFORMATEX` a decoder
        // takes its block length and its own sample count out of.
        assert_eq!(stream.extra_data().len(), 20);
        assert_eq!(&stream.extra_data()[..2], &[0x11, 0x00]);
        assert_eq!(&stream.extra_data()[16..18], &[2, 0], "cbSize");
        assert_eq!(&stream.extra_data()[18..20], &[0xf9, 0x07], "2 041");
    }

    #[test]
    fn a_microsoft_stream_carries_its_prediction_tables_in_the_setup_record() {
        let mut stream = AviAudioReader::open(MS_STEREO, Limits::default()).expect("MS stereo");
        assert_eq!(stream.codec(), "adpcm_ms");
        assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
        // A stereo block of the same length holds half as many samples as a mono
        // one's, because the two channels share every code: 1 012 of them, so the
        // ten records of the file run to 10 120 samples.
        assert_eq!(
            timeline(&mut stream),
            (0..10)
                .map(|at| (1_024, i64::from(at) * 1_012, 1_012))
                .collect::<Vec<_>>()
        );
        assert_eq!(stream.extra_data().len(), 50);
        assert_eq!(&stream.extra_data()[..2], &[0x02, 0x00]);
        assert_eq!(&stream.extra_data()[16..18], &[0x20, 0x00], "cbSize");
        // The block's own sample count follows the size of what comes after the
        // record, and then the seven coefficient pairs, which a decoder cannot
        // rebuild from the coding alone.
        assert_eq!(&stream.extra_data()[18..20], &[0xf4, 0x03], "1 012");
        assert_eq!(
            &stream.extra_data()[20..24],
            &[0x07, 0x00, 0x00, 0x01],
            "the first coefficient pair"
        );
    }

    /// The reason the block's sample count comes from one arithmetic in both this
    /// reader and the decoder: a reader that handed over a block the decoder sliced
    /// differently would still run, and produce a track that slowly changed pitch.
    #[test]
    fn the_records_the_reader_hands_over_are_the_blocks_the_decoder_wants() {
        for (bytes, codec, samples) in [
            (IMA_MONO, "adpcm_ima_wav", 10_205usize),
            (MS_MONO, "adpcm_ms", 10_180),
            (MS_STEREO, "adpcm_ms", 10_120),
        ] {
            let mut stream = AviAudioReader::open(bytes, Limits::default()).expect("stream");
            let mut decoder = crate::codec::make_audio_decoder(
                stream.codec(),
                stream.extra_data(),
                stream.sample_rate(),
                stream.channels(),
                stream.bits_per_sample(),
            )
            .expect("decoder");
            let mut heard = 0usize;
            let mut packets = 0usize;
            while let Some(packet) = stream.next_packet().expect("packet") {
                packets += 1;
                let pcm = decoder
                    .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                    .expect("decode")
                    .expect("every block of a real file decodes");
                assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 48_000));
                heard += pcm.data.len() / (size_of::<f32>() * usize::from(stream.channels()));
            }
            assert_eq!(packets, stream.program().records, "{codec} lost a record");
            assert_eq!(heard, samples, "{codec} decoded {heard} samples");
        }
    }

    #[test]
    fn seeking_lands_on_the_block_a_timestamp_falls_in() {
        let mut stream = AviAudioReader::open(IMA_MONO, Limits::default()).expect("IMA mono");
        // The middle of the third block, and the start of the fourth.
        assert_eq!(stream.seek_to(5_000), 4_082);
        assert_eq!(stream.seek_to(6_123), 6_123);
        // Past the end the cursor stops at the last block rather than running off.
        assert_eq!(stream.seek_to(100_000), 4 * 2_041);
        let rest = timeline(&mut stream);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0], (1_024, 8_164, 2_041));
        stream.rewind();
        assert_eq!(timeline(&mut stream).len(), 5);
    }

    #[test]
    fn a_second_track_key_has_nothing_to_land_on() {
        let (said, opened) = said(IMA_MONO, 1);
        assert!(!opened);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI has no audio track 1: it frames 1 of them"
        );
        // The file does list one track, and that is the list a track key counts.
        let stream = AviAudioReader::open(IMA_MONO, Limits::default()).expect("IMA mono");
        assert_eq!(
            stream.audio_tracks(),
            vec![crate::audio::AudioTrack {
                sample_rate: 48_000,
                channels: 1,
                name: String::new(),
                language: String::new(),
            }]
        );
    }

    #[test]
    fn a_length_that_does_not_count_the_run_is_a_disagreement() {
        // `dwLength` counts records for these codings, which is what the muxer's own
        // files state; a length one past the run is the file doubting its own index.
        let mut bytes = IMA_STEREO.to_vec();
        poke(&mut bytes, b"strh", 32, 11);
        let (said, _) = said(&bytes, 0);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio stream 0 states 11 records and the run holds 10"
        );
    }

    #[test]
    fn a_record_that_is_not_one_block_of_its_coding_is_refused() {
        // Halving the stated block length leaves the records as they are, so the
        // stream's own geometry no longer matches the bytes it holds.
        let mut bytes = IMA_MONO.to_vec();
        poke16(&mut bytes, b"strf", 12, 512);
        let (said, _) = said(&bytes, 0);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio stream 0 record 0 holds 1024 bytes where its coding's block is 512"
        );
    }

    #[test]
    fn a_timeline_that_is_not_the_blocks_duration_is_refused() {
        // The pair in the stream header is one block's time in whatever terms the
        // writer chose - these files reduce it, and 2 041 over 48 000 does not
        // reduce - so a pair that states a different length is refused however
        // strongly it is written.
        let mut bytes = IMA_MONO.to_vec();
        poke(&mut bytes, b"strh", 20, 2_042);
        let (said, _) = said(&bytes, 0);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio stream 0 plays a record for 2042 over 48000 where its block of 2041 samples at 48000 Hz plays 2041 over 48000"
        );
        // The reduced spelling of the same duration is the one a real Microsoft
        // mono stream uses, and it reads.
        let mut bytes = MS_MONO.to_vec();
        poke(&mut bytes, b"strh", 20, 2_036);
        poke(&mut bytes, b"strh", 24, 48_000);
        AviAudioReader::open(&bytes[..], Limits::default()).expect("the unreduced pair");
    }

    #[test]
    fn a_stream_that_starts_later_is_not_offset_for() {
        let mut bytes = MS_MONO.to_vec();
        poke(&mut bytes, b"strh", 28, 2_036);
        let (said, _) = said(&bytes, 0);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio stream 0 starts at 2036 on the timeline, which this reader does not offset its packets for"
        );
    }

    #[test]
    fn a_byte_rate_the_block_and_rate_do_not_give_is_refused() {
        let mut bytes = IMA_MONO.to_vec();
        poke(&mut bytes, b"strf", 8, 24_083);
        let (said, _) = said(&bytes, 0);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio stream 0 plays at 24083 bytes a second where its block and rate give 24082"
        );
    }

    #[test]
    fn a_geometry_the_coding_does_not_have_is_refused_not_guessed_at() {
        // Sixteen bits a sample with a Microsoft ADPCM number is one of the two
        // statements wrong, and the reader cannot say which.
        let mut bytes = MS_MONO.to_vec();
        poke16(&mut bytes, b"strf", 14, 16);
        let (said, _) = said(&bytes, 0);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio stream 0 states a block of 1024 bytes at 16 bits a sample, which is not adpcm_ms geometry"
        );
    }

    #[test]
    fn a_coding_with_an_arm_elsewhere_is_a_framing_gap_not_a_missing_decoder() {
        // Uncompressed 16-bit integers: this build decodes them out of a Wave file,
        // so the number says which half of the player is missing.
        let mut bytes = IMA_MONO.to_vec();
        poke16(&mut bytes, b"strf", 0, 1);
        let (said, opened) = said(&bytes, 0);
        assert!(!opened);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio coded as Wave format 0x0001 is a coding this reader frames no records for"
        );
    }

    #[test]
    fn a_coding_with_no_arm_is_named_by_the_number_the_file_states() {
        // Intel Music Codec: a Wave number this build has no decoder for at all.
        let mut bytes = IMA_MONO.to_vec();
        poke16(&mut bytes, b"strf", 0, 0x0041);
        let (said, opened) = said(&bytes, 0);
        assert!(!opened);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio coded as Wave format 0x0041 has no decoder arm here"
        );
    }
}
