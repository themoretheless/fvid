//! Indexed ISO BMFF demultiplexing implemented in FVid, over both the one
//! complete sample table a non-fragmented file writes and the run of `moof`
//! fragments a stream-derived file writes in its place.
//! Timestamps are in the track media timeline; edit lists are returned separately.
use crate::{Result, container::FileTags, invalid, unsupported};
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub metadata_bytes: usize,
    pub samples: usize,
    pub tracks: usize,
    pub packet_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            metadata_bytes: 32 << 20,
            samples: 1_000_000,
            tracks: 64,
            packet_bytes: 32 << 20,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Sample {
    pub offset: u64,
    pub size: u32,
    pub dts: u64,
    pub pts: i64,
    pub duration: u32,
    pub sync: bool,
}
/// A stretch of a uniform track: `frames` back-to-back frames of one size,
/// starting at `offset` and numbered from `first`.
#[derive(Clone, Copy, Debug)]
pub struct FrameRun {
    pub first: usize,
    pub offset: u64,
    pub frames: usize,
}
/// The index of a track whose frames all say the same thing, which is the shape
/// an uncompressed audio track has: the sample table gives one size for every
/// frame, `stts` one duration, and the frames follow each other inside a chunk.
/// Keeping it this way costs a record per chunk rather than one per frame, so a
/// five-minute recording of 14 million audio frames indexes in kilobytes.
#[derive(Clone, Debug)]
pub struct UniformIndex {
    pub bytes_per_frame: u32,
    pub duration: u32,
    pub count: usize,
    pub runs: Vec<FrameRun>,
}
/// Everything the sample tables of one track add up to.
#[derive(Clone, Debug)]
pub enum SampleIndex {
    /// One record per sample: the general case, and the only one a video track
    /// with varying sizes, edit shifts or reference pictures can use.
    Expanded(Vec<Sample>),
    Uniform(UniformIndex),
}

impl SampleIndex {
    /// Frames the track holds, whichever form indexes them.
    pub fn len(&self) -> usize {
        match self {
            Self::Expanded(samples) => samples.len(),
            Self::Uniform(index) => index.count,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Records the index itself costs, which is what the sample budget is
    /// charged against: a frame each for an expanded table, a chunk each for a
    /// uniform one.
    pub fn cost(&self) -> usize {
        match self {
            Self::Expanded(samples) => samples.len(),
            Self::Uniform(index) => index.runs.len(),
        }
    }
    pub fn get(&self, index: usize) -> Option<Sample> {
        match self {
            Self::Expanded(samples) => samples.get(index).cloned(),
            Self::Uniform(uniform) => {
                let found = uniform.runs.partition_point(|run| run.first <= index);
                let run = *uniform.runs.get(found.checked_sub(1)?)?;
                if index >= run.first + run.frames {
                    return None;
                }
                let at = index - run.first;
                let bytes = u64::try_from(at)
                    .ok()?
                    .checked_mul(u64::from(uniform.bytes_per_frame))?;
                // Byte offsets restart with the chunk; the timeline does not.
                let ticks = u64::try_from(index)
                    .ok()?
                    .checked_mul(u64::from(uniform.duration))?;
                Some(Sample {
                    offset: run.offset.checked_add(bytes)?,
                    size: uniform.bytes_per_frame,
                    dts: ticks,
                    pts: i64::try_from(ticks).ok()?,
                    duration: uniform.duration,
                    sync: true,
                })
            }
        }
    }
    /// The per-sample slice, for the code that has to index every sample of a
    /// video track. `None` for a uniform table, which holds no such slice.
    pub fn expanded(&self) -> Option<&[Sample]> {
        match self {
            Self::Expanded(samples) => Some(samples),
            Self::Uniform(_) => None,
        }
    }
    /// Greatest index whose presentation time is at or before `pts`, or 0 when
    /// none is. Samples run in presentation order in both forms.
    pub fn at_or_before(&self, pts: i64) -> usize {
        let last = self.len().saturating_sub(1);
        match self {
            Self::Expanded(samples) => samples
                .iter()
                .enumerate()
                .filter(|(_, s)| s.pts <= pts)
                .max_by_key(|(_, s)| s.pts)
                .map_or(0, |(i, _)| i),
            Self::Uniform(uniform) => {
                if uniform.duration == 0 {
                    return 0;
                }
                (pts.max(0) as usize / usize::try_from(uniform.duration).unwrap_or(1)).min(last)
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct Edit {
    /// Duration in movie ticks, not track ticks.
    pub duration: u64,
    /// Track ticks; -1 represents an empty edit.
    pub media_time: i64,
}
#[derive(Clone, Debug)]
pub struct Track {
    pub id: u32,
    pub handler: [u8; 4],
    pub codec: [u8; 4],
    /// The title from `trak/udta/name`, empty when the file names no track.
    pub name: String,
    /// The language `mdhd` states as three letters, empty when the field says
    /// `und` or states nothing that reads as letters.
    pub language: String,
    pub timescale: u32,
    pub duration: u64,
    pub width: u16,
    pub height: u16,
    pub channels: u16,
    pub sample_rate: u32,
    /// `sample_size` of the audio sample entry: bits in one PCM sample, and 0
    /// for a video or subtitle track. A compressed audio track carries its own
    /// depth in its configuration and its decoder ignores this.
    pub bit_depth: u16,
    /// Raw avcC / hvcC / esds payload. Decoders validate the codec-specific syntax.
    pub configuration: Vec<u8>,
    pub edits: Vec<Edit>,
    pub samples: SampleIndex,
    /// How much wider a coded pixel is than it is tall, as the file states it.
    /// `(1, 1)` when the file states nothing, which is what most writers mean.
    /// Stated for the picture after its `rotation`, since that is the one the
    /// player stretches.
    pub pixel_aspect: (u32, u32),
    /// Degrees clockwise the coded picture is turned to be shown upright, as the
    /// `tkhd` transform states it. One of 0, 90, 180 or 270; a transform that
    /// mirrors or skews states none of them and leaves the picture as decoded.
    pub rotation: u16,
}
/// A named part of the film, from the movie's own chapter list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    pub start_ns: u64,
    pub title: String,
}
/// A track this reader read as far as its sample entry and then set aside,
/// because that entry names a coding with no arm here. Which is a coding to
/// implement, not a file that failed to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkippedTrack {
    /// The `hdlr` type: `vide`, `soun`, or one of the subtitle handlers.
    pub handler: [u8; 4],
    /// The fourcc the sample entry calls itself.
    pub codec: [u8; 4],
}
pub struct Mp4Reader<R> {
    reader: R,
    tracks: Vec<Track>,
    /// Tracks read all the way to their sample entry and then set aside because
    /// that entry names a coding with no arm here. Kept so a caller can tell a
    /// coding it has to implement apart from a file it failed to read.
    refused: Vec<SkippedTrack>,
    chapters: Vec<Chapter>,
    /// What the movie's own tags say about it; every fact it states none of
    /// leaves empty.
    tags: FileTags,
    movie_timescale: u32,
    limits: Limits,
    /// Byte position the reader was left at by the last packet read, or
    /// `u64::MAX` when unknown (open, rewind or a failed read).
    position: u64,
}

#[derive(Clone, Copy)]
struct Atom<'a> {
    kind: [u8; 4],
    data: &'a [u8],
}
fn u16be(b: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(
        b.get(at..at + 2)
            .ok_or_else(|| invalid("truncated MP4 field"))?
            .try_into()
            .unwrap(),
    ))
}
fn u32be(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        b.get(at..at + 4)
            .ok_or_else(|| invalid("truncated MP4 field"))?
            .try_into()
            .unwrap(),
    ))
}
fn u64be(b: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_be_bytes(
        b.get(at..at + 8)
            .ok_or_else(|| invalid("truncated MP4 field"))?
            .try_into()
            .unwrap(),
    ))
}
fn atoms(mut data: &[u8]) -> Result<Vec<Atom<'_>>> {
    let mut out = Vec::new();
    while !data.is_empty() {
        // QuickTime writers end a sample entry's extension list with a four-byte
        // zero terminator; a short all-zero tail carries no box and is ignored.
        if data.len() < 8 && data.iter().all(|b| *b == 0) {
            break;
        }
        let size = u32be(data, 0)?;
        let kind = data
            .get(4..8)
            .ok_or_else(|| invalid("truncated MP4 box"))?
            .try_into()
            .unwrap();
        let (size, header) = match size {
            0 => (data.len(), 8),
            1 => (
                usize::try_from(u64be(data, 8)?).map_err(|_| invalid("MP4 box size overflow"))?,
                16,
            ),
            n => (n as usize, 8),
        };
        if size < header || size > data.len() {
            return Err(invalid("MP4 box exceeds parent"));
        }
        if out.len() >= 100_000 {
            return Err(invalid("too many MP4 boxes"));
        }
        out.push(Atom {
            kind,
            data: &data[header..size],
        });
        data = &data[size..];
    }
    Ok(out)
}
fn optional<'a>(items: &[Atom<'a>], kind: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    let mut found = items.iter().filter(|a| &a.kind == kind);
    let value = found.next().map(|a| a.data);
    if found.next().is_some() {
        return Err(invalid("duplicate MP4 singleton box"));
    }
    Ok(value)
}
fn required<'a>(items: &[Atom<'a>], kind: &[u8; 4]) -> Result<&'a [u8]> {
    optional(items, kind)?.ok_or_else(|| {
        invalid(&format!(
            "missing MP4 {} box",
            String::from_utf8_lossy(kind)
        ))
    })
}
fn version(data: &[u8], maximum: u8) -> Result<u8> {
    let v = *data
        .first()
        .ok_or_else(|| invalid("missing MP4 box version"))?;
    if v > maximum {
        return Err(invalid("unsupported MP4 box version"));
    }
    Ok(v)
}
fn table(data: &[u8], stride: usize, limit: usize) -> Result<usize> {
    let count = u32be(data, 4)? as usize;
    if count > limit {
        return Err(invalid("MP4 table exceeds configured limit"));
    }
    if count.checked_mul(stride).and_then(|v| v.checked_add(8)) != Some(data.len()) {
        return Err(invalid("MP4 table count does not match box size"));
    }
    Ok(count)
}
fn media_time(data: &[u8]) -> Result<(u32, u64)> {
    let v = version(data, 1)?;
    let (scale, duration) = if v == 0 {
        (u32be(data, 12)?, u64::from(u32be(data, 16)?))
    } else {
        (u32be(data, 20)?, u64be(data, 24)?)
    };
    if scale == 0 {
        return Err(invalid("MP4 timescale is zero"));
    }
    Ok((scale, duration))
}
/// The language `mdhd` states as three packed 5-bit groups, one per letter
/// counted from the code just below `a`. `und` is what writers put there when
/// nothing was said, and a group left at zero is padding rather than a letter,
/// so both come back as nothing for the player to show.
fn media_language(mdhd: &[u8]) -> Option<String> {
    let at = if version(mdhd, 1).ok()? == 0 { 20 } else { 32 };
    let packed = u16be(mdhd, at).ok()?;
    let mut letters = String::with_capacity(3);
    for shift in [10, 5, 0] {
        let letter = ((packed >> shift) & 31) as u8;
        if letter == 0 {
            return None;
        }
        letters.push(char::from(letter + 0x60));
    }
    (letters != "und").then_some(letters)
}
/// The title of a track, from `trak/udta/name`: plain UTF-8 with no length
/// prefix, which is what both ffmpeg and QuickTime write. A `udta` this reader
/// cannot walk, or a name that is not text, costs only the title.
fn track_name(trak: &[Atom<'_>]) -> Option<String> {
    let udta = atoms(optional(trak, b"udta").ok()??).ok()?;
    let text = String::from_utf8(optional(&udta, b"name").ok()??.to_vec()).ok()?;
    (!text.is_empty()).then_some(text)
}

impl<R: Read + Seek> Mp4Reader<R> {
    pub fn open(mut reader: R, limits: Limits) -> Result<Self> {
        let end = reader.seek(SeekFrom::End(0))?;
        let mut at = 0u64;
        let mut moov = None;
        let mut mdats = Vec::new();
        let mut fragments: Vec<Fragment> = Vec::new();
        let mut fragment_bytes = 0usize;
        let mut boxes = 0;
        while at < end {
            boxes += 1;
            if boxes > 100_000 || end - at < 8 {
                return Err(invalid("invalid MP4 top-level boxes"));
            }
            reader.seek(SeekFrom::Start(at))?;
            let mut h = [0; 8];
            reader.read_exact(&mut h)?;
            let size = u32::from_be_bytes(h[..4].try_into().unwrap());
            let (size, header) = if size == 1 {
                let mut large = [0; 8];
                reader.read_exact(&mut large)?;
                (u64::from_be_bytes(large), 16)
            } else if size == 0 {
                (end - at, 8)
            } else {
                (u64::from(size), 8)
            };
            if size < header || size > end - at {
                return Err(invalid(&format!(
                    "Invalid MP4 box {:?} at byte {at}: declared size {size}, header {header}, remaining file bytes {}",
                    String::from_utf8_lossy(&h[4..]),
                    end - at
                )));
            }
            match &h[4..] {
                b"moov" => {
                    if moov.is_some() {
                        return Err(invalid("duplicate moov"));
                    }
                    let len = usize::try_from(size - header)
                        .map_err(|_| invalid("moov size overflow"))?;
                    if len > limits.metadata_bytes {
                        return Err(invalid("MP4 metadata exceeds budget"));
                    }
                    let mut data = crate::buffer(len)?;
                    reader.read_exact(&mut data)?;
                    moov = Some(data);
                }
                b"mdat" => mdats.push(at + header..at + size),
                // A fragment's header is index, not media: like `moov` it is read
                // into memory and walked, and the bytes it points at stay on disk
                // until a packet is asked for.
                b"moof" => {
                    let len = usize::try_from(size - header)
                        .map_err(|_| invalid("moof size overflow"))?;
                    fragment_bytes += len;
                    if fragment_bytes > limits.metadata_bytes {
                        return Err(invalid("MP4 fragment index exceeds budget"));
                    }
                    let mut data = crate::buffer(len)?;
                    reader.read_exact(&mut data)?;
                    fragments.push(Fragment { start: at, data });
                }
                _ => {}
            }
            at += size;
        }
        let moov = moov.ok_or_else(|| invalid("missing MP4 moov"))?;
        let movie = atoms(&moov)?;
        // Either half says the file indexes its samples by fragment: the movie
        // header can promise them with `mvex` before a single one is written, and
        // a file that holds a `moof` has written one whatever it promised.
        let fragmented = optional(&movie, b"mvex")?.is_some() || !fragments.is_empty();
        if fragmented && fragments.is_empty() {
            // An init segment on its own is the whole of what such a file holds,
            // and it names no byte of media: VLC opens it as nothing, so it is
            // refused as the incomplete file it is rather than played silent.
            return Err(invalid(
                "fragmented MP4 states no fragments: it is an init segment without media",
            ));
        }
        let (movie_timescale, movie_duration) = media_time(required(&movie, b"mvhd")?)?;
        let mut tracks = Vec::new();
        let mut refused = Vec::new();
        let mut remaining = limits.samples;
        let mut skipped = 0usize;
        for atom in movie.iter().filter(|a| &a.kind == b"trak") {
            if tracks.len() >= limits.tracks {
                return Err(invalid("MP4 track limit exceeded"));
            }
            let Some(track) = parse_track(atom.data, &mdats, remaining, fragmented, &mut refused)?
            else {
                skipped += 1;
                continue;
            };
            if tracks.iter().any(|t: &Track| t.id == track.id) {
                return Err(invalid("duplicate track ID"));
            }
            remaining -= track.samples.cost();
            tracks.push(track);
        }
        if fragmented {
            index_fragments(
                &fragments,
                &fragment_defaults(&movie)?,
                &mdats,
                &mut tracks,
                &mut remaining,
            )?;
        }
        if tracks.is_empty() {
            // A file whose every track is a format there is no decoder for is
            // refused by naming those formats, as something the player has to
            // implement; a file whose tracks it could not even index is refused
            // as the container gap it is. "no tracks" would send the reader
            // looking for an empty moov instead of either.
            return Err(if refused.is_empty() {
                invalid(if skipped == 0 {
                    "MP4 contains no tracks"
                } else {
                    "MP4 has no track in a codec this reader plays (supported: AVC, HEVC, VP9, AV1 video; MPEG-4 audio)"
                })
            } else {
                unsupported(&format!(
                    "MP4 has no track in a codec this player plays: it read tracks coded {}",
                    refused_names(&refused)
                ))
            });
        }
        let udta = optional(&movie, b"udta")?;
        let chapters = read_chpl(udta, movie_timescale, movie_duration);
        let tags = read_tags(udta);
        Ok(Self {
            reader,
            tracks,
            refused,
            chapters,
            tags,
            movie_timescale,
            limits,
            position: u64::MAX,
        })
    }
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }
    /// Tracks read to the end of their sample entry and then set aside because
    /// the coding it names has no arm here, in container order.
    pub fn refused(&self) -> &[SkippedTrack] {
        &self.refused
    }
    pub fn movie_timescale(&self) -> u32 {
        self.movie_timescale
    }
    /// The `chpl` list of `moov/udta`, in order of their start; empty when the
    /// movie names no chapters.
    pub fn chapters(&self) -> &[Chapter] {
        &self.chapters
    }
    /// What `moov/udta` says about the movie as a whole.
    pub fn tags(&self) -> &FileTags {
        &self.tags
    }
    /// Reads exactly one indexed packet; the caller may reuse the buffer.
    pub fn read_packet(&mut self, track: usize, sample: usize, output: &mut Vec<u8>) -> Result<()> {
        let sample = self
            .tracks
            .get(track)
            .and_then(|t| t.samples.get(sample))
            .ok_or_else(|| invalid("MP4 packet index out of range"))?;
        let size = sample.size as usize;
        if size > self.limits.packet_bytes {
            return Err(invalid("MP4 packet exceeds budget"));
        }
        output.clear();
        output
            .try_reserve(size)
            .map_err(|_| invalid("packet allocation failed"))?;
        output.resize(size, 0);
        // Sequential playback reads samples back-to-back inside one mdat chunk,
        // so the file is usually already positioned at the next packet.
        if self.position != sample.offset {
            self.reader.seek(SeekFrom::Start(sample.offset))?;
        }
        if let Err(error) = self.reader.read_exact(output) {
            output.clear();
            self.position = u64::MAX;
            return Err(error.into());
        }
        self.position = sample.offset.saturating_add(u64::from(sample.size));
        Ok(())
    }
    /// Forget the tracked read position; the next `read_packet` seeks again.
    pub fn invalidate_position(&mut self) {
        self.position = u64::MAX;
    }
}

