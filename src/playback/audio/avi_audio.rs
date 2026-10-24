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
//! Microsoft-spelled ADPCM codings the unit is one fixed-size block. `dwScale`
//! over `dwRate` is that block's sample count over the sample rate in `strf`.
//! A chunk may group several blocks; the reader exposes each as an independent
//! decoder packet, and `dwLength` is checked against the total block count.
//! Chunk indexing remains the container's job. The logical block view preserves
//! packet clocks and block-rounded seek regardless of how chunks group samples.
//!
//! The block's sample count is not read from the setup record either: it is
//! computed from the block's byte length, the channel count and the coding, by the
//! same arithmetic the decoder divides its bytes by, so a file whose headers state
//! a different number contradicts itself and the reader says so.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::codec::adpcm_decoder::block_frames;
use crate::container::avi::{Audio, Avi, Limits, Record};
use crate::{Error, Result, invalid, unsupported};
use std::io::Read;
use std::time::Duration;

/// Wave format numbers of the two codings this reader frames records for: the
/// Microsoft ADPCM block and IMA's in a Wave-shaped block. Both state their
/// geometry in the `strf` record and carry the block's own predictor, so one
/// record is one block and nothing between them.
const ADPCM_MS: u16 = 0x0002;
const ADPCM_IMA_WAV: u16 = 0x0011;

/// Bound linear PCM packets independently of the source chunk size.
const PCM_PACKET_FRAMES: usize = 2_048;

/// Native decoder for one supported Wave format and sample width.
fn coding(tag: u16, bits: u16) -> Option<&'static str> {
    match (tag, bits) {
        (0x0001, 8 | 16 | 24 | 32) => Some("A_PCM/INT/LIT"),
        (0x0003, 32 | 64) => Some("A_PCM/FLOAT/IEEE"),
        (6, _) => Some("pcm_alaw"),
        (7, _) => Some("pcm_mulaw"),
        (ADPCM_MS, _) => Some("adpcm_ms"),
        (ADPCM_IMA_WAV, _) => Some("adpcm_ima_wav"),
        _ => None,
    }
}

