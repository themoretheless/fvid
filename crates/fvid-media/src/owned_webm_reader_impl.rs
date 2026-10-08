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
include!("owned_ebml_impl.rs");

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
            metadata: Default::default(),
            writing_app: String::new(),
            track_uids: Default::default(),
            track_languages: Default::default(),
            track_legacy_languages: Default::default(),
            track_dispositions: Default::default(),
            track_metadata: Default::default(),
            metadata_complete: true,
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
            metadata,
            writing_app,
            track_uids,
            track_languages,
            track_legacy_languages,
            track_dispositions,
            track_metadata,
            metadata_complete,
            elements,
            scanned,
            tail_ns,
            duration_ns: _,
            chapters: _,
            read_packet_bytes: _,
        } = self;
        let indexed = packets.len();
        let mut lace_groups = Vec::new();
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
                        } else if f.id == 0x5741 {
                            *writing_app = lenient_text(&mut *reader, f, 1024).unwrap_or_default();
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
                        let mut output_sample_rate = None;
                        let mut uid = None;
                        let mut ietf = None;
                        let mut disposition = 1i32;
                        let mut language_present = false;
                        let mut legacy_language = None;
                        let mut older = String::new();
                        let mut current = String::new();
                        for f in fields(&mut *reader, entry, &mut *elements, limits.elements)? {
                            match f.id {
                                0xd7 => track.number = uint(&mut *reader, f)?,
                                0x73c5 => uid = Some(uint(&mut *reader,f)?),
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
                                0x22b59c => {
                                    language_present = true;
                                    current = text(&mut *reader, f, 128)?;
                                    legacy_language = Some(current.clone());
                                }
                                0x22b59d => {
                                    language_present = true;
                                    ietf = Some(text(&mut *reader, f, 128)?);
                                }
                                // Keep the historical TagLanguage-in-TrackEntry alias lenient.
                                0x447a => {
                                    language_present = true;
                                    older = text(&mut *reader, f, 128)?;
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
                                    if uint(&mut *reader, f)? != 0 {
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
                                                let value = float(&mut *reader, v)?;
                                                if !value.is_finite() || value <= 0.0 {
                                                    return Err(invalid(
                                                        "invalid WebM sampling frequency",
                                                    ));
                                                }
                                                if v.id == 0x78b5 {
                                                    output_sample_rate = Some(value.round() as u64);
                                                } else {
                                                    track.sample_rate = value.round() as u64;
                                                }
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
                                    return Err(unsupported(
                                        "encoded/encrypted WebM tracks not supported",
                                    ));
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
                        if let Some(uid) = uid {track_uids.insert(track.number,uid);}
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
                                    && (track.sample_rate == 0
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
                                    && (track.channels == 0
                                        || track.channels == u64::from(config.core.channels))
                                {
                                    track.channels = u64::from(config.output_channels());
                                }
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
                                BlockKind::Simple,
                                &mut *packets,
                                *limits,
                                &mut lace_groups,
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
                                            BlockKind::Group { keyframe: key },
                                            &mut *packets,
                                            *limits,
                                            &mut lace_groups,
                                        )?;
                                    }
                                }
                                for packet in &mut packets[first..] {
                                    packet.duration_ns = duration;
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
        for group in lace_groups {
            let first = &packets[group.start];
            let origin = i128::from(first.pts_ns);
            let count = (group.end-group.start) as u128;
            let track = tracks.iter().find(|t| t.number == first.track).unwrap();
            let default = track.default_duration_ns;
            // BlockDuration describes the whole block, not an equal duration
            // for every lace. Opus TOCs provide individual packet durations.
            if default == 0 && track.codec == "A_OPUS" {
                opus_lace_channels(&track.codec_private).map_err(|_| unsupported(
                    "Opus lace timing requires mono/stereo mapping family 0"))?;
                let mut elapsed = 0i128;
                let mut data = Vec::new();
                for packet in &mut packets[group] {
                    goto(&mut *reader, packet.offset)?;
                    data.clear();
                    data.try_reserve_exact(packet.size).map_err(|_| invalid("Opus lace timing allocation failed"))?;
                    data.resize(packet.size,0);
                    reader.read_exact(&mut data)?;
                    let duration = opus_lace_duration(&data).map_err(|e| invalid(&e.to_string()))?;
                    packet.pts_ns = i64::try_from(origin+elapsed).map_err(|_| invalid("WebM lace timestamp overflow"))?;
                    packet.duration_ns = Some(duration);
                    elapsed += i128::from(duration);
                    *tail_ns = (*tail_ns).max(packet.pts_ns);
                }
                continue;
            }
            let total = first.duration_ns.map(u128::from)
                .or_else(|| (default != 0).then_some(u128::from(default)*count))
                .ok_or_else(|| unsupported("Matroska lacing requires declared frame or block duration"))?;
            for (index, packet) in packets[group].iter_mut().enumerate() {
                let start = total * index as u128 / count;
                let finish = total * (index+1) as u128 / count;
                packet.pts_ns = i64::try_from(origin + i128::try_from(start)
                    .map_err(|_| invalid("WebM lace timestamp overflow"))?)
                    .map_err(|_| invalid("WebM lace timestamp overflow"))?;
                packet.duration_ns = Some(u64::try_from(finish-start)
                    .map_err(|_| invalid("WebM lace duration overflow"))?);
                *tail_ns = (*tail_ns).max(packet.pts_ns);
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
include!("owned_matroska_metadata_read_impl.rs");
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
enum BlockKind {
    Simple,
    Group { keyframe: bool },
}

fn read_block<R: Read + Seek>(
    r: &mut R,
    e: Element,
    timestamp: Option<u64>,
    kind: BlockKind,
    out: &mut Vec<Packet>,
    limits: Limits,
    groups: &mut Vec<std::ops::Range<usize>>,
) -> Result<()> {
    goto(r, e.data)?;
    let limit = end(e)?;
    let (track, width) = lace_vint(r, limit)?;
    if track == 0 || track == (1u64 << (7 * width)) - 1 {
        return Err(invalid("invalid WebM block track"));
    }
    if limit.saturating_sub(r.stream_position()?) < 4 {
        return Err(invalid("truncated WebM block"));
    }
    let mut h = [0; 3];
    r.read_exact(&mut h)?;
    let mode = h[2] & 6;
    let mut count = 1usize;
    let mut sizes = [0u64; 256];
    if mode != 0 {
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
        .any(|&n| n == 0 || n > limits.packet_bytes as u64)
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
    let pts =
        i128::from(timestamp.ok_or_else(|| invalid("WebM block precedes Cluster timestamp"))?)
            + i128::from(i16::from_be_bytes([h[0], h[1]]));
    let pts_ns = i64::try_from(pts).map_err(|_| invalid("WebM timestamp overflow"))?;
    let first = out.len();
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