/// The uncompressed PCM tags whose sample entry is the whole description.
/// QuickTime's chapter list, `chpl` under `moov/udta`: a count, then one moment
/// and one length-stated title per chapter. QuickTime counts those moments in
/// the movie timescale and `ffmpeg` in tens of megahertz; the film's own length
/// tells the two readings apart, since no chapter starts after it ends. What a
/// chapter runs until is not asked here, so the entries are taken as the starts
/// `ffmpeg` writes them as rather than the ends an older writer means.
fn read_chpl(udta: Option<&[u8]>, timescale: u32, duration: u64) -> Vec<Chapter> {
    // A list this reader cannot walk leaves no chapters at all instead of
    // failing the file, which plays perfectly well without them.
    let Ok(children) = atoms(udta.unwrap_or_default()) else {
        return Vec::new();
    };
    let Ok(Some(list)) = optional(&children, b"chpl") else {
        return Vec::new();
    };
    // Version and flags, then the four bytes no writer explains.
    let Some(&count) = list.get(8) else {
        return Vec::new();
    };
    let nanos = |ticks: i64, per_second: u32| {
        i128::from(ticks)
            .checked_mul(1_000_000_000)?
            .checked_div(i128::from(u64::from(per_second.max(1))))
            .and_then(|value| i64::try_from(value).ok())
    };
    let mut runs: Vec<(i64, String)> = Vec::new();
    let mut at = 9;
    for _ in 0..usize::from(count).min(1_024) {
        let Some(bytes) = list.get(at..at + 8) else {
            break;
        };
        let raw = <[u8; 8]>::try_from(bytes).unwrap_or([0; 8]);
        at += 8;
        let Some(width) = list.get(at).copied() else {
            break;
        };
        at += 1;
        let Some(title) = list.get(at..at + usize::from(width)) else {
            break;
        };
        at += usize::from(width);
        runs.push((
            i64::try_from(u64::from_be_bytes(raw)).unwrap_or(i64::MAX),
            String::from_utf8_lossy(title).into_owned(),
        ));
    }
    let ends = nanos(i64::try_from(duration).unwrap_or(i64::MAX), timescale).unwrap_or(i64::MAX);
    let last = runs.iter().map(|(start, _)| *start).max().unwrap_or(0);
    let per_second = match (nanos(last, timescale), nanos(last, 10_000_000)) {
        (Some(spaced), Some(quick)) if spaced > ends && quick <= ends => 10_000_000,
        _ => timescale,
    };
    runs.into_iter()
        .filter_map(|(start, title)| {
            Some(Chapter {
                start_ns: nanos(start.max(0), per_second)? as u64,
                title,
            })
        })
        .collect()
}
/// The text a tag holds, in the one encoding this reader can spell: UTF-8, with
/// the trailing NUL byte some writers add trimmed away. A fact written any other
/// way is left unstated, since a wrong name is worse than no name at all.
fn tag_text(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim_end_matches('\0');
    (!text.is_empty()).then(|| text.to_owned())
}

/// The two numbers of a place-box, `trkn` for a track and `disk` for a disc,
/// kept the way the file states them: the place on its own, or `number/total`
/// when the file knows the album's length too. A zero number is no place at
/// all — a writer that fills the box with the other half only leaves this one
/// unstated — and so is a box too short to hold the pair. Text answers `None`:
/// `tag_text` is the reader for those.
fn pair_text(bytes: &[u8]) -> Option<String> {
    let number = u16be(bytes, 2).ok()?;
    if number == 0 {
        return None;
    }
    let total = u16be(bytes, 4).ok()?;
    Some(if total == 0 {
        number.to_string()
    } else {
        format!("{number}/{total}")
    })
}

/// The movie tag boxes, by the fact each one holds. The last two are the
/// iTunes-style spellings `ffmpeg` writes for copyright and description —
/// plain lowercase atoms, not the `©`-prefixed QuickTime boxes — which is what
/// the fixture-generation gate produces.
const TAGS: [(&str, &[u8; 4]); 9] = [
    ("TITLE", b"\xa9nam"),
    ("ALBUMARTIST", b"aART"),
    ("ARTIST", b"\xa9ART"),
    ("ALBUM", b"\xa9alb"),
    ("GENRE", b"\xa9gen"),
    ("DATE", b"\xa9day"),
    ("COMMENT", b"\xa9cmt"),
    ("COPYRIGHT", b"cprt"),
    ("DESCRIPTION", b"desc"),
];

