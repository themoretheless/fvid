//! Owned AVC/HEVC/AAC/FFV1 Matroska muxing. No external muxer is used.
//! Mapping: https://www.matroska.org/technical/codec_specs.html#a_aac
use crate::{Result, invalid};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::io::{Read, Seek, SeekFrom, Write};
include!("../../crates/fvid-media/src/owned_matroska_ebml_impl.rs");

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
    let metadata = options.and_then(|o| o.video.as_ref());
    let rotation = options.map_or(0, |o| o.rotation);
    let delay = options.map_or(0, |o| o.codec_delay_ns);
    if delay > i64::MAX as u64 {
        return Err(invalid("Matroska codec delay overflow"));
    }
    use crate::codec::config::{AacConfig, AvcConfig, HevcConfig};
    let (id, config, kind, geometry) = match spec.encoding {
        Encoding::Ass { configuration } => {
            if metadata.is_some() || rotation != 0 || delay != 0 {
                return Err(invalid("video metadata or codec delay supplied for ASS track"));
            }
            let header = std::str::from_utf8(configuration).map_err(|_| invalid("ASS header is not UTF-8"))?;
            if !header.contains("[Script Info]") || !header.contains("[V4+ Styles]")
                || !header.contains("ScriptType: v4.00+") || header.contains('\0') {
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
            let channels=super::opus_packet::header_channels(configuration)?;
            if delay!=super::opus_packet::pre_skip_ns(configuration)? {return Err(invalid("Opus codec delay differs from pre-skip"));}
            let sample_rate=u32::from_le_bytes(configuration[12..16].try_into().unwrap());
            if sample_rate==0 {return Err(invalid("owned Matroska Opus output requires a nonzero input sample rate"));}
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
    use fvid_media::owned_matroska as owned;
    let metadata = metadata.map(|m| owned::VideoMetadata {
        crop: m.crop,
        pixel_aspect: m.pixel_aspect,
        colour: m.colour.map(|c| owned::ColourDescription {
            matrix: c.matrix,
            transfer: c.transfer,
            primaries: c.primaries,
            full_range: c.full_range,
        }),
        hdr: owned::HdrMetadata {
            light: owned::ContentLight {
                max_cll: m.hdr.light.max_cll,
                max_fall: m.hdr.light.max_fall,
            },
            mastering: m.hdr.mastering.map(|d| owned::MasteringDisplay {
                red: owned::Chromaticity {
                    x: d.red.x,
                    y: d.red.y,
                },
                green: owned::Chromaticity {
                    x: d.green.x,
                    y: d.green.y,
                },
                blue: owned::Chromaticity {
                    x: d.blue.x,
                    y: d.blue.y,
                },
                white: owned::Chromaticity {
                    x: d.white.x,
                    y: d.white.y,
                },
                min_luminance: d.min_luminance,
                max_luminance: d.max_luminance,
            }),
        },
    });
    owned::video_element(width, height, metadata.as_ref(), rotation)
        .map_err(|e| invalid(&e.to_string()))
}

include!("../../crates/fvid-media/src/owned_matroska_packet_impl.rs");

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

/// Stream strict ADTS packets into Matroska. Per-packet clusters avoid signed
/// block timestamp overflow at a 1 ns clock. No cue table/index is accumulated.
/// Caller must discard partial output after error and publish only on success.
pub fn write_adts<R: Read, W: Write + Seek>(
    mut input: super::adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let asc = input.audio_specific_config().to_vec();
    write_aac_packets(
        input.configuration(),
        &asc,
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
    let asc = sequence.audio_specific_config().to_vec();
    write_aac_packets(
        sequence.configuration(),
        &asc,
        || sequence.next_packet(),
        output,
        cancel,
        progress,
    )
}

fn write_aac_packets<W: Write + Seek>(
    config: super::adts::Header,
    asc: &[u8],
    mut next_packet: impl FnMut() -> Result<Option<Vec<u8>>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let samples = u64::from(crate::codec::config::AacConfig::parse(asc)?.frame_samples);
    let spec = TrackSpec {
        encoding: Encoding::Aac {
            configuration: asc,
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
