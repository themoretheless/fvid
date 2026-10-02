//! Bounded seekable WebM/Matroska indexing, without an external demultiplexer.
use crate::color::hdr::{ColourDescription, HdrMetadata, MasteringDisplay};
use crate::color::tonemap::ContentLight;
use crate::{Result, container::FileTags, invalid};
use std::io::{Read, Seek, SeekFrom};
const SEGMENT: u32 = 0x18538067;
const CLUSTER: u32 = 0x1f43b675;
#[derive(Clone, Debug)]
pub struct Track {
    pub number: u64,
    pub kind: u64,
    pub codec: String,
    /// `Name`, which muxers fill from the stream's `title` tag; empty when the
    /// file gives the track no name.
    pub name: String,
    /// The track's language as the file states it, empty when it states none.
    /// Read from whichever of the current element and the older one the muxer
    /// wrote, and left in the shape the file used, so a three-letter code and
    /// a `pt-BR`-style tag reach the player alike.
    pub language: String,
    pub width: u64,
    pub height: u64,
    /// `DisplayWidth`/`DisplayHeight`: the size in pixels the coded picture is
    /// drawn at, or the display aspect ratio when DisplayUnit is 3.
    /// `(0, 0)` when absent or stated in physical/unknown units.
    pub display: (u64, u64),
    /// `PixelCropLeft/Top/Right/Bottom`: the border of coded pixels the track
    /// asks not to be shown, as read from the elements in that order. A track
    /// that states none, or states one the file cannot show with — a crop that
    /// leaves nothing, a crop too wide to subtract — keeps the whole picture.
    pub crop: [u64; 4],
    /// Clockwise rectangular projection rotation; unsupported projections stay coded.
    pub rotation: u16,
    pub sample_rate: u64,
    pub channels: u64,
    pub bit_depth: u64,
    /// `DefaultDuration` in nanoseconds, 0 when the track declares none. Text
    /// subtitle blocks carry no length of their own, so this is the timing a
    /// track-level reader has left for the last block.
    pub default_duration_ns: u64,
    /// CodecDelay in nanoseconds, independent of Segment TimestampScale.
    pub codec_delay_ns: u64,
    /// Codec setup data from `CodecPrivate`. Vorbis carries its three header
    /// packets concatenated here, and the decoder cannot initialize without it.
    pub codec_private: Vec<u8>,
    /// The `Colour` element's H.273 triple and range, as coded by the muxer.
    ///
    /// Zeros when the track states none, which an SD track usually does; an
    /// HEVC or AV1 stream also states this in its own sequence header, and the
    /// decoder reads it there.
    pub colour: ColourDescription,
    /// `MasteringMetadata` and `MaxCLL`/`MaxFALL`, which a tone map needs and no
    /// bitstream of a container-only stream carries.
    pub hdr: HdrMetadata,
}
/// A named point in the file a player can jump to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    pub start_ns: u64,
    /// Explicit exclusive end in Matroska ticks (nanoseconds), when present.
    pub end_ns: Option<u64>,
    /// The first `ChapterDisplay` string, empty when the atom names no title.
    pub title: String,
}
#[derive(Clone, Debug)]
pub struct Packet {
    pub track: u64,
    pub pts_ns: i64,
    pub keyframe: bool,
    /// Decode to establish references, but do not display the resulting frame.
    pub invisible: bool,
    pub offset: u64,
    pub size: usize,
    /// DiscardPadding in nanoseconds: positive trims end, negative trims start.
    pub discard_padding_ns: i64,
    /// Explicit BlockDuration, scaled to nanoseconds; absent for SimpleBlock.
    pub duration_ns: Option<u64>,
}
impl Track {
    /// The size the picture is meant to be seen at: the coded one with the
    /// stated crop borders taken off it. Equal to the coded size when the
    /// track states no crop, or states one that leaves nothing to show.
    pub fn visible(&self) -> (u64, u64) {
        (
            self.width - self.crop[0] - self.crop[2],
            self.height - self.crop[1] - self.crop[3],
        )
    }
    /// How much wider a coded pixel is than it is tall, as the file states it.
    /// `(1, 1)` when the file states no display size, which is what most
    /// writers mean. The display size is a statement about the picture the
    /// viewer is meant to see, so the coded size it is compared against is the
    /// cropped one.
    pub fn pixel_aspect(&self) -> (u32, u32) {
        let coded = self.visible();
        let drawn = (self.display.0, self.display.1);
        // Products of two u64 dimensions fit u128. Reduce before narrowing so
        // large exact display ratios do not silently become square pixels.
        let wide = u128::from(drawn.0) * u128::from(coded.1);
        let tall = u128::from(drawn.1) * u128::from(coded.0);
        if wide == 0 || tall == 0 { return (1, 1); }
        let (mut a, mut b) = (wide, tall);
        while b != 0 { let rest = a % b; a = b; b = rest; }
        match (u32::try_from(wide / a), u32::try_from(tall / a)) {
            (Ok(n), Ok(d)) => (n, d),
            _ => (1, 1),
        }
    }
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub packets: usize,
    pub packet_bytes: usize,
    pub elements: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            packets: 1_000_000,
            packet_bytes: 32 << 20,
            elements: 4_000_000,
        }
    }
}
pub struct WebmReader<R> {
    reader: R,
    pub tracks: Vec<Track>,
    pub packets: Vec<Packet>,
    /// Declared Segment duration, converted from TimestampScale units.
    pub duration_ns: Option<u64>,
    /// Chapter atoms of the file's editions, in order of their start. Empty
    /// when the file carries none or its chapter list cannot be walked.
    pub chapters: Vec<Chapter>,
    /// What the file's own tags say about it as a whole. Its title answers by
    /// either of the two places a writer can put it: the `Title` of a tag of the
    /// whole segment and the older `Title` of the information block.
    pub tags: FileTags,
    limits: Limits,
    read_packet_bytes: usize,
    /// Where the walk of the Segment's children stands. Blocks are indexed a
    /// cluster at a time, so the first picture does not wait for the whole file.
    at: u64,
    segment_end: u64,
    /// The `TimestampScale` the file's `Info` block states, or the one the
    /// specification gives it. Kept because a cluster's blocks arrive in it.
    scale: u64,
    duration_ticks: Option<f64>,
    /// Whether `Info` has been read, which is what makes a scale final and a
    /// track nameable: blocks before it are not indexed for a first picture.
    info_seen: bool,
    chapter_runs: Vec<(u64, Option<u64>, String)>,
    block_title: String,
    /// How many EBML elements the walk has made, counted over the whole file so
    /// the limit means the same thing whether the index grew in one pass or ten.
    elements: usize,
    /// Whether the walk reached the end of the Segment.
    scanned: bool,
    /// The latest block timestamp indexed so far, so a jump knows how far ahead
    /// of itself the walk has already been.
    tail_ns: i64,
}
#[derive(Clone, Copy)]
struct Element {
    id: u32,
    data: u64,
    end: Option<u64>,
}
fn vint(r: &mut impl Read, id: bool) -> Result<(u64, bool)> {
    let mut first = [0];
    r.read_exact(&mut first)?;
    if first[0] == 0 {
        return Err(invalid("zero EBML variable integer"));
    }
    let len = first[0].leading_zeros() as usize + 1;
    if len > if id { 4 } else { 8 } {
        return Err(invalid("oversized EBML variable integer"));
    }
    let mut value = u64::from(if id {
        first[0]
    } else {
        first[0] & ((1u8 << (8 - len)) - 1)
    });
    for _ in 1..len {
        let mut b = [0];
        r.read_exact(&mut b)?;
        value = (value << 8) | u64::from(b[0]);
    }
    Ok((value, !id && value == (1u64 << (7 * len)) - 1))
}
/// Stand at `pos` on the way past it rather than by starting again. A walk moves
/// from one element to the next, and the bytes in between are the payload it has
/// just measured: reading them costs their own length, while a seek costs the
/// buffer behind the reader its whole capacity and the next read fetches the
/// same bytes back. Only a gap longer than any block the reader steps over, or a
/// move backwards, is a jump worth the seek.
const SKIP: u64 = 8 << 20;
fn goto<R: Read + Seek>(r: &mut R, pos: u64) -> Result<()> {
    let here = r.stream_position()?;
    if pos == here {
        return Ok(());
    }
    if pos > here && pos - here <= SKIP {
        let mut buf = [0u8; 1 << 13];
        let mut left = pos - here;
        while left > 0 {
            let want = (left as usize).min(buf.len());
            // A short read is the source answering less than asked, not an end:
            // no bytes at all means the item stopped before the element its own
            // size promised.
            let taken = r.read(&mut buf[..want])?;
            if taken == 0 {
                return Err(invalid("truncated EBML element"));
            }
            left -= taken as u64;
        }
        return Ok(());
    }
    r.seek(SeekFrom::Start(pos))?;
    Ok(())
}
fn element<R: Read + Seek>(
    r: &mut R,
    limit: u64,
    count: &mut usize,
    max: usize,
) -> Result<Element> {
    *count += 1;
    if *count > max {
        return Err(invalid("WebM element limit exceeded"));
    }
    let (id, _) = vint(r, true)?;
    let (size, unknown) = vint(r, false)?;
    let data = r.stream_position()?;
    if data > limit {
        return Err(invalid("truncated EBML element header"));
    }
    let end = if unknown {
        None
    } else {
        Some(
            data.checked_add(size)
                .filter(|&n| n <= limit)
                .ok_or_else(|| invalid("EBML element exceeds parent"))?,
        )
    };
    Ok(Element {
        id: id as u32,
        data,
        end,
    })
}
fn bytes<R: Read + Seek>(r: &mut R, e: Element, max: usize) -> Result<Vec<u8>> {
    let end = e
        .end
        .ok_or_else(|| invalid("unknown size for EBML value"))?;
    let len = usize::try_from(end - e.data)
        .ok()
        .filter(|&n| n <= max)
        .ok_or_else(|| invalid("EBML value exceeds limit"))?;
    r.seek(SeekFrom::Start(e.data))?;
    let mut out = vec![0; len];
    r.read_exact(&mut out)?;
    Ok(out)
}
/// A UTF-8 string element, without the trailing NUL byte some muxers write.
fn text<R: Read + Seek>(r: &mut R, e: Element, max: usize) -> Result<String> {
    Ok(String::from_utf8(bytes(r, e, max)?)
        .map_err(|_| invalid("invalid WebM text"))?
        .trim_end_matches('\0')
        .to_owned())
}
fn uint<R: Read + Seek>(r: &mut R, e: Element) -> Result<u64> {
    Ok(bytes(r, e, 8)?
        .into_iter()
        .fold(0, |v, b| (v << 8) | u64::from(b)))
}
fn sint<R: Read + Seek>(r: &mut R, e: Element) -> Result<i64> {
    let value = bytes(r, e, 8)?;
    let mut signed = if value.first().is_some_and(|byte| byte & 0x80 != 0) { -1i64 } else { 0 };
    for byte in value { signed = (signed << 8) | i64::from(byte); }
    Ok(signed)
}
/// Timestamps and rates are IEEE-754, written as either four or eight bytes.
fn float<R: Read + Seek>(r: &mut R, e: Element) -> Result<f64> {
    let value = bytes(r, e, 8)?;
    Ok(match value.as_slice() {
        [a, b, c, d] => f64::from(f32::from_be_bytes([*a, *b, *c, *d])),
        [a, b, c, d, e0, e1, e2, e3] => f64::from_be_bytes([*a, *b, *c, *d, *e0, *e1, *e2, *e3]),
        _ => return Err(invalid("invalid WebM float size")),
    })
}
fn end(e: Element) -> Result<u64> {
    e.end
        .ok_or_else(|| invalid("unsupported unknown-sized EBML element"))
}
fn fields<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
) -> Result<Vec<Element>> {
    let limit = end(e)?;
    let mut at = e.data;
    let mut out = Vec::new();
    while at < limit {
        goto(r, at)?;
        let child = element(r, limit, count, max)?;
        at = end(child)?;
        out.push(child);
    }
    Ok(out)
}
/// An ITU-T H.273 index stored as a `uInt` element.
///
/// The indices are one byte wide; a value past that is not an index the
/// standard defines, and 2 is what the standard itself uses for "unspecified"
/// in all three of them, so it reads as nothing stated.
fn cicp_code<R: Read + Seek>(r: &mut R, e: Element) -> Result<u8> {
    Ok(uint(r, e)?.try_into().unwrap_or(2))
}
/// Rectangular projection roll is counter-clockwise in Matroska.
/// Preserve the existing coded view for spherical, mirrored or arbitrary poses.
fn read_rotation<R: Read + Seek>(r: &mut R, projection: Element, count: &mut usize, max: usize) -> Result<u16> {
    let (mut kind, mut yaw, mut pitch, mut roll) = (0, 0.0, 0.0, 0.0);
    for field in fields(r, projection, count, max)? {
        match field.id {
            0x7671 => kind = uint(r, field)?,
            0x7673 => yaw = float(r, field)?,
            0x7674 => pitch = float(r, field)?,
            0x7675 => roll = float(r, field)?,
            _ => {},
        }
    }
    if !yaw.is_finite() || !pitch.is_finite() || !roll.is_finite() { return Err(invalid("nonfinite Matroska projection pose")); }
    if kind != 0 || yaw != 0.0 || pitch != 0.0 { return Ok(0); }
    Ok(match roll { 90.0 => 270, -90.0 => 90, 180.0 | -180.0 => 180, _ => 0 })
}

