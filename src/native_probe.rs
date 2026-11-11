//! Container-only descriptions, without opening an external demuxer/decoder.
pub use fvid_media_info::{ChapterInfo, MediaInfo, StreamInfo};
pub use fvid_media::owned_webm_probe::declared_video_frame_rate;
type Result<T> = std::result::Result<T, String>;
use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};

fn tags(tags: &crate::container::FileTags) -> BTreeMap<String, String> {
    [
        ("title", &tags.title),
        ("artist", &tags.artist),
        ("album", &tags.album),
        ("genre", &tags.genre),
        ("date", &tags.date),
        ("comment", &tags.comment),
        ("track", &tags.track),
        ("album_artist", &tags.album_artist),
        ("disc", &tags.disc),
        ("publisher", &tags.publisher),
        ("copyright", &tags.copyright),
        ("description", &tags.description),
        ("rating", &tags.rating),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(key, value)| (key.into(), value.clone()))
    .collect()
}

pub use fvid_media::owned_mp4_probe::{
    mp4, mp4_from_file_with_options, mp4_with_metadata_limit, mp4_with_options, try_mp4,
    try_mp4_from_file, try_mp4_from_file_with_options, try_mp4_with_metadata_limit,
    try_mp4_with_options,
};

pub(crate) fn matroska_with_options(
    path: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<MediaInfo> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_media::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    matroska_with_limits(
        path,
        options.max_packet_bytes,
        options.max_controlled_bytes,
        options.max_packets,
    )
}

pub(crate) fn matroska_from_file_with_options(
    path: &Path,
    file: File,
    options: &fvid_media::CopyOptions,
) -> Result<MediaInfo> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_media::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    matroska_from_file_with_limits(
        path,
        file,
        options.max_packet_bytes,
        options.max_controlled_bytes,
        options.max_packets,
    )
}

fn matroska_with_limits(
    path: &Path,
    packet_bytes: usize,
    max_controlled_bytes: Option<usize>,
    max_packets: Option<u64>,
) -> Result<MediaInfo> {
    if packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    matroska_from_file_with_limits(
        path,
        file,
        packet_bytes,
        max_controlled_bytes,
        max_packets,
    )
}

