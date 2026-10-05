//! The Audio Video Interleave wrapper, as far as one audio track of it needs it.
//!
//! AVI is a RIFF run of chunks in which almost everything a player wants is stated
//! twice: the stream headers in `hdrl` promise how many records a stream holds and at
//! what rate they advance the timeline, `strf` repeats the geometry the Wave registry
//! spells as a `WAVEFORMATEX`, the interleaved records carry the bytes, and `idx1`
//! states the whole record list once more. No single one of those statements is needed
//! (the records alone are enough to play a file), and that is the point: a reader that
//! believes one field cannot tell a header from a mistake, while a reader that checks
//! them against each other refuses the file that only looks like one. So this module
//! indexes the records and then holds the file's own restatement of that list against
//! them, and a disagreement is an error rather than a guess.
//!
//! The claims that depend on what a stream codes are left to the reader that names the
//! decoder. `dwLength` is the clearest of them: it counts a stream's samples, and a
//! "sample" is one block of a block-coded track and one frame of an uncompressed one.
//! Measured on this machine's muxer, a fifth of a second of ADPCM states five records
//! and a `dwLength` of 5, while the same length of PCM states ten records and a
//! `dwLength` of 9 600 - one field counting two different things in one file type.
//!
//! Records are found by their ids, which are the stream's decimal number and the two
//! letters saying what kind of data they hold: audio records are `00wb`, `01wb` and so
//! on, and a stream's number is its place among the `strl` groups in file order, so a
//! file whose second stream is audio numbers its records `01wb` even when the first is
//! a picture. Records may sit directly in `movi` or inside nested lists, which is how
//! an interleaved file groups one moment of every stream together, and both spellings
//! are read here.
//!
//! What a record means is another matter, and this module keeps its hands off it: it
//! hands over the `strf` bytes as setup data, the `dwScale` over `dwRate` pair the
//! header states beside them, and the record list, leaving the coding's own arithmetic
//! to the reader that names a decoder for it.

use crate::{Result, invalid};

/// What a reader may take in: how large a file, how many streams, how many records of
/// one stream and how wide a track it may agree to lay out.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest file accepted.
    pub file_bytes: usize,
    /// Most records one audio stream may hold.
    pub records: usize,
    /// Most streams one file may describe.
    pub streams: usize,
    /// Most channels one track may name.
    pub channels: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // The ceiling the other RIFF reader in this player works to, and for the
            // same reason: the whole file has to be taken in, because a stream's records
            // are scattered through it behind every other stream's.
            file_bytes: 128 << 20,
            // The list, not the audio, is what this bounds: two hours of the narrowest
            // block this player reads - 1 024-byte Microsoft ADPCM at 8 kHz - is some
            // 3 400 records, and a file whose records alone outnumber a million is a
            // recording longer than anything this limit is set for.
            records: 1 << 20,
            // Record ids name a stream with two decimal digits, so a hundredth stream
            // could not be reached by them at all.
            streams: 99,
            channels: 32,
        }
    }
}

/// The `strh` word that says which kind of stream a group describes.
const STREAM_TYPE: usize = 0;
/// How many samples the stream holds. What one of them is for a given coding is the
/// reader's question, not this one; see the note on `dwLength` above.
const STREAM_LENGTH: usize = 32;
/// The unit one record advances the timeline by, and the unit it is counted in.
const STREAM_SCALE: usize = 20;
const STREAM_RATE: usize = 24;
/// Where the stream begins on the timeline, in the same unit as `STREAM_RATE`.
const STREAM_START: usize = 28;
/// The shortest header that still states all of those. Writers add the frame rectangle
/// after them and this reader stops at the ten words.
const STREAM_HEADER: usize = 48;
/// A stream group describes audio when its type says `auds`.
const AUDIO: [u8; 4] = *b"auds";
/// The two letters an audio record's id carries. Measured on this machine's muxer, and
/// the only spelling it writes; a file that groups its audio under another pair of
/// letters reads as a stream holding no records, which is a refusal rather than a guess
/// at what the other pairs mean.
const AUDIO_LETTERS: [u8; 2] = *b"wb";
/// Offsets inside a `WAVEFORMATEX`, which is what `strf` holds for an audio stream.
const FORMAT_TAG: usize = 0;
const FORMAT_CHANNELS: usize = 2;
const FORMAT_RATE: usize = 4;
const FORMAT_BYTES_PER_SECOND: usize = 8;
const FORMAT_BLOCK_ALIGN: usize = 12;
const FORMAT_BITS: usize = 14;
/// The plain `WAVEFORMATEX`, which is all an uncompressed Wave record writes. The
/// extension count at byte 16 and whatever follows it belong to a coding.
const FORMAT_HEADER: usize = 16;
/// The nesting followed inside `movi`. Deeper than this is not a grouping any writer
/// produces, and following it unbounded is a stack the file controls.
const NESTING: usize = 8;
/// The bytes of one `idx1` entry: the id, the flags, the offset and the length.
const INDEX_ENTRY: usize = 16;
/// The offset an index entry's length word sits at, and its id's number of bytes.
const INDEX_LENGTH: usize = 12;
/// Where an entry's own offset word sits.
const INDEX_OFFSET: usize = 8;
/// How far past a list's own type word its first chunk header sits, which is also
/// where an `idx1` offset starts counting from: the first record of `movi` is at
/// offset four.
const INDEX_BASE: usize = 4;
/// The bytes of a chunk header an index offset points at rather than through: the id
/// and the length, which is why a record's bytes start this far past the entry's
/// offset. Measured on this machine's muxer, whose first audio record sits at index
/// offset 4 and at file offset 12 from the list's type word.
const CHUNK_HEADER: usize = 8;
/// Where the main header counts the streams it promises.
const MAIN_STREAMS: usize = 24;

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn id_at(bytes: &[u8], at: usize) -> Option<[u8; 4]> {
    bytes.get(at..at + 4)?.try_into().ok()
}

