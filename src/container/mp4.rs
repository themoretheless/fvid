//! Indexed, non-fragmented ISO BMFF demultiplexing implemented in FVid.
//! Timestamps are in the track media timeline; edit lists are returned separately.
use crate::{Result, invalid};
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
    pub timescale: u32,
    pub duration: u64,
    pub width: u16,
    pub height: u16,
    pub channels: u16,
    pub sample_rate: u32,
    /// Raw avcC / hvcC / esds payload. Decoders validate the codec-specific syntax.
    pub configuration: Vec<u8>,
    pub edits: Vec<Edit>,
    pub samples: Vec<Sample>,
}
pub struct Mp4Reader<R> {
    reader: R,
    tracks: Vec<Track>,
    movie_timescale: u32,
    limits: Limits,
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

impl<R: Read + Seek> Mp4Reader<R> {
    pub fn open(mut reader: R, limits: Limits) -> Result<Self> {
        let end = reader.seek(SeekFrom::End(0))?;
        let mut at = 0u64;
        let mut moov = None;
        let mut mdats = Vec::new();
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
                b"moof" => return Err(invalid("fragmented MP4 is not yet supported")),
                _ => {}
            }
            at += size;
        }
        let moov = moov.ok_or_else(|| invalid("missing MP4 moov"))?;
        let movie = atoms(&moov)?;
        if optional(&movie, b"mvex")?.is_some() {
            return Err(invalid("fragmented MP4 is not yet supported"));
        }
        let (movie_timescale, _) = media_time(required(&movie, b"mvhd")?)?;
        let mut tracks = Vec::new();
        let mut remaining = limits.samples;
        for atom in movie.iter().filter(|a| &a.kind == b"trak") {
            if tracks.len() >= limits.tracks {
                return Err(invalid("MP4 track limit exceeded"));
            }
            let track = parse_track(atom.data, &mdats, remaining)?;
            if tracks.iter().any(|t: &Track| t.id == track.id) {
                return Err(invalid("duplicate track ID"));
            }
            remaining -= track.samples.len();
            tracks.push(track);
        }
        if tracks.is_empty() {
            return Err(invalid("MP4 contains no tracks"));
        }
        Ok(Self {
            reader,
            tracks,
            movie_timescale,
            limits,
        })
    }
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }
    pub fn movie_timescale(&self) -> u32 {
        self.movie_timescale
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
        self.reader.seek(SeekFrom::Start(sample.offset))?;
        if let Err(error) = self.reader.read_exact(output) {
            output.clear();
            return Err(error.into());
        }
        Ok(())
    }
}