fn matroska_from_file_with_limits(
    path: &Path,
    file: File,
    packet_bytes: usize,
    max_controlled_bytes: Option<usize>,
    max_packets: Option<u64>,
) -> Result<MediaInfo> {
    if packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    let mut limits = crate::container::webm::Limits {
        packet_bytes,
        ..Default::default()
    };
    if let Some(max_pkts) = max_packets {
        limits.packets = limits.packets.min(usize::try_from(max_pkts).unwrap_or(usize::MAX));
    }
    let memory_packets = max_controlled_bytes
        .map(|bytes| bytes / std::mem::size_of::<crate::container::webm::Packet>());
    if let Some(maximum) = memory_packets {
        limits.packets = limits.packets.min(maximum);
    }
    let map_packet_limit = |error: crate::Error| {
        if memory_packets
            .is_some_and(|max| max < crate::container::webm::Limits::default().packets && max_packets.is_none_or(|p| (max as u64) <= p))
            && error
                .to_string()
                .contains("WebM packet count exceeds limit")
        {
            "controlled memory budget exceeded: Matroska packet index limit".to_owned()
        } else {
            error.to_string()
        }
    };
    let mut reader = crate::container::webm::WebmReader::open(
        BufReader::new(file),
        limits,
    )
    .map_err(map_packet_limit)?;
    // Tags/chapters can follow clusters. Complete the bounded index before
    // describing the file, without decoding or retaining encoded payloads.
    reader.scan_all().map_err(map_packet_limit)?;
    if let Some(limit) = max_controlled_bytes {
        let index_bytes = reader
            .estimated_index_payload_bytes()
            .map_err(|e| e.to_string())?;
        if index_bytes > limit {
            return Err("controlled memory budget exceeded: Matroska index payload limit".into());
        }
        for i in 0..reader.packets.len() {
            let overhead = reader.packet_read_overhead(i).map_err(|e| e.to_string())?;
            if overhead == 0 {
                continue;
            }
            let pkt_bytes = reader
                .packet_payload_size(i)
                .map_err(|e| e.to_string())?
                .saturating_add(overhead);
            if index_bytes.saturating_add(pkt_bytes) > limit {
                return Err("controlled memory budget exceeded: Matroska index payload limit".into());
            }
        }
    }
    let duration_us = reader
        .duration_ns
        .map(|ns| {
            i64::try_from(ns / 1000).map_err(|_| "Matroska duration exceeds API range".to_owned())
        })
        .transpose()?;
    let mut file_metadata = reader.metadata.clone();
    let encoder = file_metadata
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("encoder"))
        .map(|(_, value)| value.clone())
        .or_else(|| (!reader.writing_app.is_empty()).then(|| reader.writing_app.clone()));
    if let Some(encoder) = encoder {
        file_metadata.entry("encoder".into()).or_insert(encoder);
    }
    file_metadata.extend(tags(&reader.tags));
    let mut start_ns = None::<i64>;
    let mut streams = Vec::with_capacity(reader.tracks.len());
    for (index, track) in reader.tracks.iter().enumerate() {
        let start = reader
            .packets
            .iter()
            .filter(|p| p.track == track.number)
            .map(|p| p.pts_ns)
            .min()
            .map(|pts| {
                i64::try_from(i128::from(pts) - i128::from(track.codec_delay_ns))
                    .map_err(|_| "Matroska start exceeds API range".to_owned())
            })
            .transpose()?;
        if let Some(start) = start {
            start_ns = Some(start_ns.map_or(start, |old| old.min(start)));
        }
        let mut metadata = reader.track_metadata.get(&0).cloned().unwrap_or_default();
        if let Some(uid) = reader.track_uids.get(&track.number) {
            if let Some(scoped) = reader.track_metadata.get(uid) {
                metadata.extend(scoped.clone());
            }
        }
        if !track.name.is_empty() {
            metadata.insert("title".into(), track.name.clone());
        }
        let language = reader
            .track_languages
            .get(&track.number)
            .unwrap_or(&track.language);
        if !language.is_empty() {
            metadata.insert("language".into(), language.clone());
        }
        let codec = match (track.codec.as_str(), track.bit_depth) {
            ("A_PCM/INT/LIT" | "A_PCM/INT/BIG", 8) => "pcm_u8".to_owned(),
            ("A_PCM/INT/LIT", bits @ (16 | 24 | 32)) => format!("pcm_s{bits}le"),
            ("A_PCM/INT/BIG", bits @ (16 | 24 | 32)) => format!("pcm_s{bits}be"),
            ("A_PCM/FLOAT/IEEE", bits @ (32 | 64)) => format!("pcm_f{bits}le"),
            (other, _) => match other {
                "V_MPEG4/ISO/AVC" => "h264",
                "V_MPEGH/ISO/HEVC" => "hevc",
                "V_VP8" => "vp8",
                "V_VP9" => "vp9",
                "V_AV1" => "av1",
                "V_FFV1" => "ffv1",
                "A_AAC" => "aac",
                "A_ALAC" => "alac",
                "A_OPUS" => "opus",
                "A_VORBIS" => "vorbis",
                "A_FLAC" => "flac",
                "A_AC3" | "A_AC3/BSID9" | "A_AC3/BSID10" => "ac3",
                "A_EAC3" => "eac3",
                "A_WAVPACK4" => "wavpack",
                "A_MPEG/L3" => "mp3",
                "A_MPEG/L2" => "mp2",
                "A_MPEG/L1" => "mp1",
                "S_TEXT/UTF8" | "S_TEXT/ASCII" | "S_UTF8" => "subrip",
                "S_TEXT/ASS" | "S_ASS" => "ass",
                "S_TEXT/SSA" | "S_SSA" => "ssa",
                "S_TEXT/WEBVTT" => "webvtt",
                "S_VOBSUB" => "dvd_subtitle",
                "S_HDMV/PGS" => "hdmv_pgs_subtitle",
                "S_DVBSUB" => "dvb_subtitle",
                other => other,
            }
            .to_owned(),
        };
        // DefaultDuration is a declared nominal cadence, not a measured average.
        // Leave average_frame_rate unknown instead of substituting that value.
        let (profile, level) = if codec == "h264" {
            fvid_media::owned_avc::configuration_profile_level(&track.codec_private)
                .map(|(p, l)| (Some(p.into()), Some(i32::from(l))))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
        streams.push(StreamInfo {
            index,
            media_type: match track.kind {
                1 => "video",
                2 => "audio",
                17 => "subtitle",
                _ => "data",
            }
            .into(),
            codec,
            time_base: [1, 1_000_000_000],
            start,
            duration: None,
            bit_rate: None,
            average_frame_rate: [0, 1],
            profile,
            level,
            disposition: reader
                .track_dispositions
                .get(&track.number)
                .copied()
                .unwrap_or(1),
            metadata,
            width: i32::try_from(track.width).map_err(|_| "Matroska width exceeds API range")?,
            height: i32::try_from(track.height).map_err(|_| "Matroska height exceeds API range")?,
            pixel_format: -1,
            sample_rate: i32::try_from(track.sample_rate)
                .map_err(|_| "Matroska sample rate exceeds API range")?,
            channels: i32::try_from(track.channels)
                .map_err(|_| "Matroska channels exceed API range")?,
            video_delay: 0,
            extradata_bytes: track.codec_private.len(),
        });
    }
    let chapters = reader
        .chapters
        .iter()
        .enumerate()
        .map(|(index, chapter)| {
            let start = i64::try_from(chapter.start_ns / 1000)
                .map_err(|_| "Matroska chapter exceeds API range")?;
            let end = if let Some(end) = chapter.end_ns {
                i64::try_from(end / 1000).map_err(|_| "Matroska chapter exceeds API range")?
            } else {
                match reader.chapters.get(index + 1) {
                    Some(next) => i64::try_from(next.start_ns / 1000)
                        .map_err(|_| "Matroska chapter exceeds API range")?,
                    None => duration_us.unwrap_or(start).max(start),
                }
            };
            Ok(ChapterInfo {
                id: index as i64,
                time_base: [1, 1_000_000],
                start,
                end,
                metadata: if chapter.title.is_empty() {
                    BTreeMap::new()
                } else {
                    BTreeMap::from([("title".into(), chapter.title.clone())])
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(MediaInfo {
        path: path.to_path_buf(),
        format: "matroska,webm".into(),
        start_us: start_ns.map(|ns| ns / 1000),
        duration_us,
        bit_rate: None,
        metadata: file_metadata,
        chapters,
        streams,
    })
}

/// Inspect packed PCM without reading or decoding sample payloads.
fn wave_with_options(source: &Path, options: &fvid_media::CopyOptions) -> Result<MediaInfo> {
    fvid_media::owned_probe::probe_wave_with_options(source, options).map_err(|e| e.to_string())
}

fn wave_from_file_with_options(
    source: &Path,
    file: File,
    options: &fvid_media::CopyOptions,
) -> Result<MediaInfo> {
    fvid_media::owned_probe::probe_wave_from_file_with_options(source, file, options)
        .map_err(|e| e.to_string())
}

fn try_y4m_with_options(
    source: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<Option<MediaInfo>> {
    fvid_media::owned_y4m_probe::try_y4m_with_options(source, options)
}

fn try_y4m_from_file_with_options(
    source: &Path,
    file: File,
    options: &fvid_media::CopyOptions,
) -> Result<Option<MediaInfo>> {
    fvid_media::owned_y4m_probe::try_y4m_from_file_with_options(source, file, options)
}

pub fn probe(source: &Path) -> Result<MediaInfo> {
    probe_as(source, None)
}
pub(crate) fn probe_from_file(source: &Path, file: File) -> Result<MediaInfo> {
    try_probe_from_file_with_options(source, file, None, &fvid_media::CopyOptions::default())?
        .ok_or_else(|| "native probe does not yet support this container".into())
}
pub(crate) fn probe_with_options(
    source: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<MediaInfo> {
    try_probe_as_with_options(source, None, options)?
        .ok_or_else(|| "native probe does not yet support this container".into())
}
pub(crate) fn probe_from_file_with_options(
    source: &Path,
    file: File,
    options: &fvid_media::CopyOptions,
) -> Result<MediaInfo> {
    try_probe_from_file_with_options(source, file, None, options)?
        .ok_or_else(|| "native probe does not yet support this container".into())
}
pub fn probe_as(source: &Path, format: Option<&str>) -> Result<MediaInfo> {
    try_probe_as(source, format)?
        .ok_or_else(|| "native probe does not yet support this container".into())
}
/// `None` means no owned parser recognizes the requested format/signature.
/// A recognized but malformed source returns an error, never `None`.
pub fn try_probe_as(source: &Path, format: Option<&str>) -> Result<Option<MediaInfo>> {
    try_probe_as_with_options(source, format, &fvid_media::CopyOptions::default())
}

fn validate_probe_options(options: &fvid_media::CopyOptions) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_media::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    Ok(())
}

/// Read metadata independently of an active encoded-packet window. Callers
/// retain the original limits for codec/interval admission before publication.
pub(crate) fn try_probe_metadata_with_options(
    source: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<Option<MediaInfo>> {
    validate_probe_options(options)?;
    let mut metadata = options.clone();
    metadata.max_packet_bytes = fvid_media::CopyOptions::default().max_packet_bytes;
    metadata.max_packets = None;
    try_probe_as_with_options(source, None, &metadata)
}

pub(crate) fn try_probe_metadata_from_file_with_options(
    source: &Path,
    file: File,
    options: &fvid_media::CopyOptions,
) -> Result<Option<MediaInfo>> {
    validate_probe_options(options)?;
    let mut metadata = options.clone();
    metadata.max_packet_bytes = fvid_media::CopyOptions::default().max_packet_bytes;
    metadata.max_packets = None;
    try_probe_from_file_with_options(source, file, None, &metadata)
}

pub(crate) fn try_probe_as_with_options(
    source: &Path,
    format: Option<&str>,
    options: &fvid_media::CopyOptions,
) -> Result<Option<MediaInfo>> {
    validate_probe_options(options)?;
    match format {
        Some("mov" | "mp4" | "m4a" | "3gp" | "3g2" | "mj2" | "mov,mp4,m4a,3gp,3g2,mj2") => {
            return mp4_with_options(source, options).map(Some);
        }
        Some("matroska" | "webm" | "matroska,webm") => {
            return matroska_with_options(source, options).map(Some);
        }
        Some("wav") => return wave_with_options(source, options).map(Some),
        Some("flac") => {
            return fvid_media::owned_flac_probe::probe_flac_with_options(source, options)
                .map(Some);
        }
        Some("mp1" | "mp2" | "mp3" | "mpeg_audio") => {
            return fvid_media::owned_mpeg_probe::probe_mpeg_with_options(source, options)
                .map(Some);
        }
        Some("ogg" | "opus") => {
            return fvid_media::owned_probe::probe_ogg_with_options(source, options).map(Some);
        }
        Some("yuv4mpegpipe" | "y4m") => return try_y4m_with_options(source, options),
        Some("aac") | None => {}
        Some(_) => return Ok(None),
    }
    let file = File::open(source).map_err(|e| e.to_string())?;
    try_probe_from_file_with_options(source, file, format, options)
}

pub(crate) fn try_probe_from_file_with_options(
    source: &Path,
    mut file: File,
    format: Option<&str>,
    options: &fvid_media::CopyOptions,
) -> Result<Option<MediaInfo>> {
    use std::io::{Read, Seek, SeekFrom};
    validate_probe_options(options)?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut signature = [0; 12];
    // Read up to twelve bytes, retaining a valid seven-byte ADTS header at EOF.
    let mut count = 0;
    while count < signature.len() {
        match file.read(&mut signature[count..]) {
            Ok(0) => break,
            Ok(n) => count += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    if format.is_none() {
        if signature[..count].starts_with(b"YUV4MPEG2") {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            return try_y4m_from_file_with_options(source, file, options);
        }
        if count == 12
            && &signature[8..12] == b"WAVE"
            && matches!(&signature[..4], b"RIFF" | b"RIFX" | b"RF64")
        {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            return wave_from_file_with_options(source, file, options).map(Some);
        }
        if crate::container::mp4::recognizes_prefix(&signature[..count]) {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            return mp4_from_file_with_options(source, file, options).map(Some);
        }
        if signature[..count].starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            return matroska_from_file_with_options(source, file, options).map(Some);
        }
        if signature[..count].starts_with(b"fLaC") {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            return fvid_media::owned_flac_probe::probe_flac_from_file_with_options(
                source, file, options,
            )
            .map(Some);
        }
        if signature[..count].starts_with(b"OggS") {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            return fvid_media::owned_probe::probe_ogg_from_file_with_options(
                source, file, options,
            )
            .map(Some);
        }
        if fvid_media::owned_aac::adts::header(&signature[..count]).is_some() {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            return fvid_media::owned_probe::probe_adts_from_file_with_options(
                source, file, options,
            )
            .map(Some);
        }
        if fvid_media::owned_mpeg_audio::has_signature(&signature[..count]) {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            return fvid_media::owned_mpeg_probe::probe_mpeg_from_file_with_options(
                source, file, options,
            )
            .map(Some);
        }
        return Ok(None);
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    fvid_media::owned_probe::probe_adts_from_file_with_options(source, file, options).map(Some)
}