/// One chunk of a RIFF run: its id and where its bytes sit in the file. `end` is the
/// chunk's own claim cut short at the end of the run it sits in, and `truncated` says
/// when that happened: what a file claims about itself and what it carries are compared
/// by the caller that has to know, and walking the bytes either way is not such a
/// comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chunk {
    id: [u8; 4],
    start: usize,
    end: usize,
    truncated: bool,
}

/// The chunk that begins at `at` in the run ending at `stop`, and the offset the next
/// one begins at. Chunks are word aligned, so an odd length brings a pad byte along
/// that belongs to the run rather than to the next chunk.
fn chunk_at(bytes: &[u8], at: usize, stop: usize) -> Option<(Chunk, usize)> {
    if at.checked_add(8)? > stop {
        return None;
    }
    let id = id_at(bytes, at)?;
    let declared = usize::try_from(u32_at(bytes, at + 4)?).ok()?;
    let start = at + 8;
    let claimed = start.checked_add(declared)?;
    let chunk = Chunk {
        id,
        start,
        end: claimed.min(stop),
        truncated: claimed > stop,
    };
    let next = claimed
        .saturating_add(declared & 1)
        .min(bytes.len())
        .min(stop.max(at + 8));
    Some((chunk, next.max(at + 8)))
}

/// The chunks of a run of the file, from `from` to `stop`.
struct Chunks<'a> {
    bytes: &'a [u8],
    at: usize,
    stop: usize,
}

impl<'a> Chunks<'a> {
    fn range(bytes: &'a [u8], from: usize, stop: usize) -> Self {
        Self {
            bytes,
            at: from,
            stop,
        }
    }

    /// The chunks of a `LIST`: its body, past the four bytes naming the kind of list.
    fn list(bytes: &'a [u8], chunk: &Chunk) -> Self {
        Self::range(bytes, chunk.start + 4, chunk.end)
    }
}

impl Iterator for Chunks<'_> {
    type Item = Chunk;

    fn next(&mut self) -> Option<Chunk> {
        let (chunk, at) = chunk_at(self.bytes, self.at, self.stop)?;
        self.at = at;
        Some(chunk)
    }
}

/// The geometry an audio stream's `strf` states, read as the Wave registry spells it.
/// Only the fields every one of these records has are kept; what follows them belongs
/// to a coding and travels on in [`Audio::setup`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// The Wave format number, which is what says which coding the records hold.
    pub tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub bytes_per_second: u32,
    /// How many bytes one block of the coding is, as the record states it.
    pub block_align: u16,
    pub bits: u16,
}

/// One record of the interleaved run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// Where its bytes start in the file.
    pub start: usize,
    /// How many bytes it holds.
    pub len: usize,
}

/// An audio stream of the file: what its two headers state, and the records that turned
/// out to hold it.
#[derive(Clone, Debug)]
pub struct Audio {
    /// The stream's place among all of them, which is what its record ids spell.
    pub number: usize,
    /// `dwLength`, as the header states it.
    pub declared: u32,
    /// Where the stream starts on the timeline, in the same unit as `rate`.
    pub start: u32,
    /// `dwScale` over `dwRate`: the unit one record advances the timeline by, in
    /// seconds. The pair is kept as written because writers differ about what it counts
    /// and only the coding can say which of them a file meant.
    pub scale: u32,
    pub rate: u32,
    pub format: Format,
    /// The whole `strf` body: the `WAVEFORMATEX` a decoder reads its own block geometry
    /// - and for Microsoft's ADPCM its prediction tables - out of.
    pub setup: Vec<u8>,
    pub records: Vec<Record>,
}

impl Audio {
    /// Bytes of one of this stream's records, out of the file they were read from.
    pub fn record<'a>(&self, file: &'a [u8], at: usize) -> Option<&'a [u8]> {
        let record = self.records.get(at)?;
        file.get(record.start..record.start + record.len)
    }
}

/// A file's audio streams, with their records attached.
#[derive(Clone, Debug)]
pub struct Avi {
    audio: Vec<Audio>,
    /// The streams the file's groups list, of any kind, which is what the record ids
    /// count.
    pub streams: usize,
}