/// What the movie's own boxes say about it, which two families of writers spell
/// differently. A QuickTime file states each fact as text standing in the tag
/// itself under `moov/udta`; an iTunes-style file, which is what `ffmpeg`
/// writes, nests the same tags in `meta/ilst` with the text behind a `data`
/// box. Where a file states one fact both ways the plain box answers, being the
/// older and the more direct of the two writings. The flags naming a text's
/// encoding are not consulted — the bytes are read as UTF-8, which is what a
/// fact in any other encoding fails to be. Nothing here fails a file: a list
/// this reader cannot walk leaves the movie stating nothing.
fn read_tags(udta: Option<&[u8]>) -> FileTags {
    let mut tags = FileTags::default();
    let Ok(children) = atoms(udta.unwrap_or_default()) else {
        return tags;
    };
    for (fact, kind) in TAGS {
        let text = optional(&children, kind).ok().flatten().and_then(tag_text);
        if let Some(text) = text {
            tags.insert(fact, &text);
        }
    }
    // QuickTime writes the track's two numbers as a bare binary box beside
    // the text ones.
    for (fact, kind) in [("TRACK", b"trkn"), ("DISC", b"disk")] {
        if let Some(place) = optional(&children, kind).ok().flatten().and_then(pair_text) {
            tags.insert(fact, &place);
        }
    }
    let Ok(Some(meta)) = optional(&children, b"meta") else {
        return tags;
    };
    // `meta` is a versioned box: its children start behind four bytes of version
    // and flags, which are no box of their own.
    let Ok(lists) = atoms(meta.get(4..).unwrap_or_default()) else {
        return tags;
    };
    let Ok(Some(list)) = optional(&lists, b"ilst") else {
        return tags;
    };
    let Ok(entries) = atoms(list) else {
        return tags;
    };
    for (fact, kind) in TAGS {
        let text = optional(&entries, kind)
            .ok()
            .flatten()
            .and_then(|entry| atoms(entry).ok())
            .and_then(|parts| optional(&parts, b"data").ok().flatten())
            // The `data` box puts its version and flags, then four bytes no
            // writer uses, before the text itself.
            .and_then(|data| tag_text(data.get(8..).unwrap_or_default()));
        if let Some(text) = text {
            tags.insert(fact, &text);
        }
    }
    // The iTunes layout is the one ffmpeg writes for these places too: the
    // same bare keys, the same binary behind the `data` box header.
    for (fact, kind) in [("TRACK", b"trkn"), ("DISC", b"disk")] {
        let place = optional(&entries, kind)
            .ok()
            .flatten()
            .and_then(|entry| atoms(entry).ok())
            .and_then(|parts| optional(&parts, b"data").ok().flatten())
            .and_then(|data| pair_text(data.get(8..).unwrap_or_default()));
        if let Some(place) = place {
            tags.insert(fact, &place);
        }
    }
    tags
}

fn is_pcm(codec: &[u8; 4]) -> bool {
    matches!(codec, b"sowt" | b"twos" | b"fl32" | b"fl64")
}

/// The ADPCM tags QuickTime writes, each spelling its coding in the entry's own
/// fourcc: the two `ms\0\xNN` ones carry the WAVE format tag in their last byte
/// (2 for Microsoft's coding, 0x11 for IMA's in a WAV-shaped block) and `ima4`
/// is Apple's IMA.
fn is_adpcm(codec: &[u8; 4]) -> bool {
    matches!(codec, b"ima4" | b"ms\x00\x02" | b"ms\x00\x11")
}

/// Apple's two MACE codings, whose entries state nothing beyond the geometry every
/// audio entry carries: each coding's block is its own - two bytes a channel for
/// 3-to-1, one for 6-to-1 - and the entry's sample width field, which a writer sets
/// to 8 for both, describes no layout anyone reads.
fn is_mace(codec: &[u8; 4]) -> bool {
    matches!(codec, b"MAC3" | b"MAC6")
}

/// How many coded samples one MACE block holds, whichever coding spells it: the
/// 3-to-1 spends two bytes a channel on six samples and the 6-to-1 one byte for
/// the same six, because the second interpolates each of a byte's three codes
/// into a pair. The reference derives both widths from the fourcc rather than
/// from a field of the entry, which is right - the shipped takes' sample width
/// states 8 for codings that cost four bytes a block.
const MACE_SAMPLES_PER_BLOCK: usize = 6;

/// The size of one MACE block for an entry's channel count, and the number of
/// samples it codes, or `None` for any other coding. A track of this coding is
/// indexed in blocks rather than in the samples its `stsz` entries count,
/// because its packets are blocks and no field of the table says so.
fn mace_block(codec: &[u8; 4], channels: u16) -> Option<(usize, usize)> {
    let bytes_per_channel = match codec {
        b"MAC3" => 2,
        b"MAC6" => 1,
        _ => return None,
    };
    let channels = usize::from(channels);
    Some((
        MACE_SAMPLES_PER_BLOCK,
        channels.checked_mul(bytes_per_channel)?,
    ))
}

