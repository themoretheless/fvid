//! Owned AVC/HEVC/AAC/FFV1 Matroska muxing. No external muxer is used.
//! Mapping: https://www.matroska.org/technical/codec_specs.html#a_aac
use crate::{Result, invalid};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
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

/// File-level metadata. Chapter timestamps are nanoseconds on the presentation
/// timeline; editions are flat and unordered. No implicit chapter ends are added.
#[derive(Clone, Debug, Default)]
pub struct FileMetadata {
    pub tags: super::FileTags,
    pub chapters: Vec<super::webm::Chapter>,
}

impl FileMetadata {
    pub fn from_mp4<R: Read + Seek>(input: &super::mp4::Mp4Reader<R>) -> Self {
        Self {
            tags: input.tags().clone(),
            chapters: input
                .chapters()
                .iter()
                .map(|c| super::webm::Chapter {
                    start_ns: c.start_ns,
                    end_ns: None,
                    title: c.title.clone(),
                })
                .collect(),
        }
    }
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

/// Per-block presentation controls. Invisible video blocks still establish
/// decoder references but do not extend the presentation duration.
#[derive(Clone, Copy, Debug, Default)]
pub struct PacketOptions {
    pub discard_padding_ns: i64,
    pub invisible: bool,
}

/// Encoded packet storage is passed through without rewriting codec payloads.
pub enum Encoding<'a> {
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

/// Container-level video properties, independent of codec configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoMetadata {
    /// Left, top, right, bottom in coded pixels.
    pub crop: [u32; 4],
    /// Width/height of one visible pixel; must be nonzero.
    pub pixel_aspect: (u32, u32),
    /// None leaves the container colour declaration absent.
    pub colour: Option<crate::color::hdr::ColourDescription>,
    pub hdr: crate::color::hdr::HdrMetadata,
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

/// Per-track container options. Rotation is clockwise in display space.
#[derive(Clone, Copy, Debug, Default)]
pub struct TrackOptions {
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
    let metadata = options.and_then(|o| o.video.as_ref());
    let rotation = options.map_or(0, |o| o.rotation);
    let delay = options.map_or(0, |o| o.codec_delay_ns);
    if delay > i64::MAX as u64 {
        return Err(invalid("Matroska codec delay overflow"));
    }
    use crate::codec::config::{AacConfig, AvcConfig, HevcConfig};
    let (id, config, kind, geometry) = match spec.encoding {
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
        Encoding::Aac {
            configuration,
            sample_rate,
            channels,
        } => {
            if metadata.is_some() || rotation != 0 {
                return Err(invalid("video metadata supplied for AAC track"));
            }
            let config = AacConfig::parse(configuration)?;
            if sample_rate == 0
                || channels == 0
                || config.sample_rate != sample_rate
                || u16::from(config.channels) != channels
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
    if delay != 0 {
        data.extend(uint(0x56aa, delay)?);
    }
    if !spec.name.is_empty() {
        data.extend(element(0x536e, spec.name.as_bytes())?);
    }
    // Write an explicit undetermined language instead of Matroska's default eng.
    data.extend(element(
        0x22b59c,
        if spec.language.is_empty() {
            b"und"
        } else {
            spec.language.as_bytes()
        },
    )?);
    element(0xae, &data)
}
fn video(
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

/// Streaming packet writer. Feed each track in decode order, interleaving tracks
/// at the call site. PTS can move backwards for B-frames; each packet has its own
/// nanosecond-clock cluster, avoiding signed 16-bit block timestamp overflow.
/// No payload copies or accumulated packet index. Discard output on any error.
pub struct PacketWriter<'a, W> {
    output: &'a mut W,
    segment_size: u64,
    duration_offset: u64,
    written: Vec<bool>,
    delays: Vec<u64>,
    end_ns: u64,
    event: ProgressEvent,
    failed: bool,
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
        let file_elements = file_metadata(metadata)?;
        if tracks.is_empty() || tracks.len() > 126 {
            return Err(invalid("Matroska requires 1..=126 tracks"));
        }
        if !options.is_empty() && options.len() != tracks.len() {
            return Err(invalid("Matroska metadata track count mismatch"));
        }
        let mut entries = Vec::new();
        for (index, track) in tracks.iter().enumerate() {
            entries.extend(track_entry(track, index as u64 + 1, options.get(index))?);
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
        output.write_all(&element(0x1654ae6b, &entries)?)?;
        output.write_all(&file_elements)?;
        Ok(Self {
            output,
            segment_size,
            duration_offset,
            written: vec![false; tracks.len()],
            delays: (0..tracks.len())
                .map(|i| options.get(i).map_or(0, |o| o.codec_delay_ns))
                .collect(),
            end_ns: 0,
            event: ProgressEvent {
                packets: 0,
                payload_bytes: 0,
                done: false,
            },
            failed: false,
        })
    }
    pub fn event(&self) -> ProgressEvent {
        self.event
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
        if self.failed || self.written.iter().any(|v| !v) {
            return Err(invalid("incomplete Matroska tracks"));
        }
        if self.end_ns == 0 {
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
        self.output.seek(SeekFrom::Start(self.duration_offset))?;
        self.output.write_all(&(self.end_ns as f64).to_be_bytes())?;
        self.output.seek(SeekFrom::Start(end))?;
        Ok(self.event)
    }
}

/// Stream strict ADTS packets into Matroska. Per-packet clusters avoid signed
/// block timestamp overflow at a 1 ns clock. No cue table/index is accumulated.
/// Caller must discard partial output after error and publish only on success.
pub fn write_adts<R: Read, W: Write + Seek>(
    mut input: super::adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    write_aac_packets(
        input.configuration(),
        || input.next_packet(),
        output,
        cancel,
        progress,
    )
}

/// Append compatible ADTS segments to one Matroska track without decoding.
pub fn concat_adts<R: Read, W: Write + Seek>(
    readers: Vec<super::adts::StreamReader<R>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let mut sequence = super::adts::SequenceReader::new(readers)?;
    write_aac_packets(
        sequence.configuration(),
        || sequence.next_packet(),
        output,
        cancel,
        progress,
    )
}

fn write_aac_packets<W: Write + Seek>(
    config: super::adts::Header,
    mut next_packet: impl FnMut() -> Result<Option<Vec<u8>>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let samples = u64::from(crate::codec::config::AacConfig::parse(&config.asc)?.frame_samples);
    let spec = TrackSpec {
        encoding: Encoding::Aac {
            configuration: &config.asc,
            sample_rate: config.sample_rate,
            channels: config.channels.into(),
        },
        name: "",
        language: "",
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
    let mut writer = PacketWriter::new(output, &[spec])?;
    if let Some(h) = progress {
        h.emit(writer.event());
    }
    loop {
        check()?;
        let Some(packet) = next_packet()? else {
            break;
        };
        let index = writer.event().packets;
        let next = index
            .checked_add(1)
            .ok_or_else(|| invalid("packet count overflow"))?;
        let start = time(index)?;
        writer.write_packet(0, start, time(next)? - start, true, &packet)?;
        if let Some(h) = progress {
            h.emit(writer.event());
        }
    }
    check()?;
    let event = writer.finish()?;
    check()?;
    Ok(event)
}

/// Copy one MP4 AAC track into Matroska, retaining decoder pre-roll and the
/// audible boundaries of a single media edit. This is a track-level operation:
/// file tags, chapters and other tracks belong to the caller's remux policy.
/// Empty/repeated edits require timeline reconstruction and are rejected here.
/// The caller must discard partial output on failure.
pub fn write_mp4_aac<R: Read + Seek, W: Write + Seek>(
    input: &mut super::mp4::Mp4Reader<R>,
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
/// Other tracks are rejected rather than silently discarded.
pub fn write_mp4_aac_file<R: Read + Seek, W: Write + Seek>(
    input: &mut super::mp4::Mp4Reader<R>,
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
    input: &mut super::mp4::Mp4Reader<R>,
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
    let asc = crate::codec::config::aac_specific_config(&track.configuration)?;
    let mut writer = PacketWriter::new_with_metadata(
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
    track: &super::mp4::Track,
    movie_scale: u32,
    cancel: Option<&CancelFlag>,
) -> Result<AacPacketPlan> {
    if track.handler != *b"soun" || track.codec != *b"mp4a" {
        return Err(invalid("MP4 track is not AAC"));
    }
    let asc = crate::codec::config::aac_specific_config(&track.configuration)?;
    let config = crate::codec::config::AacConfig::parse(asc)?;
    if track.timescale == 0
        || track.sample_rate != config.sample_rate
        || track.channels != u16::from(config.channels)
    {
        return Err(invalid("MP4 AAC clock or geometry mismatch"));
    }
    let rate = u128::from(config.sample_rate);
    let position = |ticks: u64| -> Result<u64> {
        let n = u128::from(ticks) * rate;
        let d = u128::from(track.timescale);
        if n % d != 0 {
            return Err(invalid("AAC time is not sample aligned"));
        }
        u64::try_from(n / d).map_err(|_| invalid("AAC sample position overflow"))
    };
    let media_end = position(track.duration)?;
    let (start, end) = match track.edits.as_slice() {
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
    let frame = u64::from(config.frame_samples);
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
    let delay = sample_ns(start, config.sample_rate)?;
    let coded_end = (count as u64)
        .checked_mul(frame)
        .ok_or_else(|| invalid("AAC timeline overflow"))?;
    let padding = i64::try_from(sample_ns(coded_end - end, config.sample_rate)?)
        .map_err(|_| invalid("AAC padding overflow"))?;
    Ok(AacPacketPlan {
        count,
        delay,
        padding,
        frame,
        rate: config.sample_rate,
    })
}
