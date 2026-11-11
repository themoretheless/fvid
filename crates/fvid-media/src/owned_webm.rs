//! Owned bounded, incrementally indexed WebM/Matroska reader.
use crate::owned_matroska_time::{timestamp_ns as fvid_media_clock_timestamp, duration_ns as fvid_media_clock_duration};
use crate::owned_matroska_zlib as fvid_matroska_zlib;
use crate::owned_matroska_grid_clock::PacketGrid as MatroskaPacketGrid;
pub use fvid_control::error::{Error, Result};
use fvid_control::error::{invalid, unsupported};
use crate::owned_file_tags::FileTags;
pub use crate::owned_matroska::{
    Chapter, ColourDescription, ContentLight, HdrMetadata, MasteringDisplay,
};
use std::io::{Read, Seek, SeekFrom};
use crate::owned_opus_packet::{duration_ns as opus_lace_duration, header_channels as opus_lace_channels};
use crate::owned_aac::config::AudioSpecificConfig as WebmAacConfig;
use crate::owned_ebml as reader_ebml;
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
    /// Explicit Matroska Range; None for absent/unspecified/derived values.
    pub full_range: Option<bool>,
    /// `MasteringMetadata` and `MaxCLL`/`MaxFALL`, which a tone map needs and no
    /// bitstream of a container-only stream carries.
    pub hdr: HdrMetadata,
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
    /// File-wide text tags, retaining names beyond the player tag catalog.
    pub metadata: std::collections::BTreeMap<String, String>,
    /// Segment WritingApp, independent of a tag named ENCODER.
    pub writing_app: String,
    /// Track number to TrackUID, independent of element ordering.
    pub track_uids: std::collections::BTreeMap<u64,u64>,
    /// Original language declaration, retaining und and the container default.
    pub track_languages: std::collections::BTreeMap<u64,String>,
    pub track_legacy_languages: std::collections::BTreeMap<u64,String>,
    /// Default/forced/original/commentary/accessibility disposition bits.
    pub track_dispositions: std::collections::BTreeMap<u64,i32>,
    /// Text tags scoped to TrackUID (zero denotes all tracks).
    pub track_metadata: std::collections::BTreeMap<u64,std::collections::BTreeMap<String,String>>,
    /// False for unsupported scopes, nested or oversized tags requiring a richer exporter.
    pub metadata_complete: bool,
    disabled_tracks: std::collections::BTreeSet<u64>,
    limits: Limits,
    read_packet_bytes: usize,
    block_additions: std::collections::BTreeMap<usize,Vec<(u64,u64,usize)>>,
    /// Where the walk of the Segment's children stands. Blocks are indexed a
    /// cluster at a time, so the first picture does not wait for the whole file.
    at: u64,
    segment_end: u64,
    /// The `TimestampScale` the file's `Info` block states, or the one the
    /// specification gives it. Kept because a cluster's blocks arrive in it.
    scale: u64,
    track_scales: Vec<(u64, f64)>,
    scaled_audio_timings: Vec<BlockTiming>,
    frame_prefixes: Vec<(u64, Vec<u8>)>,
    zlib_tracks: Vec<u64>,
    zlib_sizes: Vec<usize>,
    /// Admitted compact mode tables, at most one per encountered Vorbis track.
    vorbis_lace_setups: std::collections::BTreeMap<u64, Option<VorbisLaceSetup>>,
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
use self::reader_ebml::{
    Element, bytes, element, end, fields, float, goto, lenient_text, read_chapters,
    read_tags_collect, sint, text, terminated_text_bytes, uint,
};

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
            0x7673 | 0x7674 | 0x7675 => {
                // RFC 8794 permits an empty float: these three Matroska
                // elements all declare zero as their schema default.
                let value = if end(field)? == field.data { 0.0 } else { float(r, field)? };
                match field.id {
                    0x7673 => yaw = value,
                    0x7674 => pitch = value,
                    _ => roll = value,
                }
            }
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
            0x55b9 => {
                track.full_range = match uint(r, f)? { 1 => Some(false), 2 => Some(true), _ => None };
                track.colour.full_range = track.full_range.unwrap_or(false);
            },
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
    /// Borrow the underlying reader.
    pub fn reader_ref(&self) -> &R {
        &self.reader
    }
    /// Consume the WebM reader and return the underlying reader.
    pub fn into_inner(self) -> R {
        self.reader
    }
    /// Precision of block timestamps; CodecDelay and DiscardPadding use ns.
    pub fn timestamp_scale_ns(&self) -> u64 { self.scale }
    /// The track's exact IEEE TrackTimestampScale; absent tracks return None.
    pub fn track_timestamp_scale(&self, number:u64)->Option<f64> {
        self.tracks.iter().any(|t|t.number==number).then(||self.track_scales.iter()
            .find(|&&(track,_)|track==number).map_or(1.0,|&(_,factor)|factor))
    }
    /// Original block grid for a scaled audio packet, including a lace offset.
    /// Compact records are retained only for non-unit audio tracks.
    pub fn packet_timestamp_grid(&self,index:usize)->Option<MatroskaPacketGrid> {
        let packet=self.packets.get(index)?;
        let at=self.scaled_audio_timings.partition_point(|t|t.range.end<=index);
        let timing=self.scaled_audio_timings.get(at).filter(|t|t.range.contains(&index))?;
        let factor=self.track_timestamp_scale(packet.track)?;
        let base=fvid_media_clock_timestamp(timing.cluster,timing.relative,self.scale,factor)?;
        Some(MatroskaPacketGrid{cluster:timing.cluster,relative:i32::from(timing.relative),
            offset_ns:i128::from(packet.pts_ns)-i128::from(base)})
    }
    /// Whether an indexed track is usable for automatic playback selection.
    /// Disabled tracks and their packets remain available for explicit inspection/copy.
    pub fn track_enabled(&self, number: u64) -> bool {
        self.tracks.iter().any(|t|t.number==number) && !self.disabled_tracks.contains(&number)
    }
    /// Prefer default tracks among the caller's usable/decodable candidates.
    /// Retain candidate order and fall back to all usable candidates when none
    /// states a default preference. Explicit index/packet access is unchanged.
    pub fn preferred_track_numbers(&self, candidates: &[u64]) -> Vec<u64> {
        let mut result = Vec::new();
        for &number in candidates {
            if self.track_enabled(number) && !result.contains(&number) { result.push(number); }
        }
        let is_default = |number|self.track_dispositions.get(&number).copied().unwrap_or(1)&1!=0;
        if result.iter().any(|&number|is_default(number)) { result.retain(|&number|is_default(number)); }
        result
    }
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
                doctype = Some(terminated_text_bytes(bytes(&mut reader, e, 16)?));
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
            metadata: Default::default(),
            writing_app: String::new(),
            track_uids: Default::default(),
            track_languages: Default::default(),
            track_legacy_languages: Default::default(),
            track_dispositions: Default::default(),
            track_metadata: Default::default(),
            metadata_complete: true,
            disabled_tracks: Default::default(),
            limits,
            read_packet_bytes: limits.packet_bytes,
            block_additions: Default::default(),
            segment_end: segment.end.unwrap_or(file_end),
            at: segment.data,
            scale: 1_000_000u64,
            track_scales: Vec::new(),
            scaled_audio_timings: Vec::new(),
            frame_prefixes: Vec::new(),
            zlib_tracks: Vec::new(),
            zlib_sizes: Vec::new(),
            vorbis_lace_setups: Default::default(),
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
        self.walk_navigation(one_cluster, false)
    }
    fn walk_navigation(&mut self, one_cluster: bool, seek_payloads: bool) -> Result<()> {
        let Self {
            reader,
            tracks,
            packets,
            limits,
            at,
            segment_end,
            scale,
            track_scales,
            scaled_audio_timings,
            frame_prefixes,
            zlib_tracks,
            zlib_sizes,
            vorbis_lace_setups,
            read_packet_bytes,
            block_additions,
            duration_ticks,
            info_seen,
            chapter_runs,
            block_title,
            tags,
            metadata,
            writing_app,
            track_uids,
            track_languages,
            track_legacy_languages,
            track_dispositions,
            track_metadata,
            metadata_complete,
            disabled_tracks,
            elements,
            scanned,
            tail_ns,
            duration_ns: _,
            chapters: _,
        } = self;
        let indexed = packets.len();
        let mut lace_groups = Vec::new();
        let mut block_timings = Vec::new();
        while *at < *segment_end {
            if seek_payloads { reader.seek(SeekFrom::Start(*at))?; } else { goto(&mut *reader, *at)?; }
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
                        } else if f.id == 0x5741 {
                            *writing_app = lenient_text(&mut *reader, f, 1024).unwrap_or_default();
                        } else if f.id == 0x2ad7b1 {
                            let proposed_scale = if end(f)? == f.data { 1_000_000 } else { uint(&mut *reader, f)? };
                            if proposed_scale == 0 {
                                return Err(invalid("zero WebM timestamp scale"));
                            }
                            if *info_seen && proposed_scale != *scale {
                                return Err(invalid("changing WebM timestamp scale"));
                            }
                            *scale = proposed_scale;
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
                            full_range: None,
                            hdr: HdrMetadata::default(),
                        };
                        // Two elements can state a language: the one in use now
                        // and, in files muxed before it existed, the older one in
                        // the same place. Whichever the newer spelling is wins, in
                        // either writing order, so a track that carries both does
                        // not read as two different languages.
                        let mut output_sample_rate = None;
                        let mut sampling_frequency_present = false;
                        let mut channels_present = false;
                        let mut track_scale = None;
                        let mut frame_prefix = None;
                        let mut uid = None;
                        let mut ietf = None;
                        let mut disposition = 1i32;
                        let mut enabled = None;
                        let mut default_seen = false;
                        let mut language_present = false;
                        let mut legacy_language = None;
                        let mut older = String::new();
                        let mut current = String::new();
                        for f in fields(&mut *reader, entry, &mut *elements, limits.elements)? {
                            match f.id {
                                0x23314f => {
                                    let value = if end(f)? == f.data { 1.0 } else { float(&mut *reader, f)? };
                                    if track_scale.is_some() || !value.is_finite() || value <= 0.0 {
                                        return Err(invalid("invalid Matroska TrackTimestampScale"));
                                    }
                                    track_scale = Some(value);
                                }
                                0xd7 => track.number = uint(&mut *reader, f)?,
                                0x73c5 => uid = Some(uint(&mut *reader,f)?),
                                0x83 => track.kind = uint(&mut *reader, f)?,
                                0x86 => {
                                    // Some muxers NUL-terminate the CodecID string.
                                    track.codec = String::from_utf8(terminated_text_bytes(bytes(&mut *reader, f, 128)?))
                                        .map_err(|_| invalid("invalid WebM codec ID"))?;
                                }
                                0x536e => {
                                    track.name = text(&mut *reader, f, 1024)?;
                                }
                                0x22b59c => {
                                    language_present = true;
                                    current = text(&mut *reader, f, 128)?;
                                    legacy_language = Some(current.clone());
                                }
                                0x22b59d => {
                                    language_present = true;
                                    // BCP47 private-use/variant sequences have no
                                    // 128-byte format limit. Match the bounded
                                    // writer allowance; legacy fields stay small.
                                    ietf = Some(text(&mut *reader, f, 16 * 1024)?);
                                }
                                // Keep the historical TagLanguage-in-TrackEntry alias lenient.
                                0x447a => {
                                    language_present = true;
                                    older = text(&mut *reader, f, 128)?;
                                }
                                0xb9 => {
                                    if enabled.is_some() { return Err(invalid("duplicate Matroska FlagEnabled")); }
                                    let value = if end(f)? == f.data { 1 } else { uint(&mut *reader,f)? };
                                    if value > 1 { return Err(invalid("invalid Matroska FlagEnabled")); }
                                    enabled = Some(value != 0);
                                }
                                0x88 | 0x55aa | 0x55ab | 0x55ac | 0x55ad | 0x55ae | 0x55af => {
                                    let bit = match f.id {
                                        0x88 => 1,
                                        0x55aa => 64,
                                        0x55ab => 128,
                                        0x55ac => 256,
                                        0x55ad => 131072,
                                        0x55ae => 4,
                                        _ => 8,
                                    };
                                    if f.id == 0x88 {
                                        if default_seen { return Err(invalid("duplicate Matroska FlagDefault")); }
                                        default_seen = true;
                                    }
                                    let value = if f.id==0x88 && end(f)?==f.data {1} else {uint(&mut *reader,f)?};
                                    if f.id==0x88 && value>1 { return Err(invalid("invalid Matroska FlagDefault")); }
                                    if value != 0 {
                                        disposition |= bit;
                                    } else {
                                        disposition &= !bit;
                                    }
                                }
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
                                            0xb5 | 0x78b5 => {
                                                // Empty OutputSamplingFrequency inherits SamplingFrequency,
                                                // including when the core frequency appears later.
                                                if v.id == 0x78b5 && end(v)? == v.data { continue; }
                                                let value = if v.id == 0xb5 && end(v)? == v.data {
                                                    8000.0
                                                } else { float(&mut *reader, v)? };
                                                if !value.is_finite() || value <= 0.0 {
                                                    return Err(invalid(
                                                        "invalid WebM sampling frequency",
                                                    ));
                                                }
                                                if v.id == 0x78b5 {
                                                    output_sample_rate = Some(value.round() as u64);
                                                } else {
                                                    sampling_frequency_present = true;
                                                    track.sample_rate = value.round() as u64;
                                                }
                                            }
                                            0x9f => {
                                                channels_present = true;
                                                track.channels = if end(v)? == v.data { 1 } else { uint(&mut *reader, v)? };
                                                if track.channels == 0 { return Err(invalid("invalid WebM channel count")); }
                                            },
                                            // `BitDepth` under `Audio`, in the two-byte
                                            // form the specification gives it; PCM has
                                            // nothing else to state its sample width in.
                                            0x6264 => track.bit_depth = uint(&mut *reader, v)?,
                                            _ => {}
                                        }
                                    }
                                }
                                0x6d80 => {
                                    if frame_prefix.is_some() {
                                        return Err(invalid("duplicate Matroska ContentEncodings"));
                                    }
                                    frame_prefix = Some(frame_compression(
                                        &mut *reader, f, &mut *elements, *limits,
                                    )?);
                                }
                                _ => {}
                            }
                        }
                        // A track whose only statement is the `und` writers use
                        // for "nothing was said" leaves the player knowing no
                        // more than one that states nothing at all.
                        let stated = ietf.unwrap_or(if current.is_empty() {older} else {current});
                        track_languages.insert(track.number,if language_present {stated.clone()} else {"eng".into()});
                        track_dispositions.insert(track.number,disposition);
                        track_legacy_languages.insert(track.number,legacy_language.unwrap_or_else(||"eng".into()));
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
                        if enabled == Some(false) {
                            disabled_tracks.insert(track.number);
                            // Transform writers cannot yet preserve this track attribute.
                            *metadata_complete = false;
                        }
                        if let Some(uid) = uid {track_uids.insert(track.number,uid);}
                        if track.kind == 2 {
                            if !sampling_frequency_present { track.sample_rate = 8000; }
                            if !channels_present { track.channels = 1; }
                        }
                        // OutputSamplingFrequency overrides the core frequency
                        // regardless of element order; timestamps stay in ns.
                        if let Some(rate) = output_sample_rate {
                            track.sample_rate = rate;
                        } else if track.codec == "A_AAC" {
                            // ASC may declare the decoded SBR clock while Audio
                            // gives only its core clock. Never guess a multiplier
                            // or replace an explicitly stated output clock.
                            if let Ok(config) = WebmAacConfig::parse(&track.codec_private) {
                                if config.sbr_present == Some(true)
                                    && (!sampling_frequency_present || track.sample_rate == 0
                                        || track.sample_rate == u64::from(config.core.sample_rate))
                                {
                                    track.sample_rate = u64::from(config.output_sample_rate());
                                }
                            }
                        }
                        if track.codec == "A_AAC" {
                            if let Ok(config) = WebmAacConfig::parse(&track.codec_private) {
                                // PS expands a mono core even when container
                                // channel metadata describes that core.
                                if config.ps_present == Some(true)
                                    && (!channels_present || track.channels == 0
                                        || track.channels == u64::from(config.core.channels))
                                {
                                    track.channels = u64::from(config.output_channels());
                                }
                            }
                        }
                        if let Some(value) = track_scale.filter(|&value| value != 1.0) {
                            track_scales.try_reserve(1)
                                .map_err(|_| invalid("Matroska track clock allocation failed"))?;
                            track_scales.push((track.number, value));
                        }
                        if let Some((prefix, zlib)) = frame_prefix {
                            if zlib {
                                zlib_tracks.try_reserve(1)
                                    .map_err(|_| invalid("Matroska compression index allocation failed"))?;
                                zlib_tracks.push(track.number);
                            } else if !prefix.is_empty() {
                                frame_prefixes.try_reserve(1)
                                    .map_err(|_| invalid("Matroska frame prefix allocation failed"))?;
                                frame_prefixes.push((track.number, prefix));
                            }
                        }
                        tracks.push(track);
                    }
                }
                0x1254c367 => {
                    let mut exceeded = false;
                    *metadata_complete &= read_tags_collect(
                        &mut *reader, e, &mut *elements, limits.elements, &mut *tags,
                        &mut |uid, name, value| {
                            let count = metadata.len() + track_metadata.values().map(|m|m.len()).sum::<usize>();
                            if count >= 256 {exceeded=true;return;}
                            let target = if let Some(uid) = uid {track_metadata.entry(uid).or_default()} else {&mut *metadata};
                            target.entry(name.to_owned()).or_insert_with(||value.to_owned());
                        },
                    );
                    if exceeded {
                        *metadata_complete = false;
                    }
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
                        if seek_payloads { reader.seek(SeekFrom::Start(pos))?; } else { goto(&mut *reader, pos)?; }
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
                                BlockKind::Simple,
                                &mut *packets,
                                *limits,
                                &mut lace_groups,
                                (&*tracks, &*track_scales, &*frame_prefixes),
                                &mut block_timings,
                            )?,
                            0xa0 => {
                                let fs = if seek_payloads {
                                    let limit=end(child)?; let mut at=child.data; let mut fs=Vec::new();
                                    while at<limit {
                                        reader.seek(SeekFrom::Start(at))?;
                                        let field=element(&mut *reader,limit,&mut *elements,limits.elements)?;
                                        at=end(field)?; fs.push(field);
                                    }
                                    fs
                                } else { fields(&mut *reader, child, &mut *elements, limits.elements)? };
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
                                let mut additions=Vec::new();
                                for field in fs.iter().filter(|f|f.id==0x75a1) {
                                    *metadata_complete=false; // Transform writers do not preserve additions yet.
                                    for more in fields(&mut *reader,*field,&mut *elements,limits.elements)? {
                                        if more.id!=0xa6 {continue;}
                                        let mut id=None;let mut payload=None;
                                        for part in fields(&mut *reader,more,&mut *elements,limits.elements)? {
                                            match part.id {
                                                0xee=>{if id.is_some(){return Err(invalid("duplicate Matroska BlockAddID"));}id=Some(uint(&mut *reader,part)?);},
                                                0xa5=>{if payload.is_some(){return Err(invalid("duplicate Matroska BlockAdditional"));}payload=Some(part);},
                                                _=>{},
                                            }
                                        }
                                        let id=id.unwrap_or(1);
                                        if id==0 || additions.iter().any(|&(old,_,_)|old==id){return Err(invalid("invalid or duplicate Matroska BlockAddID"));}
                                        let p=payload.ok_or_else(||invalid("Matroska BlockMore lacks BlockAdditional"))?;
                                        let size=usize::try_from(end(p)?-p.data).map_err(|_|invalid("Matroska BlockAdditional size overflow"))?;
                                        if size>limits.packet_bytes {return Err(invalid("Matroska BlockAdditional exceeds packet budget"));}
                                        additions.try_reserve(1).map_err(|_|invalid("cannot allocate Matroska additions index"))?;
                                        additions.push((id,p.data,size));
                                    }
                                }
                                let first = packets.len();
                                let mut seen_block = false;
                                for block in fs {
                                    if block.id == 0xa1 {
                                        if seen_block {
                                            return Err(invalid("duplicate Matroska Block in BlockGroup"));
                                        }
                                        seen_block = true;
                                        read_block(
                                            &mut *reader,
                                            block,
                                            timestamp,
                                            BlockKind::Group { keyframe: key },
                                            &mut *packets,
                                            *limits,
                                            &mut lace_groups,
                                            (&*tracks, &*track_scales, &*frame_prefixes),
                                            &mut block_timings,
                                        )?;
                                    }
                                }
                                if !seen_block {
                                    return Err(invalid("Matroska BlockGroup lacks Block"));
                                }
                                for packet in &mut packets[first..] {
                                    packet.duration_ns = duration;
                                }
                                if !additions.is_empty() {
                                    if packets.len()!=first+1 && additions.iter().any(|&(id,_,_)|id==1) && packets.get(first).is_some_and(|p|tracks.iter().any(|t|t.number==p.track && t.codec=="A_WAVPACK4")) {return Err(invalid("laced Matroska WavPack correction is not supported"));}
                                    if packets.len()>first {block_additions.insert(first,additions);}
                                }
                                if let Some(padding) = padding.filter(|_| packets.len() > first) {
                                    let position = if padding < 0 { first } else { packets.len()-1 };
                                    packets[position].discard_padding_ns = padding;
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
        if !zlib_tracks.is_empty() {
            zlib_sizes.try_reserve(packets.len().saturating_sub(zlib_sizes.len()))
                .map_err(|_| invalid("Matroska compression index allocation failed"))?;
            zlib_sizes.resize(packets.len(), 0);
        }
        let mut timing = block_timings.iter().peekable();
        for (index, p) in packets.iter_mut().enumerate().skip(indexed) {
            let prefix = frame_prefixes.iter().find(|&&(track, _)| track == p.track)
                .map_or(&[][..], |(_, prefix)| prefix.as_slice());
            if p.size.checked_add(prefix.len()).is_none_or(|size| size == 0 || size > limits.packet_bytes) {
                return Err(invalid("WebM packet exceeds budget or has empty lace frame"));
            }
            if zlib_tracks.contains(&p.track) {
                let mut restored = Vec::new();
                read_frame_payload(&mut *reader, p, prefix, true, None, *read_packet_bytes, &mut restored)?;
                if restored.is_empty() { return Err(invalid("empty Matroska zlib frame")); }
                zlib_sizes[index] = restored.len();
            }
            while timing.peek().is_some_and(|raw| raw.range.end <= index) { timing.next(); }
            let factor = track_scales.iter().find(|&&(track, _)| track == p.track)
                .map_or(1.0, |&(_, value)| value);
            p.pts_ns = if let Some(raw) = timing.peek().filter(|raw| raw.range.contains(&index)) {
                fvid_media_clock_timestamp(raw.cluster, raw.relative, *scale, factor)
                    .ok_or_else(|| invalid("WebM timestamp overflow"))?
            } else {
                i64::try_from(i128::from(p.pts_ns) * i128::from(*scale))
                    .map_err(|_| invalid("WebM timestamp overflow"))?
            };
            p.duration_ns = p.duration_ns.map(|ticks| (if factor == 1.0 {
                ticks.checked_mul(*scale)
            } else {
                fvid_media_clock_duration(ticks, *scale, factor)
            })
                .ok_or_else(|| invalid("WebM block duration overflow"))).transpose()?;
            *tail_ns = (*tail_ns).max(p.pts_ns);
            if !tracks.iter().any(|t: &Track| t.number == p.track) {
                return Err(invalid("WebM packet references missing track"));
            }
        }
        for group in lace_groups {
            let first = &packets[group.start];
            let origin = i128::from(first.pts_ns);
            let count = (group.end-group.start) as u128;
            let track = tracks.iter().find(|t| t.number == first.track).unwrap();
            let default = track.default_duration_ns;
            // BlockDuration describes the whole block, not an equal duration
            // for every lace. Supported Opus TOCs provide individual packet
            // durations even when a track also supplies DefaultDuration.
            // Preserve declared timing for unsupported mappings that previously
            // remained indexable with a default (e.g. for video-only playback).
            if track.codec == "A_OPUS" && (default == 0 || opus_lace_channels(&track.codec_private).is_ok()) {
                opus_lace_channels(&track.codec_private).map_err(|_| unsupported(
                    "Opus lace timing requires mono/stereo mapping family 0"))?;
                let mut elapsed = 0i128;
                let mut data = Vec::new();
                for (offset, packet) in packets[group.clone()].iter_mut().enumerate() {
                    let prefix = frame_prefixes.iter().find(|&&(track, _)| track == packet.track)
                        .map_or(&[][..], |(_, prefix)| prefix.as_slice());
                    let zlib = zlib_tracks.contains(&packet.track);
                    let expected = zlib.then(|| zlib_sizes[group.start + offset]);
                    read_frame_payload(&mut *reader, packet, prefix, zlib, expected, *read_packet_bytes, &mut data)?;
                    let duration = opus_lace_duration(&data).map_err(|e| invalid(&e.to_string()))?;
                    packet.pts_ns = i64::try_from(origin+elapsed).map_err(|_| invalid("WebM lace timestamp overflow"))?;
                    packet.duration_ns = Some(duration);
                    elapsed += i128::from(duration);
                    *tail_ns = (*tail_ns).max(packet.pts_ns);
                }
                spread_lace_padding(&mut packets[group.clone()])?;
                continue;
            }
            if track.codec == "A_VORBIS"
                && let Some((blocks, modes, rate)) = vorbis_lace_setups.entry(track.number)
                    .or_insert_with(|| vorbis_lace_setup(&track.codec_private)).as_ref()
            {
                let mut data = Vec::new();
                let mut block_size = |index: usize, packet: &Packet| -> Result<u64> {
                    let prefix = frame_prefixes.iter().find(|&&(number, _)| number == packet.track)
                        .map_or(&[][..], |(_, prefix)| prefix.as_slice());
                    let zlib = zlib_tracks.contains(&packet.track);
                    let expected = zlib.then(|| zlib_sizes[index]);
                    read_frame_payload(&mut *reader, packet, prefix, zlib, expected, *read_packet_bytes, &mut data)?;
                    let mode = fvid_codecs::vorbis::PacketMode::parse(
                        &mut fvid_codecs::vorbis::PacketBits::new(&data), modes,
                    ).map_err(|error| invalid(&error.to_string()))?;
                    Ok(1u64 << blocks[usize::from(modes[mode.mode].long)])
                };
                // Vorbis overlap belongs to both adjacent blocks, including
                // a predecessor outside this lace or an earlier scan pass.
                let previous = packets[..group.start].iter().rposition(|packet| packet.track == track.number);
                let mut previous = previous.map(|index| block_size(index, &packets[index])).transpose()?;
                let mut samples = 0u128;
                for (offset, packet) in packets[group.clone()].iter_mut().enumerate() {
                    let block = block_size(group.start + offset, packet)?;
                    let frames = previous.map_or(0, |previous| (previous + block) / 4);
                    previous = Some(block);
                    let start = samples * 1_000_000_000 / u128::from(*rate);
                    samples += u128::from(frames);
                    let finish = samples * 1_000_000_000 / u128::from(*rate);
                    packet.pts_ns = i64::try_from(origin + i128::try_from(start)
                        .map_err(|_| invalid("Vorbis lace timestamp overflow"))?)
                        .map_err(|_| invalid("Vorbis lace timestamp overflow"))?;
                    packet.duration_ns = Some(u64::try_from(finish - start)
                        .map_err(|_| invalid("Vorbis lace duration overflow"))?);
                    *tail_ns = (*tail_ns).max(packet.pts_ns);
                }
                spread_lace_padding(&mut packets[group])?;
                continue;
            }
            let total = first.duration_ns.map(u128::from)
                .or_else(|| (default != 0).then_some(u128::from(default)*count))
                .ok_or_else(|| unsupported("Matroska lacing requires declared frame or block duration"))?;
            for (index, packet) in packets[group.clone()].iter_mut().enumerate() {
                let start = total * index as u128 / count;
                let finish = total * (index+1) as u128 / count;
                packet.pts_ns = i64::try_from(origin + i128::try_from(start)
                    .map_err(|_| invalid("WebM lace timestamp overflow"))?)
                    .map_err(|_| invalid("WebM lace timestamp overflow"))?;
                packet.duration_ns = Some(u64::try_from(finish-start)
                    .map_err(|_| invalid("WebM lace duration overflow"))?);
                *tail_ns = (*tail_ns).max(packet.pts_ns);
            }
            spread_lace_padding(&mut packets[group])?;
        }
        // Preserve raw scaled audio coordinates after exact PTS conversion.
        // Video tracks and ordinary audio need no additional retained records.
        for timing in block_timings {
            let Some(packet)=packets.get(timing.range.start) else {continue;};
            if tracks.iter().any(|t|t.number==packet.track&&t.kind==2)
                && track_scales.iter().any(|&(number,factor)|number==packet.track&&factor!=1.0) {
                scaled_audio_timings.try_reserve(1).map_err(|_|invalid("scaled audio block clock allocation failed"))?;
                scaled_audio_timings.push(timing);
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
                let nanos = fvid_media_clock_duration(1, self.scale, ticks)
                    .filter(|&nanos| nanos != 0)
                    .ok_or_else(|| invalid("WebM Duration overflow"))?;
                Some(nanos)
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
    /// Complete metadata indexing while seeking past stored packet bodies.
    /// Codec-dependent lace timing and zlib admission retain their normal reads.
    pub fn scan_all_headers(&mut self) -> Result<()> {
        if self.scanned { return Ok(()); }
        self.walk_navigation(false, true)?;
        self.settle()
    }
    /// Whether every cluster the file holds has been indexed. A reader that
    /// measures the item from its own blocks can only do that once the tail has
    /// been met, so it asks before trusting a short answer.
    pub fn fully_indexed(&self) -> bool {
        self.scanned
    }
    /// Conservative retained index payload estimate. Vector/string capacities
    /// are counted, including duplicated canonical/raw metadata and chapter
    /// titles. Each BTreeMap record reserves 4 KiB for node slots/branches;
    /// reader-owned I/O buffers, packet payloads and allocator headers are excluded.
    /// Successful inspection allocates no heap memory. This does not impose a limit.
    pub fn estimated_index_payload_bytes(&self) -> Result<usize> {
        fn add(total: &mut usize, bytes: usize) -> Result<()> {
            *total = total
                .checked_add(bytes)
                .ok_or_else(|| invalid("WebM index memory estimate overflow"))?;
            Ok(())
        }
        fn vector<T>(total: &mut usize, values: &Vec<T>) -> Result<()> {
            add(
                total,
                values
                    .capacity()
                    .checked_mul(std::mem::size_of::<T>())
                    .ok_or_else(|| invalid("WebM index memory estimate overflow"))?,
            )
        }
        fn records(total: &mut usize, count: usize) -> Result<()> {
            add(
                total,
                count
                    .checked_mul(4096)
                    .ok_or_else(|| invalid("WebM index memory estimate overflow"))?,
            )
        }
        fn tags(total: &mut usize, tags: &std::collections::BTreeMap<String, String>) -> Result<()> {
            records(total, tags.len())?;
            for (key, value) in tags {
                add(total, key.capacity())?;
                add(total, value.capacity())?;
            }
            Ok(())
        }
        let mut total = 0;
        vector(&mut total, &self.packets)?;
        records(&mut total,self.block_additions.len())?;
        for additions in self.block_additions.values(){vector(&mut total,additions)?;}
        vector(&mut total, &self.track_scales)?;
        vector(&mut total, &self.scaled_audio_timings)?;
        vector(&mut total, &self.frame_prefixes)?;
        vector(&mut total, &self.zlib_tracks)?;
        vector(&mut total, &self.zlib_sizes)?;
        for (_, prefix) in &self.frame_prefixes { add(&mut total, prefix.capacity())?; }
        vector(&mut total, &self.tracks)?;
        for track in &self.tracks {
            for text in [&track.codec, &track.name, &track.language] {
                add(&mut total, text.capacity())?;
            }
            vector(&mut total, &track.codec_private)?;
        }
        vector(&mut total, &self.chapters)?;
        for chapter in &self.chapters {
            add(&mut total, chapter.title.capacity())?;
        }
        vector(&mut total, &self.chapter_runs)?;
        for (_, _, title) in &self.chapter_runs {
            add(&mut total, title.capacity())?;
        }
        for text in [
            &self.writing_app,
            &self.block_title,
            &self.tags.title,
            &self.tags.artist,
            &self.tags.album,
            &self.tags.genre,
            &self.tags.date,
            &self.tags.comment,
            &self.tags.track,
            &self.tags.album_artist,
            &self.tags.disc,
            &self.tags.publisher,
            &self.tags.copyright,
            &self.tags.description,
            &self.tags.rating,
        ] {
            add(&mut total, text.capacity())?;
        }
        tags(&mut total, &self.metadata)?;
        records(&mut total, self.track_uids.len())?;
        records(&mut total, self.track_dispositions.len())?;
        records(&mut total, self.disabled_tracks.len())?;
        for languages in [&self.track_languages, &self.track_legacy_languages] {
            records(&mut total, languages.len())?;
            for language in languages.values() {
                add(&mut total, language.capacity())?;
            }
        }
        records(&mut total, self.track_metadata.len())?;
        for scoped in self.track_metadata.values() {
            tags(&mut total, scoped)?;
        }
        Ok(total)
    }
    /// Check retained-index admission initially and after each indexed cluster.
    /// The callback runs before a caller constructs its decoder or emits PCM.
    pub fn scan_all_with_admission(
        &mut self,
        mut admission: impl FnMut(&Self) -> Result<()>,
    ) -> Result<()> {
        admission(self)?;
        if self.scanned {
            return Ok(());
        }
        while !self.scanned {
            self.walk(true)?;
            admission(self)?;
        }
        self.settle()?;
        admission(self)
    }
    /// Retained packet index payload including spare capacity, excluding the
    /// reader, tracks, metadata, allocator headers and packet data buffers.
    pub fn packet_index_payload_bytes(&self) -> Result<usize> {
        self.packets
            .capacity()
            .checked_mul(std::mem::size_of::<Packet>())
            .ok_or_else(|| invalid("WebM packet index allocation overflow"))
    }
    /// Tighten encoded payload reads without treating codec metadata as packets.
    pub fn restrict_packet_bytes(&mut self, maximum: usize) {
        self.read_packet_bytes = self.read_packet_bytes.min(maximum);
    }

    /// Whether a given Matroska track uses frame-level zlib compression.
    pub fn zlib_tracks_contains(&self, track_number: u64) -> bool {
        self.zlib_tracks.contains(&track_number)
    }

    /// Codec payload bytes after container frame decompression. Packet
    /// offsets and sizes still describe the stored source-file span.
    pub fn packet_payload_size(&self, index: usize) -> Result<usize> {
        let packet = self.packets.get(index).ok_or_else(|| invalid("WebM packet index out of bounds"))?;
        if self.zlib_tracks.contains(&packet.track) {
            return self.zlib_sizes.get(index).copied().filter(|&size| size != 0)
                .ok_or_else(|| invalid("missing Matroska decompressed packet size"));
        }
        let prefix = self.frame_prefixes.iter().find(|&&(track, _)| track == packet.track)
            .map_or(0, |(_, prefix)| prefix.len());
        packet.size.checked_add(prefix).ok_or_else(|| invalid("WebM packet size overflow"))
    }

    /// Temporary packet-read storage beyond the restored payload: compressed
    /// input, fixed inflater state and one byte to detect output overflow.
    pub fn packet_read_overhead(&self, index: usize) -> Result<usize> {
        let packet = self.packets.get(index).ok_or_else(|| invalid("WebM packet index out of bounds"))?;
        if !self.zlib_tracks.contains(&packet.track) { return Ok(0); }
        packet.size.checked_add(fvid_matroska_zlib::state_bytes())
            .and_then(|size| size.checked_add(1))
            .ok_or_else(|| invalid("Matroska packet scratch size overflow"))
    }

    /// Read a bounded restored-payload range. Uncompressed frames seek directly
    /// past the body; header stripping is restored before applying the range.
    /// Zlib needs the existing bounded full-packet inflater.
    pub fn read_packet_range(&mut self, index: usize, start: usize, out: &mut [u8]) -> Result<usize> {
        let size = self.packet_payload_size(index)?;
        if size > self.read_packet_bytes { return Err(invalid("WebM packet exceeds budget")); }
        if start > size { return Err(invalid("WebM packet range starts beyond payload")); }
        let count = out.len().min(size - start);
        if count == 0 { return Ok(0); }
        let packet = &self.packets[index];
        if self.zlib_tracks.contains(&packet.track) {
            let data = self.read_packet(index)?;
            out[..count].copy_from_slice(&data[start..start+count]);
            return Ok(count);
        }
        let prefix = self.frame_prefixes.iter().find(|&&(track, _)| track == packet.track)
            .map_or(&[][..], |(_, prefix)| prefix.as_slice());
        let copied = if start < prefix.len() {
            let n = count.min(prefix.len()-start);
            out[..n].copy_from_slice(&prefix[start..start+n]); n
        } else { 0 };
        if copied < count {
            let offset = packet.offset.checked_add((start+copied-prefix.len()) as u64)
                .ok_or_else(|| invalid("WebM packet range offset overflow"))?;
            self.reader.seek(SeekFrom::Start(offset))?;
            self.reader.read_exact(&mut out[copied..count])?;
        }
        Ok(count)
    }

    pub fn read_packet_into(&mut self, index: usize, out: &mut Vec<u8>) -> Result<()> {
        let restored_size = self.packet_payload_size(index)?;
        if restored_size > self.read_packet_bytes { return Err(invalid("WebM packet exceeds budget")); }
        let p = self
            .packets
            .get(index)
            .ok_or_else(|| invalid("WebM packet index out of bounds"))?;
        let prefix = self.frame_prefixes.iter().find(|&&(track, _)| track == p.track)
            .map_or(&[][..], |(_, prefix)| prefix.as_slice());
        let zlib = self.zlib_tracks.contains(&p.track);
        read_frame_payload(&mut self.reader, p, prefix, zlib, zlib.then_some(restored_size), self.read_packet_bytes, out)
    }

    /// Read codec supplemental bytes associated with a BlockGroup packet.
    pub fn read_block_additional(&mut self,index:usize,id:u64)->Result<Option<Vec<u8>>>{
        if index>=self.packets.len(){return Err(invalid("WebM packet index out of bounds"));}
        let Some(&(_,offset,size))=self.block_additions.get(&index).and_then(|a|a.iter().find(|&&(key,_,_)|key==id)) else {return Ok(None);};
        let main=self.packet_payload_size(index)?;
        if size.checked_add(main).is_none_or(|n|n>self.read_packet_bytes){return Err(invalid("Matroska packet and additions exceed budget"));}
        let mut data=Vec::new();data.try_reserve_exact(size).map_err(|_|invalid("cannot allocate Matroska BlockAdditional"))?;data.resize(size,0);
        goto(&mut self.reader,offset)?;self.reader.read_exact(&mut data)?;Ok(Some(data))
    }

    pub fn read_packet(&mut self, index: usize) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        self.read_packet_into(index, &mut data)?;
        Ok(data)
    }
}
// Lace VINTs use every data value, including the all-ones bit pattern.
fn lace_vint<R: Read + Seek>(r: &mut R, limit: u64) -> Result<(u64, u32)> {
    let mut byte = [0];
    if r.stream_position()? >= limit {
        return Err(invalid("truncated Matroska lace header"));
    }
    r.read_exact(&mut byte)?;
    if byte[0] == 0 {
        return Err(invalid("zero Matroska lace integer"));
    }
    let width = byte[0].leading_zeros() + 1;
    let mut value = u64::from(byte[0] & ((1u8 << (8 - width)) - 1));
    for _ in 1..width {
        if r.stream_position()? >= limit {
            return Err(invalid("truncated Matroska lace header"));
        }
        r.read_exact(&mut byte)?;
        value = (value << 8) | u64::from(byte[0]);
    }
    Ok((value, width))
}
// RFC 9559 5.1.4.1.31: one frame-only zlib or header-stripping transform.
// Other compression algorithms, codec-private transforms and encryption retain
// their explicit refusal instead of publishing undecoded content as packets.
fn frame_compression<R: Read + Seek>(
    r: &mut R, e: Element, count: &mut usize, limits: Limits,
) -> Result<(Vec<u8>, bool)> {
    let unavailable = || unsupported("encoded/encrypted WebM tracks not supported");
    let mut encodings = fields(r, e, count, limits.elements)?.into_iter().filter(|e| e.id == 0x6240);
    let encoding = encodings.next().ok_or_else(unavailable)?;
    if encodings.next().is_some() { return Err(unavailable()); }
    let mut scope = 1;
    let mut kind = 0;
    let mut compression = None;
    let mut seen = 0u8;
    for field in fields(r, encoding, count, limits.elements)? {
        let bit = match field.id { 0x5031 => 1, 0x5032 => 2, 0x5033 => 4, 0x5034 => 8, _ => 0 };
        if seen & bit != 0 { return Err(invalid("duplicate Matroska ContentEncoding field")); }
        seen |= bit;
        match field.id {
            0x5031 => { uint(r, field)?; } // A lone transform can have any order.
            0x5032 => scope = if end(field)? == field.data { 1 } else { uint(r, field)? },
            0x5033 => kind = uint(r, field)?,
            0x5034 => compression = Some(field),
            _ => {} // ContentEncryption must be ignored for compression type 0.
        }
    }
    if scope != 1 || kind != 0 { return Err(unavailable()); }
    let compression = compression.ok_or_else(unavailable)?;
    let mut algorithm = 0;
    let mut settings = None;
    let mut seen_algorithm = false;
    for field in fields(r, compression, count, limits.elements)? {
        match field.id {
            0x4254 => {
                if seen_algorithm { return Err(invalid("duplicate Matroska ContentCompAlgo")); }
                seen_algorithm = true;
                algorithm = uint(r, field)?;
            }
            0x4255 => {
                if settings.is_some() { return Err(invalid("duplicate Matroska ContentCompSettings")); }
                settings = Some(field);
            }
            _ => {}
        }
    }
    if algorithm == 0 {
        if settings.is_some_and(|field| field.end != Some(field.data)) { return Err(unavailable()); }
        return Ok((Vec::new(), true));
    }
    if algorithm != 3 { return Err(unavailable()); }
    let prefix = settings.map(|field| bytes(r, field, limits.packet_bytes.min(256 << 10))).transpose()?;
    Ok((prefix.unwrap_or_default(), false))
}

fn read_frame_payload<R: Read + Seek>(
    reader: &mut R, packet: &Packet, prefix: &[u8], zlib: bool,
    expected: Option<usize>, maximum: usize, out: &mut Vec<u8>,
) -> Result<()> {
    let size = packet.size.checked_add(prefix.len())
        .filter(|&size| size <= maximum)
        .ok_or_else(|| invalid("WebM packet exceeds budget"))?;
    goto(reader, packet.offset)?;
    if zlib {
        let mut encoded = Vec::new();
        encoded.try_reserve_exact(packet.size)
            .map_err(|_| invalid("Matroska compressed buffer allocation failed"))?;
        encoded.resize(packet.size, 0);
        reader.read_exact(&mut encoded)?;
        // Release a previous packet buffer before allocating its replacement.
        // Admission checks above still precede any change to caller output.
        *out = Vec::new();
        let restored = fvid_matroska_zlib::decode(&encoded, expected.unwrap_or(maximum).min(maximum))
            .map_err(invalid)?;
        if expected.is_some_and(|expected| restored.len() != expected) {
            return Err(invalid("Matroska decompressed packet size changed"));
        }
        *out = restored;
        return Ok(());
    }
    out.clear();
    if out.capacity() < size {
        out.try_reserve_exact(size)
            .map_err(|_| invalid("WebM packet buffer allocation failed"))?;
    }
    out.extend_from_slice(prefix);
    out.resize(size, 0);
    reader.read_exact(&mut out[prefix.len()..])?;
    Ok(())
}

enum BlockKind {
    Simple,
    Group { keyframe: bool },
}

struct BlockTiming {
    range: std::ops::Range<usize>,
    cluster: u64,
    relative: i16,
}

/// Only admitted setup enables codec-derived lace timing. Other headers keep
/// their prior declared-timing indexability for non-audio consumers.
type VorbisLaceSetup = ([u8; 2], Vec<fvid_codecs::vorbis::Mode>, u32);
fn vorbis_lace_setup(private: &[u8]) -> Option<VorbisLaceSetup> {
    let (ident, setup) = fvid_codecs::vorbis_audio::configuration_headers(private).ok()?;
    let ident = fvid_codecs::vorbis::Identification::parse(ident).ok()?;
    let limits = fvid_codecs::vorbis_admission::inspect(setup).ok()?.limits;
    let setup = fvid_codecs::vorbis::Setup::parse(setup, ident.channels, limits).ok()?;
    Some((ident.block_exponents, setup.modes, ident.sample_rate))
}

/// DiscardPadding belongs to the entire Block, including every laced frame.
fn spread_lace_padding(packets: &mut [Packet]) -> Result<()> {
    let signed: i128 = packets.iter().map(|p| i128::from(p.discard_padding_ns)).sum();
    if signed == 0 { return Ok(()); }
    let total = packets.iter().try_fold(0u128, |total, packet| {
        packet.duration_ns.map(|duration| total + u128::from(duration))
            .ok_or_else(|| invalid("Matroska lace padding requires frame durations"))
    })?;
    let mut remaining = signed.unsigned_abs();
    if remaining > total {
        return Err(invalid("Matroska discard padding exceeds laced block duration"));
    }
    let mut trim = |packet: &mut Packet| -> Result<()> {
        let take = remaining.min(u128::from(packet.duration_ns.expect("lace durations validated")));
        let signed_take = if signed < 0 { -(take as i128) } else { take as i128 };
        packet.discard_padding_ns = i64::try_from(signed_take)
            .map_err(|_| invalid("Matroska lace padding overflow"))?;
        remaining -= take;
        Ok(())
    };
    if signed < 0 {
        for packet in packets.iter_mut() { trim(packet)?; }
    } else {
        for packet in packets.iter_mut().rev() { trim(packet)?; }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_block<R: Read + Seek>(
    r: &mut R,
    e: Element,
    timestamp: Option<u64>,
    kind: BlockKind,
    out: &mut Vec<Packet>,
    limits: Limits,
    groups: &mut Vec<std::ops::Range<usize>>,
    clocks: (&[Track], &[(u64, f64)], &[(u64, Vec<u8>)]),
    timings: &mut Vec<BlockTiming>,
) -> Result<()> {
    goto(r, e.data)?;
    let limit = end(e)?;
    let (track, width) = lace_vint(r, limit)?;
    if track == 0 || track == (1u64 << (7 * width)) - 1 {
        return Err(invalid("invalid WebM block track"));
    }
    let metadata_pending = !clocks.0.iter().any(|value| value.number == track);
    let has_prefix = clocks.2.iter().any(|(number, prefix)| *number == track && !prefix.is_empty());
    if limit.saturating_sub(r.stream_position()?) < if metadata_pending || has_prefix { 3 } else { 4 } {
        return Err(invalid("truncated WebM block"));
    }
    let mut h = [0; 3];
    r.read_exact(&mut h)?;
    let reserved_mask = match kind {
        BlockKind::Simple => 0x70,
        BlockKind::Group { .. } => 0x71,
    };
    if h[2] & reserved_mask != 0 {
        return Err(invalid("invalid Matroska block flags"));
    }
    let mode = h[2] & 6;
    let mut count = 1usize;
    let mut sizes = [0u64; 256];
    if mode != 0 {
        if r.stream_position()? >= limit { return Err(invalid("truncated Matroska lace header")); }
        let mut n = [0];
        r.read_exact(&mut n)?;
        count = usize::from(n[0]) + 1;
        if count == 1 {
            return Err(invalid("Matroska lacing requires multiple frames"));
        }
        match mode {
            2 => {
                for size in &mut sizes[..count - 1] {
                    loop {
                        if r.stream_position()? >= limit {
                            return Err(invalid("truncated Matroska lace header"));
                        }
                        r.read_exact(&mut n)?;
                        *size = size
                            .checked_add(u64::from(n[0]))
                            .ok_or_else(|| invalid("Matroska lace size overflow"))?;
                        if *size > limits.packet_bytes as u64 {
                            return Err(invalid("WebM packet exceeds budget"));
                        }
                        if n[0] != 255 {
                            break;
                        }
                    }
                }
            }
            6 => {
                sizes[0] = lace_vint(r, limit)?.0;
                for index in 1..count - 1 {
                    let (delta, width) = lace_vint(r, limit)?;
                    let signed = i128::from(delta) - ((1i128 << (7 * width - 1)) - 1);
                    sizes[index] = u64::try_from(i128::from(sizes[index - 1]) + signed)
                        .map_err(|_| invalid("invalid Matroska lace size difference"))?;
                }
            }
            _ => {}
        }
    }
    let mut offset = r.stream_position()?;
    let remaining = limit
        .checked_sub(offset)
        .ok_or_else(|| invalid("Matroska lace header exceeds block"))?;
    if mode == 4 {
        if !remaining.is_multiple_of(count as u64) {
            return Err(invalid("fixed Matroska lace has unequal frame sizes"));
        }
        sizes[..count].fill(remaining / count as u64);
    } else {
        let known = sizes[..count - 1]
            .iter()
            .try_fold(0u64, |a, &b| a.checked_add(b))
            .ok_or_else(|| invalid("Matroska lace size overflow"))?;
        sizes[count - 1] = remaining
            .checked_sub(known)
            .ok_or_else(|| invalid("Matroska lace sizes exceed block"))?;
    }
    if sizes[..count]
        .iter()
        .any(|&n| (n == 0 && !metadata_pending && !has_prefix) || n > limits.packet_bytes as u64)
    {
        return Err(invalid(
            "WebM packet exceeds budget or has empty lace frame",
        ));
    }
    let required = out
        .len()
        .checked_add(count)
        .filter(|&n| n <= limits.packets)
        .ok_or_else(|| invalid("WebM packet count exceeds limit"))?;
    if required > out.capacity() {
        let capacity = out
            .capacity()
            .saturating_mul(2)
            .max(required)
            .max(4)
            .min(limits.packets);
        out.try_reserve_exact(capacity - out.len())
            .map_err(|_| invalid("WebM packet index allocation failed"))?;
    }
    let cluster = timestamp.ok_or_else(|| invalid("WebM block precedes Cluster timestamp"))?;
    let relative = i16::from_be_bytes([h[0], h[1]]);
    // Late track declarations need the original relative clock too. Preserve
    // that clock per block only for these cases and non-unit track scales.
    let raw = metadata_pending
        || clocks.1.iter().any(|&(number, _)| number == track);
    let pts_ns = if raw { 0 } else {
        i64::try_from(i128::from(cluster) + i128::from(relative))
            .map_err(|_| invalid("WebM timestamp overflow"))?
    };
    let first = out.len();
    if raw {
        timings.try_reserve(1).map_err(|_| invalid("Matroska block clock allocation failed"))?;
        timings.push(BlockTiming { range: first..required, cluster, relative });
    }
    for &size in &sizes[..count] {
        out.push(Packet {
            track,
            pts_ns,
            keyframe: match kind {
                BlockKind::Simple => h[2] & 0x80 != 0,
                BlockKind::Group { keyframe } => keyframe,
            },
            invisible: h[2] & 8 != 0,
            offset,
            size: usize::try_from(size).map_err(|_| invalid("WebM packet size overflow"))?,
            discard_padding_ns: 0,
            duration_ns: None,
        });
        offset += size;
    }
    if count > 1 {
        groups
            .try_reserve(1)
            .map_err(|_| invalid("Matroska lace index allocation failed"))?;
        groups.push(first..out.len());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn library_indexes_synthetic_video_and_reads_packet() {
        fn atom(id: &[u8], data: &[u8]) -> Vec<u8> {
            assert!(data.len() < 127);
            [id, &[0x80 | data.len() as u8], data].concat()
        }
        let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
        let video = atom(
            &[0xe0],
            &[atom(&[0xb0], &[16]), atom(&[0xba], &[8])].concat(),
        );
        let track = atom(
            &[0xae],
            &[
                atom(&[0xd7], &[1]),
                atom(&[0x83], &[1]),
                atom(&[0x86], b"V_VP9"),
                video,
            ]
            .concat(),
        );
        let tracks = atom(&[0x16, 0x54, 0xae, 0x6b], &track);
        let cluster = atom(
            &[0x1f, 0x43, 0xb6, 0x75],
            &[
                atom(&[0xe7], &[2]),
                atom(&[0xa3], &[0x81, 0, 1, 0x80, 0x82, 0x49]),
            ]
            .concat(),
        );
        let file = [header, vec![0x18, 0x53, 0x80, 0x67, 0xff], tracks, cluster].concat();
        let mut reader = WebmReader::open(Cursor::new(file), Limits::default()).unwrap();
        assert_eq!(reader.tracks[0].codec, "V_VP9");
        assert_eq!(reader.tracks[0].visible(), (16, 8));
        assert_eq!(reader.packets[0].pts_ns, 3_000_000);
        assert_eq!(reader.read_packet(0).unwrap(), [0x82, 0x49]);
    }
}

#[cfg(test)]
mod index_allocation_tests {
    use super::*;
    #[test]
    fn packet_capacity_respects_non_power_of_two_index_limit() {
        let fixture = include_bytes!("../../../tests/fixtures/playback-errors/ffv1-six-frames.mkv");
        let mut reader = WebmReader::open(
            std::io::Cursor::new(fixture),
            Limits {
                packets: 6,
                ..Default::default()
            },
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 6);
        assert_eq!(reader.packets.capacity(), 6);
        assert_eq!(
            reader.packet_index_payload_bytes().unwrap(),
            6 * std::mem::size_of::<Packet>()
        );
        let error = match WebmReader::open(
            std::io::Cursor::new(fixture),
            Limits {
                packets: 5,
                ..Default::default()
            },
        ) {
            Ok(_) => panic!("index limit must refuse the sixth packet"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("WebM packet count exceeds limit")
        );
    }
    #[test]
    fn index_estimate_counts_metadata_and_codec_private_spare_capacity() {
        let fixture = include_bytes!("../../../tests/fixtures/playback-errors/ffv1-six-frames.mkv");
        let mut reader =
            WebmReader::open(std::io::Cursor::new(fixture), Default::default()).unwrap();
        reader.scan_all().unwrap();
        let before = reader.estimated_index_payload_bytes().unwrap();
        assert!(before >= reader.packet_index_payload_bytes().unwrap());
        let mut key = String::with_capacity(64);
        key.push_str("NOTE");
        let mut value = String::with_capacity(128);
        value.push_str("synthetic");
        let metadata_bytes = key.capacity() + value.capacity();
        reader.metadata.insert(key, value);
        assert_eq!(
            reader.estimated_index_payload_bytes().unwrap() - before,
            4096 + metadata_bytes
        );
        let before = reader.estimated_index_payload_bytes().unwrap();
        let old = reader.tracks[0].codec_private.capacity();
        reader.tracks[0].codec_private.reserve(16384);
        let growth = reader.tracks[0].codec_private.capacity() - old;
        assert_eq!(
            reader.estimated_index_payload_bytes().unwrap() - before,
            growth
        );
    }
    #[test]
    fn admitted_scan_preserves_full_scan_metadata_and_timestamps() {
        let fixture =
            include_bytes!("../../../tests/fixtures/playback-errors/ffv1-custom-tags.mkv");
        let mut expected =
            WebmReader::open(std::io::Cursor::new(fixture), Default::default()).unwrap();
        expected.scan_all().unwrap();
        let mut admitted =
            WebmReader::open(std::io::Cursor::new(fixture), Default::default()).unwrap();
        admitted
            .scan_all_with_admission(|reader| {
                reader.estimated_index_payload_bytes()?;
                Ok(())
            })
            .unwrap();
        assert_eq!(admitted.tags, expected.tags);
        assert_eq!(admitted.metadata, expected.metadata);
        assert_eq!(admitted.track_metadata, expected.track_metadata);
        assert_eq!(admitted.chapters, expected.chapters);
        assert_eq!(admitted.duration_ns, expected.duration_ns);
        assert_eq!(
            admitted
                .packets
                .iter()
                .map(|packet| (packet.pts_ns, packet.size))
                .collect::<Vec<_>>(),
            expected
                .packets
                .iter()
                .map(|packet| (packet.pts_ns, packet.size))
                .collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod lace_integer_tests {
    use super::*;
    #[test]
    fn lace_integers_keep_all_ones_data_and_obey_block_boundary() {
        assert_eq!(lace_vint(&mut Cursor::new([255]),1).unwrap(),(127,1));
        assert_eq!(lace_vint(&mut Cursor::new([127,255]),2).unwrap(),(16383,2));
        assert!(lace_vint(&mut Cursor::new([127,255]),1).unwrap_err().to_string().contains("truncated"));
        assert!(lace_vint(&mut Cursor::new([0]),1).unwrap_err().to_string().contains("zero"));
    }
    use std::io::Cursor;
}

#[cfg(test)]
mod scaled_aac_clock_memory_tests {
    use super::*;
    #[test]
    fn retained_scaled_aac_clock_records_are_charged_and_unit_scale_allocates_none() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        for(name,blocks)in [("half.mkv",6),("lace-third.mkv",2),("warm-third.mkv",72)] {
            let mut reader=WebmReader::open(std::io::Cursor::new(std::fs::read(root.join("aac-scaled-clock").join(name)).unwrap()),Default::default()).unwrap();
            reader.scan_all().unwrap();assert_eq!(reader.scaled_audio_timings.len(),blocks);
            let with=reader.estimated_index_payload_bytes().unwrap();let records=std::mem::take(&mut reader.scaled_audio_timings);
            let without=reader.estimated_index_payload_bytes().unwrap();
            assert_eq!(with-without,records.capacity()*std::mem::size_of::<BlockTiming>());
            assert!(reader.track_timestamp_scale(u64::MAX).is_none());assert!(reader.packet_timestamp_grid(usize::MAX).is_none());
        }
        let mut reader=WebmReader::open(std::io::Cursor::new(std::fs::read(root.join("aac-device-layout/normal-ten.mkv")).unwrap()),Default::default()).unwrap();reader.scan_all().unwrap();
        assert!(reader.scaled_audio_timings.is_empty());assert_eq!(reader.scaled_audio_timings.capacity(),0);
    }
}

#[cfg(test)]
mod frontend_tests {
    use super::*;
    use super::reader_ebml::vint;
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
    const CHAPTERS: &[u8] = include_bytes!("../../../tests/fixtures/chapters/chapters.mkv");
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
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/tracks/named.mkv");
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
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/tags/title.mkv");
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
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/tags/tags.mkv");
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
        use fvid_codecs::color::primaries::Primaries;
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