fn refused(tag: u16, bits: u16) -> Error {
    if matches!(tag, 0x0001 | 0x0003) {
        return unsupported(&format!(
            "AVI PCM Wave format {tag:#06x} at {bits} bits is unsupported"
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
    /// timing agree on. ADPCM packets each carry one independently coded block.
    frames: u64,
    /// The whole `strf` body: the `WAVEFORMATEX` a decoder reads its block length
    /// and, for Microsoft's coding, its prediction tables out of.
    setup: Vec<u8>,
    /// This stream's place in the container's own list, whose records it reads.
    stream: usize,
    /// Indexed ADPCM blocks or PCM/G.711 records. PCM records may span windows.
    records: usize,
    /// Encoded bytes per frame for PCM/G.711 record spans;unused for ADPCM.
    input_frame_bytes: usize,
    /// PCM spans are streamed in bounded windows;G.711 keeps its record grain.
    packet_frames: Option<usize>,
    // Only grouped ADPCM chunks need a separate logical-block view.
    block_records: Option<Vec<Record>>,
    samples: Option<Vec<(u64,u64)>>,
}

/// An AVI file's playable audio, with its records still in the file's bytes.
pub struct AviAudioReader {
    file: Vec<u8>,
    avi: Avi,
    programs: Vec<Program>,
    chosen: usize,
    packet: usize,
    frame_skip: usize,
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
        reader.by_ref().take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
        let avi = Avi::parse(&bytes, &limits)?;
        let mut programs = Vec::new();
        for (index, stream) in avi.audio().iter().enumerate() {
            let Some(codec) = coding(stream.format.tag, stream.format.bits) else {
                continue;
            };
            programs.push(laid_out(codec, index, stream, &limits)?);
        }
        if programs.is_empty() {
            return Err(match avi.audio().first() {
                Some(stream) => refused(stream.format.tag, stream.format.bits),
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
            frame_skip: 0,
        })
    }

    fn program(&self) -> &Program {
        &self.programs[self.chosen]
    }

    /// One indexed ADPCM block or PCM/G.711 record;PCM is windowed afterwards.
    fn record(&self, at: usize) -> Option<&[u8]> {
        let program = self.program();
        if let Some(records) = &program.block_records {
            let record = records.get(at)?;
            self.file.get(record.start..record.start.checked_add(record.len)?)
        } else {
            self.avi.audio()[program.stream].record(&self.file, at)
        }
    }
}

/// Lay a stream out as a decoder's timeline, or say where its two headers and its
/// records fail to agree. Every check here compares one claim of the file with
/// another; none of them fills in a number the file left out.
fn laid_out(codec: &'static str, index: usize, stream: &Audio, limits: &Limits) -> Result<Program> {
    if matches!(codec,"pcm_alaw"|"pcm_mulaw") {return laid_out_g711(codec,index,stream);}
    if matches!(stream.format.tag, 0x0001 | 0x0003) {
        return laid_out_pcm(codec, index, stream);
    }
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
    // A fixed-size AVI sample is one ADPCM block. Several samples may share
    // a physical chunk, so dwLength counts blocks rather than chunk headers.
    let block_size = usize::from(format.block_align);
    let mut count = 0usize;
    let mut grouped = false;
    for (at, record) in stream.records.iter().enumerate() {
        if record.len == 0 || record.len % block_size != 0 {
            return Err(invalid(&format!(
                "AVI audio stream {} record {at} holds {} bytes and does not contain whole ADPCM blocks of {block_bytes} bytes",
                stream.number, record.len
            )));
        }
        let blocks = record.len / block_size;
        grouped |= blocks > 1;
        count = count.checked_add(blocks)
            .ok_or_else(|| invalid("AVI ADPCM block count overflow"))?;
        if count > limits.records {
            return Err(invalid(&format!(
                "AVI audio stream {} expands to more than {} ADPCM blocks",
                stream.number, limits.records
            )));
        }
    }
    if usize::try_from(stream.declared) != Ok(count) {
        return Err(invalid(&format!(
            "AVI audio stream {} states {} ADPCM blocks and the run holds {count}",
            stream.number, stream.declared
        )));
    }
    // The pair in the stream header is one block's duration, and the block's sample
    // count over the sample rate is the same duration, so the two products are one
    // claim in two spellings. A writer reduces the pair or does not; either way the
    // products match.
    if u64::from(stream.scale) * rate != u64::from(stream.rate) * frames {
        return Err(invalid(&format!(
            "AVI audio stream {} plays a block for {} over {} where its block of {frames} samples at {rate} Hz plays {frames} over {rate}",
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
    let block_records = if grouped {
        let mut records = Vec::new();
        records.try_reserve_exact(count)
            .map_err(|_| invalid("AVI ADPCM block index allocation failed"))?;
        for record in &stream.records {
            for offset in (0..record.len).step_by(block_size) {
                records.push(Record {
                    start: record.start.checked_add(offset)
                        .ok_or_else(|| invalid("AVI ADPCM block offset overflow"))?,
                    len: block_size,
                });
            }
        }
        Some(records)
    } else {
        None
    };
    Ok(Program {
        codec,
        sample_rate: format.sample_rate,
        channels: format.channels,
        bits: format.bits,
        frames,
        setup: stream.setup.clone(),
        stream: index,
        records: count,
        input_frame_bytes: 0,
        packet_frames: None,
        block_records,
        samples: None,
    })
}

fn laid_out_pcm(codec: &'static str, index: usize, stream: &Audio) -> Result<Program> {
    let f = stream.format;
    let stride = u64::from(f.channels) * u64::from(f.bits / 8);
    if stride == 0 || u64::from(f.block_align) != stride
        || u64::from(f.bytes_per_second) != stride * u64::from(f.sample_rate)
    {
        return Err(invalid("inconsistent AVI PCM sample geometry"));
    }
    if stream.scale == 0
        || u64::from(stream.scale) * u64::from(f.sample_rate) != u64::from(stream.rate)
    {
        return Err(invalid("AVI PCM clock disagrees with its sample rate"));
    }
    let stride = usize::from(f.block_align);
    let mut samples = Vec::new();
    samples.try_reserve_exact(stream.records.len())
        .map_err(|_| invalid("AVI PCM sample index allocation failed"))?;
    let start = u64::from(stream.start);
    let mut cursor = start;
    for record in &stream.records {
        if record.len == 0 || record.len % stride != 0 {
            return Err(invalid("AVI PCM record ends in an incomplete channel frame"));
        }
        let frames = u64::try_from(record.len / stride)
            .map_err(|_| invalid("AVI PCM frame count overflow"))?;
        samples.push((cursor, frames));
        cursor = cursor.checked_add(frames)
            .ok_or_else(|| invalid("AVI PCM clock overflow"))?;
        if cursor > i64::MAX as u64 {
            return Err(invalid("AVI PCM clock exceeds the packet time range"));
        }
    }
    if samples.is_empty() || cursor - start != u64::from(stream.declared) {
        return Err(invalid("AVI PCM declared frame count disagrees with records"));
    }
    Ok(Program {
        codec, sample_rate: f.sample_rate, channels: f.channels, bits: f.bits,
        frames: 1, setup: stream.setup.clone(), stream: index, records: samples.len(),
        input_frame_bytes: stride, packet_frames: Some(PCM_PACKET_FRAMES),
        block_records: None, samples: Some(samples),
    })
}

fn laid_out_g711(codec: &'static str,index:usize,stream:&Audio)->Result<Program>{
    let f=stream.format;
    if f.bits!=8 || f.block_align!=f.channels || u64::from(f.sample_rate)*u64::from(f.channels)!=u64::from(f.bytes_per_second){return Err(invalid("inconsistent AVI G.711 sample geometry"));}
    if stream.scale==0 || u64::from(stream.scale)*u64::from(f.sample_rate)!=u64::from(stream.rate){return Err(invalid("AVI G.711 clock disagrees with its sample rate"));}
    let mut samples=Vec::new();let mut cursor=u64::from(stream.start);
    for record in &stream.records {
        if record.len==0||record.len%usize::from(f.channels)!=0{return Err(invalid("AVI G.711 record ends in an incomplete channel frame"));}
        let frames=(record.len/usize::from(f.channels))as u64;samples.push((cursor,frames));
        cursor=cursor.checked_add(frames).ok_or_else(||invalid("AVI G.711 clock overflow"))?;
    }
    if samples.is_empty() || cursor-u64::from(stream.start)!=u64::from(stream.declared){return Err(invalid("AVI G.711 declared frame count disagrees with records"));}
    Ok(Program{codec,sample_rate:f.sample_rate,channels:f.channels,bits:8,frames:1,setup:stream.setup.clone(),stream:index,records:samples.len(),input_frame_bytes:usize::from(f.channels),packet_frames:None,block_records:None,samples:Some(samples)})
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
        let frames = program.samples.as_ref().map_or(program.frames*program.records as u64,|s|s.last().map_or(0,|&(at,n)|at+n));
        let frames = i64::try_from(frames).unwrap_or(i64::MAX);
        (frames > 0)
            .then(|| crate::audio::sample_duration(frames as u64, program.sample_rate.max(1)))
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
        if let Some(samples) = &program.samples {
            let (start, frames) = samples[at];
            let skip = self.frame_skip;
            let frames = usize::try_from(frames)
                .map_err(|_| invalid("AVI sample span exceeds addressable memory"))?;
            let left = frames.checked_sub(skip)
                .ok_or_else(|| invalid("AVI sample cursor exceeds its record"))?;
            let take = left.min(program.packet_frames.unwrap_or(usize::MAX));
            if take == 0 {
                return Err(invalid("AVI sample cursor holds no frames"));
            }
            let stride = program.input_frame_bytes;
            let data = self.record(at).ok_or_else(|| invalid("AVI sample record missing"))?
                [skip * stride..(skip + take) * stride].to_vec();
            if skip + take == frames {
                self.packet += 1;
                self.frame_skip = 0;
            } else {
                self.frame_skip = skip + take;
            }
            return Ok(Some(EncodedPacket {
                data,
                pts: (start + skip as u64) as i64,
                duration: take as i64,
            }));
        }
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
        self.packet = 0;self.frame_skip=0;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let program = self.program();
        if let Some(samples)=&program.samples {
            let start=samples.first().map_or(0,|&(at,_)|at);
            let end=samples.last().map_or(0,|&(at,n)|at+n);
            let target=(pts.max(0)as u64).clamp(start,end);
            let packet=samples.iter().position(|&(at,n)|at+n>target).unwrap_or(samples.len());
            let skip=samples.get(packet).map_or(0,|&(at,_)|target.saturating_sub(at)as usize);
            self.packet=packet;self.frame_skip=skip;return target as i64;
        }
        let end = program.frames.saturating_mul(program.records as u64);
        if pts.max(0) as u64 >= end {
            self.packet = program.records;
            return i64::try_from(end).unwrap_or(i64::MAX);
        }
        // Inside the stream, land at the preceding independently coded block.
        // At its end there is no block left to present.
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
        // At/past the end there is no final block to replay.
        assert_eq!(stream.seek_to(100_000), 5 * 2_041);
        let rest = timeline(&mut stream);
        assert!(rest.is_empty());
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
        // These reference files each put one block in a chunk. The declared
        // block count must agree with the payload even when chunks group blocks.
        let mut bytes = IMA_STEREO.to_vec();
        poke(&mut bytes, b"strh", 32, 11);
        let (said, _) = said(&bytes, 0);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio stream 0 states 11 ADPCM blocks and the run holds 10"
        );
    }

    #[test]
    fn a_record_that_contains_a_partial_block_is_refused() {
        // A chunk may contain whole blocks, but not an incomplete final block.
        let mut bytes = IMA_MONO.to_vec();
        poke16(&mut bytes, b"strf", 12, 1016);
        let (said, _) = said(&bytes, 0);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI audio stream 0 record 0 holds 1024 bytes and does not contain whole ADPCM blocks of 1016 bytes"
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
            "AVI audio stream 0 plays a block for 2042 over 48000 where its block of 2041 samples at 48000 Hz plays 2041 over 48000"
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
    fn an_unsupported_pcm_width_is_named_without_claiming_no_decoder() {
        // This ADPCM reference still states four bits per sample after changing
        // its tag. That is not a supported linear PCM width.
        let mut bytes = IMA_MONO.to_vec();
        poke16(&mut bytes, b"strf", 0, 1);
        let (said, opened) = said(&bytes, 0);
        assert!(!opened);
        assert_eq!(
            said.expect("a reason").as_str(),
            "AVI PCM Wave format 0x0001 at 4 bits is unsupported"
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