/// The codings a movie was set aside for, spelled as their entries spell
/// themselves and listed once each, each with the layer it sits in: a refusal
/// that names them says which arms are missing rather than reciting which ones
/// are not, and a fourcc alone does not say whether it codes a picture or a
/// sound.
fn refused_names(refused: &[SkippedTrack]) -> String {
    let mut names: Vec<String> = Vec::new();
    for track in refused {
        let name = format!(
            "{} ({})",
            String::from_utf8_lossy(&track.codec),
            match &track.handler {
                b"vide" => "video",
                b"soun" => "audio",
                _ => "another handler",
            }
        );
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.join(", ")
}

/// The `WAVEFORMATEX` a compressed audio entry stores inside its `wave` atom,
/// which is where Microsoft's two ADPCM spellings state how long a block is.
/// `at` is where the entry's own atoms begin, which differs between the two
/// versions of the description. `None` when the entry carries no such record,
/// which the caller reads as a track it has no geometry for.
fn wave_record(data: &[u8], at: usize, tag: [u8; 4]) -> Option<Vec<u8>> {
    let children = atoms(data.get(at..)?).ok()?;
    let wave = optional(&children, b"wave").ok().flatten()?;
    let parts = atoms(wave).ok()?;
    optional(&parts, &tag).ok().flatten().map(<[u8]>::to_vec)
}

/// The fields every audio sample entry has: channel count, sample width and the
/// 16.16 fixed-point rate, all at the offsets QuickTime version 0 puts them at.
/// Version 1 only appends fields after the rate, so they stay put there too;
/// `false` reports an entry of a version this reader will not guess at.
fn audio_entry(data: &[u8], result: &mut Track) -> Result<bool> {
    if u16be(data, 8)? > 1 {
        return Ok(false);
    }
    result.channels = u16be(data, 16)?;
    result.bit_depth = u16be(data, 18)?;
    result.sample_rate = u32be(data, 24)? >> 16;
    Ok(true)
}

/// How much wider a coded pixel is than it is tall.
///
/// Two boxes can state it. `pasp` gives the shape of one stored pixel outright;
/// `tkhd` gives the size the track is drawn at, which for a picture set down
/// upright can differ from the coded size for no other reason. A writer that
/// knows both writes the same answer twice, and the pixel's own box is the one
/// that travels with the stream, so it is asked first; a writer that knows only
/// `tkhd` — which is how most non-square video is muxed — is still answered.
///
/// A quarter turn changes which coded edge the drawn width runs along, so the
/// coded size is turned the same way before the two are compared. A phone
/// recording of 1920×1080 coded pixels drawn at 1080×1920 is square, which is
/// what it is.
fn pixel_aspect(
    spacing: Option<(u32, u32)>,
    drawn: (u32, u32),
    rotation: u16,
    coded: (u32, u32),
) -> (u32, u32) {
    // Both boxes state the shape in the axes the picture is stored in, and the
    // player stretches the picture it is shown, which for a quarter turn is the
    // other way round.
    let quarter = rotation == 90 || rotation == 270;
    if let Some((h, v)) = spacing.filter(|(h, v)| *h != 0 && *v != 0) {
        let (h, v) = if quarter { (v, h) } else { (h, v) };
        return super::reduce_ratio(u64::from(h), u64::from(v));
    }
    // A writer that draws the track at exactly its coded size says nothing
    // about the shape of a pixel. That has to be heard as nothing before the
    // turn is taken into account, because turning a square pair of ratios
    // leaves the same answer only for a square picture: a 1920x1080 recording
    // drawn at 1920x1080 through a quarter turn would otherwise be read as
    // pixels more than three times as wide as they are tall.
    if u64::from(drawn.0) * u64::from(coded.1) == u64::from(drawn.1) * u64::from(coded.0) {
        return (1, 1);
    }
    let coded = if quarter { (coded.1, coded.0) } else { coded };
    if drawn.0 == 0 || drawn.1 == 0 || coded.0 == 0 || coded.1 == 0 {
        return (1, 1);
    }
    // (drawn width / coded width) over (drawn height / coded height), with the
    // fixed-point scale of the drawn size cancelling between the two.
    super::reduce_ratio(
        u64::from(drawn.0) * u64::from(coded.1),
        u64::from(drawn.1) * u64::from(coded.0),
    )
}

/// Which quarter turn makes the coded picture upright, read from the linear
/// part of `tkhd`'s transform: `m11`, `m12`, `m21`, `m22` in 16.16. A header
/// places a point with a row vector, so the coded corner `(x, y)` lands at
/// `(m11·x + m21·y, m12·x + m22·y)`, and the turn is whichever of the four
/// identity-sized matrices does that. A matrix that mirrors, scales or skews is
/// none of the four, and the picture is left as it was decoded rather than
/// guessed at.
fn rotation_from_matrix(linear: [i32; 4]) -> u16 {
    const UNIT: i32 = 0x0001_0000;
    if linear == [UNIT, 0, 0, UNIT] {
        0
    } else if linear == [0, UNIT, -UNIT, 0] {
        90
    } else if linear == [-UNIT, 0, 0, -UNIT] {
        180
    } else if linear == [0, -UNIT, UNIT, 0] {
        270
    } else {
        0
    }
}

/// Parse one `trak` into an indexable track, or `None` when it holds a format
/// this reader has no decoder for. Unplayable tracks are dropped before their
/// sample tables are built so a subtitle, timecode or ALAC track alongside the
/// picture costs nothing, the way it does in a player with a narrower codec set.
/// A track dropped for the coding its own entry names is recorded in `refused`
/// on the way out, so the caller knows which arm is missing.
fn parse_track(
    data: &[u8],
    mdats: &[Range<u64>],
    limit: usize,
    fragmented: bool,
    refused: &mut Vec<SkippedTrack>,
) -> Result<Option<Track>> {
    let track = atoms(data)?;
    let tkhd = required(&track, b"tkhd")?;
    let narrow = version(tkhd, 1)? == 0;
    let id = u32be(tkhd, if narrow { 12 } else { 20 })?;
    if id == 0 {
        return Err(invalid("zero MP4 track ID"));
    }
    // The transform that places the picture and the size it is drawn at, which
    // are the last fields of the header and whose place depends only on its
    // version. Both are 16.16, so the drawn size stays fixed-point here: the
    // ratio of two of them is the same whether or not the whole part is taken.
    let at = if narrow { 40 } else { 52 };
    // The transform, in the order the header lays it out: the linear part is
    // `m11`, `m12` on the first row and `m21`, `m22` on the second, so the two
    // off-diagonal terms are the ones that turn the picture.
    let linear = [
        u32be(tkhd, at)? as i32,
        u32be(tkhd, at + 4)? as i32,
        u32be(tkhd, at + 12)? as i32,
        u32be(tkhd, at + 16)? as i32,
    ];
    let rotation = rotation_from_matrix(linear);
    let drawn = (u32be(tkhd, at + 36)?, u32be(tkhd, at + 40)?);
    let mdia = atoms(required(&track, b"mdia")?)?;
    let mdhd = required(&mdia, b"mdhd")?;
    let (timescale, duration) = media_time(mdhd)?;
    let language = media_language(mdhd).unwrap_or_default();
    let name = track_name(&track).unwrap_or_default();
    let handler: [u8; 4] = required(&mdia, b"hdlr")?
        .get(8..12)
        .ok_or_else(|| invalid("truncated MP4 handler"))?
        .try_into()
        .unwrap();
    let minf = atoms(required(&mdia, b"minf")?)?;
    // External data references must never turn packet offsets into arbitrary file access.
    let dinf = atoms(required(&minf, b"dinf")?)?;
    let dref = required(&dinf, b"dref")?;
    version(dref, 0)?;
    let references = atoms(
        dref.get(8..)
            .ok_or_else(|| invalid("truncated data references"))?,
    )?;
    // ISO files use `url `, QuickTime (macOS screen recordings) `alis`; either is
    // acceptable only with the self-contained flag and no external location.
    if u32be(dref, 4)? != 1
        || references.len() != 1
        || !(references[0].kind == *b"url " || references[0].kind == *b"alis")
        || references[0].data != [0, 0, 0, 1]
    {
        return Err(invalid(
            "only self-contained MP4 data references are supported",
        ));
    }
    let stbl = atoms(required(&minf, b"stbl")?)?;
    let stsd = required(&stbl, b"stsd")?;
    version(stsd, 0)?;
    if u32be(stsd, 4)? != 1 {
        return Err(invalid(
            "multiple sample descriptions are not yet supported",
        ));
    }
    let entries = atoms(
        stsd.get(8..)
            .ok_or_else(|| invalid("truncated sample description"))?,
    )?;
    if entries.len() != 1 {
        return Err(invalid("invalid sample description count"));
    }
    let entry = entries[0];
    if u16be(entry.data, 6)? != 1 {
        return Err(invalid("invalid sample data reference"));
    }
    let mut result = Track {
        id,
        handler,
        codec: entry.kind,
        name,
        language,
        timescale,
        duration,
        width: 0,
        height: 0,
        channels: 0,
        sample_rate: 0,
        bit_depth: 0,
        configuration: Vec::new(),
        edits: Vec::new(),
        samples: SampleIndex::Expanded(Vec::new()),
        pixel_aspect: (1, 1),
        rotation: 0,
    };
    // The byte offset the entry's child atoms start at, and which of them holds
    // the codec configuration; PCM has none, its entry is the whole description.
    let (config_at, config_kind) = match (&handler, &entry.kind) {
        (b"vide", b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"vp09" | b"av01") => {
            result.width = u16be(entry.data, 24)?;
            result.height = u16be(entry.data, 26)?;
            if result.width == 0 || result.height == 0 {
                return Err(invalid("zero video dimensions"));
            }
            (
                78,
                Some(match &entry.kind {
                    b"avc1" | b"avc3" => b"avcC",
                    b"hvc1" | b"hev1" => b"hvcC",
                    b"vp09" => b"vpcC",
                    b"av01" => b"av1C",
                    _ => unreachable!(),
                }),
            )
        }
        (b"soun", b"mp4a") => {
            // Only version 0 keeps `esds` at the offset the spec gives it; a later
            // version inserts its extra fields before the configuration, and the
            // track is skipped rather than searched at the wrong place.
            if !audio_entry(entry.data, &mut result)? || u16be(entry.data, 8)? != 0 {
                return Ok(None);
            }
            (28, Some(b"esds"))
        }
        // Uncompressed PCM, as QuickTime writes it for a screen recording: the
        // fourcc states the byte order and `sample_size` the depth, so the entry
        // is the whole description and no configuration atom follows. The other
        // PCM spellings (`in24`, `l16`, `raw `) state their width or signedness
        // in child atoms the reader does not resolve, so they are skipped.
        (b"soun", codec) if is_pcm(codec) => {
            if !audio_entry(entry.data, &mut result)? {
                return Ok(None);
            }
            (entry.data.len(), None)
        }
        // ADPCM. Apple's `ima4` needs nothing beyond the entry's own fields: its
        // block is fixed by the format. Microsoft's two spellings are read in
        // blocks whose length only the `wave` record states, so a track without
        // one goes by unfound rather than by a length this reader makes up.
        (b"soun", codec) if is_adpcm(codec) => {
            if !audio_entry(entry.data, &mut result)? {
                return Ok(None);
            }
            // A version 1 description, which is how a QuickTime writer spells
            // these, puts sixteen bytes of its own between the sample rate and
            // the atoms; version 0 goes straight to them.
            let at = usize::from(u16be(entry.data, 8)? != 0) * 16 + 28;
            result.configuration = wave_record(entry.data, at, entry.kind).unwrap_or_default();
            (entry.data.len(), None)
        }
        // Apple MACE: the entry's own fields are the whole description, as with PCM.
        // What the fourcc adds is the block - two bytes a channel for 3-to-1, one for
        // 6-to-1, six samples for either - which no field of the entry states, and
        // which is why a track of this coding is indexed in blocks rather than in the
        // samples its `stsz` counts: measured over the shipped take, that table states
        // one byte for each of 630 630 entries where the 420 420 bytes of data are
        // 105 105 blocks of four. The reference coalesces those blocks into packets of
        // up to 1020 samples, which is a demuxer's convenience rather than a fact of
        // the coding; this reader hands one block a packet, and measures the take's
        // decoded output as identical either way.
        (b"soun", codec) if is_mace(codec) => {
            if !audio_entry(entry.data, &mut result)? {
                return Ok(None);
            }
            (entry.data.len(), None)
        }
        // Apple Lossless: the entry's own `alac` box, nested where an `esds` would
        // sit, holds the geometry every frame is measured against. Its first four
        // bytes are the cookie's version, which says nothing a decoder needs, so
        // the setup data handed on starts at the frame length - the same 24 bytes a
        // Matroska track carries as `A_ALAC` private data.
        (b"soun", b"alac") => {
            if !audio_entry(entry.data, &mut result)? {
                return Ok(None);
            }
            (28, Some(b"alac"))
        }
        // A text subtitle: `tx3g`, which is what a muxer writes for a track
        // whose handler calls itself a subtitle (`sbtl`), plain text (`text`) or
        // MPEG-4 text (`subt`). The sample entry describes how the letters are
        // drawn, none of which this player asks; the samples are the lines
        // themselves, so the entry is the whole description here too.
        (b"sbtl" | b"text" | b"subt", b"tx3g") => (entry.data.len(), None),
        // The entry was read right through and says what it codes; there is only
        // no arm here for that coding, which is a gap of a different kind from a
        // box this reader could not parse.
        _ => {
            refused.push(SkippedTrack {
                handler,
                codec: entry.kind,
            });
            return Ok(None);
        }
    };
    let configs = atoms(
        entry
            .data
            .get(config_at.min(entry.data.len())..)
            .ok_or_else(|| invalid("truncated sample entry"))?,
    )?;
    if let Some(kind) = config_kind {
        let atom = required(&configs, kind)?;
        // An ALAC cookie opens with four bytes of its own version, which a Matroska
        // track leaves out; past them both spell the same fields.
        let at = usize::from(kind == b"alac") * 4;
        result.configuration = atom.get(at..).unwrap_or_default().to_vec();
    }
    // A fragmented track describes its coding here and its samples nowhere: the
    // tables beside this entry either are absent or hold one entry each at zero,
    // and the `moof` boxes further along the file own the index. The empty list
    // is what the fragment pass then fills.
    let index = if fragmented {
        SampleIndex::Expanded(Vec::new())
    } else {
        let Some(index) = sample_index(&stbl, &result.codec, result.channels, mdats, limit)? else {
            return Ok(None);
        };
        index
    };
    result.samples = index;
    let spacing =
        optional(&configs, b"pasp")?.and_then(|p| Some((u32be(p, 0).ok()?, u32be(p, 4).ok()?)));
    result.rotation = rotation;
    result.pixel_aspect = pixel_aspect(
        spacing,
        drawn,
        rotation,
        (u32::from(result.width), u32::from(result.height)),
    );
    if let Some(edts) = optional(&track, b"edts")? {
        let children = atoms(edts)?;
        let elst = required(&children, b"elst")?;
        let v = version(elst, 1)?;
        let stride = if v == 0 { 12 } else { 20 };
        for i in 0..table(elst, stride, 1024)? {
            let at = 8 + i * stride;
            let (duration, media_time, rate_at) = if v == 0 {
                (
                    u64::from(u32be(elst, at)?),
                    i64::from(u32be(elst, at + 4)? as i32),
                    at + 8,
                )
            } else {
                (u64be(elst, at)?, u64be(elst, at + 8)? as i64, at + 16)
            };
            if media_time < -1 || u32be(elst, rate_at)? != 0x0001_0000 {
                return Err(invalid("unsupported MP4 edit rate or media time"));
            }
            result.edits.push(Edit {
                duration,
                media_time,
            });
        }
    }
    Ok(Some(result))
}

/// The sample tables of one track collapsed into an index, or `None` when the
/// track holds more index than the budget allows.
///
/// An uncompressed audio track states one size for every frame in `stsz` and
/// one length for every frame in a single `stts` run, so frame `n` of a chunk
/// starts `n * size` past the chunk offset and `n * length` past the run's
/// start. Indexing that as a record per chunk instead of a record per frame is
/// what lets a long recording be played at all: five minutes of 48 kHz stereo
/// PCM is 13 million frames, which as per-frame records cost 529 MB of index
/// against the 861 bytes of header the file actually carries.
///
/// A MACE track is indexed the same way over blocks rather than frames, since
/// its packets are blocks and its tables count samples; `channels` is what the
/// coding needs to size a block.
fn sample_index(
    stbl: &[Atom<'_>],
    codec: &[u8; 4],
    channels: u16,
    mdats: &[Range<u64>],
    limit: usize,
) -> Result<Option<SampleIndex>> {
    let stsz = required(stbl, b"stsz")?;
    version(stsz, 0)?;
    let fixed_size = u32be(stsz, 4)?;
    let count = u32be(stsz, 8)? as usize;
    let expected = if fixed_size == 0 {
        count.checked_mul(4).and_then(|n| n.checked_add(12))
    } else {
        Some(12)
    };
    if expected != Some(stsz.len()) {
        return Err(invalid("invalid sample-size table"));
    }
    let size_at = |index: usize| -> Result<u32> {
        if fixed_size == 0 {
            u32be(stsz, 12 + index * 4)
        } else {
            Ok(fixed_size)
        }
    };
    let stts = required(stbl, b"stts")?;
    version(stts, 0)?;
    let mut timing = Vec::new();
    let mut timed = 0usize;
    let runs = table(stts, 8, count)?;
    for i in 0..runs {
        let run = u32be(stts, 8 + i * 8)? as usize;
        let delta = u32be(stts, 12 + i * 8)?;
        // The last run of a table may state no length at all: a muxer that ends
        // a track with a sample of its own duration nothing says so, and no
        // later sample needs the gap. A zero in the middle would put two
        // samples at one instant, which is refused.
        if run == 0 || run > count - timed || (delta == 0 && i + 1 < runs) {
            return Err(invalid("invalid decode-time run"));
        }
        timed += run;
        timing.push((run, delta));
    }
    if timed != count {
        return Err(invalid("decode-time count differs from sample count"));
    }
    // Everything the compact form would have to guess is a table it refuses to
    // read alongside it: varying sizes, a second frame length, a composition
    // shift, or a subset of frames worth restarting from.
    //
    // The unit the compact form indexes in is one frame for a table that states
    // a frame's whole size, which uncompressed PCM does. A MACE track's table
    // states something else: measured over the shipped takes, `stsz` counts one
    // entry per coded sample - 630 630 of them at one byte each in the 3-to-1
    // movie, 17 856 in the 6-to-1 one - and `stts` runs the same count at one
    // tick, while the data is blocks of six samples costing four bytes and one.
    // So the unit there is the block, sized and timed by the coding; the chunks
    // divide into it exactly in both takes, as they must for the reference's own
    // reading of the same tables.
    let unit = if is_pcm(codec) && fixed_size != 0 {
        Some((1, usize::try_from(fixed_size).unwrap_or(usize::MAX)))
    } else {
        mace_block(codec, channels)
    };
    let Some((samples_per_unit, bytes_per_unit)) = unit else {
        return expanded_index(stbl, codec, count, timing, limit, size_at, mdats);
    };
    if timing.len() == 1 && optional(stbl, b"ctts")?.is_none() && optional(stbl, b"stss")?.is_none()
    {
        let units = match count.checked_div(samples_per_unit) {
            Some(units) if count % samples_per_unit == 0 => units,
            _ => return Err(invalid("sample count is not a whole number of blocks")),
        };
        let ticks = u64::from(timing[0].1)
            .checked_mul(u64::try_from(samples_per_unit).unwrap_or(u64::MAX))
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| invalid("block duration overflow"))?;
        let chunk_bytes = |_: usize, units: usize| {
            u64::try_from(units)
                .ok()
                .and_then(|n| n.checked_mul(u64::try_from(bytes_per_unit).unwrap_or(u64::MAX)))
                .ok_or_else(|| invalid("chunk size overflow"))
        };
        let runs = chunk_runs(stbl, units, mdats, samples_per_unit, chunk_bytes)?;
        let index = SampleIndex::Uniform(UniformIndex {
            bytes_per_frame: u32::try_from(bytes_per_unit)
                .map_err(|_| invalid("block size exceeds a frame's byte count"))?,
            duration: ticks,
            count: units,
            runs,
        });
        if index.cost() > limit {
            return Ok(None);
        }
        return Ok(Some(index));
    }
    expanded_index(stbl, codec, count, timing, limit, size_at, mdats)
}

/// The per-sample form: one record for every entry the sample tables count,
/// which is the only shape a track with varying sizes, a composition shift or
/// reference pictures can take.
fn expanded_index(
    stbl: &[Atom<'_>],
    codec: &[u8; 4],
    count: usize,
    timing: Vec<(usize, u32)>,
    limit: usize,
    size_at: impl Fn(usize) -> Result<u32>,
    mdats: &[Range<u64>],
) -> Result<Option<SampleIndex>> {
    if count > limit {
        // A PCM frame costs an index entry each, so a long recording of it can
        // be over budget for a reason no compressed track is. Leaving such a
        // track out costs only the sound a reader without PCM support would not
        // have played anyway; refusing the file would cost the picture beside
        // it. The uniform form is what most such tracks use now.
        if is_pcm(codec) {
            return Ok(None);
        }
        return Err(invalid("MP4 sample count exceeds budget"));
    }
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(count)
        .map_err(|_| invalid("sample index allocation failed"))?;
    for index in 0..count {
        samples.push(Sample {
            offset: 0,
            size: size_at(index)?,
            dts: 0,
            pts: 0,
            duration: 0,
            sync: true,
        });
    }
    let mut at = 0usize;
    let mut dts = 0u64;
    for (run, delta) in timing {
        for sample in &mut samples[at..at + run] {
            sample.dts = dts;
            sample.pts = i64::try_from(dts).map_err(|_| invalid("MP4 timestamp overflow"))?;
            sample.duration = delta;
            dts = dts
                .checked_add(u64::from(delta))
                .ok_or_else(|| invalid("MP4 timestamp overflow"))?;
        }
        at += run;
    }
    if let Some(ctts) = optional(stbl, b"ctts")? {
        let v = version(ctts, 1)?;
        let mut at = 0;
        for i in 0..table(ctts, 8, count)? {
            let run = u32be(ctts, 8 + i * 8)? as usize;
            let offset = u32be(ctts, 12 + i * 8)?;
            let offset = if v == 1 {
                i64::from(offset as i32)
            } else {
                i64::from(offset)
            };
            if run == 0 || run > count - at {
                return Err(invalid("invalid composition-time run"));
            }
            for sample in &mut samples[at..at + run] {
                sample.pts = sample
                    .pts
                    .checked_add(offset)
                    .ok_or_else(|| invalid("MP4 PTS overflow"))?;
            }
            at += run;
        }
        if at != count {
            return Err(invalid("composition-time count differs from sample count"));
        }
    }
    if let Some(stss) = optional(stbl, b"stss")? {
        version(stss, 0)?;
        for s in &mut samples {
            s.sync = false;
        }
        let mut last = 0;
        for i in 0..table(stss, 4, count)? {
            let index = u32be(stss, 8 + i * 4)? as usize;
            if index <= last || index > count {
                return Err(invalid("invalid sync sample index"));
            }
            samples[index - 1].sync = true;
            last = index;
        }
    }
    let chunk_bytes = |first: usize, frames: usize| {
        samples[first..first + frames]
            .iter()
            .try_fold(0u64, |sum, s| sum.checked_add(u64::from(s.size)))
            .ok_or_else(|| invalid("chunk size overflow"))
    };
    let runs = chunk_runs(stbl, count, mdats, 1, chunk_bytes)?;
    for run in runs {
        let mut offset = run.offset;
        for s in &mut samples[run.first..run.first + run.frames] {
            s.offset = offset;
            offset += u64::from(s.size);
        }
    }
    Ok(Some(SampleIndex::Expanded(samples)))
}

/// One `moof` read into memory: the byte its header starts at, and the box's own
/// contents. A fragment is index rather than media, so like `moov` it is walked
/// from a buffer while the samples it names stay on disk until asked for.
#[derive(Clone, Debug)]
struct Fragment {
    /// Where the box begins. This is what a run's data offset counts from
    /// whenever the fragment header says its own `moof` is the base, which is
    /// how this build's muxer spells every fragment it writes.
    start: u64,
    data: Vec<u8>,
}

/// What `moov/mvex/trex` promises for one track, for the fields a `traf` leaves
/// out. A sample's length and size can live in the init segment alone, which is
/// how a file that will hold a hundred fragments states them once.
#[derive(Clone, Copy, Debug)]
struct TrackExtends {
    track: u32,
    duration: u32,
    size: u32,
    /// `None` when the box states no flags at all, which leaves a sample's own
    /// run to say whether it may be restarted from.
    flags: Option<u32>,
}

/// The `tfhd` flag bits this reader acts on: the two ways of naming where a
/// fragment's bytes begin, the three defaults a run may leave out, and which
/// sample description the track uses - one, always, because a fragmented entry
/// list this reader describes holds exactly that.
const TFHD_SUPPORTED: u32 = 0x00_0001 | 0x00_0002 | 0x00_0008 | 0x00_0010 | 0x00_0020 | 0x02_0000;
/// The `trun` bits likewise: where the media starts, the flags of the run's first
/// sample, and the four per-sample fields, which the box always states in the
/// order length, size, flags, display shift.
const TRUN_SUPPORTED: u32 = 0x00_0001 | 0x00_0004 | 0x00_0100 | 0x00_0200 | 0x00_0400 | 0x00_0800;

fn fragment_defaults(moov: &[Atom<'_>]) -> Result<Vec<TrackExtends>> {
    let Some(mvex) = optional(moov, b"mvex")? else {
        return Ok(Vec::new());
    };
    let children = atoms(mvex)?;
    let mut out = Vec::new();
    for trex in children.iter().filter(|a| &a.kind == b"trex") {
        version(trex.data, 0)?;
        if u32be(trex.data, 8)? > 1 {
            return Err(invalid("MP4 fragment sample description index"));
        }
        let flags = u32be(trex.data, 20)?;
        out.push(TrackExtends {
            track: u32be(trex.data, 4)?,
            duration: u32be(trex.data, 12)?,
            size: u32be(trex.data, 16)?,
            flags: (flags != 0).then_some(flags),
        });
    }
    Ok(out)
}

/// Append what every fragment indexes to the track its own `traf` names, in the
/// order the boxes stand in the file.
///
/// A fragment states nothing of where its media sits by itself: the base is
/// either the absolute offset the track header gives or the `moof` the header
/// stands in, and only the first run of a track fragment offsets from that base.
/// Every later sample is measured from the end of the one before, so a file that
/// interleaves one fragment's picture and sound still resolves to one ordered
/// list per track.
///
/// Times are the track's own, in the track's own units: `tfdt` says where a
/// fragment's first sample is decoded and each sample's length carries the count
/// to the next, so a fragment that leaves `tfdt` out - which is what a muxer
/// appending to a live stream writes - continues where the last sample ended.
/// A run's composition shift is the only place a fragmented file says anything
/// about display order, and the one bit of a sample's flags that says "not
/// restartable from here" is the fragment form of a `stss` list.
fn index_fragments(
    fragments: &[Fragment],
    defaults: &[TrackExtends],
    mdats: &[Range<u64>],
    tracks: &mut [Track],
    limit: &mut usize,
) -> Result<()> {
    for fragment in fragments {
        for traf in atoms(&fragment.data)?
            .iter()
            .filter(|atom| &atom.kind == b"traf")
        {
            let boxes = atoms(traf.data)?;
            let header = required(&boxes, b"tfhd")?;
            version(header, 0)?;
            let flags = u32be(header, 0)? & 0x00ff_ffff;
            if flags & !TFHD_SUPPORTED != 0 {
                return Err(invalid("unsupported MP4 track-fragment header flags"));
            }
            let id = u32be(header, 4)?;
            let mut at = 8usize;
            let mut stated_base = None;
            if flags & 0x00_0001 != 0 {
                stated_base = Some(u64be(header, at)?);
                at += 8;
            }
            if flags & 0x00_0002 != 0 {
                if u32be(header, at)? > 1 {
                    return Err(invalid("MP4 fragment sample description index"));
                }
                at += 4;
            }
            let mut duration = 0;
            if flags & 0x00_0008 != 0 {
                duration = u32be(header, at)?;
                at += 4;
            }
            let mut size = None;
            if flags & 0x00_0010 != 0 {
                size = Some(u32be(header, at)?);
                at += 4;
            }
            let mut header_flags = None;
            if flags & 0x00_0020 != 0 {
                header_flags = Some(u32be(header, at)?);
            }
            let base = match (stated_base, flags & 0x02_0000) {
                (Some(value), _) => value,
                (None, 0x02_0000) => fragment.start,
                (None, _) => {
                    return Err(invalid(
                        "MP4 fragment states neither a base offset nor its own moof as the base",
                    ));
                }
            };
            // A track the file's own `trak` list set aside for its coding, or one
            // no init segment ever described, keeps its bytes unread: that list is
            // what says which tracks exist, and it already answered.
            let Some(position) = tracks.iter().position(|track| track.id == id) else {
                continue;
            };
            let extends = defaults.iter().find(|entry| entry.track == id);
            let duration = if duration == 0 {
                extends.map_or(0, |entry| entry.duration)
            } else {
                duration
            };
            let size = size.or_else(|| extends.and_then(|e| (e.size != 0).then_some(e.size)));
            let header_flags = header_flags.or_else(|| extends.and_then(|e| e.flags));
            let track = &mut tracks[position];
            let SampleIndex::Expanded(list) = &mut track.samples else {
                // Only a track whose own sample tables were read can hold the
                // compact form, and the fragment pass never reads them.
                return Err(invalid("fragmented MP4 track holds a compact sample index"));
            };
            let mut dts = match optional(&boxes, b"tfdt")? {
                Some(stamp) => {
                    if version(stamp, 1)? == 0 {
                        u64::from(u32be(stamp, 4)?)
                    } else {
                        u64be(stamp, 4)?
                    }
                }
                None => list
                    .last()
                    .map_or(0, |last| last.dts.saturating_add(u64::from(last.duration))),
            };
            for run in boxes.iter().filter(|atom| &atom.kind == b"trun") {
                let bytes = run.data;
                let v = version(bytes, 1)?;
                let flags = u32be(bytes, 0)? & 0x00ff_ffff;
                if flags & !TRUN_SUPPORTED != 0 {
                    return Err(invalid("unsupported MP4 track-run flags"));
                }
                let count = u32be(bytes, 4)? as usize;
                let mut at = 8usize;
                // A run that says nothing about where it begins follows the last
                // sample read for this track, which for the first run of the
                // first fragment is the base itself.
                let mut offset = match (flags & 0x00_0001 != 0, list.last()) {
                    (true, _) => {
                        let shift = u32be(bytes, at)? as i32;
                        at += 4;
                        if shift < 0 {
                            base.checked_sub(u64::try_from(-shift).unwrap_or(u64::MAX))
                                .ok_or_else(|| {
                                    invalid("MP4 fragment data offset before its base")
                                })?
                        } else {
                            base.checked_add(u64::try_from(shift).unwrap_or(u64::MAX))
                                .ok_or_else(|| invalid("MP4 fragment data offset overflow"))?
                        }
                    }
                    (false, Some(last)) => last
                        .offset
                        .checked_add(u64::from(last.size))
                        .ok_or_else(|| invalid("MP4 fragment sample offset overflow"))?,
                    (false, None) => base,
                };
                let leading = if flags & 0x00_0004 != 0 {
                    let value = u32be(bytes, at)?;
                    at += 4;
                    Some(value)
                } else {
                    None
                };
                let fields = usize::from(flags & 0x00_0100 != 0)
                    + usize::from(flags & 0x00_0200 != 0)
                    + usize::from(flags & 0x00_0400 != 0)
                    + usize::from(flags & 0x00_0800 != 0);
                if count
                    .checked_mul(fields * 4)
                    .and_then(|n| n.checked_add(at))
                    != Some(bytes.len())
                {
                    return Err(invalid("MP4 track-run size does not match its rows"));
                }
                for index in 0..count {
                    if *limit == 0 {
                        return Err(invalid("MP4 sample count exceeds budget"));
                    }
                    *limit -= 1;
                    let step = if flags & 0x00_0100 != 0 {
                        let value = u32be(bytes, at)?;
                        at += 4;
                        value
                    } else {
                        duration
                    };
                    let bytes_of_sample = if flags & 0x00_0200 != 0 {
                        let value = u32be(bytes, at)?;
                        at += 4;
                        value
                    } else {
                        size.ok_or_else(|| {
                            invalid("MP4 fragment sample states no size and none is defaulted")
                        })?
                    };
                    let bits = if flags & 0x00_0400 != 0 {
                        let value = u32be(bytes, at)?;
                        at += 4;
                        value
                    } else if index == 0 {
                        leading.or(header_flags).unwrap_or(0)
                    } else {
                        header_flags.unwrap_or(0)
                    };
                    let shift = if flags & 0x00_0800 != 0 {
                        let value = u32be(bytes, at)?;
                        at += 4;
                        if v == 1 {
                            i64::from(value as i32)
                        } else {
                            i64::from(value)
                        }
                    } else {
                        0
                    };
                    let end = offset
                        .checked_add(u64::from(bytes_of_sample))
                        .ok_or_else(|| invalid("MP4 sample span overflow"))?;
                    let containing = mdats.partition_point(|range| range.start <= offset);
                    if containing == 0 || end > mdats[containing - 1].end {
                        return Err(invalid("sample chunk outside mdat"));
                    }
                    let pts = i64::try_from(dts)
                        .map_err(|_| invalid("MP4 timestamp overflow"))?
                        .checked_add(shift)
                        .ok_or_else(|| invalid("MP4 PTS overflow"))?;
                    list.push(Sample {
                        offset,
                        size: bytes_of_sample,
                        dts,
                        pts,
                        duration: step,
                        sync: bits & 0x0001_0000 == 0,
                    });
                    offset = end;
                    dts = dts
                        .checked_add(u64::from(step))
                        .ok_or_else(|| invalid("MP4 timestamp overflow"))?;
                }
            }
        }
    }
    // A fragmented track's length is what its samples add up to: the init segment
    // writes `mdhd` at zero because it indexes nothing, and no later box states a
    // total, so the end of the last sample says it - once, after every fragment
    // has been read, since a fragment is only ever part of the answer.
    for track in tracks {
        let stated = track
            .samples
            .expanded()
            .and_then(|list| list.last())
            .map(|last| last.dts.saturating_add(u64::from(last.duration)));
        if track.duration == 0 {
            track.duration = stated.unwrap_or(0);
        }
    }
    Ok(())
}

/// Walk `stsc` and the chunk-offset table together into one record per chunk,
/// each checked against the `mdat`s the file declares so a chunk can never read
/// outside media data. `chunk_bytes(first, units)` reports how much a chunk of
/// `units` index units starting at unit `first` occupies, which the two index
/// forms know in different ways. `samples_per_unit` is how many of the samples
/// the table counts one unit holds: one for every coding whose packets are the
/// frames `stsz` and `stsc` speak of, and six for MACE, whose chunks are written
/// in samples but read in blocks.
fn chunk_runs(
    stbl: &[Atom<'_>],
    count: usize,
    mdats: &[Range<u64>],
    samples_per_unit: usize,
    chunk_bytes: impl Fn(usize, usize) -> Result<u64>,
) -> Result<Vec<FrameRun>> {
    let stco = optional(stbl, b"stco")?;
    let co64 = optional(stbl, b"co64")?;
    let (offsets, stride) = match (stco, co64) {
        (Some(b), None) => (b, 4),
        (None, Some(b)) => (b, 8),
        _ => return Err(invalid("expected exactly one chunk-offset table")),
    };
    version(offsets, 0)?;
    let chunks = table(offsets, stride, count)?;
    let stsc = required(stbl, b"stsc")?;
    version(stsc, 0)?;
    let runs = table(stsc, 12, chunks)?;
    let mut mapping = Vec::new();
    for i in 0..runs {
        let first = u32be(stsc, 8 + i * 12)? as usize;
        let per_chunk = u32be(stsc, 12 + i * 12)? as usize;
        if first == 0
            || first > chunks
            || per_chunk == 0
            || u32be(stsc, 16 + i * 12)? != 1
            || per_chunk % samples_per_unit != 0
            || mapping.last().is_some_and(|&(prev, _)| first <= prev)
        {
            return Err(invalid("invalid sample-to-chunk mapping"));
        }
        mapping.push((first, per_chunk / samples_per_unit));
    }
    if count > 0 && mapping.first().map(|p| p.0) != Some(1) {
        return Err(invalid("chunk mapping must start at one"));
    }
    let mut walked = Vec::new();
    walked
        .try_reserve_exact(chunks)
        .map_err(|_| invalid("chunk index allocation failed"))?;
    let mut first = 0usize;
    let mut run = 0usize;
    for chunk in 1..=chunks {
        while run + 1 < runs && chunk >= mapping[run + 1].0 {
            run += 1;
        }
        let frames = mapping
            .get(run)
            .ok_or_else(|| invalid("missing chunk mapping"))?
            .1;
        if frames > count - first {
            return Err(invalid("chunk mapping exceeds sample count"));
        }
        let offset = if stride == 4 {
            u64::from(u32be(offsets, 8 + (chunk - 1) * 4)?)
        } else {
            u64be(offsets, 8 + (chunk - 1) * 8)?
        };
        let end = offset
            .checked_add(chunk_bytes(first, frames)?)
            .ok_or_else(|| invalid("chunk offset overflow"))?;
        let containing = mdats.partition_point(|range| range.start <= offset);
        if containing == 0 || end > mdats[containing - 1].end {
            return Err(invalid("sample chunk outside mdat"));
        }
        walked.push(FrameRun {
            first,
            offset,
            frames,
        });
        first += frames;
    }
    if first != count {
        return Err(invalid("chunk mapping omits samples"));
    }
    Ok(walked)
}

#[cfg(test)]
mod tests {
    use super::read_tags;
    use crate::container::FileTags;

    fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len() + 8).unwrap().to_be_bytes();
        [&size[..], kind, payload].concat()
    }
    /// The iTunes `data` box, whose text stands behind a stated encoding and
    /// four bytes no writer explains.
    fn data(text: &[u8]) -> Vec<u8> {
        atom(b"data", &[&[0, 0, 0, 1, 0, 0, 0, 0][..], text].concat())
    }
    /// A Microsoft ADPCM entry: the fixed part every audio description carries,
    /// then the sixteen bytes of packet geometry a version 1 one adds, then the
    /// `wave` atom holding the `WAVEFORMATEX` the block length comes from.
    #[test]
    fn the_wave_record_sits_where_the_entry_version_puts_the_children() {
        use super::{is_adpcm, wave_record};
        let record: Vec<u8> = (0..18u8).collect();
        let wave = atom(b"wave", &atom(b"ms\x00\x02", &record));
        let mut entry = vec![0u8; 28];
        entry[8..10].copy_from_slice(&1u16.to_be_bytes());
        entry.extend_from_slice(&[255u8; 16]);
        entry.extend_from_slice(&wave);
        assert_eq!(
            wave_record(&entry, 44, *b"ms\x00\x02").as_deref(),
            Some(&record[..]),
            "a version 1 description states its own geometry before the atoms"
        );
        let mut plain = vec![0u8; 28];
        plain.extend_from_slice(&wave);
        assert_eq!(
            wave_record(&plain, 28, *b"ms\x00\x02").as_deref(),
            Some(&record[..]),
            "version 0 goes straight from the sample rate to them"
        );
        // Apple's entry carries a `chan` atom instead and needs no record; one
        // that is looked for in it comes back absent, which the decoder reads as
        // a track it has no geometry for rather than a length of its own.
        let mut ima4 = vec![0u8; 28];
        ima4[8..10].copy_from_slice(&1u16.to_be_bytes());
        ima4.extend_from_slice(&[255u8; 16]);
        ima4.extend_from_slice(&atom(b"chan", &[0u8; 16]));
        assert_eq!(wave_record(&ima4, 44, *b"ms\x00\x02"), None);
        assert!(is_adpcm(b"ima4") && is_adpcm(b"ms\x00\x11") && !is_adpcm(b"mp4a"));
    }
    /// Every fact the movie can be named by, in the spelling a QuickTime file
    /// states it — text standing in the tag itself — and in the one an
    /// iTunes-style file nests it in.
    #[test]
    fn the_movie_tags_name_it_in_either_writers_spelling() {
        let plain = [
            atom(b"\xa9nam", "Имя".as_bytes()),
            atom(b"\xa9ART", b"A"),
            atom(b"\xa9alb", b"B"),
            atom(b"\xa9gen", b"G"),
            atom(b"\xa9day", b"2026"),
            atom(b"\xa9cmt", b"C"),
        ];
        assert_eq!(
            read_tags(Some(&plain.concat())),
            FileTags {
                title: "Имя".to_owned(),
                artist: "A".to_owned(),
                album: "B".to_owned(),
                genre: "G".to_owned(),
                date: "2026".to_owned(),
                comment: "C".to_owned(),
                track: String::new(),
                album_artist: String::new(),
                disc: String::new(),
                publisher: String::new(),
                copyright: String::new(),
                description: String::new(),
                rating: String::new(),
            }
        );
        // A name the writer ended with a NUL byte is the name without it.
        assert_eq!(read_tags(Some(&atom(b"\xa9nam", b"Ok\0"))).title, "Ok");
        let nested = atom(
            b"meta",
            &[
                &[0, 0, 0, 0][..],
                &atom(b"ilst", &atom(b"\xa9nam", &data(b"Ok"))),
            ]
            .concat(),
        );
        assert_eq!(read_tags(Some(&nested)).title, "Ok");
        // Where a file states one fact both ways, the plain box answers.
        let both = [atom(b"\xa9nam", b"plain"), nested].concat();
        assert_eq!(read_tags(Some(&both)).title, "plain");
    }
    /// A file this reader finds no facts in states nothing: no `udta` at all, a
    /// tag of something else, a value in bytes no UTF-8 text spells, and a tag
    /// with nothing behind it all answer the same way.
    #[test]
    fn tags_this_reader_cannot_read_leave_the_file_unstated() {
        assert_eq!(read_tags(None), FileTags::default());
        // The writer's own name is no fact about the file it tags.
        assert_eq!(
            read_tags(Some(&atom(b"\xa9too", b"Lavf"))),
            FileTags::default()
        );
        assert_eq!(read_tags(Some(&atom(b"\xa9nam", &[]))), FileTags::default());
        assert_eq!(
            read_tags(Some(&atom(b"\xa9nam", &[0xff, 0xfe]))),
            FileTags::default()
        );
        let empty = atom(
            b"meta",
            &[&[0, 0, 0, 0][..], &atom(b"ilst", &atom(b"\xa9nam", &[]))].concat(),
        );
        assert_eq!(read_tags(Some(&empty)), FileTags::default());
    }

    /// Who holds the rights and what the file says of itself reach the panel
    /// beside the rest. `ffmpeg` files both under iTunes-style names with no
    /// `©` mark — `cprt` and `desc` — and this reader asks the one table for
    /// them in both of a file's writings: wrapped in a `data` box under
    /// `meta/ilst`, the way the muxer leaves them, and standing as a plain box
    /// under `udta`, the way an older writer might.
    #[test]
    fn the_rights_and_the_note_read_from_their_atoms_both_ways() {
        let nested = atom(
            b"meta",
            &[
                &[0, 0, 0, 0][..],
                &atom(
                    b"ilst",
                    &[
                        atom(b"cprt", &data(b"Right")),
                        atom(b"desc", &data(b"Note")),
                    ]
                    .concat(),
                ),
            ]
            .concat(),
        );
        let tags = read_tags(Some(&nested));
        assert_eq!(
            (tags.copyright.as_str(), tags.description.as_str()),
            ("Right", "Note")
        );
        let plain = [atom(b"cprt", b"R"), atom(b"desc", b"D")].concat();
        let tags = read_tags(Some(&plain));
        assert_eq!(
            (tags.copyright.as_str(), tags.description.as_str()),
            ("R", "D")
        );
    }

    /// A track number is the one tag whose value is not text: two sixteen-bit
    /// numbers behind two reserved bytes, which QuickTime writes as a bare box
    /// under `udta` and ffmpeg's iTunes layout wraps in a `data` box, and both
    /// ways land in the panel's `Track` line — the total only when the file
    /// knows it. A box whose number is zero states no place, however full the
    /// rest of it is, and one too short for the pair states nothing at all.
    #[test]
    fn a_track_number_reads_from_its_binary_box_both_writers_spell_it() {
        fn stated(number: u16, total: u16, short: bool) -> Vec<u8> {
            let mut bytes = vec![0, 0];
            bytes.extend(number.to_be_bytes());
            if !short {
                bytes.extend(total.to_be_bytes());
                bytes.extend([0; 6]);
            }
            bytes
        }
        assert_eq!(
            read_tags(Some(&atom(b"trkn", &stated(3, 12, false)))).track,
            "3/12"
        );
        assert_eq!(
            read_tags(Some(&atom(b"trkn", &stated(7, 0, false)))).track,
            "7"
        );
        let inside = atom(
            b"trkn",
            &atom(
                b"data",
                &[&[0; 8][..], stated(2, 9, false).as_slice()].concat(),
            ),
        );
        let list = atom(
            b"meta",
            &[&[0, 0, 0, 0][..], &atom(b"ilst", &inside)].concat(),
        );
        assert_eq!(read_tags(Some(&list)).track, "2/9");
        assert_eq!(
            read_tags(Some(&atom(b"trkn", &stated(0, 12, false)))),
            FileTags::default(),
            "a zero place is no place"
        );
        assert_eq!(
            read_tags(Some(&atom(b"trkn", &stated(3, 12, true)))),
            FileTags::default(),
            "a box cut short states nothing"
        );
    }

    /// The disc's place travels in the same binary pair as the track's, under
    /// its own `disk` key, and the album's artist is plain text under `aART` —
    /// ffmpeg writes both ways, the bare QuickTime box and the `data`-wrapped
    /// iTunes one, and both reach the panel's `Disc` and `Album artist` lines.
    #[test]
    fn a_disc_and_the_albums_own_artist_read_from_their_boxes() {
        fn stated(number: u16, total: u16) -> Vec<u8> {
            let mut bytes = vec![0, 0];
            bytes.extend(number.to_be_bytes());
            bytes.extend(total.to_be_bytes());
            bytes.extend([0; 6]);
            bytes
        }
        let file = [atom(b"disk", &stated(2, 10)), atom(b"aART", b"AA")].concat();
        let tags = read_tags(Some(&file));
        assert_eq!(tags.disc, "2/10");
        assert_eq!(tags.album_artist, "AA");
        let inside = atom(
            b"disk",
            &atom(b"data", &[&[0; 8][..], stated(4, 0).as_slice()].concat()),
        );
        let list = atom(
            b"meta",
            &[&[0, 0, 0, 0][..], &atom(b"ilst", &inside)].concat(),
        );
        assert_eq!(read_tags(Some(&list)).disc, "4");
    }

    /// A Mac screen recording whose audio track is MACE 6-to-1 mono at 8 000 Hz: `stsz` counts
    /// 17 856 samples of one byte each in one `stts` run of one tick, `stsc` lays them out as
    /// two chunks of 12 252 and 5 604, and `stco` puts them at 633 536 and 886 874 inside an
    /// `mdat` that ends at 887 808. Every one of those numbers is the file's own.
    const MJPEGA: &[u8] = include_bytes!("../../tests/fixtures/mace/mjpega.mov");
    /// Where the take states the four of those numbers a lying file can get wrong: the
    /// `stts` run and the `stsz` count that both name 17 856 samples, the `stsc` entry that
    /// names a chunk of 12 252 of them, and the `stco` offset the last chunk starts at.
    const STTS_COUNT: usize = 889_166;
    const STSZ_COUNT: usize = 889_230;
    const STSC_PER_CHUNK: usize = 889_194;
    const STCO_SECOND: usize = 889_254;

    /// The take with its big-endian table fields rewritten.
    fn stating(patches: &[(usize, u32)]) -> Vec<u8> {
        let mut bytes = MJPEGA.to_vec();
        for (at, value) in patches {
            bytes[*at..*at + 4].copy_from_slice(&value.to_be_bytes());
        }
        bytes
    }

    use super::{is_mace, mace_block};

    fn open(
        bytes: &[u8],
    ) -> crate::Result<crate::playback_mp4_audio::Mp4AudioReader<std::io::Cursor<&[u8]>>> {
        crate::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(bytes),
            super::Limits::default(),
        )
    }

    #[test]
    fn the_coding_s_block_is_two_or_one_bytes_a_channel_and_six_samples_whichever_it_is() {
        assert_eq!(mace_block(b"MAC3", 1), Some((6, 2)));
        assert_eq!(mace_block(b"MAC3", 2), Some((6, 4)));
        assert_eq!(mace_block(b"MAC6", 1), Some((6, 1)));
        assert_eq!(mace_block(b"MAC6", 2), Some((6, 2)));
        // Every other coding's `stsz` entry is the frame it names, blocks included: a track of
        // any of them goes on being indexed per sample as it was before this arm existed.
        for codec in [*b"twos", *b"MAC0", *b"lpcm", *b"sowt", *b"mp4a"] {
            assert_eq!(mace_block(&codec, 2), None, "{:?} is not MACE", &codec);
        }
        assert!(is_mace(b"MAC3") && is_mace(b"MAC6") && !is_mace(b"MAC0"));
    }

    #[test]
    fn a_mace_track_is_indexed_in_blocks_though_its_table_counts_samples() {
        use crate::audio::AudioStream as _;
        // The reader's grain is a block, so the packets are the 2 976 of them the 17 856 samples
        // hold - one byte and six ticks each, stamped end to end over the run `stsz` counts. The
        // reference reads the same tables into packets of up to 1 020 samples, and both come out
        // the same samples: the coding carries nothing about where a packet starts.
        let mut reader = open(MJPEGA).expect("the take opens");
        let mut packets = 0;
        let mut ticks = 0i64;
        while let Some(packet) = reader.next_packet().expect("packet") {
            assert_eq!(packet.data.len(), 1, "one block a packet");
            assert_eq!(packet.duration, 6, "six samples a block");
            assert_eq!(packet.pts, ticks, "the stamps follow the run");
            ticks += packet.duration;
            packets += 1;
        }
        assert_eq!(packets, 2_976);
        assert_eq!(ticks, 17_856, "as many samples as `stsz` counts");
    }

    #[test]
    fn a_mace_table_that_does_not_divide_into_blocks_is_refused() {
        // The three ways the geometry can lie, each caught before a packet is read: a sample
        // count that is not a whole number of six-sample blocks, a chunk that is not either,
        // and a chunk whose blocks would run past the `mdat` its offset names.
        // Both fields that count the run have to say 17 855, or the two tables disagree with
        // each other and that is what gets noticed first.
        let error = open(&stating(&[(STTS_COUNT, 17_855), (STSZ_COUNT, 17_855)]))
            .err()
            .expect("17 855 samples are not a whole number of blocks");
        assert!(
            error.to_string().contains("whole number of blocks"),
            "{error}"
        );
        let error = open(&stating(&[(STSC_PER_CHUNK, 12_253)]))
            .err()
            .expect("a chunk of 12 253 samples is not a whole number of blocks");
        assert!(
            error.to_string().contains("sample-to-chunk"),
            "{error}: {error}"
        );
        let error = open(&stating(&[(STCO_SECOND, 886_875)]))
            .err()
            .expect("the last chunk must stay inside the mdat");
        assert!(error.to_string().contains("outside mdat"), "{error}");
        // Sizing the run the way its `stsz` entries spell it - one byte each, 17 856 of them,
        // laid out as the table says - is exactly the same data, so the refusal is the block
        // reading having noticed something the per-sample reading could not.
        assert!(open(MJPEGA).is_ok());
    }

    /// The two fragmented takes this reader was written against, each exactly as
    /// the command above its own path in `tests/fixtures/fragmented` reproduces
    /// it. One is mono AAC at 48 kHz, indexed by a single `moof`; the other is
    /// H.264 with B-frames, indexed by five, each pointing into its own `mdat`.
    const FRAG_AUDIO: &[u8] = include_bytes!("../../tests/fixtures/fragmented/audio.mp4");
    const FRAG_VIDEO: &[u8] = include_bytes!("../../tests/fixtures/fragmented/video.mp4");

    /// Where the audio take states the four things a lying file can get wrong:
    /// the `tfhd` flags that say how its samples are described, the `trun` count
    /// that says how many rows follow, the first row's size, and the moment the
    /// one `moof` begins - which is also where the media it indexes does not.
    const AUDIO_TFHD_FLAGS: usize = 768;
    const AUDIO_TRUN_COUNT: usize = 820;
    const AUDIO_TRUN_ROW0_SIZE: usize = 832;
    /// Where the audio take's one `moof` begins, which is also where an init
    /// segment on its own stops.
    const AUDIO_MOOF: usize = 728;
    /// The second video fragment's `tfdt`: its box header, where the kind stands,
    /// and the `baseMediaDecodeTime` the take-away test compares against.
    const VIDEO_TFDT2_KIND: usize = 4_884;

    /// The take with big-endian fields rewritten at the byte offsets above.
    fn rewriting(take: &[u8], patches: &[(usize, u32)]) -> Vec<u8> {
        let mut bytes = take.to_vec();
        for (at, value) in patches {
            bytes[*at..*at + 4].copy_from_slice(&value.to_be_bytes());
        }
        bytes
    }
    /// A box renamed so that a reader walks past it as past any it does not know,
    /// which is how a file that simply never wrote it looks - with every other
    /// offset in the take left alone.
    fn hiding(take: &[u8], at: usize) -> Vec<u8> {
        let mut bytes = take.to_vec();
        bytes[at..at + 4].copy_from_slice(b"zzzz");
        bytes
    }

    fn demux(bytes: &[u8]) -> crate::Result<super::Mp4Reader<std::io::Cursor<&[u8]>>> {
        super::Mp4Reader::open(std::io::Cursor::new(bytes), super::Limits::default())
    }

    /// One sample's worth of an index, in the order the reader keeps it.
    fn entry(sample: &super::Sample) -> (u64, u32, u64, i64, u32, bool) {
        (
            sample.offset,
            sample.size,
            sample.dts,
            sample.pts,
            sample.duration,
            sample.sync,
        )
    }

    #[test]
    fn a_fragment_of_audio_is_indexed_as_its_own_run_states() {
        let file = demux(FRAG_AUDIO).expect("the take opens");
        assert_eq!(file.refused().len(), 0, "nothing here lacks a coding");
        let [track] = file.tracks() else {
            panic!("one audio track, as ffprobe says");
        };
        assert_eq!((track.handler, track.codec), (*b"soun", *b"mp4a"));
        assert_eq!(
            (track.timescale, track.sample_rate, track.channels),
            (48_000, 48_000, 1)
        );
        // The `mvex` promises samples by fragment and the `stbl` holds none, so
        // every number below came out of the `moof`: 58 rows of a duration and a
        // size, laid from the run's own offset inside the `mdat` that follows it.
        let samples = track.samples.expanded().expect("an expanded index");
        assert_eq!(samples.len(), 58, "as many rows as the run counts");
        assert_eq!(
            entry(&samples[0]),
            (1_300, 451, 0, 0, 1_024, true),
            "the first row, at the offset the run offsets from its `moof`"
        );
        assert_eq!(entry(&samples[1]), (1_751, 359, 1_024, 1_024, 1_024, true));
        assert_eq!(
            entry(&samples[57]),
            (20_013, 5, 58_368, 58_368, 256, true),
            "the fill row the encoder ends on, and the last byte of media the take holds"
        );
        assert!(
            samples
                .iter()
                .all(|s| s.pts == i64::try_from(s.dts).unwrap()),
            "sound is displayed as it is decoded"
        );
        assert!(
            samples.iter().all(|s| s.sync),
            "every row of sound restarts"
        );
        assert_eq!(
            samples.iter().map(|s| u64::from(s.duration)).sum::<u64>(),
            track.duration,
            "the track is as long as its samples, which is what the empty init segment left unsaid"
        );
        assert_eq!(track.duration, 58_624);
    }

    #[test]
    fn fragments_of_video_keep_the_decode_stamps_and_the_restart_points() {
        let file = demux(FRAG_VIDEO).expect("the take opens");
        let [track] = file.tracks() else {
            panic!("one video track, as ffprobe says");
        };
        assert_eq!((track.handler, track.codec), (*b"vide", *b"avc1"));
        assert_eq!(
            (track.width, track.height, track.timescale),
            (320, 240, 12_800)
        );
        // The init segment describes the coding in its `stsd`, exactly as a file
        // with a complete sample table does: a fragment adds nothing to it, and a
        // decoder that got no `avcC` here could not start at any of these rows.
        assert_eq!(
            track.configuration.get(..4),
            Some(&[1, 0x64, 0x00, 0x0d][..]),
            "an H.264 configuration record, version and profile and length size"
        );
        let samples = track.samples.expanded().expect("an expanded index");
        assert_eq!(samples.len(), 25, "five fragments of five rows each");
        // Each tuple is one row as the fragments state it: where its bytes start
        // and how many, then decode time, display time and length in ticks of
        // 12 800 Hz, then whether a player may start from it. The display times
        // are the decode ones plus the composition shift each row carries, and
        // only the first row of a fragment is a keyframe - which is the fragment
        // form of a `stss` list, and what a seek lands on.
        #[rustfmt::skip]
        let expected: [(u64, u32, u64, i64, u32, bool); 25] = [
            (   930,  3485,      0,   1024, 512, true),
            (  4415,   268,    512,   3072, 512, false),
            (  4683,    68,   1024,   2048, 512, false),
            (  4751,    36,   1536,   1536, 512, false),
            (  4787,    33,   2048,   2560, 512, false),
            (  4972,  3066,   2560,   3584, 512, true),
            (  8038,   258,   3072,   5632, 512, false),
            (  8296,    63,   3584,   4608, 512, false),
            (  8359,    25,   4096,   4096, 512, false),
            (  8384,    30,   4608,   5120, 512, false),
            (  8566,  3066,   5120,   6144, 512, true),
            ( 11632,   245,   5632,   8192, 512, false),
            ( 11877,    70,   6144,   7168, 512, false),
            ( 11947,    50,   6656,   6656, 512, false),
            ( 11997,    42,   7168,   7680, 512, false),
            ( 12191,  3087,   7680,   8704, 512, true),
            ( 15278,   232,   8192,  10240, 512, false),
            ( 15510,    50,   8704,   9216, 512, false),
            ( 15560,    28,   9216,   9728, 512, false),
            ( 15588,   138,   9728,  10752, 512, false),
            ( 15878,  3018,  10240,  11264, 512, true),
            ( 18896,   240,  10752,  12800, 512, false),
            ( 19136,    64,  11264,  11776, 512, false),
            ( 19200,    27,  11776,  12288, 512, false),
            ( 19227,   125,  12288,  13312, 512, false),        ];
        for (index, sample) in samples.iter().enumerate() {
            assert_eq!(entry(sample), expected[index], "sample {index}");
        }
        assert_eq!(
            track.duration, 12_800,
            "one second, as the last sample ends"
        );
    }

    #[test]
    fn every_fragmented_sample_reads_the_bytes_its_index_points_at() {
        // The index is only worth what the reader can fetch by it: each packet
        // has to come back the length the row said, and the whole walk has to
        // cover the media exactly once - which is what the audio take's one
        // `mdat` and the video take's five each of their own test together.
        for (take, samples, media) in [(FRAG_AUDIO, 58usize, 18_718usize), (FRAG_VIDEO, 25, 17_814)]
        {
            let mut file = demux(take).expect("the take opens");
            let track = &file.tracks()[0];
            let indexed: usize = track
                .samples
                .expanded()
                .expect("an expanded index")
                .iter()
                .map(|s| usize::try_from(s.size).unwrap())
                .sum();
            assert_eq!(indexed, media, "the rows add up to the media they name");
            let mut read = 0usize;
            for index in 0..samples {
                let mut packet = Vec::new();
                file.read_packet(0, index, &mut packet).expect("packet");
                assert_eq!(
                    packet.len(),
                    usize::try_from(file.tracks()[0].samples.get(index).unwrap().size).unwrap()
                );
                read += packet.len();
            }
            assert_eq!(read, media, "and nothing was read twice or left over");
        }
    }

    #[test]
    fn a_fragment_that_leaves_out_its_decode_time_continues_the_track() {
        // `tfdt` is optional, and a muxer appending to a live stream is what
        // leaves it out. The take says 2 560 for its second fragment either way,
        // so hiding that box only keeps the test honest if the stamps are still
        // the ones the first fragment's samples add up to.
        let take = hiding(FRAG_VIDEO, VIDEO_TFDT2_KIND);
        let file = demux(&take).expect("the take opens");
        let samples = file.tracks()[0]
            .samples
            .expanded()
            .expect("an expanded index");
        assert_eq!(samples.len(), 25);
        assert_eq!(
            samples[5].dts, 2_560,
            "where the last sample of the one before ended"
        );
        assert_eq!(samples[5].pts, 3_584, "and its own shift on top");
        assert_eq!(
            samples[10].dts, 5_120,
            "the next fragment still says its own"
        );
        assert_eq!(
            samples[9].offset, 8_384,
            "and the run still measures from a row's end"
        );
    }

    #[test]
    fn the_sound_a_fragment_indexes_is_the_sound_the_player_hears() {
        // The route a file takes to the speaker, not just the index behind it:
        // the audio reader opens the fragmented take the same way it opens an
        // indexed one, the decoder the dispatch answers accepts its packets, and
        // each of the first twenty comes back as one raw block of mono sound.
        use crate::audio::AudioStream as _;
        let mut reader = open(FRAG_AUDIO).expect("the take opens for playback");
        assert_eq!(reader.codec(), "mp4a");
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::new(1, 221_333_333))
        );
        let mut decoder = crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("the dispatch answers this tag");
        let mut stamps = Vec::new();
        for _ in 0..20 {
            let packet = reader.next_packet().expect("packet").expect("sample");
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("a frame of a real file decodes")
                .expect("and sounds");
            assert_eq!(
                audio.data.len() / 4,
                1_024,
                "one raw block of interleaved f32 mono"
            );
            assert_eq!((audio.pts, audio.timebase_den), (packet.pts as u64, 48_000));
            stamps.push((packet.pts, packet.duration));
        }
        assert_eq!(stamps[0], (0, 1_024));
        assert_eq!(stamps[19], (19_456, 1_024), "the stamps are the rows' own");
    }

    #[test]
    fn the_picture_a_fragment_indexes_reaches_the_picture_route() {
        // The container is only half of a file playing: the reader the window
        // drives asks the same index for packets and reorders what it decodes by
        // the display times the rows carried. Five fragments of five rows each,
        // with no `stss` and no `ctts` anywhere in the take, is the case that
        // would quietly show B-frames in decode order if the shift were lost.
        let mut reader = crate::playback_mp4::Mp4VideoReader::open(
            std::io::Cursor::new(FRAG_VIDEO),
            super::Limits::default(),
            // What the reader splits between the references its decoder keeps and
            // the pictures it holds for reordering - which 320x240 with four
            // reference frames does not fit into at a megabyte.
            32 << 20,
        )
        .expect("the picture route opens the take");
        let mut shown = Vec::new();
        for _ in 0..5 {
            let frame = reader.read_frame().expect("frame").expect("picture");
            shown.push((
                frame.presentation_time.ticks,
                frame.duration.ticks,
                frame.sample_index,
                frame.picture.coded_width,
                frame.picture.coded_height,
                frame.picture.bit_depth,
            ));
        }
        assert_eq!(
            shown,
            vec![
                (1_024, 512, 0, 320, 240, 8),
                (1_536, 512, 3, 320, 240, 8),
                (2_048, 512, 2, 320, 240, 8),
                (2_560, 512, 4, 320, 240, 8),
                (3_072, 512, 1, 320, 240, 8),
            ],
            "display order, from a file whose rows said only decode order"
        );
    }

    #[test]
    fn a_fragment_saying_something_no_reader_can_honour_is_refused() {
        // Flags this reader has no rule for, a run whose rows do not fill it, a
        // sample reaching past the media, and a base nothing measures from: each
        // is said in the bytes of the take above, and each has to stop the file
        // rather than index a wrong sample list a player would play quietly.
        let error = demux(&rewriting(FRAG_AUDIO, &[(AUDIO_TFHD_FLAGS, 0x0002_0078)]))
            .err()
            .expect("a reserved `tfhd` bit is no reading this one has");
        assert!(
            error.to_string().contains("track-fragment header flags"),
            "{error}"
        );
        let error = demux(&rewriting(FRAG_AUDIO, &[(AUDIO_TRUN_COUNT, 57)]))
            .err()
            .expect("57 rows are not 476 bytes of them");
        assert!(error.to_string().contains("track-run size"), "{error}");
        let error = demux(&rewriting(
            FRAG_AUDIO,
            &[(AUDIO_TRUN_ROW0_SIZE, 0x0fff_ffff)],
        ))
        .err()
        .expect("no sample is 268 MB of a 20 KB take");
        assert!(error.to_string().contains("outside mdat"), "{error}");
        // `default-base-is-moof` is what lets a fragment say nothing; take it away
        // with no absolute offset left and the bytes have no measured start.
        let error = demux(&rewriting(FRAG_AUDIO, &[(AUDIO_TFHD_FLAGS, 0x0000_0038)]))
            .err()
            .expect("an offset is needed from somewhere");
        assert!(error.to_string().contains("neither a base"), "{error}");
        // An init segment on its own names no media at all: `mvex` says the file
        // indexes by fragment and no fragment was ever written, which is what a
        // stream that stopped before it began leaves behind.
        let error = demux(&FRAG_AUDIO[..AUDIO_MOOF])
            .err()
            .expect("an init segment is not a playable file");
        assert!(
            error.to_string().contains("init segment without media"),
            "{error}"
        );
        // And the same take whole is what all four of those bytes came from.
        assert!(demux(FRAG_AUDIO).is_ok());
    }
}