fn parse_track(data: &[u8], mdats: &[Range<u64>], limit: usize) -> Result<Track> {
    let track = atoms(data)?;
    let tkhd = required(&track, b"tkhd")?;
    let id = u32be(tkhd, if version(tkhd, 1)? == 0 { 12 } else { 20 })?;
    if id == 0 {
        return Err(invalid("zero MP4 track ID"));
    }
    let mdia = atoms(required(&track, b"mdia")?)?;
    let (timescale, duration) = media_time(required(&mdia, b"mdhd")?)?;
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
        timescale,
        duration,
        width: 0,
        height: 0,
        channels: 0,
        sample_rate: 0,
        configuration: Vec::new(),
        edits: Vec::new(),
        samples: Vec::new(),
    };
    let (config_at, config_kind) = match (&handler, &entry.kind) {
        (b"vide", b"avc1" | b"avc3" | b"hvc1" | b"hev1") => {
            result.width = u16be(entry.data, 24)?;
            result.height = u16be(entry.data, 26)?;
            if result.width == 0 || result.height == 0 {
                return Err(invalid("zero video dimensions"));
            }
            (
                78,
                if matches!(&entry.kind, b"avc1" | b"avc3") {
                    b"avcC"
                } else {
                    b"hvcC"
                },
            )
        }
        (b"soun", b"mp4a") => {
            if u16be(entry.data, 8)? != 0 {
                return Err(invalid("unsupported audio sample entry version"));
            }
            result.channels = u16be(entry.data, 16)?;
            result.sample_rate = u32be(entry.data, 24)? >> 16;
            (28, b"esds")
        }
        _ => {
            return Err(invalid(
                "unsupported MP4 sample entry (supported: AVC, HEVC, MPEG-4 audio)",
            ));
        }
    };
    let configs = atoms(
        entry
            .data
            .get(config_at..)
            .ok_or_else(|| invalid("truncated sample entry"))?,
    )?;
    result.configuration = required(&configs, config_kind)?.to_vec();
    let stsz = required(&stbl, b"stsz")?;
    version(stsz, 0)?;
    let fixed_size = u32be(stsz, 4)?;
    let count = u32be(stsz, 8)? as usize;
    if count > limit {
        return Err(invalid("MP4 sample count exceeds budget"));
    }
    let expected = if fixed_size == 0 {
        count.checked_mul(4).and_then(|n| n.checked_add(12))
    } else {
        Some(12)
    };
    if expected != Some(stsz.len()) {
        return Err(invalid("invalid sample-size table"));
    }
    result
        .samples
        .try_reserve_exact(count)
        .map_err(|_| invalid("sample index allocation failed"))?;
    for i in 0..count {
        result.samples.push(Sample {
            offset: 0,
            size: if fixed_size == 0 {
                u32be(stsz, 12 + i * 4)?
            } else {
                fixed_size
            },
            dts: 0,
            pts: 0,
            duration: 0,
            sync: true,
        });
    }
    let stts = required(&stbl, b"stts")?;
    version(stts, 0)?;
    let mut index = 0usize;
    let mut dts = 0u64;
    for i in 0..table(stts, 8, count)? {
        let run = u32be(stts, 8 + i * 8)? as usize;
        let delta = u32be(stts, 12 + i * 8)?;
        if run == 0 || delta == 0 || run > count - index {
            return Err(invalid("invalid decode-time run"));
        }
        for sample in &mut result.samples[index..index + run] {
            sample.dts = dts;
            sample.pts = i64::try_from(dts).map_err(|_| invalid("MP4 timestamp overflow"))?;
            sample.duration = delta;
            dts = dts
                .checked_add(u64::from(delta))
                .ok_or_else(|| invalid("MP4 timestamp overflow"))?;
        }
        index += run;
    }
    if index != count {
        return Err(invalid("decode-time count differs from sample count"));
    }
    if let Some(ctts) = optional(&stbl, b"ctts")? {
        let v = version(ctts, 1)?;
        let mut index = 0;
        for i in 0..table(ctts, 8, count)? {
            let run = u32be(ctts, 8 + i * 8)? as usize;
            let offset = u32be(ctts, 12 + i * 8)?;
            let offset = if v == 1 {
                i64::from(offset as i32)
            } else {
                i64::from(offset)
            };
            if run == 0 || run > count - index {
                return Err(invalid("invalid composition-time run"));
            }
            for sample in &mut result.samples[index..index + run] {
                sample.pts = sample
                    .pts
                    .checked_add(offset)
                    .ok_or_else(|| invalid("MP4 PTS overflow"))?;
            }
            index += run;
        }
        if index != count {
            return Err(invalid("composition-time count differs from sample count"));
        }
    }
    if let Some(stss) = optional(&stbl, b"stss")? {
        version(stss, 0)?;
        for s in &mut result.samples {
            s.sync = false;
        }
        let mut last = 0;
        for i in 0..table(stss, 4, count)? {
            let index = u32be(stss, 8 + i * 4)? as usize;
            if index <= last || index > count {
                return Err(invalid("invalid sync sample index"));
            }
            result.samples[index - 1].sync = true;
            last = index;
        }
    }
    let stco = optional(&stbl, b"stco")?;
    let co64 = optional(&stbl, b"co64")?;
    let (offsets, stride) = match (stco, co64) {
        (Some(b), None) => (b, 4),
        (None, Some(b)) => (b, 8),
        _ => return Err(invalid("expected exactly one chunk-offset table")),
    };
    version(offsets, 0)?;
    let chunks = table(offsets, stride, count)?;
    let stsc = required(&stbl, b"stsc")?;
    version(stsc, 0)?;
    let runs = table(stsc, 12, chunks)?;
    let mut chunk_map = Vec::new();
    for i in 0..runs {
        let first = u32be(stsc, 8 + i * 12)? as usize;
        let per_chunk = u32be(stsc, 12 + i * 12)? as usize;
        if first == 0
            || first > chunks
            || per_chunk == 0
            || u32be(stsc, 16 + i * 12)? != 1
            || chunk_map.last().is_some_and(|&(prev, _)| first <= prev)
        {
            return Err(invalid("invalid sample-to-chunk mapping"));
        }
        chunk_map.push((first, per_chunk));
    }
    if count > 0 && chunk_map.first().map(|p| p.0) != Some(1) {
        return Err(invalid("chunk mapping must start at one"));
    }
    let mut sample_index = 0;
    let mut run = 0;
    for chunk in 1..=chunks {
        while run + 1 < runs && chunk >= chunk_map[run + 1].0 {
            run += 1;
        }
        let n = chunk_map
            .get(run)
            .ok_or_else(|| invalid("missing chunk mapping"))?
            .1;
        if n > count - sample_index {
            return Err(invalid("chunk mapping exceeds sample count"));
        }
        let mut offset = if stride == 4 {
            u64::from(u32be(offsets, 8 + (chunk - 1) * 4)?)
        } else {
            u64be(offsets, 8 + (chunk - 1) * 8)?
        };
        let bytes = result.samples[sample_index..sample_index + n]
            .iter()
            .try_fold(0u64, |sum, s| sum.checked_add(u64::from(s.size)))
            .ok_or_else(|| invalid("chunk size overflow"))?;
        let end = offset
            .checked_add(bytes)
            .ok_or_else(|| invalid("chunk offset overflow"))?;
        let containing = mdats.partition_point(|range| range.start <= offset);
        if containing == 0 || end > mdats[containing - 1].end {
            return Err(invalid("sample chunk outside mdat"));
        }
        for s in &mut result.samples[sample_index..sample_index + n] {
            s.offset = offset;
            offset += u64::from(s.size);
        }
        sample_index += n;
    }
    if sample_index != count {
        return Err(invalid("chunk mapping omits samples"));
    }
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
    Ok(result)
}