impl Avi {
    /// Read a whole file. The records point into `bytes` rather than copying it, so a
    /// caller that wants one of them takes it out of the same buffer.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        if id_at(bytes, 0) != Some(*b"RIFF") || id_at(bytes, 8) != Some(*b"AVI ") {
            return Err(invalid("file is not a RIFF/AVI"));
        }
        let mut hdrl = None;
        let mut movi = None;
        let mut index = None;
        for chunk in Chunks::range(bytes, 12, bytes.len()) {
            match chunk.id {
                id if id == *b"LIST" => match id_at(bytes, chunk.start) {
                    Some(kind) if kind == *b"hdrl" && hdrl.is_none() => hdrl = Some(chunk),
                    Some(kind) if kind == *b"movi" && movi.is_none() => movi = Some(chunk),
                    // `INFO` is a title and a comment; a second `movi` is what a writer
                    // that ran past the 1 GB one index can reach opens. Neither holds a
                    // stream's geometry.
                    _ => {}
                },
                id if id == *b"idx1" && index.is_none() => index = Some(chunk),
                // `JUNK` is the space a writer left for an index it may never come back
                // to write, and `on2f`/`vprp` are a picture's own notes.
                _ => {}
            }
        }
        let hdrl = hdrl.ok_or_else(|| invalid("an AVI file with no header list"))?;
        let movi = movi.ok_or_else(|| invalid("an AVI file with no movi list"))?;

        let mut promised = None;
        let mut groups: Vec<(Option<Chunk>, Option<Chunk>)> = Vec::new();
        for chunk in Chunks::list(bytes, &hdrl) {
            if chunk.id == *b"avih" {
                promised =
                    u32_at(bytes, chunk.start + MAIN_STREAMS).and_then(|n| usize::try_from(n).ok());
                continue;
            }
            if chunk.id != *b"LIST" || id_at(bytes, chunk.start) != Some(*b"strl") {
                continue;
            }
            if groups.len() >= limits.streams {
                return Err(invalid(&format!(
                    "the file lists more than {} streams",
                    limits.streams
                )));
            }
            let mut strh = None;
            let mut strf = None;
            for inner in Chunks::list(bytes, &chunk) {
                if inner.id == *b"strh" && strh.is_none() {
                    strh = Some(inner);
                } else if inner.id == *b"strf" && strf.is_none() {
                    strf = Some(inner);
                }
            }
            groups.push((strh, strf));
        }
        // The main header's count is the second statement of a thing the groups already
        // state by being there, so the two are compared rather than one believed.
        let promised =
            promised.ok_or_else(|| invalid("an AVI file whose main header says no streams"))?;
        if promised != groups.len() {
            return Err(invalid(&format!(
                "the main header promises {promised} streams and the file lists {} of them",
                groups.len()
            )));
        }

        let mut audio: Vec<Audio> = Vec::new();
        for (number, (strh, strf)) in groups.iter().enumerate() {
            let (Some(strh), Some(strf)) = (*strh, *strf) else {
                // A group with no header or no format record describes a stream nothing
                // can be placed on a timeline; a reader of pictures would want its own
                // answer to that, and this one leaves the group alone.
                continue;
            };
            if strh.end - strh.start < STREAM_HEADER {
                return Err(invalid(&format!(
                    "stream {number} states {} bytes of stream header where {STREAM_HEADER} are read",
                    strh.end - strh.start
                )));
            }
            if id_at(bytes, strh.start + STREAM_TYPE) != Some(AUDIO) {
                continue;
            }
            let format = format(bytes, &strf, limits)?;
            audio.push(Audio {
                number,
                declared: u32_at(bytes, strh.start + STREAM_LENGTH).unwrap_or(0),
                start: u32_at(bytes, strh.start + STREAM_START).unwrap_or(0),
                scale: u32_at(bytes, strh.start + STREAM_SCALE).unwrap_or(0),
                rate: u32_at(bytes, strh.start + STREAM_RATE).unwrap_or(0),
                format,
                setup: bytes[strf.start..strf.end].to_vec(),
                records: Vec::new(),
            });
        }

        let mut found: Vec<Vec<Record>> = (0..groups.len()).map(|_| Vec::new()).collect();
        records(bytes, &movi, limits, groups.len(), &mut found)?;
        for stream in &mut audio {
            stream.records = std::mem::take(&mut found[stream.number]);
        }
        if let Some(chunk) = index {
            check_index(bytes, &movi, &chunk, &audio, groups.len())?;
        }
        Ok(Self {
            audio,
            streams: groups.len(),
        })
    }

    /// The streams the header described as audio, in the order the file lists them:
    /// whether or not a decoder exists for what they are coded as.
    pub fn audio(&self) -> &[Audio] {
        &self.audio
    }
}

/// A stream's `strf` body as the six fields every Wave record states. The three that
/// can be said to be absurd are refused outright, because a geometry that cannot be
/// laid out is not one a decoder can be built from.
fn format(bytes: &[u8], strf: &Chunk, limits: &Limits) -> Result<Format> {
    let at = strf.start;
    if strf.end - at < FORMAT_HEADER {
        return Err(invalid(&format!(
            "a Wave format record of {} bytes states no geometry",
            strf.end - at
        )));
    }
    let out = Format {
        tag: u16_at(bytes, at + FORMAT_TAG).unwrap_or(0),
        channels: u16_at(bytes, at + FORMAT_CHANNELS).unwrap_or(0),
        sample_rate: u32_at(bytes, at + FORMAT_RATE).unwrap_or(0),
        bytes_per_second: u32_at(bytes, at + FORMAT_BYTES_PER_SECOND).unwrap_or(0),
        block_align: u16_at(bytes, at + FORMAT_BLOCK_ALIGN).unwrap_or(0),
        bits: u16_at(bytes, at + FORMAT_BITS).unwrap_or(0),
    };
    if out.channels == 0 || usize::from(out.channels) > limits.channels {
        return Err(invalid(&format!(
            "a track of {} channels is outside the layouts this reader lays out",
            out.channels
        )));
    }
    if out.sample_rate == 0 {
        return Err(invalid("a track states a sample rate of zero"));
    }
    if out.block_align == 0 {
        return Err(invalid("a track states a block of no bytes"));
    }
    Ok(out)
}

