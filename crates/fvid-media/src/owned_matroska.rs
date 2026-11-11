//! Owned streaming Matroska packet muxing. The shared writer is also used by
//! the frontend; callers discard partial output on error and own publication.
use fvid_control::{ProgressEvent, CancelFlag, ProgressHook};
pub use fvid_control::error::{Error, Result};
use fvid_control::error::invalid;
use crate::owned_file_tags::FileTags;
use std::io::{Read, Seek, SeekFrom, Write};
fn size(value: u64) -> Result<Vec<u8>> {
    for width in 1..=8 {
        if value < (1u64 << (7 * width)) - 1 {
            let encoded = (value | (1u64 << (7 * width))).to_be_bytes();
            return Ok(encoded[8 - width..].to_vec());
        }
    }
    Err(invalid("Matroska element exceeds size range"))
}
fn head(output: &mut impl Write, id: u32, length: u64) -> Result<()> {
    let bytes = id.to_be_bytes();
    let start = bytes
        .iter()
        .position(|&b| b != 0)
        .ok_or_else(|| invalid("zero EBML ID"))?;
    output.write_all(&bytes[start..])?;
    output.write_all(&size(length)?)?;
    Ok(())
}
fn element(id: u32, data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    head(&mut out, id, data.len() as u64)?;
    out.extend_from_slice(data);
    Ok(out)
}
fn uint(id: u32, value: u64) -> Result<Vec<u8>> {
    let b = value.to_be_bytes();
    element(id, &b[b.iter().position(|&v| v != 0).unwrap_or(7)..])
}
/// Per-block presentation controls. Invisible video blocks still establish
/// decoder references but do not extend the presentation duration.
#[derive(Clone, Copy, Debug, Default)]
pub struct PacketOptions {
    pub discard_padding_ns: i64,
    pub invisible: bool,
}

/// Streaming packet writer. Feed each track in decode order, interleaving tracks
/// at the call site. PTS can move backwards for B-frames; each packet has its own
/// nanosecond-clock cluster, avoiding signed 16-bit block timestamp overflow.
/// No payload copies or accumulated packet index. Discard output on any error.
pub struct PacketWriter<'a, W> {
    output: &'a mut W,
    segment_size: u64,
    duration_offset: u64,
    tracks_offset: u64,
    tracks_bytes: usize,
    written: Vec<bool>,
    delays: Vec<u64>,
    pcm: Vec<Option<(u32, u16)>>,
    end_ns: u64,
    event: ProgressEvent,
    failed: bool,
}
impl<'a, W: Write + Seek> PacketWriter<'a, W> {
    fn new_prepared(
        output: &'a mut W,
        entries: &[u8],
        file_elements: &[u8],
        delays: Vec<u64>,
        pcm: Vec<Option<(u32, u16)>>,
    ) -> Result<Self> {
        if delays.is_empty() || delays.len() > 126 || delays.len() != pcm.len() {
            return Err(invalid("Matroska requires consistent 1..=126 tracks"));
        }
        if output.stream_position()? != 0 {
            return Err(invalid("Matroska output must start at zero"));
        }
        let ebml = [
            uint(0x4286, 1)?,
            uint(0x42f7, 1)?,
            uint(0x42f2, 4)?,
            uint(0x42f3, 8)?,
            element(0x4282, b"matroska")?,
            uint(0x4287, 4)?,
            uint(0x4285, 2)?,
        ]
        .concat();
        output.write_all(&element(0x1a45dfa3, &ebml)?)?;
        output.write_all(&0x18538067u32.to_be_bytes())?;
        let segment_size = output.stream_position()?;
        output.write_all(&[1, 255, 255, 255, 255, 255, 255, 255])?;
        let info = element(
            0x1549a966,
            &[
                uint(0x2ad7b1, 1)?,
                element(0x4d80, b"FVid")?,
                element(0x5741, b"FVid")?,
                element(0x4489, &0f64.to_be_bytes())?,
            ]
            .concat(),
        )?;
        let duration_offset = output.stream_position()? + info.len() as u64 - 8;
        output.write_all(&info)?;
        let tracks = element(0x1654ae6b, entries)?;
        let tracks_offset = output.stream_position()?;
        let tracks_bytes = tracks.len();
        output.write_all(&tracks)?;
        output.write_all(file_elements)?;
        Ok(Self {
            output,
            segment_size,
            duration_offset,
            tracks_offset,
            tracks_bytes,
            written: vec![false; delays.len()],
            delays,
            pcm,
            end_ns: 0,
            event: ProgressEvent {
                packets: 0,
                payload_bytes: 0,
                done: false,
            },
            failed: false,
        })
    }
    // Only the private single-track ADTS adapter uses this fixed-size rewrite.
    // Payload timestamps remain in nanoseconds, independent of output rate.
    fn rewrite_adts_rate(
        &mut self,
        asc: &[u8],
        rate: u32,
        channels: u16,
        name: &str,
        language: &str,
    ) -> Result<()> {
        let spec = TrackSpec {
            encoding: Encoding::Aac {
                configuration: asc,
                sample_rate: rate,
                channels,
            },
            name,
            language,
        };
        let tracks = element(0x1654ae6b, &track_entry(&spec, 1, None)?)?;
        if self.written.len() != 1 || tracks.len() != self.tracks_bytes {
            return Err(invalid("ADTS track rewrite changed header size"));
        }
        let end = self.output.stream_position()?;
        self.output.seek(SeekFrom::Start(self.tracks_offset))?;
        self.output.write_all(&tracks)?;
        self.output.seek(SeekFrom::Start(end))?;
        Ok(())
    }
    pub fn event(&self) -> ProgressEvent {
        self.event
    }
    /// Flush pending container bytes without finalizing or publishing output.
    pub fn flush(&mut self) -> Result<()> {
        if self.failed {return Err(invalid("Matroska writer failed"));}
        if let Err(error) = self.output.flush() {self.failed=true;return Err(error.into());}
        Ok(())
    }