/// Read a `Colour` element and everything under it into the track it describes.
///
/// Matroska states the H.273 triple and the mastering volume as one element per
/// value instead of the byte payload MP4 and the bitstream use, and two of its
/// spellings differ from the standards they otherwise copy: `Range` numbers
/// studio as 1 and full as 2, where the flag in an HEVC VUI or an MP4 `colr`
/// sets the high bit for full; and `MasteringMetadata` states its corners in
/// the named red, green, blue order the payload never uses, as floats already
/// in candelas rather than in ST 2086 units.
fn read_colour<R: Read + Seek>(
    r: &mut R,
    colour: Element,
    count: &mut usize,
    max: usize,
    track: &mut Track,
) -> Result<()> {
    let mut corners = [0.0f64; 10];
    let mut stated = [false; 10];
    let mut light = ContentLight::default();
    for f in fields(r, colour, count, max)? {
        match f.id {
            0x55b1 => track.colour.matrix = cicp_code(r, f)?,
            0x55ba => track.colour.transfer = cicp_code(r, f)?,
            0x55bb => track.colour.primaries = cicp_code(r, f)?,
            0x55b9 => track.colour.full_range = uint(r, f)? == 2,
            0x55bc => light.max_cll = uint(r, f)? as f32,
            0x55bd => light.max_fall = uint(r, f)? as f32,
            0x55d0 => {
                for m in fields(r, f, count, max)? {
                    let slot = match m.id {
                        0x55d1 => Some(0),
                        0x55d2 => Some(1),
                        0x55d3 => Some(2),
                        0x55d4 => Some(3),
                        0x55d5 => Some(4),
                        0x55d6 => Some(5),
                        0x55d7 => Some(6),
                        0x55d8 => Some(7),
                        0x55d9 => Some(8),
                        0x55da => Some(9),
                        _ => None,
                    };
                    if let Some(i) = slot {
                        corners[i] = float(r, m)?;
                        stated[i] = true;
                    }
                }
            }
            _ => {}
        }
    }
    // A volume missing a corner describes no display a tone map could fit into,
    // so it is not read at all rather than read as a partial one.
    if stated.iter().all(|s| *s) {
        track.hdr.mastering = MasteringDisplay::from_corners(
            (corners[0], corners[1]),
            (corners[2], corners[3]),
            (corners[4], corners[5]),
            (corners[6], corners[7]),
            corners[8] as f32,
            corners[9] as f32,
        );
    }
    track.hdr.light = light;
    Ok(())
}
impl<R: Read + Seek> WebmReader<R> {
    /// Precision of block timestamps; CodecDelay and DiscardPadding use ns.
    pub fn timestamp_scale_ns(&self) -> u64 { self.scale }
    pub fn open(mut reader: R, limits: Limits) -> Result<Self> {
        let file_end = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut count = 0;
        let header = element(&mut reader, file_end, &mut count, limits.elements)?;
        if header.id != 0x1a45dfa3 {
            return Err(invalid("missing EBML header"));
        }
        let mut doctype = None;
        for e in fields(&mut reader, header, &mut count, limits.elements)? {
            if e.id == 0x4282 {
                doctype = Some(bytes(&mut reader, e, 16)?);
            }
        }
        if !matches!(doctype.as_deref(), Some(b"webm" | b"matroska")) {
            return Err(invalid("unsupported EBML document type"));
        }
        reader.seek(SeekFrom::Start(end(header)?))?;
        let segment = element(&mut reader, file_end, &mut count, limits.elements)?;
        if segment.id != SEGMENT {
            return Err(invalid("missing WebM Segment"));
        }
        let mut this = Self {
            reader,
            tracks: Vec::new(),
            packets: Vec::new(),
            duration_ns: None,
            chapters: Vec::new(),
            tags: FileTags::default(),
            limits,
            read_packet_bytes: limits.packet_bytes,
            segment_end: segment.end.unwrap_or(file_end),
            at: segment.data,
            scale: 1_000_000u64,
            duration_ticks: None,
            info_seen: false,
            chapter_runs: Vec::new(),
            block_title: String::new(),
            elements: count,
            scanned: false,
            tail_ns: i64::MIN,
        };
        // One cluster is what a first picture needs, and indexing the rest of
        // them is the whole file read on a mount that answers in bursts: a WebM
        // with no `Cues` element has every block walked to be listed at all.
        this.walk(true)?;
        // Two things a reader cannot do without a complete index, both of which
        // the head answers before the first cluster is reached: tracks some
        // streaming muxers leave at the end, and a length the file never states,
        // which only the last block's timestamp can supply.
        if this.tracks.is_empty() || this.duration_ticks.is_none() {
            this.walk(false)?;
        }
        this.settle()?;
        Ok(this)
    }