/// The audio records of a `movi` run, in the order the file has them, grouped by the
/// stream number their id spells. Nested lists are followed to [`NESTING`] deep, which
/// is how an interleaved writer groups one moment of every stream; an id naming a
/// stream the header never listed is left where it lies, since the only claim it makes
/// is about a stream this file does not have.
fn records(
    bytes: &[u8],
    movi: &Chunk,
    limits: &Limits,
    groups: usize,
    found: &mut [Vec<Record>],
) -> Result<()> {
    #[derive(Clone, Copy, Debug)]
    struct Frame {
        at: usize,
        stop: usize,
        depth: usize,
    }
    let mut open = vec![Frame {
        at: movi.start.saturating_add(INDEX_BASE),
        stop: movi.end,
        depth: 0,
    }];
    while let Some(top) = open.last() {
        let (at, stop, depth) = (top.at, top.stop, top.depth);
        let Some((chunk, next)) = chunk_at(bytes, at, stop) else {
            open.pop();
            continue;
        };
        open.last_mut().expect("the run is still open").at = next;
        if chunk.id == *b"LIST" {
            if depth >= NESTING {
                return Err(invalid(&format!(
                    "a movi list {NESTING} groups deep is not a grouping this reader follows"
                )));
            }
            open.push(Frame {
                at: chunk.start.saturating_add(INDEX_BASE),
                stop: chunk.end,
                depth: depth + 1,
            });
            continue;
        }
        let Some(number) = stream_of(&chunk.id, groups) else {
            continue;
        };
        if chunk.start >= chunk.end {
            return Err(invalid(&format!(
                "stream {number} holds a record with no bytes in it"
            )));
        }
        if found[number].len() >= limits.records {
            return Err(invalid(&format!(
                "stream {} holds more than {} records",
                number, limits.records
            )));
        }
        found[number].push(Record {
            start: chunk.start,
            len: chunk.end - chunk.start,
        });
    }
    Ok(())
}

/// The stream number a record id names, or `None` when the id spells neither digits nor
/// the letters an audio record carries, nor a stream the header listed.
fn stream_of(id: &[u8; 4], groups: usize) -> Option<usize> {
    if id[2..] != AUDIO_LETTERS {
        return None;
    }
    let (first, second) = (id[0].wrapping_sub(b'0'), id[1].wrapping_sub(b'0'));
    if first > 9 || second > 9 {
        return None;
    }
    let number = usize::from(first * 10 + second);
    (number < groups).then_some(number)
}