    /// `track` is zero-based. Durations/PTS are in nanoseconds. The sync flag
    /// states independent decodability, not whether PTS follows the last packet.
    pub fn write_packet(
        &mut self,
        track: usize,
        pts_ns: u64,
        duration_ns: u64,
        sync: bool,
        payload: &[u8],
    ) -> Result<()> {
        self.write_packet_with_padding(track, pts_ns, duration_ns, sync, payload, 0)
    }
    /// Positive padding discards the end; negative padding discards the start.
    /// Nanoseconds are independent of the segment timestamp scale.
    pub fn write_packet_with_padding(
        &mut self,
        track: usize,
        pts_ns: u64,
        duration_ns: u64,
        sync: bool,
        payload: &[u8],
        discard_padding_ns: i64,
    ) -> Result<()> {
        self.write_packet_with_options(
            track,
            pts_ns,
            duration_ns,
            sync,
            payload,
            PacketOptions {
                discard_padding_ns,
                invisible: false,
            },
        )
    }
    /// Write a packet with explicit decode-only or padding controls.
    pub fn write_packet_with_options(
        &mut self,
        track: usize,
        pts_ns: u64,
        duration_ns: u64,
        sync: bool,
        payload: &[u8],
        options: PacketOptions,
    ) -> Result<()> {
        if self.failed {
            return Err(invalid("Matroska writer failed"));
        }
        let result = self.packet(track, pts_ns, duration_ns, sync, payload, options);
        self.failed = result.is_err();
        result
    }
    fn packet(
        &mut self,
        track: usize,
        pts_ns: u64,
        duration_ns: u64,
        sync: bool,
        payload: &[u8],
        options: PacketOptions,
    ) -> Result<()> {
        let discard_padding_ns = options.discard_padding_ns;
        if track >= self.written.len() || duration_ns == 0 || payload.is_empty() {
            return Err(invalid("invalid Matroska packet"));
        }
        if let Some((rate, channels)) = self.pcm[track] {
            let frame_bytes = usize::from(channels) * 4;
            if !payload.len().is_multiple_of(frame_bytes) || options.invisible || !sync
                || payload.as_chunks::<4>().0.iter().any(|p|!f32::from_le_bytes(*p).is_finite()) {
                return Err(invalid("invalid Matroska float PCM packet"));
            }
            let frames = (payload.len() / frame_bytes) as u128;
            let span = frames * 1_000_000_000;
            if u128::from(duration_ns) < span / u128::from(rate)
                || u128::from(duration_ns) > span.div_ceil(u128::from(rate)) {
                return Err(invalid("Matroska PCM duration disagrees with sample count"));
            }
        }
        if discard_padding_ns.unsigned_abs() > duration_ns {
            return Err(invalid("Matroska padding exceeds packet duration"));
        }
        let end = pts_ns
            .checked_add(duration_ns)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or_else(|| invalid("Matroska timestamp overflow"))?;
        let packets = self
            .event
            .packets
            .checked_add(1)
            .ok_or_else(|| invalid("packet count overflow"))?;
        let bytes = self
            .event
            .payload_bytes
            .checked_add(payload.len() as u64)
            .ok_or_else(|| invalid("payload count overflow"))?;
        let timestamp = uint(0xe7, pts_ns)?;
        let duration = uint(0x9b, duration_ns)?;
        // ReferenceBlock=0 is the specified marker for dependent blocks whose
        // precise reference graph is unknown to the container-only writer.
        let reference = if sync {
            Vec::new()
        } else {
            element(0xfb, &[0])?
        };
        let padding = if discard_padding_ns == 0 {
            Vec::new()
        } else {
            element(0x75a2, &discard_padding_ns.to_be_bytes())?
        };
        let block = 4 + payload.len() as u64;
        let group = 1
            + size(block)?.len() as u64
            + block
            + duration.len() as u64
            + reference.len() as u64
            + padding.len() as u64;
        let cluster = timestamp.len() as u64 + 1 + size(group)?.len() as u64 + group;
        head(self.output, 0x1f43b675, cluster)?;
        self.output.write_all(&timestamp)?;
        head(self.output, 0xa0, group)?;
        head(self.output, 0xa1, block)?;
        self.output.write_all(&[
            0x80 | (track as u8 + 1),
            0,
            0,
            if options.invisible { 0x08 } else { 0 },
        ])?;
        self.output.write_all(payload)?;
        self.output.write_all(&duration)?;
        self.output.write_all(&reference)?;
        self.output.write_all(&padding)?;
        self.written[track] = true;
        let presented_end = end
            .saturating_sub(self.delays[track])
            .saturating_sub(discard_padding_ns.max(0) as u64);
        if !options.invisible {
            self.end_ns = self.end_ns.max(presented_end);
        }
        self.event.packets = packets;
        self.event.payload_bytes = bytes;
        Ok(())
    }
    /// Patch finite segment size and presentation duration. `done` remains false:
    /// the caller owns flushing, syncing and atomic publication.
    pub fn finish(self) -> Result<ProgressEvent> {
        self.finish_inner(false)
    }
    /// Finalize an intentionally bounded prefix, retaining declared empty tracks.
    /// Failed writes still prevent finalization; a prefix with no presented duration omits Duration.
    pub fn finish_prefix(self) -> Result<ProgressEvent> {
        self.finish_inner(true)
    }
    fn finish_inner(self, prefix: bool) -> Result<ProgressEvent> {
        if self.failed || (!prefix && self.written.iter().any(|v| !v)) {
            return Err(invalid("incomplete Matroska tracks"));
        }
        if !prefix && self.end_ns == 0 {
            return Err(invalid("Matroska has no presentation duration"));
        }
        let end = self.output.stream_position()?;
        let length = end - self.segment_size - 8;
        if length >= (1u64 << 56) - 1 {
            return Err(invalid("Matroska segment exceeds size range"));
        }
        self.output.seek(SeekFrom::Start(self.segment_size))?;
        self.output
            .write_all(&(length | (1u64 << 56)).to_be_bytes())?;
        if self.end_ns == 0 {
            // Replace the complete 11-byte Duration element with equally sized Void.
            self.output.seek(SeekFrom::Start(self.duration_offset - 3))?;
            self.output.write_all(&[0xec, 0x89, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
        } else {
            self.output.seek(SeekFrom::Start(self.duration_offset))?;
            self.output.write_all(&(self.end_ns as f64).to_be_bytes())?;
        }
        self.output.seek(SeekFrom::Start(end))?;
        Ok(self.event)
    }
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
/// File-wide tags and flat, unordered chapters; times are nanoseconds.
#[derive(Clone, Debug, Default)]
pub struct FileMetadata {
    pub tags: crate::owned_file_tags::FileTags,
    pub chapters: Vec<Chapter>,
}
fn file_metadata(value: &FileMetadata) -> Result<Vec<u8>> {
    let tags = &value.tags;
    let mut simple = Vec::new();
    for (name, value) in [
        ("TITLE", &tags.title),
        ("ARTIST", &tags.artist),
        ("ALBUM", &tags.album),
        ("GENRE", &tags.genre),
        ("DATE", &tags.date),
        ("COMMENT", &tags.comment),
        ("PART_NUMBER", &tags.track),
        ("ALBUM_ARTIST", &tags.album_artist),
        ("DISCNUMBER", &tags.disc),
        ("PUBLISHER", &tags.publisher),
        ("COPYRIGHT", &tags.copyright),
        ("DESCRIPTION", &tags.description),
        ("RATING", &tags.rating),
    ] {
        if value.contains('\0') {
            return Err(invalid("NUL in Matroska tag"));
        }
        if !value.is_empty() {
            simple.extend(element(
                0x67c8,
                &[
                    element(0x45a3, name.as_bytes())?,
                    element(0x4487, value.as_bytes())?,
                ]
                .concat(),
            )?);
        }
    }
    let mut out = Vec::new();
    if !simple.is_empty() {
        let mut tag = element(0x63c0, &[])?; // No target UID: the whole file.
        tag.extend(simple);
        out.extend(element(0x1254c367, &element(0x7373, &tag)?)?);
    }
    let mut atoms = Vec::new();
    for (i, chapter) in value.chapters.iter().enumerate() {
        if chapter.end_ns.is_some_and(|end| end < chapter.start_ns) || chapter.title.contains('\0')
        {
            return Err(invalid("invalid Matroska chapter"));
        }
        let mut atom = uint(0x73c4, i as u64 + 1)?;
        atom.extend(uint(0x91, chapter.start_ns)?);
        if let Some(end) = chapter.end_ns {
            atom.extend(uint(0x92, end)?);
        }
        if !chapter.title.is_empty() {
            atom.extend(element(
                0x80,
                &[
                    element(0x85, chapter.title.as_bytes())?,
                    element(0x437c, b"und")?,
                ]
                .concat(),
            )?);
        }
        atoms.extend(element(0xb6, &atom)?);
    }
    if !atoms.is_empty() {
        out.extend(element(0x1043a770, &element(0x45b9, &atoms)?)?);
    }
    Ok(out)
}

fn extra_text_tags(tags: &std::collections::BTreeMap<String, String>) -> Result<Vec<u8>> {
    text_tags_element(tags,None)
}
fn text_tags_element(
    tags: &std::collections::BTreeMap<String, String>,
    uid: Option<u64>,
) -> Result<Vec<u8>> {
    if tags.len() > 256 {
        return Err(invalid("Matroska text tag count exceeds limit"));
    }
    let mut entries = Vec::new();
    for (name, value) in tags {
        if name.is_empty()
            || name.len() > 128
            || value.len() > 1024
            || name.contains('\0')
            || value.contains('\0')
        {
            return Err(invalid("invalid Matroska text tag"));
        }
        if uid.is_none() && FileTags::supports_key(name) {
            continue;
        }
        let simple = element(
            0x67c8,
            &[
                element(0x45a3, name.as_bytes())?,
                element(0x4487, value.as_bytes())?,
            ]
            .concat(),
        )?;
        let targets = uid
            .map(|uid| element(0x63c0, &uint(0x63c5, uid)?))
            .transpose()?
            .unwrap_or_default();
        entries.extend(element(0x7373, &[targets, simple].concat())?);
    }
    if entries.is_empty() {
        Ok(Vec::new())
    } else {
        element(0x1254c367, &entries)
    }
}

pub use fvid_codecs::color::{
    hdr::{ColourDescription, HdrMetadata, MasteringDisplay},
    primaries::Chromaticity,
    tonemap::ContentLight,
};
/// Crop in coded pixels and exact pixel aspect, with optional colour/HDR tags.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoMetadata {
    pub crop: [u32; 4],
    pub pixel_aspect: (u32, u32),
    pub colour: Option<ColourDescription>,
    pub hdr: HdrMetadata,
}
impl Default for VideoMetadata {
    fn default() -> Self {
        Self {
            crop: [0; 4],
            pixel_aspect: (1, 1),
            colour: None,
            hdr: Default::default(),
        }
    }
}
pub fn video_element(
    width: u32,
    height: u32,
    metadata: Option<&VideoMetadata>,
    rotation: u16,
) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(invalid("empty Matroska video dimensions"));
    }
    let mut data = [
        uint(0xb0, u64::from(width))?,
        uint(0xba, u64::from(height))?,
    ]
    .concat();
    if let Some(m) = metadata {
        let [left, top, right, bottom] = m.crop;
        let w = width
            .checked_sub(left)
            .and_then(|w| w.checked_sub(right))
            .filter(|w| *w > 0);
        let h = height
            .checked_sub(top)
            .and_then(|h| h.checked_sub(bottom))
            .filter(|h| *h > 0);
        let (Some(w), Some(h)) = (w, h) else {
            return Err(invalid("Matroska crop leaves no picture"));
        };
        let (num, den) = m.pixel_aspect;
        if num == 0 || den == 0 {
            return Err(invalid("zero Matroska pixel aspect ratio"));
        }
        for (id, value) in [
            (0x54cc, left),
            (0x54bb, top),
            (0x54dd, right),
            (0x54aa, bottom),
        ] {
            if value != 0 {
                data.extend(uint(id, u64::from(value))?);
            }
        }
        // DisplayUnit=3 states the exact display aspect, avoiding integer pixel
        // rounding for ratios such as 16:15 or 64:45.
        if num != den {
            data.extend(uint(0x54b2, 3)?);
            data.extend(uint(0x54b0, u64::from(w) * u64::from(num))?);
            data.extend(uint(0x54ba, u64::from(h) * u64::from(den))?);
        }
        let mut colour = Vec::new();
        if let Some(c) = m.colour {
            for (id, value) in [
                (0x55b1, c.matrix),
                (0x55ba, c.transfer),
                (0x55bb, c.primaries),
            ] {
                colour.extend(uint(id, u64::from(value))?);
            }
            colour.extend(uint(0x55b9, if c.full_range { 2 } else { 1 })?);
        }
        for (id, value) in [
            (0x55bc, m.hdr.light.max_cll),
            (0x55bd, m.hdr.light.max_fall),
        ] {
            if !value.is_finite()
                || value < 0.0
                || value.fract() != 0.0
                || f64::from(value) >= u64::MAX as f64
            {
                return Err(invalid(
                    "Matroska content light must be a nonnegative integer",
                ));
            }
            if value > 0.0 {
                colour.extend(uint(id, value as u64)?);
            }
        }
        if let Some(master) = m.hdr.mastering {
            let mut values = Vec::new();
            for (id, value) in [
                (0x55d1, master.red.x),
                (0x55d2, master.red.y),
                (0x55d3, master.green.x),
                (0x55d4, master.green.y),
                (0x55d5, master.blue.x),
                (0x55d6, master.blue.y),
                (0x55d7, master.white.x),
                (0x55d8, master.white.y),
            ] {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(invalid("invalid Matroska mastering chromaticity"));
                }
                values.extend(element(id, &value.to_be_bytes())?);
            }
            if master.min_luminance > master.max_luminance {
                return Err(invalid("invalid Matroska mastering luminance range"));
            }
            for (id, value) in [
                (0x55d9, master.max_luminance),
                (0x55da, master.min_luminance),
            ] {
                if !value.is_finite() || value < 0.0 {
                    return Err(invalid("invalid Matroska mastering luminance"));
                }
                values.extend(element(id, &f64::from(value).to_be_bytes())?);
            }
            colour.extend(element(0x55d0, &values)?);
        }
        if !colour.is_empty() {
            data.extend(element(0x55b0, &colour)?);
        }
    }
    let roll: f64 = match rotation {
        0 => 0.0,
        90 => -90.0,
        180 => 180.0,
        270 => 90.0,
        _ => return Err(invalid("Matroska rotation must be 0, 90, 180, or 270")),
    };
    if rotation != 0 {
        data.extend(element(
            0x7670,
            &[uint(0x7671, 0)?, element(0x7675, &roll.to_be_bytes())?].concat(),
        )?);
    }
    element(0xe0, &data)
}