    /// Index what the file holds next: the cluster that opens the picture, or
    /// every cluster that is left when a reader has to know the whole item.
    fn walk(&mut self, one_cluster: bool) -> Result<()> {
        let Self {
            reader,
            tracks,
            packets,
            limits,
            at,
            segment_end,
            scale,
            duration_ticks,
            info_seen,
            chapter_runs,
            block_title,
            tags,
            elements,
            scanned,
            tail_ns,
            duration_ns: _,
            chapters: _,
            read_packet_bytes: _,
        } = self;
        let indexed = packets.len();
        while *at < *segment_end {
            goto(&mut *reader, *at)?;
            let e = element(&mut *reader, *segment_end, &mut *elements, limits.elements)?;
            match e.id {
                0x1549a966 => {
                    for f in fields(&mut *reader, e, &mut *elements, limits.elements)? {
                        if f.id == 0x4489 {
                            let value = float(&mut *reader, f)?;
                            if !value.is_finite() || value <= 0.0 {
                                return Err(invalid("invalid WebM Duration"));
                            }
                            *duration_ticks = Some(value);
                        } else if f.id == 0x7ba9 {
                            *block_title = lenient_text(&mut *reader, f, 1024).unwrap_or_default();
                        } else if f.id == 0x2ad7b1 {
                            *scale = uint(&mut *reader, f)?;
                            if *scale == 0 {
                                return Err(invalid("zero WebM timestamp scale"));
                            }
                        }
                    }
                    // Whatever the block states or leaves out, the scale it did
                    // not name is the one the specification gives, and no later
                    // cluster can change what a timestamp of this file means.
                    *info_seen = true;
                }
                0x1654ae6b => {
                    for entry in fields(&mut *reader, e, &mut *elements, limits.elements)? {
                        if entry.id != 0xae {
                            continue;
                        }
                        if tracks.len() >= 64 {
                            return Err(invalid("too many WebM tracks"));
                        }
                        let mut track = Track {
                            number: 0,
                            kind: 0,
                            codec: String::new(),
                            name: String::new(),
                            language: String::new(),
                            width: 0,
                            height: 0,
                            display: (0, 0),
                            crop: [0; 4],
                            rotation: 0,
                            sample_rate: 0,
                            channels: 0,
                            bit_depth: 0,
                            default_duration_ns: 0,
                            codec_delay_ns: 0,
                            codec_private: Vec::new(),
                            colour: ColourDescription::default(),
                            hdr: HdrMetadata::default(),
                        };
                        // Two elements can state a language: the one in use now
                        // and, in files muxed before it existed, the older one in
                        // the same place. Whichever the newer spelling is wins, in
                        // either writing order, so a track that carries both does
                        // not read as two different languages.
                        let mut older = String::new();
                        let mut current = String::new();
                        for f in fields(&mut *reader, entry, &mut *elements, limits.elements)? {
                            match f.id {
                                0xd7 => track.number = uint(&mut *reader, f)?,
                                0x83 => track.kind = uint(&mut *reader, f)?,
                                0x86 => {
                                    // Some muxers NUL-terminate the CodecID string.
                                    track.codec = String::from_utf8(bytes(&mut *reader, f, 128)?)
                                        .map_err(|_| invalid("invalid WebM codec ID"))?
                                        .trim_end_matches('\0')
                                        .to_owned();
                                }
                                0x536e => {
                                    track.name = text(&mut *reader, f, 1024)?;
                                }
                                0x22b59c => current = text(&mut *reader, f, 128)?,
                                0x447a => older = text(&mut *reader, f, 128)?,
                                0x23e383 => track.default_duration_ns = uint(&mut *reader, f)?,
                                0x56aa => track.codec_delay_ns = uint(&mut *reader, f)?,
                                0x63a2 => {
                                    track.codec_private = bytes(&mut *reader, f, 1 << 20)?;
                                }
                                0xe0 => {
                                    let mut drawn = (0, 0);
                                    // A display size is only in pixels while
                                    // `DisplayUnit` says so; counted lines or
                                    // centimetres divide into nothing here.
                                    let mut unit = 0;
                                    // Read in the order the elements state —
                                    // bottom, top, left, right — and kept in
                                    // the order a viewer cuts them: left, top,
                                    // right, bottom.
                                    let mut crop = [0; 4];
                                    for v in fields(&mut *reader, f, &mut *elements, limits.elements)? {
                                        match v.id {
                                            0xb0 => track.width = uint(&mut *reader, v)?,
                                            0xba => track.height = uint(&mut *reader, v)?,
                                            0x54b0 => drawn.0 = uint(&mut *reader, v)?,
                                            0x54ba => drawn.1 = uint(&mut *reader, v)?,
                                            0x54b2 => unit = uint(&mut *reader, v)?,
                                            0x54aa => crop[3] = uint(&mut *reader, v)?,
                                            0x54bb => crop[1] = uint(&mut *reader, v)?,
                                            0x54cc => crop[0] = uint(&mut *reader, v)?,
                                            0x54dd => crop[2] = uint(&mut *reader, v)?,
                                            0x7670 => track.rotation = read_rotation(&mut *reader, v, &mut *elements, limits.elements)?,
                                            0x55b0 => read_colour(
                                                &mut *reader,
                                                v,
                                                &mut *elements,
                                                limits.elements,
                                                &mut track,
                                            )?,
                                            _ => {}
                                        }
                                    }
                                    track.display = if unit == 0 || unit == 3 { drawn } else { (0, 0) };
                                    // A crop that leaves no picture is a file
                                    // making a statement it cannot keep; the
                                    // whole picture is shown.
                                    if crop[0].checked_add(crop[2]).is_some_and(|n|n<track.width)
                                        && crop[1].checked_add(crop[3]).is_some_and(|n|n<track.height)
                                    {
                                        track.crop = crop;
                                    }
                                }
                                0xe1 => {
                                    for v in fields(&mut *reader, f, &mut *elements, limits.elements)? {
                                        match v.id {
                                            0xb5 => {
                                                let value = float(&mut *reader, v)?;
                                                if !value.is_finite() || value <= 0.0 {
                                                    return Err(invalid(
                                                        "invalid WebM sampling frequency",
                                                    ));
                                                }
                                                track.sample_rate = value.round() as u64;
                                            }
                                            0x9f => track.channels = uint(&mut *reader, v)?,
                                            // `BitDepth` under `Audio`, in the two-byte
                                            // form the specification gives it; PCM has
                                            // nothing else to state its sample width in.
                                            0x6264 => track.bit_depth = uint(&mut *reader, v)?,
                                            _ => {}
                                        }
                                    }
                                }
                                0x6d80 => {
                                    return Err(invalid(
                                        "encoded/encrypted WebM tracks not supported",
                                    ));
                                }
                                _ => {}
                            }
                        }
                        // A track whose only statement is the `und` writers use
                        // for "nothing was said" leaves the player knowing no
                        // more than one that states nothing at all.
                        let stated = if current.is_empty() { older } else { current };
                        track.language = if stated == "und" {
                            String::new()
                        } else {
                            stated
                        };
                        if track.number == 0
                            || tracks.iter().any(|t: &Track| t.number == track.number)
                            || track.codec.is_empty()
                        {
                            return Err(invalid("invalid WebM track"));
                        }
                        tracks.push(track);
                    }
                }
                0x1254c367 => {
                    read_tags(&mut *reader, e, &mut *elements, limits.elements, &mut *tags);
                }
                0x1043a770 => {
                    read_chapters(
                        &mut *reader,
                        e,
                        &mut *elements,
                        limits.elements,
                        &mut *chapter_runs,
                    );
                }
                CLUSTER => {
                    let cluster_end = e.end.unwrap_or(*segment_end);
                    let mut pos = e.data;
                    let mut timestamp = None;
                    while pos < cluster_end {
                        goto(&mut *reader, pos)?;
                        let child = element(&mut *reader, cluster_end, &mut *elements, limits.elements)?;
                        if e.end.is_none()
                            && matches!(
                                child.id,
                                CLUSTER
                                    | 0x1549a966
                                    | 0x1654ae6b
                                    | 0x1c53bb6b
                                    | 0x114d9b74
                                    | 0x1254c367
                                    | 0x1941a469
                                    | 0x1043a770
                            )
                        {
                            break;
                        }
                        match child.id {
                            0xe7 => timestamp = Some(uint(&mut *reader, child)?),
                            0xa3 => read_block(
                                &mut *reader,
                                child,
                                timestamp,
                                true,
                                true,
                                &mut *packets,
                                *limits,
                            )?,
                            0xa0 => {
                                let fs = fields(&mut *reader, child, &mut *elements, limits.elements)?;
                                let key = !fs.iter().any(|f| f.id == 0xfb);
                                let mut padding = None;
                                let mut duration = None;
                                for field in &fs {
                                    if field.id == 0x9b {
                                        if duration.is_some() { return Err(invalid("duplicate Matroska BlockDuration")); }
                                        duration = Some(uint(&mut *reader, *field)?);
                                    }
                                    if field.id == 0x75a2 {
                                        if padding.is_some() { return Err(invalid("duplicate Matroska DiscardPadding")); }
                                        padding = Some(sint(&mut *reader, *field)?);
                                    }
                                }
                                let first = packets.len();
                                for block in fs {
                                    if block.id == 0xa1 {
                                        read_block(
                                            &mut *reader,
                                            block,
                                            timestamp,
                                            false,
                                            key,
                                            &mut *packets,
                                            *limits,
                                        )?;
                                    }
                                }
                                for packet in &mut packets[first..] {
                                    packet.discard_padding_ns = padding.unwrap_or(0);
                                    packet.duration_ns = duration;
                                }
                            }
                            _ => {}
                        }
                        pos = end(child)?;
                    }
                    *at = pos;
                    // The first picture can be shown once a cluster's blocks are
                    // known, so a walk asked for one cluster stops here. It waits
                    // for the file to state its scale and its tracks first, which
                    // the head of a file always does and a pathological one does
                    // not get to skip past.
                    if one_cluster
                        && *info_seen
                        && !tracks.is_empty()
                        && packets.len() > indexed
                    {
                        break;
                    }
                    continue;
                }
                _ => {}
            }
            *at = end(e)?;
        }
        // Blocks are indexed in the cluster timestamps they carry, which ride in
        // `TimestampScale`, so the scale is put on what this pass recorded: an
        // index that grew over several walks still reads in nanoseconds.
        for p in &mut packets[indexed..] {
            p.pts_ns = i64::try_from(i128::from(p.pts_ns) * i128::from(*scale))
                .map_err(|_| invalid("WebM timestamp overflow"))?;
            p.duration_ns = p.duration_ns.map(|ticks| ticks.checked_mul(*scale)
                .ok_or_else(|| invalid("WebM block duration overflow"))).transpose()?;
            *tail_ns = (*tail_ns).max(p.pts_ns);
            if !tracks.iter().any(|t: &Track| t.number == p.track) {
                return Err(invalid("WebM packet references missing track"));
            }
        }
        *scanned = *at >= *segment_end;
        if tracks.is_empty() && *scanned {
            return Err(invalid("WebM has no tracks"));
        }
        Ok(())
    }

    /// What the walk has read so far, turned into the answers a reader asks the
    /// file for: its length, its chapters and the name it gave itself. Run after
    /// every pass, because any of the three can sit behind the clusters.
    fn settle(&mut self) -> Result<()> {
        self.duration_ns = match self.duration_ticks {
            Some(ticks) => {
                let nanos = (ticks * self.scale as f64).round();
                if !nanos.is_finite() || nanos < 1.0 || nanos >= u64::MAX as f64 {
                    return Err(invalid("WebM Duration overflow"));
                }
                Some(nanos as u64)
            }
            None => None,
        };
        // ChapterTimeStart/End are Matroska ticks (nanoseconds), independent
        // of Segment TimestampScale: https://www.matroska.org/technical/elements.html
        let mut chapters: Vec<Chapter> = self.chapter_runs.iter()
            .filter(|(start, end, _)| end.is_none_or(|end| end >= *start))
            .map(|(start, end, title)| Chapter {
                start_ns: *start, end_ns: *end, title: title.clone(),
            }).collect();
        chapters.sort_by_key(|chapter| chapter.start_ns);
        self.chapters = chapters;
        // A file that wrote its name in both places wrote it in the tags, which
        // is where the specification puts it; the information block is the older
        // spelling and the only one some muxers reach for.
        if self.tags.title.is_empty() {
            self.tags.title = self.block_title.clone();
        }
        Ok(())
    }
    /// Index the cluster after everything known so far, which is what a reader
    /// that has run out of blocks asks before it believes the item ended. Says
    /// whether the file still holds clusters behind this one.
    pub fn scan_more(&mut self) -> Result<bool> {
        if self.scanned {
            return Ok(false);
        }
        let before = self.packets.len();
        self.walk(true)?;
        self.settle()?;
        Ok(self.packets.len() > before)
    }