/// The record list as the file's own index states it, against the list as it was
/// walked: the same records for the same streams, in the same order, of the same
/// lengths and at the same offsets. A picture's entries are not asked about - the walk
/// never read them - and an entry whose id says audio over a stream whose header says
/// otherwise is a disagreement between the file's own two statements.
fn check_index(
    bytes: &[u8],
    movi: &Chunk,
    index: &Chunk,
    audio: &[Audio],
    groups: usize,
) -> Result<()> {
    let mut by_number: Vec<Option<usize>> = vec![None; groups];
    for (slot, stream) in audio.iter().enumerate() {
        by_number[stream.number] = Some(slot);
    }
    let base = movi.start;
    let mut counted = vec![0usize; audio.len()];
    for entry in 0..(index.end - index.start) / INDEX_ENTRY {
        let at = index.start + entry * INDEX_ENTRY;
        let Some(id) = id_at(bytes, at) else {
            break;
        };
        let Some(number) = stream_of(&id, groups) else {
            continue;
        };
        let length =
            usize::try_from(u32_at(bytes, at + INDEX_LENGTH).unwrap_or(0)).unwrap_or(usize::MAX);
        let offset =
            usize::try_from(u32_at(bytes, at + INDEX_OFFSET).unwrap_or(0)).unwrap_or(usize::MAX);
        let Some(slot) = by_number[number] else {
            return Err(invalid(&format!(
                "the index lists a record of stream {number} as audio, which its header does not"
            )));
        };
        let stream = &audio[slot];
        let Some(walked) = stream.records.get(counted[slot]) else {
            return Err(invalid(&format!(
                "the index names more records for stream {number} than the file holds"
            )));
        };
        if walked.len != length {
            return Err(invalid(&format!(
                "stream {} record {} is {} bytes where the index says {length}",
                number, counted[slot], walked.len
            )));
        }
        // An entry points at a record's header rather than through it, so the bytes it
        // describes begin one header past the place it names.
        let where_it_says = base
            .checked_add(offset)
            .and_then(|at| at.checked_add(CHUNK_HEADER));
        if where_it_says != Some(walked.start) {
            return Err(invalid(&format!(
                "stream {} record {} sits at {} where the index points at {}",
                number,
                counted[slot],
                walked.start,
                where_it_says.unwrap_or(usize::MAX)
            )));
        }
        counted[slot] += 1;
    }
    for (slot, stream) in audio.iter().enumerate() {
        if counted[slot] != stream.records.len() {
            return Err(invalid(&format!(
                "stream {} holds {} records and the index lists {} of them",
                stream.number,
                stream.records.len(),
                counted[slot]
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Avi, FORMAT_BLOCK_ALIGN, FORMAT_CHANNELS, FORMAT_RATE, Limits, chunk_at, records, stream_of,
    };

    /// One chunk as a writer of this format writes it, pad byte included.
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// A `LIST` of a kind over chunks already built.
    fn list(kind: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for child in children {
            body.extend_from_slice(child);
        }
        chunk(b"LIST", &body)
    }

    /// A `WAVEFORMATEX`: the six fields every one has, then whatever a coding adds.
    fn wave(tag: u16, channels: u16, rate: u32, align: u16, bits: u16, frames: u16) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(
            &((u32::from(align) * rate).div_euclid(u32::from(frames.max(1)))).to_le_bytes(),
        );
        out.extend_from_slice(&align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        // The two Microsoft spellings and IMA in a Wave block all end the record with
        // how many frames a block holds; the extension count says how much follows.
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&frames.to_le_bytes());
        out
    }

    /// A stream header for an audio or a picture stream, with its ten words.
    fn strh(kind: &[u8; 4], scale: u32, rate: u32, length: u32) -> Vec<u8> {
        let mut out = vec![0u8; 48];
        out[..4].copy_from_slice(kind);
        out[4..8].copy_from_slice(&[1, 0, 0, 0]);
        for (at, word) in [
            (20, scale),
            (24, rate),
            (28, 0),
            (32, length),
            (36, 1_024),
            (40, u32::MAX),
            (44, 1_024),
        ] {
            out[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// The main header, promising one stream.
    fn avih(streams: u32) -> Vec<u8> {
        let mut out = vec![0u8; 56];
        out[24..28].copy_from_slice(&streams.to_le_bytes());
        out
    }

    /// A whole file: the stream groups in file order, then the run of records the
    /// `movi` list holds, each stated as its stream number, its length in bytes and the
    /// two letters its id carries, and an index stating the audio records of that run
    /// the way a writer states them.
    ///
    /// A group that states no format - an empty body here - is written without a `strf`
    /// chunk at all, and a record whose letters are not the audio pair is left out of
    /// the index, which is what a writer that indexes only what it plays leaves behind.
    fn file(
        groups: &[(Vec<u8>, Vec<u8>)],
        run: &[(usize, usize, [u8; 2])],
        nested: bool,
    ) -> Vec<u8> {
        let mut movi = Vec::new();
        let mut entries = Vec::new();
        // The list's own type word comes before its first record, so the byte offsets
        // an index states for the records start at four.
        let mut at = super::INDEX_BASE;
        for (index, (number, len, letters)) in run.iter().enumerate() {
            let body = vec![index as u8; *len];
            let mut id = [0u8; 4];
            id[0] = b'0' + (*number / 10) as u8;
            id[1] = b'0' + (*number % 10) as u8;
            id[2..].copy_from_slice(letters);
            let record = chunk(&id, &body);
            // Every second record in a nested file is wrapped in a `LIST rec `, the way
            // an interleaved writer groups one moment of every stream together. The
            // wrapper's twelve bytes - its header and its own type word - stand in front
            // of the record, which the entry still points at.
            let (written, offset) = if nested && index % 2 == 1 {
                (list(b"rec ", std::slice::from_ref(&record)), at + 12)
            } else {
                (record, at)
            };
            if *letters == *b"wb" {
                entries.push((id, offset, *len));
            }
            at += written.len();
            movi.extend_from_slice(&written);
        }
        let mut index = Vec::new();
        for (id, offset, len) in entries {
            index.extend_from_slice(&id);
            index.extend_from_slice(&16u32.to_le_bytes());
            index.extend_from_slice(&(offset as u32).to_le_bytes());
            index.extend_from_slice(&(len as u32).to_le_bytes());
        }
        let groups = groups
            .iter()
            .map(|(header, format)| {
                let mut children = vec![chunk(b"strh", header)];
                if !format.is_empty() {
                    children.push(chunk(b"strf", format));
                }
                list(b"strl", &children)
            })
            .collect::<Vec<_>>();
        let mut hdrl = vec![chunk(b"avih", &avih(groups.len() as u32))];
        hdrl.extend(groups);
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(b"AVI ");
        out.extend_from_slice(&list(b"hdrl", &hdrl));
        out.extend_from_slice(&list(b"movi", &[movi]));
        out.extend_from_slice(&chunk(b"idx1", &index));
        out
    }

    /// The one-second mono IMA ADPCM track this machine's muxer writes, as the shape of
    /// its blocks rather than as bytes: five records of 1 024, 2 041 frames to a block.
    fn ima() -> Vec<u8> {
        wave(0x0011, 1, 48_000, 1_024, 4, 2_041)
    }

    fn parse(bytes: &[u8]) -> Avi {
        Avi::parse(bytes, &Limits::default()).expect("the file as it was written")
    }

    fn said(bytes: &[u8], limits: Limits) -> String {
        Avi::parse(bytes, &limits)
            .expect_err("refused")
            .to_string()
    }

    /// Where a chunk's id first appears in a built file, so a test can reach into the
    /// field it means. The ids this helper is used on are the ones the builders write,
    /// which no record body of these tests happens to spell.
    fn where_is(bytes: &[u8], id: &[u8]) -> usize {
        bytes
            .windows(id.len())
            .position(|window| window == id)
            .unwrap_or_else(|| panic!("no {} in the file", String::from_utf8_lossy(id)))
    }

    /// The record ids a file's streams own. Two decimal digits and the letters an audio
    /// record carries, and nothing else is one of the listed streams' bytes.
    #[test]
    fn only_a_streams_own_id_names_its_records() {
        assert_eq!(stream_of(b"00wb", 2), Some(0));
        assert_eq!(stream_of(b"01wb", 2), Some(1));
        assert_eq!(stream_of(b"00db", 2), None);
        assert_eq!(stream_of(b"abwb", 2), None);
        assert_eq!(stream_of(b"0awb", 2), None);
        // A number the header never listed: the bytes are in the file, the stream is not.
        assert_eq!(stream_of(b"02wb", 2), None);
        assert_eq!(stream_of(b"99wb", 100), Some(99));
        assert_eq!(stream_of(b"99wb", 99), None);
    }

    /// A chunk's own claim is cut at the run it sits in, and the next one starts where
    /// the claim said - not where the bytes stopped - which is what stops a truncated
    /// chunk from being read as the header of its own successor.
    #[test]
    fn a_chunk_that_overclaims_ends_where_the_run_does() {
        let bytes = b"00wb\x04\x00\x00\x00abcd";
        let (chunk, next) = chunk_at(bytes, 0, 12).expect("the chunk");
        assert_eq!(
            (chunk.id, chunk.start, chunk.end, chunk.truncated),
            (*b"00wb", 8, 12, false)
        );
        assert_eq!(next, 12);
        let bytes = b"00wb\x08\x00\x00\x00abcd";
        let (chunk, next) = chunk_at(bytes, 0, 12).expect("the short run");
        assert!(chunk.truncated);
        assert_eq!((chunk.start, chunk.end), (8, 12));
        // The run is over, so the walk stops here rather than reading past it.
        assert_eq!(next, 12);
        assert!(chunk_at(bytes, next, 12).is_none());
        // An odd length brings a pad byte: the next chunk starts past it.
        let bytes = b"00wb\x01\x00\x00\x00a\x0001wb\x00\x00\x00\x00";
        let (_, next) = chunk_at(bytes, 0, bytes.len()).expect("the padded chunk");
        assert_eq!(next, 10);
    }

    #[test]
    fn the_headers_of_a_real_audio_stream_are_the_streams_own() {
        let bytes = file(
            &[(strh(b"auds", 2_041, 48_000, 5), ima())],
            &[
                (0, 1_024, *b"wb"),
                (0, 1_024, *b"wb"),
                (0, 1_024, *b"wb"),
                (0, 1_024, *b"wb"),
                (0, 512, *b"wb"),
            ],
            false,
        );
        let avi = parse(&bytes);
        assert_eq!(avi.streams, 1);
        assert_eq!(avi.audio().len(), 1);
        let stream = &avi.audio()[0];
        assert_eq!(stream.number, 0);
        assert_eq!(stream.declared, 5);
        assert_eq!(
            (stream.scale, stream.rate, stream.start),
            (2_041, 48_000, 0)
        );
        assert_eq!(
            stream.format,
            super::Format {
                tag: 0x0011,
                channels: 1,
                sample_rate: 48_000,
                bytes_per_second: 24_082,
                block_align: 1_024,
                bits: 4,
            }
        );
        // The whole format record travels on as setup data, extension included: the
        // coding reads its own geometry out of it, not a reader's summary of it.
        assert_eq!(stream.setup.len(), 20);
        assert_eq!(stream.records.len(), 5);
        assert_eq!(stream.record(&bytes, 0).expect("first").len(), 1_024);
        assert_eq!(stream.record(&bytes, 4).expect("last").len(), 512);
        assert!(stream.record(&bytes, 5).is_none());
        // The bytes are the ones the file holds, so a record is a place, not a copy.
        assert_eq!(stream.record(&bytes, 2).expect("third"), &[2u8; 1_024]);
    }

    /// Every record found inside a `LIST rec ` as well as one sitting loose in `movi`,
    /// in the order the file has them - which is the order the sound goes in.
    #[test]
    fn a_records_own_group_does_not_move_it_in_the_timeline() {
        let bytes = file(
            &[(strh(b"auds", 2_041, 48_000, 4), ima())],
            &[
                (0, 64, *b"wb"),
                (0, 32, *b"wb"),
                (0, 16, *b"wb"),
                (0, 8, *b"wb"),
            ],
            true,
        );
        let avi = parse(&bytes);
        let stream = &avi.audio()[0];
        assert_eq!(
            stream.records.iter().map(|r| r.len).collect::<Vec<_>>(),
            vec![64, 32, 16, 8]
        );
        assert!(
            stream
                .records
                .windows(2)
                .all(|pair| pair[1].start > pair[0].start),
            "the walk reads the file in order"
        );
        // The nested bytes are not read twice: the group's own header is not a record.
        assert_eq!(stream.records.len(), 4);
    }

    /// The index is the file's own restatement of the record list, and it is held
    /// against the list. Each refusal names the claim that failed, because a reader
    /// that said "unreadable" about all of them would be guessing about each.
    #[test]
    fn an_index_that_disagrees_with_the_records_is_refused() {
        let groups = [(strh(b"auds", 2_041, 48_000, 2), ima())];
        let bytes = file(&groups, &[(0, 1_024, *b"wb"), (0, 512, *b"wb")], false);
        let index = where_is(&bytes, b"idx1") + 8;
        // A length that is not the record's.
        let mut longer = bytes.clone();
        longer[index + 12..index + 16].copy_from_slice(&2_048u32.to_le_bytes());
        assert!(
            said(&longer, Limits::default())
                .contains("record 0 is 1024 bytes where the index says 2048"),
            "{}",
            said(&longer, Limits::default())
        );
        // An offset that points at the wrong byte.
        let mut moved = bytes.clone();
        moved[index + 8..index + 12].copy_from_slice(&8u32.to_le_bytes());
        assert!(said(&moved, Limits::default()).contains("record 0 sits at"));
        // One entry more than the file has records: the entry is a copy of the first,
        // and the chunk's own length is grown over it, so the walk reaches it.
        let head = where_is(&bytes, b"idx1");
        let entry = bytes[head + 8..head + 24].to_vec();
        let declared = u32::from_le_bytes(bytes[head + 4..head + 8].try_into().expect("a word"));
        let mut extra = bytes.clone();
        extra.extend_from_slice(&entry);
        extra[head + 4..head + 8].copy_from_slice(&(declared + 16).to_le_bytes());
        assert!(
            said(&extra, Limits::default()).contains("more records for stream 0"),
            "{}",
            said(&extra, Limits::default())
        );
        // And one entry fewer.
        let fewer = &bytes[..bytes.len() - 16];
        assert!(said(fewer, Limits::default()).contains("the index lists 1"));
        // The file as written parses, so what failed in each case is the field.
        assert_eq!(parse(&bytes).audio()[0].records.len(), 2);
    }

    #[test]
    fn a_file_without_an_index_is_read_by_its_records_alone() {
        let bytes = file(
            &[(strh(b"auds", 2_041, 48_000, 3), ima())],
            &[(0, 1_024, *b"wb"), (0, 1_024, *b"wb"), (0, 1_024, *b"wb")],
            false,
        );
        let index = where_is(&bytes, b"idx1");
        let unindexed = &bytes[..index];
        let avi = parse(unindexed);
        assert_eq!(avi.audio()[0].records.len(), 3);
    }

    /// A stream header that is not audio, and a record id that is not either: both are
    /// left alone, and the audio between them keeps its own number.
    #[test]
    fn a_picture_stream_in_front_does_not_take_the_audios_number() {
        let bytes = file(
            &[
                (strh(b"vids", 1, 25, 5), vec![0u8; 40]),
                (strh(b"auds", 2_041, 48_000, 2), ima()),
            ],
            &[(1, 1_024, *b"wb"), (1, 1_024, *b"wb")],
            false,
        );
        let avi = parse(&bytes);
        assert_eq!(avi.streams, 2);
        assert_eq!(avi.audio().len(), 1);
        let stream = &avi.audio()[0];
        assert_eq!(stream.number, 1);
        assert_eq!(stream.records.len(), 2);
        // A picture's own record, ided like its stream, is not audio bytes: it sits in
        // the same run ahead of the sound, and the walk leaves it where it is.
        let with_video = file(
            &[
                (strh(b"vids", 1, 25, 1), vec![0u8; 40]),
                (strh(b"auds", 2_041, 48_000, 1), ima()),
            ],
            &[(0, 512, *b"db"), (1, 1_024, *b"wb")],
            false,
        );
        let avi = parse(&with_video);
        let stream = &avi.audio()[0];
        assert_eq!(
            stream.records.len(),
            1,
            "the picture's bytes are not a second one"
        );
        assert_eq!(stream.record(&with_video, 0).expect("audio").len(), 1_024);
    }

    /// An id carrying the audio letters over a stream whose header says picture: the
    /// file states two different things about one stream, and this reader of sound says
    /// which of its own statements it could not reconcile.
    #[test]
    fn an_id_that_claims_a_picture_stream_is_audio_is_a_disagreement() {
        let bytes = file(
            &[
                (strh(b"vids", 1, 25, 1), vec![0u8; 40]),
                (strh(b"auds", 2_041, 48_000, 1), ima()),
            ],
            &[(0, 1_024, *b"wb"), (1, 1_024, *b"wb")],
            false,
        );
        // The first record is stream 0's by its id and the index names it as audio,
        // while stream 0's header is a picture's. The sound of the second stream is
        // read, and the file is refused on the stream the two statements disagree about.
        let text = said(&bytes, Limits::default());
        assert!(
            text.contains(
                "the index lists a record of stream 0 as audio, which its header does not"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_stream_holding_nothing_is_not_a_file_holding_nothing() {
        let bytes = file(&[(strh(b"auds", 2_041, 48_000, 0), ima())], &[], false);
        let avi = parse(&bytes);
        assert_eq!(avi.audio()[0].records.len(), 0);
        // The reader that has to play it is the one that says an empty stream is no
        // track; the wrapper's own claims still add up.
    }

    #[test]
    fn a_record_with_no_bytes_in_it_is_refused() {
        let bytes = file(
            &[(strh(b"auds", 2_041, 48_000, 2), ima())],
            &[(0, 0, *b"wb"), (0, 1_024, *b"wb")],
            false,
        );
        assert!(said(&bytes, Limits::default()).contains("no bytes in it"));
    }

    #[test]
    fn the_main_header_and_the_stream_groups_have_to_agree() {
        let groups = [(strh(b"auds", 2_041, 48_000, 1), ima())];
        let mut bytes = file(&groups, &[(0, 1_024, *b"wb")], false);
        let at = where_is(&bytes, b"avih") + 8 + 24;
        bytes[at..at + 4].copy_from_slice(&3u32.to_le_bytes());
        assert!(
            said(&bytes, Limits::default()).contains("promises 3 streams and the file lists 1"),
            "{}",
            said(&bytes, Limits::default())
        );
    }

    #[test]
    fn a_geometry_that_cannot_be_laid_out_is_refused() {
        for (channels, rate, align, words) in [
            (0u16, 48_000u32, 1_024u16, "channels"),
            (33, 48_000, 1_024, "channels"),
            (1, 0, 1_024, "sample rate"),
            (1, 48_000, 0, "block"),
        ] {
            let mut bytes = file(
                &[(strh(b"auds", 2_041, 48_000, 1), ima())],
                &[(0, 1_024, *b"wb")],
                false,
            );
            let at = where_is(&bytes, b"strf") + 8;
            bytes[at + FORMAT_CHANNELS..at + FORMAT_CHANNELS + 2]
                .copy_from_slice(&channels.to_le_bytes());
            bytes[at + FORMAT_RATE..at + FORMAT_RATE + 4].copy_from_slice(&rate.to_le_bytes());
            bytes[at + FORMAT_BLOCK_ALIGN..at + FORMAT_BLOCK_ALIGN + 2]
                .copy_from_slice(&align.to_le_bytes());
            let text = said(&bytes, Limits::default());
            assert!(text.contains(words), "{words} says: {text}");
        }
    }

    /// A header written shorter than the words the timeline is read from, and a format
    /// record that stops before its sixth field. Each is a chunk of its stated length
    /// rather than a length doctored afterwards: a RIFF walk starts the next chunk where
    /// the length says, so a lie about one header moves every chunk behind it and the
    /// file would be refused for a reason other than the one under test.
    #[test]
    fn a_header_or_a_format_that_stops_short_is_refused() {
        let mut header = strh(b"auds", 2_041, 48_000, 1);
        header.truncate(32);
        let bytes = file(&[(header, ima())], &[(0, 1_024, *b"wb")], false);
        assert!(
            said(&bytes, Limits::default())
                .contains("states 32 bytes of stream header where 48 are read"),
            "{}",
            said(&bytes, Limits::default())
        );
        let mut format = ima();
        format.truncate(8);
        let bytes = file(
            &[(strh(b"auds", 2_041, 48_000, 1), format)],
            &[(0, 1_024, *b"wb")],
            false,
        );
        assert!(
            said(&bytes, Limits::default()).contains("format record of 8 bytes states no geometry"),
            "{}",
            said(&bytes, Limits::default())
        );
    }

    #[test]
    fn a_file_that_is_not_an_avi_is_not_a_file_with_no_audio() {
        assert!(said(b"RIFF\x10\x00\x00\x00WAVE", Limits::default()).contains("not a RIFF/AVI"));
        assert!(said(b"FORM\x00\x00\x00\x00AIFF", Limits::default()).contains("not a RIFF/AVI"));
        assert!(said(b"short", Limits::default()).contains("not a RIFF/AVI"));
        // The envelope on its own: no header list is not the same as an empty one.
        assert!(said(b"RIFF\x04\x00\x00\x00AVI ", Limits::default()).contains("no header list"));
        let mut bytes = b"RIFF\x14\x00\x00\x00AVI ".to_vec();
        bytes.extend_from_slice(&list(b"hdrl", &[chunk(b"avih", &avih(1))]));
        assert!(said(&bytes, Limits::default()).contains("no movi list"));
    }

    #[test]
    fn the_readers_own_budgets_are_its_to_set() {
        let groups = [(strh(b"auds", 2_041, 48_000, 2), ima())];
        let bytes = file(&groups, &[(0, 64, *b"wb"), (0, 64, *b"wb")], false);
        let tight = Limits {
            file_bytes: 32,
            ..Limits::default()
        };
        assert!(said(&bytes, tight).contains("byte limit"));
        let tight = Limits {
            records: 1,
            ..Limits::default()
        };
        assert!(said(&bytes, tight).contains("more than 1 records"));
        let tight = Limits {
            streams: 1,
            ..Limits::default()
        };
        let two = file(
            &[
                (strh(b"auds", 2_041, 48_000, 1), ima()),
                (strh(b"auds", 2_041, 48_000, 1), ima()),
            ],
            &[(0, 1_024, *b"wb"), (1, 1_024, *b"wb")],
            false,
        );
        assert!(said(&two, tight).contains("more than 1 streams"));
        // Two audio streams of their own, both read: the reader picks one, the parser
        // is not the one that decides which.
        let both = parse(&two);
        assert_eq!(
            both.audio()
                .iter()
                .map(|a| (a.number, a.records.len()))
                .collect::<Vec<_>>(),
            vec![(0, 1), (1, 1)]
        );
    }

    /// A group whose header is there and whose format record is not: no geometry can be
    /// read from it, so the parser leaves the group alone rather than calling it a
    /// stream with a zeroed one. A file whose records name that all the same stream is
    /// refused where its own two statements part.
    #[test]
    fn a_group_that_states_no_format_describes_no_stream_this_reader_reads() {
        let quiet = file(&[(strh(b"auds", 2_041, 48_000, 0), Vec::new())], &[], false);
        let avi = parse(&quiet);
        assert_eq!(avi.streams, 1, "the group is a stream the file lists");
        assert!(avi.audio().is_empty(), "and one that states no geometry");
        let spoken = file(
            &[(strh(b"auds", 2_041, 48_000, 1), Vec::new())],
            &[(0, 1_024, *b"wb")],
            false,
        );
        assert!(
            said(&spoken, Limits::default()).contains(
                "the index lists a record of stream 0 as audio, which its header does not"
            ),
            "{}",
            said(&spoken, Limits::default())
        );
    }

    /// The walk's own bound, asked of a `movi` list directly: a file whose list length
    /// lies is read as far as its bytes go, which is the only reading a stale writer
    /// leaves available - and the state the file's own index is there to catch.
    #[test]
    fn a_movi_list_shorter_than_it_claims_is_read_to_its_end() {
        let groups = [(strh(b"auds", 2_041, 48_000, 2), ima())];
        let bytes = file(&groups, &[(0, 1_024, *b"wb"), (0, 1_024, *b"wb")], false);
        // The list's header begins eight bytes before the type word the walk starts its
        // run past, so the length word is the four bytes between them.
        let at = where_is(&bytes, b"movi") - 8;
        let mut lying = bytes.clone();
        // Cut the list short inside the first record: four bytes of type word, eight of
        // the record's own header, and 1 016 of its bytes.
        lying[at + 4..at + 8].copy_from_slice(&(4 + 8 + 1_016u32).to_le_bytes());
        let movi = chunk_at(&lying, at, lying.len()).expect("the list").0;
        assert_eq!((movi.start, movi.end), (at + 8, at + 8 + 1_028));
        let mut found = vec![Vec::new(); 1];
        records(&lying, &movi, &Limits::default(), 1, &mut found).expect("the walk");
        assert_eq!(
            found[0].len(),
            1,
            "the walk stops with the run, not inside it"
        );
        assert_eq!(
            found[0][0].len, 1_016,
            "the record is cut where the list does"
        );
        // Which the parser refuses rather than shortening silently: the index still
        // states the whole record, and the two are compared.
        assert!(
            said(&lying, Limits::default())
                .contains("record 0 is 1016 bytes where the index says 1024"),
            "{}",
            said(&lying, Limits::default())
        );
    }
}