/// Descriptive track fields preserved independently of file-wide text tags.
#[derive(Clone, Debug)]
pub struct VideoTrackDescription {
    pub name: String,
    pub language: String,
    pub legacy_language: String,
    pub disposition: i32,
}

impl<'a, W: Write + Seek> PacketWriter<'a, W> {
    /// Open one FFV1 v1 video track. Codec configuration lives in its keyframes.
    /// This constructor declares coded geometry without colour/rotation tags.
    pub fn new_ffv1(output: &'a mut W, width: u32, height: u32) -> Result<Self> {
        Self::new_ffv1_with_metadata(output, width, height, None, 0, 0)
    }
    /// Validate colour/HDR, crop, exact aspect and rotation before output.
    pub fn new_ffv1_with_metadata(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
    ) -> Result<Self> {
        Self::new_ffv1_with_file_metadata(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            &FileMetadata::default(),
        )
    }
    /// Validate file tags/chapters and video metadata before emitting any bytes.
    pub fn new_ffv1_with_file_metadata(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
    ) -> Result<Self> {
        Self::new_ffv1_with_text_tags(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            file,
            &Default::default(),
        )
    }
    /// Preserve file-wide text tags outside the canonical player tag catalog.
    pub fn new_ffv1_with_text_tags(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
        text_tags: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self> {
        Self::new_ffv1_with_scoped_tags(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            file,
            text_tags,
            &Default::default(),
        )
    }
    /// Write file tags and tags scoped to the new video TrackUID (one).
    pub fn new_ffv1_with_scoped_tags(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
        text_tags: &std::collections::BTreeMap<String, String>,
        track_tags: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self> {
        Self::new_ffv1_described(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            file,
            text_tags,
            track_tags,
            None,
        )
    }
    pub fn new_ffv1_with_track_description(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
        text_tags: &std::collections::BTreeMap<String, String>,
        track_tags: &std::collections::BTreeMap<String, String>,
        description: &VideoTrackDescription,
    ) -> Result<Self> {
        Self::new_ffv1_described(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            file,
            text_tags,
            track_tags,
            Some(description),
        )
    }
    fn new_ffv1_described(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
        text_tags: &std::collections::BTreeMap<String, String>,
        track_tags: &std::collections::BTreeMap<String, String>,
        description: Option<&VideoTrackDescription>,
    ) -> Result<Self> {
        let mut file_elements = file_metadata(file)?;
        file_elements.extend(extra_text_tags(text_tags)?);
        file_elements.extend(text_tags_element(track_tags, Some(1))?);
        let geometry = video_element(width, height, metadata, rotation)?;
        let entries = element(
            0xae,
            &[
                uint(0xd7, 1)?,
                uint(0x73c5, 1)?,
                uint(0x83, 1)?,
                uint(0x9c, 0)?,
                element(0x86, b"V_FFV1")?,
                geometry,
                if default_duration_ns == 0 {
                    Vec::new()
                } else {
                    uint(0x23e383, default_duration_ns)?
                },
                track_description_elements(description)?,
            ]
            .concat(),
        )?;
        Self::new_prepared(output, &entries, &file_elements, vec![0], vec![None])
    }
}

fn track_description_elements(description: Option<&VideoTrackDescription>) -> Result<Vec<u8>> {
    let Some(description) = description else {
        return element(0x22b59c, b"und");
    };
    if description.name.len() > 1024
        || description.language.is_empty()
        || description.language.len() > 128
        || description.name.contains('\0')
        || description.language.contains('\0')
        || description.legacy_language.is_empty()
        || description.legacy_language.len() > 128
        || description.legacy_language.contains('\0')
        || description.disposition & !(1 | 4 | 8 | 64 | 128 | 256 | 131072) != 0
    {
        return Err(invalid("unsupported Matroska track description"));
    }
    let mut data = Vec::new();
    if !description.name.is_empty() {
        data.extend(element(0x536e, description.name.as_bytes())?);
    }
    data.extend(element(0x22b59c, description.legacy_language.as_bytes())?);
    data.extend(element(0x22b59d, description.language.as_bytes())?);
    for (id, bit) in [
        (0x88, 1),
        (0x55aa, 64),
        (0x55ab, 128),
        (0x55ac, 256),
        (0x55ad, 131072),
        (0x55ae, 4),
        (0x55af, 8),
    ] {
        data.extend(uint(id, u64::from(description.disposition & bit != 0))?);
    }
    Ok(data)
}

/// Encode and mux transformed Y4M frames into one FFV1/Matroska stream.
/// Preserve exact pixel aspect and colour range; no external codecs or packet index.
/// Output must begin at offset zero. `done` stays false until caller publication.
pub fn write_y4m_ffv1<W: Write + Seek>(
    source: impl std::io::BufRead,
    output: &mut W,
    transform: &fvid_media_info::DecodeTransform,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent)> {
    write_y4m_ffv1_controlled(source, output, transform, None, None)
}

/// Writer-level cancellation/progress. Completion is owned by file publication.
pub fn write_y4m_ffv1_controlled<W: Write + Seek>(
    source: impl std::io::BufRead,
    output: &mut W,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent)> {
    write_y4m_ffv1_policy(
        source,
        output,
        transform,
        cancel,
        progress,
        usize::MAX,
        None,
        false,
        &FileMetadata::default(),
        None,
    )
    .map(|(stats, event, _)| (stats, event))
}
// Check cancellation while buffering temporal filters, before they emit packets.
struct CancelReader<'a, R> {
    source: R,
    cancel: Option<&'a fvid_control::CancelFlag>,
}
impl<R: std::io::BufRead> std::io::Read for CancelReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        check(self.cancel).map_err(std::io::Error::other)?;
        self.source.read(bytes)
    }
}
impl<R: std::io::BufRead> std::io::BufRead for CancelReader<'_, R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        check(self.cancel).map_err(std::io::Error::other)?;
        self.source.fill_buf()
    }
    fn consume(&mut self, bytes: usize) {
        self.source.consume(bytes);
    }
}