    /// Index clusters until a block at or after `pts_ns` is known. A jump only
    /// needs the blocks behind its target, and a file's clusters are timed in
    /// order, so the walk can stop the moment it passes the point asked for
    /// rather than reaching the end of the item to answer it.
    pub fn scan_until(&mut self, pts_ns: i64) -> Result<()> {
        while !self.scanned && self.tail_ns < pts_ns {
            self.walk(true)?;
            self.settle()?;
        }
        Ok(())
    }

    /// Index every block the file holds. A seek needs the blocks behind the
    /// reader as well as the ones in front of it, and a reader that would take
    /// the item's length from its own blocks has no cheaper question to ask.
    pub fn scan_all(&mut self) -> Result<()> {
        if self.scanned {
            return Ok(());
        }
        self.walk(false)?;
        self.settle()
    }
    /// Whether every cluster the file holds has been indexed. A reader that
    /// measures the item from its own blocks can only do that once the tail has
    /// been met, so it asks before trusting a short answer.
    pub fn fully_indexed(&self) -> bool {
        self.scanned
    }
    /// Tighten encoded payload reads without treating codec metadata as packets.
    pub fn restrict_packet_bytes(&mut self, maximum: usize) {
        self.read_packet_bytes = self.read_packet_bytes.min(maximum);
    }

    pub fn read_packet(&mut self, index: usize) -> Result<Vec<u8>> {
        let p = self
            .packets
            .get(index)
            .ok_or_else(|| invalid("WebM packet index out of bounds"))?;
        if p.size > self.read_packet_bytes {
            return Err(invalid("WebM packet exceeds budget"));
        }
        // Blocks are read in the order the walk found them, which is the order
        // they sit in the file, so the reader is usually already standing on the
        // packet it is asked for and the bytes stay where they were read to.
        goto(&mut self.reader, p.offset)?;
        let mut data = vec![0; p.size];
        self.reader.read_exact(&mut data)?;
        Ok(data)
    }
}
/// The words a text element holds, with an encoding this reader cannot spell
/// left as the characters it happens to decode to rather than as a file it
/// refuses: a name is worth showing however it was written, and a file with a
/// badly coded one still plays.
fn lenient_text<R: Read + Seek>(r: &mut R, e: Element, max: usize) -> Option<String> {
    Some(
        String::from_utf8_lossy(&bytes(r, e, max).ok()?)
            .trim_end_matches('\0')
            .to_owned(),
    )
}
/// The children of a master up to the first one this reader cannot read, which
/// `fields` would take as the whole list being bad. A `Tags` master is where
/// this is needed: some muxers rewrite it in place and leave a byte of padding
/// behind the last tag, and the tags standing before it are still the file's.
fn fields_to_first_gap<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
) -> Vec<Element> {
    let Ok(limit) = end(e) else {
        return Vec::new();
    };
    let mut at = e.data;
    let mut out = Vec::new();
    while at < limit {
        if r.seek(SeekFrom::Start(at)).is_err() {
            break;
        }
        let Ok(child) = element(r, limit, count, max) else {
            break;
        };
        let Ok(next) = end(child) else {
            break;
        };
        out.push(child);
        at = next;
    }
    out
}
/// What the whole file says about itself among its `Tags`: the `SimpleTag`
/// values of a `Tag` whose `Targets` single out no track, edition, chapter or
/// attachment. A list this reader cannot walk, or one that names nothing of the
/// kind, leaves the file stating nothing instead of failing it — as does a tag
/// of a name this player has no line for.
fn read_tags<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
    out: &mut FileTags,
) {
    for tag in fields_to_first_gap(r, e, count, max) {
        if tag.id != 0x7373 {
            continue;
        }
        let Ok(parts) = fields(r, tag, count, max) else {
            continue;
        };
        // Either order the two parts come in is settled before the tag is kept,
        // so a `SimpleTag` written before its `Targets` still has them.
        let mut names_a_part = false;
        let mut stated: Vec<(String, String)> = Vec::new();
        for part in parts {
            match part.id {
                0x63c0 => {
                    let entries = fields(r, part, count, max).unwrap_or_default();
                    names_a_part = entries
                        .iter()
                        .any(|entry| matches!(entry.id, 0x63c4 | 0x63c5 | 0x63c6 | 0x63c9));
                }
                0x67c8 => {
                    let Ok(fields_of_tag) = fields(r, part, count, max) else {
                        continue;
                    };
                    let (mut name, mut value) = (String::new(), String::new());
                    for field in fields_of_tag {
                        match field.id {
                            0x45a3 => name = lenient_text(r, field, 128).unwrap_or_default(),
                            0x4487 => value = lenient_text(r, field, 1024).unwrap_or_default(),
                            _ => {}
                        }
                    }
                    if !name.is_empty() && !value.is_empty() {
                        stated.push((name, value));
                    }
                }
                _ => {}
            }
        }
        if !names_a_part {
            for (name, value) in stated {
                out.insert(&name, &value);
            }
        }
    }
}