fn write_y4m_ffv1_policy<W: Write + Seek>(
    source: impl std::io::BufRead,
    output: &mut W,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    rebase_interval: bool,
    file_metadata: &FileMetadata,
    track_options: Option<&fvid_control::CopyOptions>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent, u64)> {
    check(cancel)?;
    let mut description = VideoTrackDescription {
        name: String::new(),
        language: "und".into(),
        legacy_language: "und".into(),
        disposition: 1,
    };
    let mut track_tags = std::collections::BTreeMap::new();
    let mut text_tags = std::collections::BTreeMap::new();
    let mut meta_working_bytes = 0usize;
    if let Some(options) = track_options {
        for (key, value) in &options.metadata_set {
            text_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
            if !crate::owned_file_tags::FileTags::supports_key(key) && !value.is_empty() {
                text_tags.insert(key.to_ascii_uppercase(), value.clone());
            }
        }
        for key in &options.metadata_delete {
            text_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
        }
        for (_, key, value) in &options.stream_metadata_set {
            track_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
            if !crate::owned_ffv1_export::edit_track_description(&mut description, key, value)
                && !value.is_empty()
            {
                track_tags.insert(key.to_ascii_uppercase(), value.clone());
            }
        }
        for (_, key) in &options.stream_metadata_delete {
            track_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
            crate::owned_ffv1_export::edit_track_description(&mut description, key, "");
        }
        meta_working_bytes = crate::owned_lossless::estimate_matroska_metadata_working_bytes(
            file_metadata,
            &text_tags,
            &track_tags,
            Some(&description),
        );
        if let Some(limit) = options.max_controlled_bytes
            && meta_working_bytes > limit
        {
            return Err(invalid(
                "Matroska metadata working set exceeds controlled memory budget",
            ));
        }
    }
    let mut output = Some(output);
    let mut writer = None;
    let geometry = crate::owned_y4m_decode::has_geometry_or_pixel_transform(transform);
    let mut overlay_frame_len = 0usize;
    if let Some(spec) = &transform.overlay {
        if crate::owned_y4m_decode::supports(&spec.path) {
            let mut ov_reader = std::io::BufReader::new(std::fs::File::open(&spec.path)?);
            let mut ov_line = Vec::new();
            if crate::owned_y4m::line(&mut ov_reader, &mut ov_line)? {
                let ov_header = crate::owned_y4m::Header::parse(&ov_line)?;
                overlay_frame_len = ov_header.frame_len()?;
            }
        }
    }
    let (stats, consumed) = crate::owned_ffv1_encoder::encode_y4m_counted(
        CancelReader { source, cancel },
        transform,
        max_packets,
        |header, packet, pts, duration| {
            check(cancel)?;
            if let Some(options) = track_options {
                crate::owned_budget::check_rss_budget(options)?;
                let frame_len = header.frame_len()?;
                let working = crate::owned_lossless::estimate_transform_working_bytes(
                    transform,
                    frame_len,
                    header.depth(),
                    overlay_frame_len,
                )?;
                let live_frames = if geometry { 3usize } else { 2usize };
                let frame_working_bytes = frame_len
                    .saturating_mul(live_frames)
                    .max(working.saturating_add(if geometry {
                        frame_len.saturating_mul(2)
                    } else {
                        frame_len
                    }))
                    .saturating_add(meta_working_bytes);
                if options.max_controlled_bytes.is_some_and(|limit| {
                    frame_working_bytes.saturating_add(packet.len()) > limit
                }) {
                    return Err(invalid(
                        "encoded FFV1 packet exceeds controlled memory budget",
                    ));
                }
            }
            if packet.len() > max_packet_bytes {
                return Err(invalid("encoded FFV1 packet exceeds byte limit"));
            }
            if writer.is_none() {
                let width = u32::try_from(header.width)
                    .map_err(|_| invalid("Matroska width overflow"))?;
                let height = u32::try_from(header.height)
                    .map_err(|_| invalid("Matroska height overflow"))?;
                writer = Some(PacketWriter::new_ffv1_described(
                    output.take().unwrap(),
                    width,
                    height,
                    Some(&VideoMetadata {
                        pixel_aspect: header.pixel_aspect()?,
                        colour: Some(ColourDescription {
                            primaries: 0,
                            transfer: 0,
                            matrix: 6,
                            full_range: header.full_range()?,
                        }),
                        ..Default::default()
                    }),
                    0,
                    0,
                    file_metadata,
                    &text_tags,
                    &track_tags,
                    track_options
                        .filter(|options| {
                            !options.stream_metadata_set.is_empty()
                                || !options.stream_metadata_delete.is_empty()
                        })
                        .map(|_| &description),
                )?);
            }
            let writer = writer.as_mut().unwrap();
            writer.write_packet(
                0,
                if rebase_interval {
                    let origin = transform
                        .interval
                        .map_or(0, |(from, _)| from as u128 * 1000);
                    pts.checked_sub(
                        u64::try_from(origin)
                            .map_err(|_| invalid("interval timestamp overflow"))?,
                    )
                    .ok_or_else(|| invalid("interval timestamp underflow"))?
                } else {
                    pts
                },
                duration,
                true,
                packet,
            )?;
            if let Some(hook) = progress {
                hook.emit(writer.event());
            }
            check(cancel)
        },
    )?;
    let event = writer
        .ok_or_else(|| invalid("Matroska has no selected frames"))?
        .finish()?;
    Ok((stats, event, consumed))
}

fn check(cancel: Option<&fvid_control::CancelFlag>) -> Result<()> {
    if cancel.is_some_and(fvid_control::CancelFlag::is_cancelled) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Export owned progressive Y4M transforms to FFV1 in `.mkv`. No overwrite:
/// a same-directory temporary file is synced, then linked atomically. Failure
/// or cancellation removes the temporary; done is emitted only after linking.
/// This video-only API preserves Y4M aspect and colour range. The matrix follows
/// the native Y4M converter (BT.601); no primaries/transfer or HDR are inferred.
pub fn export_y4m_ffv1(
    source: &std::path::Path,
    destination: &std::path::Path,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent)> {
    export_y4m_ffv1_policy(
        source,
        destination,
        transform,
        cancel,
        progress,
        usize::MAX,
        None,
        false,
        &FileMetadata::default(),
        None,
    )
    .map(|(stats, event, _)| (stats, event))
}
pub(crate) fn export_y4m_ffv1_policy(
    source: &std::path::Path,
    destination: &std::path::Path,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    rebase_interval: bool,
    file_metadata: &FileMetadata,
    track_options: Option<&fvid_control::CopyOptions>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent, u64)> {
    let input = std::io::BufReader::new(std::fs::File::open(source)?);
    export_atomic(destination, cancel, progress, |file| {
        write_y4m_ffv1_policy(
            input, file, transform, cancel, progress, max_packet_bytes,
            max_packets, rebase_interval, file_metadata, track_options,
        )
    })
}

pub(crate) fn export_atomic(
    destination: &std::path::Path,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
    write: impl FnOnce(&mut std::fs::File) -> Result<(fvid_media_info::DecodeStats, ProgressEvent, u64)>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent, u64)> {
    check(cancel)?;
    if destination
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
    {
        return Err(invalid("owned FFV1 export requires .mkv"));
    }
    if destination.symlink_metadata().is_ok() {
        return Err(invalid("output already exists"));
    }
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut reserved = None;
    for _ in 0..100 {
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = directory.join(format!(
            ".fvid-matroska-{}-{serial}.tmp",
            std::process::id()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                reserved = Some((Temporary(path), file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let (temporary, mut file) =
        reserved.ok_or_else(|| invalid("cannot reserve Matroska output"))?;
    let (stats, mut event, consumed) = write(&mut file)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    check(cancel)?;
    std::fs::hard_link(&temporary.0, destination)?;
    drop(temporary);
    event.done = true;
    if let Some(hook) = progress {
        hook.emit(event);
    }
    Ok((stats, event, consumed))
}

use crate::owned_codec_config::{AacConfig, AvcConfig, HevcConfig};
use crate::owned_opus_packet as opus_packet;
/// Encoded packet storage is passed through without rewriting codec payloads.
#[derive(Clone, Copy)]
pub enum Encoding<'a> {
    /// Interleaved finite IEEE-754 float32 samples in little-endian order.
    PcmFloat32 { sample_rate: u32, channels: u16 },
    Opus { configuration: &'a [u8] },
    /// ASS header in CodecPrivate; each packet is one Matroska ASS event.
    Ass { configuration: &'a [u8] },
    /// FFV1 v1 packets contain their configuration in every keyframe.
    Ffv1V1 { width: u32, height: u32 },
    Avc {
        configuration: &'a [u8],
        width: u32,
        height: u32,
    },
    Hevc {
        configuration: &'a [u8],
        width: u32,
        height: u32,
    },
    Aac {
        configuration: &'a [u8],
        sample_rate: u32,
        channels: u16,
    },
}

/// Description of an encoded track on the caller's presentation timeline.
/// Container edit lists, rotation and gapless trimming must be mapped by a
/// higher-level remuxer. VideoMetadata carries colour/HDR and display geometry.
pub struct TrackSpec<'a> {
    pub encoding: Encoding<'a>,
    pub name: &'a str,
    pub language: &'a str,
}
/// Per-track container options. Rotation is clockwise in display space.
#[derive(Clone, Copy, Debug, Default)]
pub struct TrackOptions {
    /// Nominal frame/block duration in nanoseconds; zero omits the hint.
    /// Individual BlockDuration values remain authoritative for variable timing.
    pub default_duration_ns: u64,
    pub video: Option<VideoMetadata>,
    /// Rectangular rotations supported by the native player: 0, 90, 180, 270.
    pub rotation: u16,
    /// Priming to discard, in nanoseconds, subtracted from block timestamps.
    pub codec_delay_ns: u64,
}

fn track_entry(
    spec: &TrackSpec<'_>,
    number: u64,
    options: Option<&TrackOptions>,
) -> Result<Vec<u8>> {
    track_entry_with_usability(spec, number, options, true, true)
}
fn track_entry_with_usability(
    spec: &TrackSpec<'_>, number: u64, options: Option<&TrackOptions>, enabled: bool, default_track: bool,
) -> Result<Vec<u8>> {
    // Keep language metadata bounded before writing any container bytes. BCP47
    // tags can exceed the old 128-byte control-string limit (private use).
    if spec.language.len() > 16 * 1024 {
        return Err(invalid("Matroska language tag exceeds limit"));
    }
    let metadata = options.and_then(|o| o.video.as_ref());
    let rotation = options.map_or(0, |o| o.rotation);
    let delay = options.map_or(0, |o| o.codec_delay_ns);
    if delay > i64::MAX as u64 {
        return Err(invalid("Matroska codec delay overflow"));
    }
    let (id, config, kind, geometry) = match spec.encoding {
        Encoding::Ass { configuration } => {
            if metadata.is_some() || rotation != 0 || delay != 0 {
                return Err(invalid("video metadata or codec delay supplied for ASS track"));
            }
            let header = std::str::from_utf8(configuration).map_err(|_| invalid("ASS header is not UTF-8"))?;
            let has_styles = header.contains("[V4+ Styles]") || header.contains("[V4 Styles]");
            let has_script_type =
                header.contains("ScriptType: v4.00+") || header.contains("ScriptType: v4.00");
            if !header.contains("[Script Info]")
                || !has_styles
                || !has_script_type
                || header.contains('\0')
            {
                return Err(invalid("invalid ASS codec header"));
            }
            ("S_TEXT/ASS", configuration, 17, Vec::new())
        }
        Encoding::Ffv1V1 { width, height } => (
            "V_FFV1", &[][..], 1, video(width, height, metadata, rotation)?,
        ),
        Encoding::Avc {
            configuration,
            width,
            height,
        } => {
            AvcConfig::parse(configuration)?;
            (
                "V_MPEG4/ISO/AVC",
                configuration,
                1,
                video(width, height, metadata, rotation)?,
            )
        }
        Encoding::Hevc {
            configuration,
            width,
            height,
        } => {
            HevcConfig::parse(configuration)?;
            (
                "V_MPEGH/ISO/HEVC",
                configuration,
                1,
                video(width, height, metadata, rotation)?,
            )
        }
        Encoding::PcmFloat32 { sample_rate, channels } => {
            if sample_rate == 0 || !(1..=64).contains(&channels) {
                return Err(invalid("invalid Matroska PCM geometry"));
            }
            if metadata.is_some() || rotation != 0 {
                return Err(invalid("video metadata supplied for PCM track"));
            }
            ("A_PCM/FLOAT/IEEE", &[][..], 2, element(0xe1, &[
                element(0xb5, &f64::from(sample_rate).to_be_bytes())?,
                uint(0x9f, u64::from(channels))?, uint(0x6264, 32)?,
            ].concat())?)
        },
        Encoding::Opus { configuration } => {
            if metadata.is_some() || rotation!=0 {return Err(invalid("video metadata supplied for Opus track"));}
            let channels=opus_packet::header_channels(configuration)?;
            if delay!=opus_packet::pre_skip_ns(configuration)? {return Err(invalid("Opus codec delay differs from pre-skip"));}
            // OpusHead input_sample_rate is informational; Matroska uses the 48 kHz codec clock.
            let sample_rate=48000u32;
            ("A_OPUS",configuration,2,element(0xe1,&[
                element(0xb5,&f64::from(sample_rate).to_be_bytes())?,uint(0x9f,u64::from(channels))?
            ].concat())?)
        },
        Encoding::Aac {
            configuration,
            sample_rate,
            channels,
        } => {
            if metadata.is_some() || rotation != 0 {
                return Err(invalid("video metadata supplied for AAC track"));
            }
            let config = AudioSpecificConfig::parse(configuration)?;
            if sample_rate == 0
                || channels == 0
                || config.resolve_output_rate(sample_rate)? != sample_rate
                || u16::from(config.output_channels()) != channels
            {
                return Err(invalid("AAC Matroska geometry differs from configuration"));
            }
            (
                "A_AAC",
                configuration,
                2,
                element(
                    0xe1,
                    &[
                        element(0xb5, &f64::from(sample_rate).to_be_bytes())?,
                        uint(0x9f, u64::from(channels))?,
                    ]
                    .concat(),
                )?,
            )
        }
    };
    let mut data = [
        uint(0xd7, number)?,
        uint(0x73c5, number)?,
        uint(0x83, kind)?,
        uint(0x9c, 0)?,
        element(0x86, id.as_bytes())?,
        if config.is_empty() { Vec::new() } else { element(0x63a2, config)? },
        geometry,
    ]
    .concat();
    if !enabled { data.extend(uint(0xb9,0)?); }
    if !default_track { data.extend(uint(0x88,0)?); }
    if delay != 0 {
        data.extend(uint(0x56aa, delay)?);
    }
    if let Some(duration)=options.map(|o|o.default_duration_ns).filter(|&n|n!=0) {
        data.extend(uint(0x23e383,duration)?);
    }
    if matches!(spec.encoding,Encoding::Opus{..}) {
        if delay==0 {data.extend(uint(0x56aa,0)?);}
        data.extend(uint(0x56bb,80_000_000)?);
    }
    if !spec.name.is_empty() {
        data.extend(element(0x536e, spec.name.as_bytes())?);
    }
    // Keep legacy ISO639 codes compatible. Extended tags belong in the BCP47
    // element; older readers receive an explicit unknown-language fallback.
    let language = if spec.language.is_empty() { "und" } else { spec.language };
    if language.len() == 3 && language.bytes().all(|c| c.is_ascii_alphabetic()) {
        data.extend(element(0x22b59c, language.as_bytes())?);
    } else {
        data.extend(element(0x22b59c, b"und")?);
        data.extend(element(0x22b59d, language.as_bytes())?);
    }
    element(0xae, &data)
}
impl<'a, W: Write + Seek> PacketWriter<'a, W> {
    pub fn new(output: &'a mut W, tracks: &[TrackSpec<'_>]) -> Result<Self> {
        Self::new_with_video_metadata(output, tracks, &[])
    }
    /// Metadata is empty (none for every track), or one optional entry per track.
    /// Validate all track properties before writing any output bytes.
    pub fn new_with_video_metadata(
        output: &'a mut W,
        tracks: &[TrackSpec<'_>],
        metadata: &[Option<VideoMetadata>],
    ) -> Result<Self> {
        if tracks.is_empty() || tracks.len() > 126 {
            return Err(invalid("Matroska requires 1..=126 tracks"));
        }
        if !metadata.is_empty() && metadata.len() != tracks.len() {
            return Err(invalid("Matroska metadata track count mismatch"));
        }
        let options: Vec<_> = metadata
            .iter()
            .map(|video| TrackOptions {
                video: *video,
                rotation: 0,
                codec_delay_ns: 0,
                default_duration_ns: 0,
            })
            .collect();
        Self::new_with_options(output, tracks, &options)
    }
    /// Empty options select defaults, otherwise supply one entry per track.
    pub fn new_with_options(
        output: &'a mut W,
        tracks: &[TrackSpec<'_>],
        options: &[TrackOptions],
    ) -> Result<Self> {
        Self::new_with_metadata(output, tracks, options, &FileMetadata::default())
    }
    /// Supply file tags and chapters alongside track metadata. Metadata is
    /// validated before writing any output; packet payloads remain streamed.
    pub fn new_with_metadata(
        output: &'a mut W,
        tracks: &[TrackSpec<'_>],
        options: &[TrackOptions],
        metadata: &FileMetadata,
    ) -> Result<Self> {
        Self::new_with_text_metadata(output,tracks,options,metadata,&Default::default(),&Default::default())
    }
    /// Preserve file-wide and per-track text tags while replacing video coding.
    pub fn new_with_text_metadata(
        output:&'a mut W,tracks:&[TrackSpec<'_>],options:&[TrackOptions],metadata:&FileMetadata,
        text:&std::collections::BTreeMap<String,String>,
        scoped:&std::collections::BTreeMap<usize,std::collections::BTreeMap<String,String>>,
    )->Result<Self> {
        Self::new_with_track_usability(output,tracks,options,metadata,text,scoped,&[])
    }
    /// Internal transport bridge; empty usability retains legacy enabled defaults.
    pub(crate) fn new_with_track_usability(
        output:&'a mut W,tracks:&[TrackSpec<'_>],options:&[TrackOptions],metadata:&FileMetadata,
        text:&std::collections::BTreeMap<String,String>,
        scoped:&std::collections::BTreeMap<usize,std::collections::BTreeMap<String,String>>,
        enabled:&[bool],
    )->Result<Self> {
        Self::new_with_track_preferences(output,tracks,options,metadata,text,scoped,enabled,&[])
    }
    /// Private source-to-output transport for usability and default preference.
    pub fn new_with_track_preferences(
        output:&'a mut W,tracks:&[TrackSpec<'_>],options:&[TrackOptions],metadata:&FileMetadata,
        text:&std::collections::BTreeMap<String,String>,
        scoped:&std::collections::BTreeMap<usize,std::collections::BTreeMap<String,String>>,
        enabled:&[bool],defaults:&[bool],
    )->Result<Self> {
        if !defaults.is_empty() && defaults.len()!=tracks.len() {
            return Err(invalid("Matroska default track count mismatch"));
        }
        if !enabled.is_empty() && enabled.len()!=tracks.len() {
            return Err(invalid("Matroska usability track count mismatch"));
        }
        let mut file_elements = file_metadata(metadata)?;
        file_elements.extend(extra_text_tags(text)?);
        for (&track,tags) in scoped {
            if track>=tracks.len() {return Err(invalid("text metadata refers to missing track"));}
            file_elements.extend(text_tags_element(tags,Some(track as u64+1))?);
        }

        if tracks.is_empty() || tracks.len() > 126 {
            return Err(invalid("Matroska requires 1..=126 tracks"));
        }
        if !options.is_empty() && options.len() != tracks.len() {
            return Err(invalid("Matroska metadata track count mismatch"));
        }
        let mut entries = Vec::new();
        for (index, track) in tracks.iter().enumerate() {
            entries.extend(track_entry_with_usability(track, index as u64 + 1, options.get(index), enabled.get(index).copied().unwrap_or(true), defaults.get(index).copied().unwrap_or(true))?);
        }
        let delays = (0..tracks.len())
            .map(|i| options.get(i).map_or(0, |o| o.codec_delay_ns))
            .collect();
        let pcm = tracks.iter().map(|t| match t.encoding {
            Encoding::PcmFloat32 { sample_rate, channels } => Some((sample_rate, channels)),
            _ => None,
        }).collect();
        Self::new_prepared(output, &entries, &file_elements, delays, pcm)
    }
}
fn video(width: u32, height: u32, metadata: Option<&VideoMetadata>, rotation: u16) -> Result<Vec<u8>> {
    video_element(width, height, metadata, rotation)
}

use crate::owned_aac::adts;
/// Stream strict ADTS packets into Matroska. Per-packet clusters avoid signed
/// block timestamp overflow at a 1 ns clock. No cue table/index is accumulated.
/// Caller must discard partial output after error and publish only on success.
pub fn write_adts<R: Read, W: Write + Seek>(
    input: adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    write_adts_limited(input, output, cancel, progress, None)
}

/// Copy at most the requested total packet count; do not read past the limit.
pub fn write_adts_limited<R: Read, W: Write + Seek>(
    input: adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
) -> Result<ProgressEvent> {
    write_adts_with_options(input, output, cancel, progress, max_packets, None)
}

/// Copy at most the requested total packet count with optional Matroska metadata mutations.
pub fn write_adts_with_options<R: Read, W: Write + Seek>(
    mut input: adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
    options: Option<&fvid_control::CopyOptions>,
) -> Result<ProgressEvent> {
    let asc = input.audio_specific_config().to_vec();
    write_aac_packets(
        input.configuration(),
        &asc,
        || convert_adts_packet(input.next_packet()),
        output,
        cancel,
        progress,
        max_packets,
        options,
        16384,
    )
}

/// Append compatible ADTS segments to one Matroska track without transcoding.
pub fn concat_adts<R: Read, W: Write + Seek>(
    readers: Vec<adts::StreamReader<R>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    concat_adts_limited(readers, output, cancel, progress, None)
}

/// Apply one packet limit across all compatible segments.
pub fn concat_adts_limited<R: Read, W: Write + Seek>(
    readers: Vec<adts::StreamReader<R>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
) -> Result<ProgressEvent> {
    concat_adts_with_options(readers, output, cancel, progress, max_packets, None)
}

/// Apply one packet limit and optional Matroska metadata mutations across compatible segments.
pub fn concat_adts_with_options<R: Read, W: Write + Seek>(
    readers: Vec<adts::StreamReader<R>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
    options: Option<&fvid_control::CopyOptions>,
) -> Result<ProgressEvent> {
    let base_bytes = readers.len().saturating_mul(8192).saturating_add(16384);
    let mut sequence = adts::SequenceReader::new(readers)?;
    let asc = sequence.audio_specific_config().to_vec();
    write_aac_packets(
        sequence.configuration(),
        &asc,
        || convert_adts_packet(sequence.next_packet()),
        output,
        cancel,
        progress,
        max_packets,
        options,
        base_bytes,
    )
}

type AdtsMatroskaMetadata = (
    String,
    String,
    FileMetadata,
    std::collections::BTreeMap<String, String>,
    std::collections::BTreeMap<usize, std::collections::BTreeMap<String, String>>,
);

pub fn extract_adts_matroska_metadata(
    options: Option<&fvid_control::CopyOptions>,
) -> Result<AdtsMatroskaMetadata> {
    let mut name = String::new();
    let mut language = String::new();
    let mut metadata = FileMetadata::default();
    let mut text = std::collections::BTreeMap::new();
    let mut scoped = std::collections::BTreeMap::<
        usize,
        std::collections::BTreeMap<String, String>,
    >::new();
    if let Some(o) = options {
        if o.metadata_set.len() + o.metadata_delete.len() > 64 {
            return Err(invalid("at most 64 container metadata mutations"));
        }
        if o.stream_metadata_set.len() + o.stream_metadata_delete.len() > 64 {
            return Err(invalid("at most 64 stream metadata mutations"));
        }
        for (key, value) in &o.metadata_set {
            if key.is_empty()
                || key.len() > 128
                || value.len() > 1024
                || key.contains('\0')
                || value.contains('\0')
            {
                return Err(invalid("invalid Matroska container metadata"));
            }
            text.retain(|k: &String, _| !k.eq_ignore_ascii_case(key));
            if !metadata.tags.set(key, value) && !value.is_empty() {
                text.insert(key.to_ascii_uppercase(), value.clone());
            }
        }
        for key in &o.metadata_delete {
            if key.is_empty() || key.len() > 128 || key.contains('\0') {
                return Err(invalid("invalid Matroska container metadata key"));
            }
            metadata.tags.set(key, "");
            text.retain(|k, _| !k.eq_ignore_ascii_case(key));
        }
        for (index, key, value) in &o.stream_metadata_set {
            if *index != 0
                || key.is_empty()
                || key.len() > 128
                || value.len() > 1024
                || key.contains('\0')
                || value.contains('\0')
                || matches!(
                    key.to_ascii_lowercase().as_str(),
                    "rotate" | "stereo_mode" | "alpha_mode"
                )
                || (key.eq_ignore_ascii_case("language") && value.len() > 128)
            {
                return Err(invalid("invalid Matroska stream metadata"));
            }
            if key.eq_ignore_ascii_case("title") {
                name.clone_from(value);
            } else if key.eq_ignore_ascii_case("language") {
                language.clone_from(value);
            } else {
                let entries = scoped.entry(0).or_default();
                entries.retain(|k: &String, _| !k.eq_ignore_ascii_case(key));
                if !value.is_empty() {
                    if entries.len() >= 256 {
                        return Err(invalid("Matroska text tag count exceeds limit"));
                    }
                    entries.insert(key.to_ascii_uppercase(), value.clone());
                }
            }
        }
        for (index, key) in &o.stream_metadata_delete {
            if *index != 0
                || key.is_empty()
                || key.len() > 128
                || key.contains('\0')
                || matches!(
                    key.to_ascii_lowercase().as_str(),
                    "rotate" | "stereo_mode" | "alpha_mode"
                )
            {
                return Err(invalid("invalid Matroska stream metadata deletion"));
            }
            if key.eq_ignore_ascii_case("title") {
                name.clear();
            } else if key.eq_ignore_ascii_case("language") {
                language.clear();
            } else if let Some(entries) = scoped.get_mut(&0) {
                entries.retain(|k, _| !k.eq_ignore_ascii_case(key));
            }
        }
    }
    Ok((name, language, metadata, text, scoped))
}

pub fn estimate_adts_matroska_metadata_bytes(
    options: Option<&fvid_control::CopyOptions>,
) -> Result<usize> {
    let (name, language, metadata, text, scoped) = extract_adts_matroska_metadata(options)?;
    let mut raw_bytes = name.len().saturating_add(language.len());
    for (_, v) in metadata.tags.entries() {
        if !v.is_empty() {
            raw_bytes = raw_bytes.saturating_add(v.len()).saturating_add(32);
        }
    }
    for (k, v) in &text {
        raw_bytes = raw_bytes
            .saturating_add(k.len())
            .saturating_add(v.len())
            .saturating_add(32);
    }
    for entries in scoped.values() {
        for (k, v) in entries {
            raw_bytes = raw_bytes
                .saturating_add(k.len())
                .saturating_add(v.len())
                .saturating_add(32);
        }
    }
    Ok(raw_bytes.saturating_mul(8))
}

#[allow(clippy::too_many_arguments)]
fn write_aac_packets<W: Write + Seek>(
    config: adts::Header,
    asc: &[u8],
    mut next_packet: impl FnMut() -> Result<Option<Vec<u8>>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
    options: Option<&fvid_control::CopyOptions>,
    base_bytes: usize,
) -> Result<ProgressEvent> {
    let samples = u64::from(AacConfig::parse(asc)?.frame_samples);
    let meta_working_bytes = estimate_adts_matroska_metadata_bytes(options)?;
    let max_controlled_bytes = options.and_then(|o| o.max_controlled_bytes);
    if let Some(limit) = max_controlled_bytes
        && base_bytes.saturating_add(meta_working_bytes) > limit
    {
        return Err(invalid("controlled memory budget exceeded"));
    }
    let (name, language, metadata, text, scoped) = extract_adts_matroska_metadata(options)?;
    let spec = TrackSpec {
        encoding: Encoding::Aac {
            configuration: asc,
            sample_rate: config.sample_rate,
            channels: config.channels,
        },
        name: &name,
        language: &language,
    };
    let check = || {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            Err(invalid("media operation cancelled"))
        } else {
            Ok(())
        }
    };
    let time = |packet: u64| -> Result<u64> {
        u64::try_from(
            u128::from(packet) * u128::from(samples) * 1_000_000_000
                / u128::from(config.sample_rate),
        )
        .map_err(|_| invalid("Matroska timestamp overflow"))
    };
    check()?;
    let mut writer =
        PacketWriter::new_with_text_metadata(output, &[spec], &[], &metadata, &text, &scoped)?;
    let mut negotiated_rate = None;
    loop {
        check()?;
        if max_packets.is_some_and(|limit| writer.event().packets >= limit) {
            break;
        }
        let Some(packet) = next_packet()? else {
            break;
        };
        if negotiated_rate.is_none() {
            negotiated_rate = adts::probe_output_rate(&packet, asc).map_err(|e| invalid(&e.to_string()))?;
        }
        let index = writer.event().packets;
        let next = index
            .checked_add(1)
            .ok_or_else(|| invalid("packet count overflow"))?;
        if let Some(limit) = max_controlled_bytes
            && base_bytes
                .saturating_add((next as usize).saturating_mul(32))
                .saturating_add(meta_working_bytes)
                .saturating_add(packet.len())
                > limit
        {
            return Err(invalid("controlled memory budget exceeded"));
        }
        if index == 0
            && let Some(h) = progress
        {
            h.emit(writer.event());
        }
        let start = time(index)?;
        writer.write_packet(0, start, time(next)? - start, true, &packet)?;
        if let Some(h) = progress {
            h.emit(writer.event());
        }
    }
    if writer.event().packets == 0
        && let Some(h) = progress
    {
        h.emit(writer.event());
    }
    check()?;
    if let Some(rate) = negotiated_rate {
        writer.rewrite_adts_rate(asc, rate, config.channels, &name, &language)?;
    }
    let event = writer.finish()?;
    check()?;
    Ok(event)
}

// The shared muxer has different error types in the core and media crate.
fn convert_adts_packet<E, F>(
    packet: std::result::Result<Option<Vec<u8>>, E>,
) -> std::result::Result<Option<Vec<u8>>, F>
where
    F: From<E>,
{
    packet.map_err(F::from)
}
#[cfg(test)]
mod adts_mux_tests {
    use super::*;
    #[test]
    fn synthetic_adts_segments_keep_nanosecond_clock_and_payload() {
        let data = [0xff, 0xf1, 0x50, 0x80, 1, 0x1f, 0xfc, 0xe0];
        let sources = (0..2).map(|_| adts::StreamReader::open(data.as_slice()).unwrap()).collect();
        let mut output = std::io::Cursor::new(Vec::new());
        let event = concat_adts(sources, &mut output, None, None).unwrap();
        assert_eq!(event.packets, 2);
        assert!(!event.done);
        let mut reader = crate::owned_webm::WebmReader::open(output, Default::default()).unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[0].sample_rate, 44100);
        assert_eq!(reader.tracks[0].channels, 2);
        assert_eq!(reader.packets[1].pts_ns, 1024 * 1_000_000_000 / 44100);
        assert_eq!(reader.read_packet(0).unwrap(), [0xe0]);
        assert_eq!(reader.read_packet(1).unwrap(), [0xe0]);
    }
}

use crate::owned_mp4::Track as Mp4AacTrack;
use crate::owned_codec_config::aac_specific_config;
use crate::owned_aac::config::AudioSpecificConfig;
fn check_cancel(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(|c| c.is_cancelled()) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
fn sample_ns(samples: u64, rate: u32) -> Result<u64> {
    u64::try_from((u128::from(samples) * 1_000_000_000 + u128::from(rate) / 2) / u128::from(rate))
        .map_err(|_| invalid("AAC nanosecond timestamp overflow"))
}
pub(crate) struct AacPacketPlan {
    pub count: usize,
    pub delay: u64,
    padding: i64,
    frame: u64,
    pub rate: u32,
    pub trimmed_samples: u64,
}
impl AacPacketPlan {
    pub fn packet(&self, index: usize) -> Result<(u64, u64, i64)> {
        let begin = sample_ns(index as u64 * self.frame, self.rate)?;
        let end = sample_ns((index as u64 + 1) * self.frame, self.rate)?;
        Ok((
            begin,
            end - begin,
            if index + 1 == self.count {
                self.padding
            } else {
                0
            },
        ))
    }
}
pub(crate) fn aac_packet_plan(
    track: &Mp4AacTrack,
    movie_scale: u32,
    cancel: Option<&CancelFlag>,
) -> Result<AacPacketPlan> {
    aac_packet_plan_window(track, movie_scale, None, cancel)
}
pub(crate) fn aac_packet_plan_window(
    track: &Mp4AacTrack,
    movie_scale: u32,
    interval: Option<(i64, i64)>,
    cancel: Option<&CancelFlag>,
) -> Result<AacPacketPlan> {
    if track.handler != *b"soun" || track.codec != *b"mp4a" {
        return Err(invalid("MP4 track is not AAC"));
    }
    let asc = aac_specific_config(&track.configuration)?;
    let config = AudioSpecificConfig::parse(asc)?;
    let output_rate = config.resolve_output_rate(track.sample_rate)?;
    if track.timescale == 0
        || track.sample_rate != output_rate
        || track.channels != u16::from(config.output_channels())
    {
        return Err(invalid("MP4 AAC clock or geometry mismatch"));
    }
    let rate = u128::from(output_rate);
    let position = |ticks: u64| -> Result<u64> {
        let n = u128::from(ticks) * rate;
        let d = u128::from(track.timescale);
        if n % d != 0 {
            return Err(invalid("AAC time is not sample aligned"));
        }
        u64::try_from(n / d).map_err(|_| invalid("AAC sample position overflow"))
    };
    let media_end = position(track.duration)?;
    let edit_clock = track.resolved_edit_clock(movie_scale)?;
    let movie_scale = edit_clock.timescale;
    let (mut start, mut end) = match edit_clock.edits() {
        [] => (0, media_end),
        [edit] if edit.media_time >= 0 && movie_scale != 0 => {
            let start = position(edit.media_time as u64)?;
            let length =
                u64::try_from((u128::from(edit.duration) * rate).div_ceil(u128::from(movie_scale)))
                    .map_err(|_| invalid("AAC edit duration overflow"))?;
            (
                start,
                start
                    .checked_add(length)
                    .ok_or_else(|| invalid("AAC edit endpoint overflow"))?,
            )
        }
        _ => {
            return Err(invalid(
                "AAC packet remux requires one contiguous media edit",
            ));
        }
    };
    if start >= end || end > media_end {
        return Err(invalid("AAC edit exceeds media samples"));
    }
    let edited_samples = end - start;
    if let Some((from, to)) = interval {
        if from < 0 || from >= to {
            return Err(invalid("audio interval requires 0 <= from < to"));
        }
        let samples = |us: i64| -> Result<u64> {
            u64::try_from((us as u128 * rate).div_ceil(1_000_000))
                .map_err(|_| invalid("AAC interval sample overflow"))
        };
        let origin = start;
        start = origin
            .checked_add(samples(from)?)
            .ok_or_else(|| invalid("AAC interval endpoint overflow"))?;
        end = end.min(
            origin
                .checked_add(samples(to)?)
                .ok_or_else(|| invalid("AAC interval endpoint overflow"))?,
        );
        if start >= end {
            return Err(invalid("audio interval contains no samples"));
        }
    }
    let scaled = u64::from(config.core.frame_samples) * u64::from(output_rate);
    let core_rate = u64::from(config.core.sample_rate);
    if scaled % core_rate != 0 {
        return Err(invalid("AAC output frame clock is not sample aligned"));
    }
    let frame = scaled / core_rate;
    let count =
        usize::try_from(end.div_ceil(frame)).map_err(|_| invalid("AAC packet count overflow"))?;
    if count > track.samples.len() {
        return Err(invalid("AAC edit exceeds packet index"));
    }
    // Validate before producing a header. The AAC frame clock, rather than a
    // shortened final stts duration, determines the actual decoded frame size.
    for i in 0..track.samples.len() {
        check_cancel(cancel)?;
        let sample = track
            .samples
            .get(i)
            .ok_or_else(|| invalid("missing AAC sample"))?;
        let expected = (i as u64)
            .checked_mul(frame)
            .ok_or_else(|| invalid("AAC timeline overflow"))?;
        let duration = position(u64::from(sample.duration))?;
        if sample.pts < 0
            || sample.pts as u64 != sample.dts
            || position(sample.dts)? != expected
            || duration == 0
            || duration > frame
            || (i + 1 != track.samples.len() && duration != frame)
        {
            return Err(invalid(
                "AAC packet remux requires a contiguous frame clock",
            ));
        }
        if i + 1 == track.samples.len() && expected.checked_add(duration) != Some(media_end) {
            return Err(invalid("AAC media duration disagrees with packet timeline"));
        }
    }
    let delay = sample_ns(start, output_rate)?;
    let coded_end = (count as u64)
        .checked_mul(frame)
        .ok_or_else(|| invalid("AAC timeline overflow"))?;
    let padding = i64::try_from(sample_ns(coded_end - end, output_rate)?)
        .map_err(|_| invalid("AAC padding overflow"))?;
    Ok(AacPacketPlan {
        count,
        delay,
        padding,
        frame,
        rate: output_rate,
        trimmed_samples: edited_samples.saturating_sub(end.saturating_sub(start)),
    })
}
impl FileMetadata {
    pub fn from_mp4<R: Read + Seek>(input: &crate::owned_mp4::Mp4Reader<R>) -> Self {
        Self { tags: input.tags().clone(), chapters: input.chapters().iter().map(|c| Chapter { start_ns: c.start_ns, end_ns: None, title: c.title.clone() }).collect() }
    }
}

/// Copy one MP4 AAC track into Matroska, retaining decoder pre-roll and the
/// audible boundaries of a single media edit.
pub fn write_mp4_aac<R: Read + Seek, W: Write + Seek>(
    input: &mut crate::owned_mp4::Mp4Reader<R>,
    track_index: usize,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    write_mp4_aac_metadata(
        input,
        track_index,
        output,
        &FileMetadata::default(),
        cancel,
        progress,
    )
}

/// Remux a single-track AAC MP4 file, including all represented tags and chapters.
pub fn write_mp4_aac_file<R: Read + Seek, W: Write + Seek>(
    input: &mut crate::owned_mp4::Mp4Reader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    if input.tracks().len() != 1 || !input.refused().is_empty() {
        return Err(invalid("AAC Matroska remux requires a single audio track"));
    }
    let metadata = FileMetadata::from_mp4(input);
    write_mp4_aac_metadata(input, 0, output, &metadata, cancel, progress)
}

fn write_mp4_aac_metadata<R: Read + Seek, W: Write + Seek>(
    input: &mut crate::owned_mp4::Mp4Reader<R>,
    track_index: usize,
    output: &mut W,
    metadata: &FileMetadata,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let check = || {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            Err(invalid("media operation cancelled"))
        } else {
            Ok(())
        }
    };
    check()?;
    let track = input
        .tracks()
        .get(track_index)
        .ok_or_else(|| invalid("MP4 AAC track index out of range"))?
        .clone();
    let plan = aac_packet_plan(&track, input.movie_timescale(), cancel)?;
    let asc = aac_specific_config(&track.configuration)?;
    let mut text = std::collections::BTreeMap::<String, String>::new();
    let encoder = input.encoder().trim();
    if !encoder.is_empty() && encoder.len() <= 1024 && !encoder.contains('\0') {
        text.insert("ENCODER".into(), encoder.to_owned());
    }
    let mut writer = PacketWriter::new_with_text_metadata(
        output,
        &[TrackSpec {
            encoding: Encoding::Aac {
                configuration: asc,
                sample_rate: plan.rate,
                channels: track.channels,
            },
            name: &track.name,
            language: &track.language,
        }],
        &[TrackOptions {
            codec_delay_ns: plan.delay,
            ..Default::default()
        }],
        metadata,
        &text,
        &Default::default(),
    )?;
    if let Some(h) = progress {
        h.emit(writer.event());
    }
    let mut packet = Vec::new();
    for i in 0..plan.count {
        check()?;
        input.read_packet(track_index, i, &mut packet)?;
        let (begin, duration, padding) = plan.packet(i)?;
        writer.write_packet_with_padding(0, begin, duration, true, &packet, padding)?;
        if let Some(h) = progress {
            h.emit(writer.event());
        }
    }
    check()?;
    let event = writer.finish()?;
    check()?;
    Ok(event)
}


#[cfg(test)]
mod reverse_cancel_tests {
    #[test]
    fn cancellation_stops_reverse_input_before_any_replay_or_packet_write() {
        use std::io::{BufRead, Cursor, Read};
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        struct Input {
            cursor: Cursor<&'static [u8]>,
            flag: fvid_control::CancelFlag,
            at: Arc<AtomicU64>,
            cutoff: u64,
        }
        impl Read for Input {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                let n = self.cursor.read(bytes)?;
                self.at.store(self.cursor.position(), Ordering::Relaxed);
                if self.cursor.position() >= self.cutoff {
                    self.flag.cancel();
                }
                Ok(n)
            }
        }
        impl BufRead for Input {
            fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
                self.cursor.fill_buf()
            }
            fn consume(&mut self, n: usize) {
                self.cursor.consume(n);
            }
        }
        let bytes: &'static [u8] =
            include_bytes!("../../../tests/fixtures/playback-errors/reverse-six-frames.y4m");
        let cutoff = (bytes.iter().position(|&b| b == b'\n').unwrap() + 1 + 6 + 24) as u64;
        let flag = fvid_control::CancelFlag::new();
        let at = Arc::new(AtomicU64::new(0));
        let input = Input {
            cursor: Cursor::new(bytes),
            flag: flag.clone(),
            at: at.clone(),
            cutoff,
        };
        let mut output = Cursor::new(Vec::new());
        let transform = fvid_media_info::DecodeTransform {
            reverse: Some(String::new()),
            ..Default::default()
        };
        let error =
            super::write_y4m_ffv1_controlled(input, &mut output, &transform, Some(&flag), None)
                .unwrap_err();
        assert!(
            error.to_string().contains("media operation cancelled"),
            "{error}"
        );
        assert_eq!(at.load(Ordering::Relaxed), cutoff);
        assert!(output.into_inner().is_empty());
    }
}

#[cfg(test)]
mod usability_admission_tests {
    use super::*;
    #[test]
    fn default_count_mismatch_is_atomic_before_header_publication() {
        let mut output=std::io::Cursor::new(Vec::new());
        let tracks=[TrackSpec{encoding:Encoding::Ffv1V1{width:16,height:16},name:"",language:""}];
        let error=match PacketWriter::new_with_track_preferences(&mut output,&tracks,&[],&FileMetadata::default(),&Default::default(),&Default::default(),&[],&[false,true]) {
            Err(error)=>error,Ok(_)=>panic!("mismatched defaults admitted"),
        };
        assert_eq!(error.to_string(),"Matroska default track count mismatch");
        assert_eq!(output.position(),0);assert!(output.into_inner().is_empty());
    }

    #[test]
    fn usability_count_mismatch_is_atomic_before_header_publication() {
        let mut output=std::io::Cursor::new(Vec::new());
        let tracks=[TrackSpec{encoding:Encoding::Ffv1V1{width:16,height:16},name:"",language:""}];
        let error=match PacketWriter::new_with_track_usability(&mut output,&tracks,&[],&FileMetadata::default(),&Default::default(),&Default::default(),&[false,true]) {
            Err(error)=>error,Ok(_)=>panic!("mismatched usability admitted"),
        };
        assert_eq!(error.to_string(),"Matroska usability track count mismatch");
        assert_eq!(output.position(),0);assert!(output.into_inner().is_empty());
    }
}