/// The chapter atoms of one `Chapters` master, kept in the units the file's
/// `TimestampScale` states so the scale — which the `Info` may still be to come
/// by — can be applied later. A list this reader cannot walk leaves no
/// chapters at all rather than failing the file, which plays fine without them.
fn read_chapters<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
    out: &mut Vec<(u64, Option<u64>, String)>,
) {
    // A master may carry no chapters at all rather than a bad one, so an
    // unknown-sized parent or a truncated list simply yields nothing.
    let Ok(editions) = fields(r, e, count, max) else {
        return;
    };
    for edition in editions {
        if edition.id != 0x45b9 {
            continue;
        }
        let Ok(atoms) = fields(r, edition, count, max) else {
            return;
        };
        for atom in atoms {
            if atom.id != 0xb6 {
                continue;
            }
            if out.len() >= 1_024 {
                return;
            }
            let Ok(entries) = fields(r, atom, count, max) else {
                return;
            };
            let mut start = None;
            let mut end = None;
            let mut title = String::new();
            for field in entries {
                match field.id {
                    0x91 => start = uint(r, field).ok(),
                    0x92 => end = uint(r, field).ok(),
                    // A chapter may be displayed in several languages; what the
                    // file leads with is what a player has to show.
                    0x80 if title.is_empty() => {
                        if let Ok(texts) = fields(r, field, count, max) {
                            for text in texts {
                                if text.id != 0x85 {
                                    continue;
                                }
                                if let Ok(bytes) = bytes(r, text, 1024) {
                                    title = String::from_utf8_lossy(&bytes)
                                        .trim_end_matches('\0')
                                        .to_owned();
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(start) = start { out.push((start, end, title)); }
        }
    }
}
fn read_block<R: Read + Seek>(
    r: &mut R,
    e: Element,
    timestamp: Option<u64>,
    simple: bool,
    key: bool,
    out: &mut Vec<Packet>,
    limits: Limits,
) -> Result<()> {
    if out.len() >= limits.packets {
        return Err(invalid("WebM packet count exceeds limit"));
    }
    goto(r, e.data)?;
    let (track, unknown) = vint(r, false)?;
    if track == 0 || unknown {
        return Err(invalid("invalid WebM block track"));
    }
    let mut h = [0; 3];
    r.read_exact(&mut h)?;
    let offset = r.stream_position()?;
    let limit = end(e)?;
    if offset >= limit {
        return Err(invalid("truncated WebM block"));
    }
    if h[2] & 6 != 0 {
        return Err(invalid("WebM laced blocks are not yet supported"));
    }
    let pts =
        i128::from(timestamp.ok_or_else(|| invalid("WebM block precedes Cluster timestamp"))?)
            + i128::from(i16::from_be_bytes([h[0], h[1]]));
    let size = usize::try_from(limit - offset)
        .ok()
        .filter(|&n| n <= limits.packet_bytes)
        .ok_or_else(|| invalid("WebM packet exceeds budget"))?;
    out.push(Packet {
        track,
        pts_ns: i64::try_from(pts).map_err(|_| invalid("WebM timestamp overflow"))?,
        keyframe: if simple { h[2] & 0x80 != 0 } else { key },
        invisible: h[2] & 0x08 != 0,
        offset,
        size,
        discard_padding_ns: 0,
        duration_ns: None,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn atom(id: &[u8], payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 127);
        [id, &[0x80 | payload.len() as u8], payload].concat()
    }
    #[test]
    fn rectangular_projection_roll_has_clockwise_display_orientation() {
        let parse = |kind: u8, yaw: f64, pitch: f64, roll: f64| {
            let data = [atom(&[0x76, 0x71], &[kind]),
                atom(&[0x76, 0x73], &yaw.to_be_bytes()), atom(&[0x76, 0x74], &pitch.to_be_bytes()),
                atom(&[0x76, 0x75], &roll.to_be_bytes())].concat();
            let parent = Element { id: 0x7670, data: 0, end: Some(data.len() as u64) };
            read_rotation(&mut Cursor::new(data), parent, &mut 0, 100)
        };
        for (roll, angle) in [(0.0, 0), (-90.0, 90), (90.0, 270), (180.0, 180), (-180.0, 180)] {
            assert_eq!(parse(0, 0.0, 0.0, roll).unwrap(), angle);
        }
        assert_eq!(parse(1, 0.0, 0.0, 90.0).unwrap(), 0);
        assert_eq!(parse(0, 180.0, 0.0, 90.0).unwrap(), 0);
        assert_eq!(parse(0, 0.0, 90.0, 90.0).unwrap(), 0);
        assert_eq!(parse(0, 0.0, 0.0, 45.0).unwrap(), 0);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(parse(0, 0.0, 0.0, bad).is_err());
            assert!(parse(0, bad, 0.0, 0.0).is_err());
            assert!(parse(0, 0.0, bad, 0.0).is_err());
        }
    }
    #[test]
    fn block_duration_scales_once_across_lazy_clusters_and_rejects_overflow() {
        let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"matroska"));
        let info = atom(&[0x15, 0x49, 0xa9, 0x66], &atom(&[0x2a, 0xd7, 0xb1], &2_000_000u32.to_be_bytes()));
        let tracks = atom(&[0x16, 0x54, 0xae, 0x6b], &atom(&[0xae], &[
            atom(&[0xd7], &[1]), atom(&[0x83], &[2]), atom(&[0x86], b"A_AAC"),
        ].concat()));
        let cluster = |ticks: u64, duplicate: bool| {
            let mut group = atom(&[0xa1], &[0x81, 0, 0, 0, 1]);
            group.extend(atom(&[0x9b], &ticks.to_be_bytes()));
            if duplicate { group.extend(atom(&[0x9b], &[1])); }
            atom(&[0x1f, 0x43, 0xb6, 0x75], &[atom(&[0xe7], &[0]), atom(&[0xa0], &group)].concat())
        };
        let prefix = [header, vec![0x18, 0x53, 0x80, 0x67, 0xff], info, tracks].concat();
        let data = [prefix.clone(), cluster(5, false), cluster(7, false)].concat();
        let mut reader = WebmReader::open(Cursor::new(data), Limits::default()).unwrap();
        reader.scan_all().unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.iter().map(|p| p.duration_ns).collect::<Vec<_>>(), [Some(10_000_000), Some(14_000_000)]);
        for invalid in [cluster(u64::MAX, false), cluster(5, true)] {
            let result = WebmReader::open(Cursor::new([prefix.clone(), invalid].concat()), Limits::default())
                .and_then(|mut r| r.scan_all());
            assert!(result.is_err());
        }
    }
    #[test]
    fn delay_and_signed_discard_padding_keep_nanosecond_units() {
        let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"matroska"));
        let info = atom(&[0x15, 0x49, 0xa9, 0x66], &atom(&[0x2a, 0xd7, 0xb1], &2_000_000u32.to_be_bytes()));
        let track = atom(&[0xae], &[
            atom(&[0xd7], &[1]), atom(&[0x83], &[2]), atom(&[0x86], b"A_AAC"),
            atom(&[0x56, 0xaa], &21_333_333u32.to_be_bytes()),
        ].concat());
        let tracks = atom(&[0x16, 0x54, 0xae, 0x6b], &track);
        for value in [0i64, 1, -1, 128, -129, 1_000_000, -1_000_000, i64::MIN, i64::MAX] {
            // Include both compact negative/positive signed encodings and 8-byte extremes.
            let encoded = value.to_be_bytes();
            let width = if (-128..=127).contains(&value) { 1 } else { 8 };
            for padding_first in [false, true] {
                let block = atom(&[0xa1], &[0x81, 0, 3, 0, 0xe0]);
                let padding = atom(&[0x75, 0xa2], &encoded[8-width..]);
                let children = if padding_first { [padding, block] } else { [block, padding] };
                let group = atom(&[0xa0], &children.concat());
                let cluster = atom(&[0x1f, 0x43, 0xb6, 0x75], &[atom(&[0xe7], &[1]), group].concat());
                let bytes = [header.clone(), vec![0x18, 0x53, 0x80, 0x67, 0xff], info.clone(), tracks.clone(), cluster].concat();
                let mut reader = WebmReader::open(Cursor::new(bytes), Limits::default()).unwrap();
                reader.scan_all().unwrap();
                assert_eq!(reader.tracks[0].codec_delay_ns, 21_333_333);
                assert_eq!(reader.packets[0].discard_padding_ns, value);
                assert_eq!(reader.packets[0].pts_ns, 8_000_000);
                assert_eq!(reader.read_packet(0).unwrap(), [0xe0]);
            }
        }
    }

    fn fixture(lace: bool) -> Vec<u8> {
        let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
        let track = atom(
            &[0xae],
            &[
                atom(&[0xd7], &[1]),
                atom(&[0x83], &[1]),
                atom(&[0x86], b"V_VP9"),
                atom(
                    &[0xe0],
                    &[atom(&[0xb0], &[16]), atom(&[0xba], &[16])].concat(),
                ),
            ]
            .concat(),
        );
        let tracks = atom(&[0x16, 0x54, 0xae, 0x6b], &track);
        let cluster = |time: u8| {
            [
                vec![0x1f, 0x43, 0xb6, 0x75, 0xff],
                atom(&[0xe7], &[time]),
                atom(
                    &[0xa3],
                    &[0x81, 0xff, 0xff, if lace { 0x82 } else { 0x80 }, 0x82, 0x49],
                ),
            ]
            .concat()
        };
        [
            header,
            vec![0x18, 0x53, 0x80, 0x67, 0xff],
            tracks,
            cluster(2),
            cluster(4),
        ]
        .concat()
    }
    #[test]
    fn unknown_segment_and_cluster_sizes_signed_timestamps_and_packet_reads() {
        let mut r = WebmReader::open(Cursor::new(fixture(false)), Limits::default()).unwrap();
        assert_eq!(r.tracks[0].codec, "V_VP9");
        assert_eq!(r.tracks[0].width, 16);
        assert_eq!(r.packets.len(), 2);
        assert_eq!(r.packets[0].pts_ns, 1_000_000);
        assert_eq!(r.packets[1].pts_ns, 3_000_000);
        assert!(r.packets.iter().all(|p| p.keyframe));
        assert_eq!(r.read_packet(1).unwrap(), [0x82, 0x49]);
        assert!(r.read_packet(2).is_err());
    }
    /// A track's display size says what shape its pixels have only against the
    /// size it stores them at. Pixel dimensions and explicit display ratios
    /// both specify shape; physical display units remain unsupported.
    #[test]
    fn a_video_track_display_size_becomes_the_shape_of_its_pixels() {
        fn video(extra: &[Vec<u8>]) -> Vec<u8> {
            let mut block = vec![atom(&[0xb0], &[16]), atom(&[0xba], &[8])];
            block.extend_from_slice(extra);
            let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
            let track = atom(
                &[0xae],
                &[
                    atom(&[0xd7], &[1]),
                    atom(&[0x83], &[1]),
                    atom(&[0x86], b"V_VP9"),
                    atom(&[0xe0], &block.concat()),
                ]
                .concat(),
            );
            [
                header,
                vec![0x18, 0x53, 0x80, 0x67, 0xff],
                atom(&[0x16, 0x54, 0xae, 0x6b], &track),
            ]
            .concat()
        }
        let track = |extra: &[Vec<u8>]| {
            let data = video(extra);
            let reader = WebmReader::open(Cursor::new(data), Limits::default()).unwrap();
            reader.tracks[0].clone()
        };
        // 16x8 stored, drawn 32x8: every pixel twice as wide as it is tall.
        let wide = track(&[atom(&[0x54, 0xb0], &[32]), atom(&[0x54, 0xba], &[8])]);
        assert_eq!((wide.width, wide.height, wide.display), (16, 8, (32, 8)));
        assert_eq!(wide.pixel_aspect(), (2, 1));
        // Drawn at the size it is stored: square, stated rather than silent.
        assert_eq!(
            track(&[atom(&[0x54, 0xb0], &[16]), atom(&[0x54, 0xba], &[8])]).pixel_aspect(),
            (1, 1)
        );
        assert_eq!(track(&[]).pixel_aspect(), (1, 1));
        let ratio = track(&[atom(&[0x54, 0xb0], &[16]), atom(&[0x54, 0xba], &[9]), atom(&[0x54, 0xb2], &[3])]);
        assert_eq!(ratio.pixel_aspect(), (8, 9));
        // Measured in centimetres rather than pixels, the pair is a size on a
        // screen whose dots this reader does not know.
        let counted = track(&[
            atom(&[0x54, 0xb0], &[100]),
            atom(&[0x54, 0xba], &[50]),
            atom(&[0x54, 0xb2], &[2]),
        ]);
        assert_eq!((counted.display, counted.pixel_aspect()), ((0, 0), (1, 1)));
    }
    /// The four `PixelCrop*` elements state a border of coded pixels to keep
    /// off screen. They arrive in the order the specification writes them —
    /// bottom, top, left, right — and are kept in the order a viewer cuts them;
    /// a crop that leaves nothing to show is a statement the file cannot keep,
    /// so the whole picture stands and the pixels stay square.
    #[test]
    fn a_video_track_keeps_its_crop_borders_and_shows_only_what_survives_them() {
        fn video(extra: &[Vec<u8>]) -> Vec<u8> {
            let mut block = vec![atom(&[0xb0], &[16]), atom(&[0xba], &[16])];
            block.extend_from_slice(extra);
            let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
            let track = atom(
                &[0xae],
                &[
                    atom(&[0xd7], &[1]),
                    atom(&[0x83], &[1]),
                    atom(&[0x86], b"V_VP9"),
                    atom(&[0xe0], &block.concat()),
                ]
                .concat(),
            );
            [
                header,
                vec![0x18, 0x53, 0x80, 0x67, 0xff],
                atom(&[0x16, 0x54, 0xae, 0x6b], &track),
            ]
            .concat()
        }
        let track = |extra: &[Vec<u8>]| {
            let reader = WebmReader::open(Cursor::new(video(extra)), Limits::default()).unwrap();
            reader.tracks[0].clone()
        };
        let cropped = track(&[
            atom(&[0x54, 0xaa], &[2]),
            atom(&[0x54, 0xbb], &[3]),
            atom(&[0x54, 0xcc], &[4]),
            atom(&[0x54, 0xdd], &[2]),
        ]);
        assert_eq!(cropped.crop, [4, 3, 2, 2]);
        assert_eq!(cropped.visible(), (10, 11));
        // Nothing cropped, nothing to keep off screen.
        assert_eq!(track(&[]).visible(), (16, 16));
        // A border as wide as the picture leaves no picture to show.
        let greedy = track(&[atom(&[0x54, 0xcc], &[8]), atom(&[0x54, 0xdd], &[8])]);
        assert_eq!(greedy.crop, [0; 4]);
        assert_eq!(greedy.visible(), (16, 16));
        // The display size is a statement about the visible picture, so the
        // crop divides into the coded size before the shape of a pixel does:
        // 16 stored with a 4-pixel border leaves 12x16 to draw, and a file
        // that draws that into 16x16 means pixels four parts wide to three
        // parts tall.
        let stretched = track(&[
            atom(&[0x54, 0xcc], &[4]),
            atom(&[0x54, 0xb0], &[16]),
            atom(&[0x54, 0xba], &[16]),
        ]);
        assert_eq!(stretched.visible(), (12, 16));
        assert_eq!(stretched.pixel_aspect(), (4, 3));
    }
    /// An audio track's rate is a float element, not an integer, and
    /// `CodecPrivate` is where Vorbis keeps the setup headers a decoder cannot
    /// start without.
    #[test]
    fn audio_track_reads_its_float_rate_and_codec_private() {
        fn with_rate(rate: Vec<u8>) -> Vec<u8> {
            let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
            let track = atom(
                &[0xae],
                &[
                    atom(&[0xd7], &[2]),
                    atom(&[0x83], &[2]),
                    atom(&[0x86], b"A_VORBIS"),
                    atom(&[0x63, 0xa2], b"\x01vorbisidentification"),
                    atom(
                        &[0xe1],
                        &[atom(&[0xb5], &rate), atom(&[0x9f], &[2])].concat(),
                    ),
                ]
                .concat(),
            );
            [
                header,
                vec![0x18, 0x53, 0x80, 0x67, 0xff],
                atom(&[0x16, 0x54, 0xae, 0x6b], &track),
            ]
            .concat()
        }
        for rate in [
            44100.0f32.to_be_bytes().to_vec(),
            44100.0f64.to_be_bytes().to_vec(),
        ] {
            let reader = WebmReader::open(Cursor::new(with_rate(rate)), Limits::default()).unwrap();
            let track = &reader.tracks[0];
            assert_eq!(track.kind, 2);
            assert_eq!(track.sample_rate, 44_100);
            assert_eq!(track.channels, 2);
            assert_eq!(track.codec_private, b"\x01vorbisidentification");
        }
        for rate in [
            0.0f64.to_be_bytes().to_vec(),
            f64::NAN.to_be_bytes().to_vec(),
        ] {
            assert!(WebmReader::open(Cursor::new(with_rate(rate)), Limits::default()).is_err());
        }
    }
    #[test]
    fn declared_duration_uses_final_scale_and_validates_float() {
        for value in [
            12.5f32.to_be_bytes().to_vec(),
            12.5f64.to_be_bytes().to_vec(),
        ] {
            let mut data = fixture(false);
            data.extend(atom(
                &[0x15, 0x49, 0xa9, 0x66],
                &[
                    atom(&[0x44, 0x89], &value),
                    atom(&[0x2a, 0xd7, 0xb1], &[0x03, 0xe8]),
                ]
                .concat(),
            ));
            let reader = WebmReader::open(Cursor::new(data), Limits::default()).unwrap();
            assert_eq!(reader.duration_ns, Some(12_500));
        }
        for value in [f64::NAN, f64::INFINITY, -1.0, 0.0, f64::MAX] {
            let mut data = fixture(false);
            data.extend(atom(
                &[0x15, 0x49, 0xa9, 0x66],
                &atom(&[0x44, 0x89], &value.to_be_bytes()),
            ));
            assert!(WebmReader::open(Cursor::new(data), Limits::default()).is_err());
        }
        assert_eq!(
            WebmReader::open(Cursor::new(fixture(false)), Limits::default())
                .unwrap()
                .duration_ns,
            None
        );
    }
    #[test]
    fn malformed_vints_lacing_and_limits_fail() {
        assert!(vint(&mut Cursor::new([0]), false).is_err());
        assert!(vint(&mut Cursor::new([8, 0, 0, 0, 0]), true).is_err());
        assert!(WebmReader::open(Cursor::new(fixture(true)), Limits::default()).is_err());
        for limits in [
            Limits {
                packets: 1,
                ..Limits::default()
            },
            Limits {
                packet_bytes: 1,
                ..Limits::default()
            },
            Limits {
                elements: 2,
                ..Limits::default()
            },
        ] {
            assert!(WebmReader::open(Cursor::new(fixture(false)), limits).is_err());
        }
        let bytes = fixture(false);
        for len in 0..bytes.len() {
            let _ = WebmReader::open(Cursor::new(&bytes[..len]), Limits::default());
        }
    }
    /// `tests/fixtures/chapters/chapters.mkv`, made with:
    /// ffmpeg -f lavfi -i testsrc=size=16x16:rate=4:duration=4 -i chapters.txt \
    ///   -map 0:v -map_metadata 1 -c:v libvpx-vp9 -crf 63 -b:v 0 \
    ///   -pix_fmt yuv420p tests/fixtures/chapters/chapters.mkv
    /// where `chapters.txt` is an FFmetadata file naming the three chapters at
    /// 0-1 s, 1-3 s and 3-4 s.
    const CHAPTERS: &[u8] = include_bytes!("../../tests/fixtures/chapters/chapters.mkv");
    #[test]
    fn chapter_atoms_come_out_in_the_files_own_clock() {
        let reader = WebmReader::open(Cursor::new(CHAPTERS), Limits::default())
            .expect("fixture has chapters");
        let shown: Vec<(u64, &str)> = reader
            .chapters
            .iter()
            .map(|c| (c.start_ns, c.title.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (0, "Opening"),
                // A title written in another alphabet reaches the player as is.
                (1_000_000_000, "Глава 2"),
                (3_000_000_000, "End"),
            ]
        );
    }
    fn chapter(atoms: &[u8]) -> Vec<u8> {
        atom(&[0x45, 0xb9], &atom(&[0xb6], atoms))
    }
    #[test]
    fn chapter_times_are_nanoseconds_independent_of_segment_scale() {
        let mut file = fixture(false);
        // Even with default 1 ms Segment ticks and no declared duration,
        // chapter values 100 and 250 remain nanoseconds, never milliseconds.
        file.extend(atom(
            &[0x10, 0x43, 0xa7, 0x70],
            &[
                chapter(&[0x91, 0x81, 100]),
                chapter(
                    &[
                        &[0x91, 0x81, 250][..],
                        &[0x92, 0x82, 0x01, 0x90][..],
                        &atom(&[0x80], &atom(&[0x85], b"Ok\0")),
                    ]
                    .concat(),
                ),
            ]
            .concat(),
        ));
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        let shown: Vec<(u64, &str)> = reader
            .chapters
            .iter()
            .map(|c| (c.start_ns, c.title.as_str()))
            .collect();
        assert_eq!(shown, [(100, ""), (250, "Ok")]);
        assert_eq!(reader.chapters[0].end_ns, None);
        assert_eq!(reader.chapters[1].end_ns, Some(400));
    }
    #[test]
    fn a_chapter_list_that_cannot_be_walked_leaves_the_file_playable() {
        let mut file = fixture(false);
        // An edition written with an unknown size has no end for the walker to
        // stop at, so its chapters are dropped rather than the whole file,
        // which plays fine without them.
        file.extend(atom(
            &[0x10, 0x43, 0xa7, 0x70],
            &[
                &[0x45, 0xb9, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff][..],
                &[0x91, 0x81, 1],
            ]
            .concat(),
        ));
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert!(reader.chapters.is_empty());
        assert!(!reader.packets.is_empty());
    }
    #[test]
    fn overflowing_crop_borders_keep_the_whole_picture() {
        for crop in [[u64::MAX,0,1,0],[0,u64::MAX,0,1],[u64::MAX,0,u64::MAX,0],[16,0,0,0],[2,2,2,2]] {
            let video=[atom(&[0xb0],&[16]),atom(&[0xba],&[16]),
                atom(&[0x54,0xcc],&crop[0].to_be_bytes()),atom(&[0x54,0xbb],&crop[1].to_be_bytes()),
                atom(&[0x54,0xdd],&crop[2].to_be_bytes()),atom(&[0x54,0xaa],&crop[3].to_be_bytes())].concat();
            let body=[atom(&[0xd7],&[1]),atom(&[0x83],&[1]),atom(&[0x86],b"V_VP9"),atom(&[0xe0],&video)].concat();
            let file=[atom(&[0x1a,0x45,0xdf,0xa3],&atom(&[0x42,0x82],b"webm")),
                vec![0x18,0x53,0x80,0x67,0xff],atom(&[0x16,0x54,0xae,0x6b],&atom(&[0xae],&body))].concat();
            let reader=WebmReader::open(Cursor::new(file),Limits::default()).unwrap();
            assert_eq!(reader.tracks[0].crop,if crop==[2,2,2,2] {crop}else{[0;4]});
        }
    }
    /// What a track is called and which language it speaks are each written in
    /// two spellings: the element in use now and, in files muxed before it, the
    /// older one in the same place. The newer statement wins in either writing
    /// order, the `und` a writer uses for "nothing was said" reads as a track
    /// that states no language, and a name the muxer ended with a NUL byte is
    /// the name without it.
    #[test]
    fn a_track_keeps_the_name_and_the_language_either_spelling_gives_it() {
        fn track(extra: &[Vec<u8>]) -> Track {
            let mut body = vec![
                atom(&[0xd7], &[1]),
                atom(&[0x83], &[2]),
                atom(&[0x86], b"A_VORBIS"),
            ];
            body.extend_from_slice(extra);
            let file = [
                atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm")),
                vec![0x18, 0x53, 0x80, 0x67, 0xff],
                atom(&[0x16, 0x54, 0xae, 0x6b], &atom(&[0xae], &body.concat())),
            ]
            .concat();
            WebmReader::open(Cursor::new(file), Limits::default())
                .expect("built file opens")
                .tracks[0]
                .clone()
        }
        let said = |name: &[u8], current: &[u8], older: &[u8]| {
            let track = track(&[
                atom(&[0x53, 0x6e], name),
                atom(&[0x22, 0xb5, 0x9c], current),
                atom(&[0x44, 0x7a], older),
            ]);
            (track.name, track.language)
        };
        assert_eq!(
            said(b"Commentary", b"ru", b"rus"),
            ("Commentary".into(), "ru".into())
        );
        // Only the older spelling, which is all a file muxed years ago states.
        assert_eq!(said(b"", b"", b"por"), ("".into(), "por".into()));
        // The `und` of either is the writer saying nothing, not a language.
        assert_eq!(said(b"", b"und", b"und"), ("".into(), "".into()));
        assert_eq!(
            said(b"Tail\0", b"eng\0", b""),
            ("Tail".into(), "eng".into())
        );
    }
    /// The same fields as a real muxer writes them: a titled audio track, a
    /// second one that states only its language, a subtitle track with both and
    /// a picture that states neither. `tests/fixtures/tracks/named.mkv`, made
    /// with:
    ///
    /// ```text
    /// ffmpeg -f lavfi -i testsrc=size=64x64:rate=10:duration=0.4 \
    ///   -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.4 \
    ///   -f lavfi -i sine=frequency=880:sample_rate=32000:duration=0.4 \
    ///   -i one.srt -i two.srt \
    ///   -map 0:v -map 1:a -map 2:a -map 3:0 -map 4:0 \
    ///   -c:v libvpx-vp9 -pix_fmt yuv420p -c:a flac -ac 1 -c:s srt \
    ///   -metadata:s:a:0 title=Первая -metadata:s:a:0 language=rus \
    ///   -metadata:s:a:1 language=fre \
    ///   -metadata:s:s:0 title=Титры -metadata:s:s:0 language=rus \
    ///   tests/fixtures/tracks/named.mkv
    /// ```
    #[test]
    fn a_real_file_lets_the_reader_name_its_tracks() {
        const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/tracks/named.mkv");
        let reader =
            WebmReader::open(Cursor::new(FIXTURE), Limits::default()).expect("fixture opens");
        let shown: Vec<(u64, &str, &str)> = reader
            .tracks
            .iter()
            .map(|track| (track.number, track.name.as_str(), track.language.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (1, "", ""),
                (2, "Первая", "rus"),
                (3, "", "fre"),
                (4, "Титры", "rus"),
                (5, "", ""),
            ]
        );
    }
    /// What a writer that follows the specification calls the file: a `Tag` whose
    /// `Targets` single out nothing, holding a `SimpleTag` named `TITLE`. A tag
    /// aimed at one track is that track's business, and a tag written after a
    /// byte of padding the muxer left behind is still read.
    fn file_tag(name: &[u8], value: &[u8]) -> Vec<u8> {
        atom(
            &[0x73, 0x73],
            &[
                atom(&[0x63, 0xc0], &[]),
                atom(
                    &[0x67, 0xc8],
                    &[atom(&[0x45, 0xa3], name), atom(&[0x44, 0x87], value)].concat(),
                ),
            ]
            .concat(),
        )
    }
    fn track_tag(name: &[u8], value: &[u8]) -> Vec<u8> {
        atom(
            &[0x73, 0x73],
            &[
                atom(
                    &[0x63, 0xc0],
                    &atom(&[0x63, 0xc5], &[0, 0, 0, 0, 0, 0, 7, 9]),
                ),
                atom(
                    &[0x67, 0xc8],
                    &[atom(&[0x45, 0xa3], name), atom(&[0x44, 0x87], value)].concat(),
                ),
            ]
            .concat(),
        )
    }
    /// A list of tags is longer than one byte of length states, so the master
    /// carrying it writes its size as a two-byte variable integer.
    fn masters(body: &[u8]) -> Vec<u8> {
        assert!(body.len() < 16_383);
        let size = (0x4000 | body.len() as u16).to_be_bytes();
        [&[0x12, 0x54, 0xc3, 0x67][..], &size, body].concat()
    }
    #[test]
    fn a_tag_of_the_file_names_it_and_a_tag_of_one_track_does_not() {
        for (built, name) in [
            (file_tag(b"TITLE", "Имя".as_bytes()), "Имя"),
            // The name is matched whatever case the writer chose for it.
            (file_tag(b"title", b"Ok"), "Ok"),
            // A file-level tag still names the file after one aimed at a track.
            (
                [track_tag(b"TITLE", b"No"), file_tag(b"TITLE", b"Ok")].concat(),
                "Ok",
            ),
            (
                [file_tag(b"TITLE", b"Ok"), track_tag(b"TITLE", b"No")].concat(),
                "Ok",
            ),
            // The last byte is what a muxer rewrites the list over: a tag this
            // reader cannot reach is left alone, and the ones before it stand.
            ([file_tag(b"TITLE", b"Ok"), vec![0]].concat(), "Ok"),
        ] {
            let file = [fixture(false), masters(&built)].concat();
            let reader =
                WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
            assert_eq!(reader.tags.title, name);
        }
        // A list holding nothing a file is named by leaves the file unnamed.
        let file = [
            fixture(false),
            masters(&track_tag(b"AUTHOR", "Не имя".as_bytes())),
        ]
        .concat();
        let reader = WebmReader::open(Cursor::new(file), Limits::default()).expect("opens");
        assert_eq!(reader.tags, FileTags::default());
    }

    /// The rest of what a file states about itself sits in the same list as its
    /// name, one `SimpleTag` per fact. A fact this player has no line for is
    /// dropped on the way, and a fact aimed at one track does not become a fact
    /// about the file.
    #[test]
    fn a_file_keeps_every_tag_that_names_it_rather_than_one_track() {
        let built = [
            file_tag(b"TITLE", b"T"),
            file_tag(b"ARTIST", "Артист".as_bytes()),
            file_tag(b"album", b"B"),
            file_tag(b"GENRE", b"G"),
            file_tag(b"DATE", b"2026"),
            file_tag(b"COMMENT", b"C"),
            file_tag(b"ENCODER", b"Lavf"),
            track_tag(b"ARTIST", b"Not the file's"),
            // A file that states one fact twice is not asked to mean it twice.
            file_tag(b"TITLE", b"later"),
        ]
        .concat();
        let file = [fixture(false), masters(&built)].concat();
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert_eq!(
            reader.tags,
            FileTags {
                title: "T".to_owned(),
                artist: "Артист".to_owned(),
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
    }

    /// ```text
    /// ffmpeg -f lavfi -i testsrc2=size=16x16:rate=4:duration=1 \
    ///   -metadata title="Имя из контейнера" -c:v libvpx-vp9 -crf 63 -b:v 0 \
    ///   -pix_fmt yuv420p -an tests/fixtures/tags/title.mkv
    /// ```
    /// The muxer here puts the title in the file's own information block rather
    /// than in its tags, and leaves the encoder's name in the tags — which is
    /// what keeps a block's title and a tag of a track from being read as one.
    #[test]
    fn a_real_file_names_itself_in_its_information_block() {
        const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/tags/title.mkv");
        let reader =
            WebmReader::open(Cursor::new(FIXTURE), Limits::default()).expect("fixture opens");
        assert_eq!(reader.tags.title, "Имя из контейнера");
    }

    /// ```text
    /// ffmpeg -f lavfi -i testsrc2=size=16x16:rate=4:duration=1 \
    ///   -metadata title=T -metadata artist=A -metadata album=B \
    ///   -metadata genre=G -metadata date=2026 -metadata comment=C \
    ///   -c:v libvpx-vp9 -crf 63 -b:v 0 -pix_fmt yuv420p -an \
    ///   tests/fixtures/tags/tags.mkv
    /// ```
    /// What that muxer leaves behind: every fact in its own tag of the whole
    /// file, the title among the tags rather than in the information block, and
    /// the encoder's name standing between them for nothing the player asks.
    #[test]
    fn a_real_file_names_itself_and_its_author_in_its_tags() {
        const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/tags/tags.mkv");
        let reader =
            WebmReader::open(Cursor::new(FIXTURE), Limits::default()).expect("fixture opens");
        assert_eq!(
            reader.tags,
            FileTags {
                title: "T".to_owned(),
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
    }

    /// A track number travels to the player under any of the three names its
    /// writers choose: the plain word, the Vorbis field the Matroska tag docs
    /// list, and the spelling ffmpeg's Matroska muxer writes for it. The value
    /// is text here rather than a number's bytes, so a place within an album
    /// keeps its second half: `3/12` arrives as it was written.
    #[test]
    fn a_track_number_arrives_under_whichever_name_its_writer_chose() {
        for name in [b"TRACK".as_slice(), b"TRACKNUMBER", b"PART_NUMBER"] {
            let file = [fixture(false), masters(&file_tag(name, b"3/12"))].concat();
            let reader =
                WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
            assert_eq!(reader.tags.track, "3/12", "under {name:?}");
        }
        // The first spelling a file states wins, as with every other fact.
        let built = [
            file_tag(b"TRACKNUMBER", b"2"),
            file_tag(b"PART_NUMBER", b"9"),
        ]
        .concat();
        let file = [fixture(false), masters(&built)].concat();
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert_eq!(reader.tags.track, "2");
    }

    /// The album's own artist, its disc and its publisher arrive under either
    /// spelling their writers choose — the Vorbis field, and the name ffmpeg's
    /// Matroska muxer keeps when it has no mapping of its own — and the disc
    /// keeps a total the file states, text here rather than a number's bytes.
    /// The rights and the file's own note of itself arrive the same way, under
    /// the plain names both muxers give them, and so does the rating that only
    /// this container carries.
    #[test]
    fn the_album_artist_the_disc_and_the_publisher_keep_their_spellings() {
        for name in [b"ALBUMARTIST".as_slice(), b"ALBUM_ARTIST"] {
            let file = [fixture(false), masters(&file_tag(name, b"AA"))].concat();
            let reader =
                WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
            assert_eq!(reader.tags.album_artist, "AA", "under {name:?}");
        }
        for name in [b"DISC".as_slice(), b"DISCNUMBER"] {
            let file = [fixture(false), masters(&file_tag(name, b"2/10"))].concat();
            let reader =
                WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
            assert_eq!(reader.tags.disc, "2/10", "under {name:?}");
        }
        let file = [fixture(false), masters(&file_tag(b"PUBLISHER", b"PB"))].concat();
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert_eq!(reader.tags.publisher, "PB");
        let file = [
            fixture(false),
            masters(
                &[
                    file_tag(b"COPYRIGHT", b"2026 The Holder"),
                    file_tag(b"DESCRIPTION", b"A note"),
                    file_tag(b"RATING", b"5"),
                ]
                .concat(),
            ),
        ]
        .concat();
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert_eq!(
            (
                reader.tags.copyright.as_str(),
                reader.tags.description.as_str(),
                reader.tags.rating.as_str(),
            ),
            ("2026 The Holder", "A note", "5")
        );
    }
    /// An element whose size is too long for `atom`'s single length byte.
    ///
    /// Matroska allows a float element of either four or eight bytes and muxers
    /// use both, so the fixtures here do too.
    fn element(id: &[u8], payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 0x4000);
        [
            id,
            &[0x40 | (payload.len() >> 8) as u8, payload.len() as u8],
            payload,
        ]
        .concat()
    }
    /// A video track whose `Video` element carries exactly the given `Colour`
    /// payload and nothing else.
    fn coloured(colour: &[u8]) -> Track {
        let data = [
            atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm")),
            vec![0x18, 0x53, 0x80, 0x67, 0xff],
            element(
                &[0x16, 0x54, 0xae, 0x6b],
                &element(
                    &[0xae],
                    &[
                        atom(&[0xd7], &[1]),
                        atom(&[0x83], &[1]),
                        atom(&[0x86], b"V_MPEGH/ISO/HEVC"),
                        element(
                            &[0xe0],
                            &[
                                atom(&[0xb0], &[16]),
                                atom(&[0xba], &[8]),
                                element(&[0x55, 0xb0], colour),
                            ]
                            .concat(),
                        ),
                    ]
                    .concat(),
                ),
            ),
        ]
        .concat();
        WebmReader::open(Cursor::new(data), Limits::default())
            .unwrap()
            .tracks[0]
            .clone()
    }
    /// `Colour` states the same H.273 indices an MP4 `colr` atom does, but
    /// numbers its range the other way round: 1 is the studio range a flag
    /// would call 0, and 2 the full range it would call 1. A muxer that wrote
    /// the flag's numbers here would have every file it writes stretched wrong.
    #[test]
    fn a_colour_element_states_its_range_in_matroskas_own_numbers() {
        // The eight bytes ffmpeg's muxer writes for a bt2020nc studio-range
        // track, taken from the file it produced.
        let t = coloured(&[atom(&[0x55, 0xb1], &[9]), atom(&[0x55, 0xb9], &[1])].concat());
        assert_eq!(t.colour.matrix, 9);
        assert!(!t.colour.full_range);
        assert!(t.colour.primary_set().is_none());
        assert!(t.hdr.is_empty());
        let t = coloured(&[atom(&[0x55, 0xb9], &[2])].concat());
        assert!(t.colour.full_range);
        // Nothing stated at all reads the same way, and an element this reader
        // has no meaning for — `BitsPerChannel` — is passed over.
        let t = coloured(&[atom(&[0x55, 0xb2], &[10])].concat());
        assert_eq!(t.colour, ColourDescription::default());
    }
    /// The triple, the light level and the mastering volume in one element,
    /// which is what an HDR10 track muxed by a tool that knows the elements
    /// states.
    #[test]
    fn a_colour_element_carries_the_triple_and_the_mastering_volume() {
        use crate::color::primaries::Primaries;
        let t = coloured(
            &[
                atom(&[0x55, 0xbb], &[9]),
                atom(&[0x55, 0xba], &[16]),
                atom(&[0x55, 0xb1], &[9]),
                atom(&[0x55, 0xb9], &[2]),
            ]
            .concat(),
        );
        assert_eq!(
            t.colour,
            ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: true,
            }
        );
        assert!(t.colour.is_hdr());
        assert_eq!(t.colour.primary_set(), Some(Primaries::BT2020));

        // One element per coordinate, in the named red, green, blue order the
        // byte payload never uses, and already in the units they mean.
        let ids: [[u8; 2]; 10] = [
            [0x55, 0xd1],
            [0x55, 0xd2],
            [0x55, 0xd3],
            [0x55, 0xd4],
            [0x55, 0xd5],
            [0x55, 0xd6],
            [0x55, 0xd7],
            [0x55, 0xd8],
            [0x55, 0xd9],
            [0x55, 0xda],
        ];
        let corners: [f64; 10] = [
            0.708, 0.292, 0.17, 0.797, 0.131, 0.046, 0.3127, 0.329, 1000.0, 0.0001,
        ];
        let volume = |drop: Option<usize>, wide: bool| {
            let mut out = Vec::new();
            for (i, (id, value)) in ids.iter().zip(corners).enumerate() {
                if drop == Some(i) {
                    continue;
                }
                let bytes: Vec<u8> = if wide {
                    value.to_be_bytes().to_vec()
                } else {
                    (value as f32).to_be_bytes().to_vec()
                };
                out.extend_from_slice(&element(id, &bytes));
            }
            out
        };
        let light = || {
            [
                atom(&[0x55, 0xbc], &[0x04, 0xd2]),
                atom(&[0x55, 0xbd], &[0x02, 0x37]),
            ]
            .concat()
        };
        for wide in [true, false] {
            let mut payload = element(&[0x55, 0xd0], &volume(None, wide));
            payload.extend_from_slice(&light());
            let t = coloured(&payload);
            let display = t.hdr.mastering.expect("a complete volume");
            assert!(display.is_hdr10(), "{display:?}");
            assert_eq!(display.max_luminance, 1_000.0);
            assert_eq!(display.min_luminance, 0.0001);
            assert_eq!(t.hdr.light.max_cll, 1_234.0);
            assert_eq!(t.hdr.light.max_fall, 567.0);
            // The volume is also the tone map's destination panel.
            assert_eq!(display.display_target().peak_nits, 1_000.0);
        }
        // A corner short is not a display: the light level the same element
        // states still reaches the player, and no volume is claimed.
        let mut payload = element(&[0x55, 0xd0], &volume(Some(3), true));
        payload.extend_from_slice(&light());
        let t = coloured(&payload);
        assert!(t.hdr.mastering.is_none());
        assert_eq!(t.hdr.light.max_cll, 1_234.0);
    }
    /// A source that reports the furthest byte it has actually handed over, so
    /// a test can say what an opening walk read rather than what it might have.
    struct Reach {
        inner: Cursor<Vec<u8>>,
        high: u64,
    }
    impl Read for Reach {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let at = self.inner.stream_position()?;
            let n = self.inner.read(buf)?;
            self.high = self.high.max(at + n as u64);
            Ok(n)
        }
    }
    impl Seek for Reach {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(from)
        }
    }
    /// A file that states its own length, with `clusters` blocks of `payload`
    /// bytes each, and the offset at which every cluster ends.
    fn timed(clusters: usize, payload: usize) -> (Vec<u8>, Vec<u64>) {
        let info = element(
            &[0x15, 0x49, 0xa9, 0x66],
            &[
                atom(&[0x2a, 0xd7, 0xb1], &[0x0f, 0x42, 0x40]),
                element(&[0x44, 0x89], &5000.0f32.to_be_bytes()),
            ]
            .concat(),
        );
        let tracks = element(
            &[0x16, 0x54, 0xae, 0x6b],
            &element(
                &[0xae],
                &[
                    atom(&[0xd7], &[1]),
                    atom(&[0x83], &[1]),
                    atom(&[0x86], b"V_VP9"),
                    element(
                        &[0xe0],
                        &[atom(&[0xb0], &[16]), atom(&[0xba], &[16])].concat(),
                    ),
                ]
                .concat(),
            ),
        );
        let cluster = |time: u8| {
            let mut block = vec![0x81, 0x00, 0x00, 0x80];
            block.extend(std::iter::repeat_n(0x27, payload));
            element(
                &[0x1f, 0x43, 0xb6, 0x75],
                &[
                    atom(&[0xe7], &[time]),
                    element(&[0xa3], &block),
                ]
                .concat(),
            )
        };
        let mut file = [
            atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm")),
            vec![0x18, 0x53, 0x80, 0x67, 0xff],
            info,
            tracks,
        ]
        .concat();
        let mut ends = Vec::new();
        for time in 0..clusters {
            file.extend_from_slice(&cluster(time as u8));
            ends.push(file.len() as u64);
        }
        (file, ends)
    }
    /// The hang this indexing shape exists to avoid: an item on a slow mount is
    /// opened by reading it end to end, because blocks can only be listed by
    /// walking the clusters that hold them. With a stated length the walk stops
    /// at the first cluster whose blocks are known, and everything behind it is
    /// read only when a reader asks for the picture that lives there.
    #[test]
    fn opening_a_timed_file_stops_at_the_first_cluster() {
        let (file, ends) = timed(3, 4096);
        let mut r = WebmReader::open(Reach {
            inner: Cursor::new(file),
            high: 0,
        }, Limits::default())
        .expect("built file opens");
        assert_eq!(r.duration_ns, Some(5_000_000_000), "the length the file stated");
        assert_eq!(r.packets.len(), 1);
        assert!(!r.fully_indexed());
        assert!(
            r.reader.high < ends[0],
            "the second cluster begins at {} and open read to {}",
            ends[0],
            r.reader.high
        );
        assert_eq!(r.packets[0].size, 4096);
        assert_eq!(r.read_packet(0).unwrap(), vec![0x27; 4096]);
        // The rest arrives a cluster at a time, and the last cluster is reported
        // as the growth it is rather than as the end of the file.
        assert!(r.scan_more().unwrap());
        assert_eq!(r.packets.len(), 2);
        assert!(r.reader.high > ends[0]);
        assert!(r.scan_more().unwrap(), "the last cluster still indexes");
        assert_eq!(r.packets.len(), 3);
        assert!(!r.scan_more().unwrap(), "nothing is left behind it");
        assert!(r.fully_indexed());
        assert_eq!(r.duration_ns, Some(5_000_000_000), "a settled index keeps it");
    }
    /// A jump costs the distance it travels, not the length of the item: the
    /// walk stops at the first block past the point asked for, and the clusters
    /// behind it stay unread until something needs them. The fixture times one
    /// millisecond per cluster, so a target is reached by the cluster of that
    /// number.
    #[test]
    fn scanning_to_a_point_leaves_the_file_behind_it_unread() {
        let (file, ends) = timed(6, 4096);
        let mut r = WebmReader::open(
            Reach {
                inner: Cursor::new(file),
                high: 0,
            },
            Limits::default(),
        )
        .expect("built file opens");
        r.scan_until(2_000_000).unwrap();
        assert_eq!(r.packets.len(), 3, "the walk stopped at the target");
        assert!(!r.fully_indexed());
        assert!(
            r.reader.high < ends[3],
            "the fourth cluster begins at {} and the walk read to {}",
            ends[3],
            r.reader.high
        );
        // Asking past the last block is asking for the end, and gets it.
        r.scan_until(i64::MAX).unwrap();
        assert_eq!(r.packets.len(), 6);
        assert!(r.fully_indexed());
        assert!(
            r.reader.high > ends[4],
            "the walk reached the last cluster, having read to {}",
            r.reader.high
        );
    }
    /// A file that states no length has its tail as the only statement of it, so
    /// the opening walk still measures the whole item and a consumer that reads
    /// the duration off its own blocks gets the answer it always got.
    #[test]
    fn a_file_with_no_stated_length_is_indexed_whole_at_open() {
        let r = WebmReader::open(Cursor::new(fixture(false)), Limits::default())
            .expect("built file opens");
        assert_eq!(r.packets.len(), 2);
        assert!(r.fully_indexed());
    }
}
