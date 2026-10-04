//! Filter the single video track and retain selected AAC/Opus packet timelines.
use crate::{owned_matroska as mkv, owned_webm as webm};
use fvid_control::CopyOptions;
use fvid_media_info::{LosslessStats, LosslessTransform};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
type Reader = webm::WebmReader<BufReader<File>>;
struct Input {
    reader: Reader,
    video: usize,
    selected: Vec<usize>,
}
fn open(path: &Path, options: &CopyOptions) -> Result<Reader> {
    let mut reader = webm::WebmReader::open(
        BufReader::new(File::open(path).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    reader.restrict_packet_bytes(options.max_packet_bytes);
    reader.scan_all().map_err(|e| e.to_string())?;
    Ok(reader)
}
fn check(o: &CopyOptions) -> Result<()> {
    if o.cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    Ok(())
}
fn qualify(path: &Path, t: &LosslessTransform, o: &CopyOptions) -> Result<Option<Input>> {
    let mut prefix = [0; 4];
    if !File::open(path)
        .is_ok_and(|mut f| f.read_exact(&mut prefix).is_ok() && prefix == [0x1a, 0x45, 0xdf, 0xa3])
    {
        return Ok(None);
    }
    if t.interval.is_some() || crate::owned_lossless::request(t).is_none() {
        return Ok(None);
    }
    let mut normalized = o.clone();
    normalized.streams.clear();
    for (i, _, _) in &mut normalized.stream_metadata_set {
        *i = 0;
    }
    for (i, _) in &mut normalized.stream_metadata_delete {
        *i = 0;
    }
    if !crate::owned_lossless::policy(&normalized, true) {
        return Ok(None);
    }
    check(o)?;
    let mut reader = open(path, o)?;
    if !(2..=126).contains(&reader.tracks.len()) || !reader.metadata_complete {
        return Ok(None);
    }
    if reader
        .track_metadata
        .keys()
        .any(|uid| *uid != 0 && !reader.track_uids.values().any(|known| known == uid))
    {
        return Ok(None);
    }
    let videos: Vec<_> = reader
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == 1)
        .map(|(i, _)| i)
        .collect();
    if videos.len() != 1 {
        return Ok(None);
    }
    let video = videos[0];
    let track = &reader.tracks[video];
    if !matches!(track.codec.as_str(), "V_FFV1" | "V_VP9" | "V_AV1")
        || track.crop != [0; 4]
        || track.rotation != 0
    {
        return Ok(None);
    }
    let selected = crate::owned_mp4_matroska::selection(reader.tracks.len(), &o.streams)
        .map_err(|e| e.to_string())?;
    if !selected.contains(&video) {
        return Err("lossless export requires at least one selected video stream".into());
    }
    let count = reader
        .packets
        .iter()
        .filter(|p| selected.iter().any(|&i| reader.tracks[i].number == p.track))
        .count() as u64;
    if o.max_packets.is_some_and(|n| count > n) {
        return Err("WebM input packet count exceeds limit".into());
    }
    for &i in &selected {
        let track = reader.tracks[i].clone();
        if i != video && (track.kind != 2 || !matches!(track.codec.as_str(), "A_AAC" | "A_OPUS")) {
            return Ok(None);
        }
        if reader
            .track_dispositions
            .get(&track.number)
            .is_some_and(|&d| d != 1)
        {
            return Ok(None);
        }
        let current = reader.track_languages.get(&track.number);
        let legacy = reader.track_legacy_languages.get(&track.number);
        if current.zip(legacy).is_some_and(|(a, b)| a != b) {
            return Ok(None);
        }
        let mut previous = None;
        let indices: Vec<_> = reader
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == track.number)
            .map(|(index, _)| index)
            .collect();
        for index in indices {
            check(o)?;
            let p = reader.packets[index].clone();
            if p.pts_ns < 0 || p.invisible && i != video || i == video && p.discard_padding_ns != 0
            {
                return Ok(None);
            }
            if i != video {
                if previous.is_some_and(|v| p.pts_ns < v) {
                    return Ok(None);
                }
                if p.duration_ns.unwrap_or(track.default_duration_ns) == 0 {
                    let duration = if track.codec == "A_OPUS" {
                        let bytes = reader.read_packet(index).map_err(|e| e.to_string())?;
                        crate::owned_opus_packet::duration_ns(&bytes).map_err(|e| e.to_string())?
                    } else {
                        let config =
                            crate::owned_aac::config::AacConfig::parse(&track.codec_private)
                                .map_err(|e| e.to_string())?;
                        (u64::from(config.frame_samples) * 1_000_000_000)
                            .div_ceil(u64::from(config.sample_rate))
                    };
                    reader.packets[index].duration_ns = Some(duration);
                }
                previous = Some(p.pts_ns);
            }
        }
    }
    if o.stream_metadata_set
        .iter()
        .any(|(i, _, _)| *i >= reader.tracks.len())
        || o.stream_metadata_delete
            .iter()
            .any(|(i, _)| *i >= reader.tracks.len())
    {
        return Err("metadata refers to missing input track".into());
    }
    Ok(Some(Input {
        reader,
        video,
        selected,
    }))
}
pub(crate) fn supports(source: &Path, t: &LosslessTransform, o: &CopyOptions) -> bool {
    match qualify(source, t, o) {
        Ok(Some(_)) => !matches!(
            crate::owned_video_decode::try_webm(
                source,
                &crate::owned_lossless::request(t).unwrap()
            ),
            Ok(None)
        ),
        Err(_) => true,
        Ok(None) => false,
    }
}
pub(crate) fn try_export(
    source: &Path,
    destination: &Path,
    t: &LosslessTransform,
    o: &CopyOptions,
) -> Result<Option<LosslessStats>> {
    let Some(mut input) = qualify(source, t, o)? else {
        return Ok(None);
    };
    let request = crate::owned_lossless::request(t).unwrap();
    if matches!(
        crate::owned_video_decode::try_webm(source, &request),
        Ok(None)
    ) {
        return Ok(None);
    }
    if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err("lossless export requires FFV1 in .mkv".into());
    }
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    let private = crate::owned_video_temporary::Directory::new()?;
    let video_path = private.0.join("video.mkv");
    let mut stage = o.clone();
    stage.progress = None;
    stage.max_packets = None;
    stage.streams.clear();
    stage
        .stream_metadata_set
        .retain(|(i, _, _)| *i == input.video);
    for (i, _, _) in &mut stage.stream_metadata_set {
        *i = 0;
    }
    stage
        .stream_metadata_delete
        .retain(|(i, _)| *i == input.video);
    for (i, _) in &mut stage.stream_metadata_delete {
        *i = 0;
    }
    let (decoded, _, consumed) =
        crate::owned_ffv1_export::export(source, &video_path, &request, &stage)?;
    let mut video = open(&video_path, o)?;
    let vt = video.tracks[0].clone();
    let mut tracks = input.reader.tracks.clone();
    for track in &mut tracks {
        if track.language.is_empty() {
            track.language = input
                .reader
                .track_languages
                .get(&track.number)
                .or_else(|| input.reader.track_legacy_languages.get(&track.number))
                .cloned()
                .unwrap_or_else(|| "eng".into());
        }
    }

    let mut scoped = BTreeMap::new();
    for (mapped, &index) in input.selected.iter().enumerate() {
        let source_tags = if index == input.video {
            video
                .track_uids
                .get(&vt.number)
                .and_then(|uid| video.track_metadata.get(uid))
        } else {
            input
                .reader
                .track_uids
                .get(&tracks[index].number)
                .and_then(|uid| input.reader.track_metadata.get(uid))
        };
        let mut tags = if index == input.video {
            BTreeMap::new()
        } else {
            input
                .reader
                .track_metadata
                .get(&0)
                .cloned()
                .unwrap_or_default()
        };
        if let Some(specific) = source_tags {
            tags.extend(specific.clone());
        }

        if index != input.video {
            for (_, key) in o.stream_metadata_delete.iter().filter(|(i, _)| *i == index) {
                tags.retain(|k, _| !k.eq_ignore_ascii_case(key));
                if key.eq_ignore_ascii_case("title") {
                    tracks[index].name.clear();
                } else if key.eq_ignore_ascii_case("language") {
                    tracks[index].language = "und".into();
                }
            }
            for (_, key, value) in o.stream_metadata_set.iter().filter(|(i, _, _)| *i == index) {
                tags.retain(|k, _| !k.eq_ignore_ascii_case(key));
                if key.eq_ignore_ascii_case("title") {
                    tracks[index].name = value.clone();
                } else if key.eq_ignore_ascii_case("language") {
                    tracks[index].language = value.clone();
                } else if !value.is_empty() {
                    tags.insert(key.to_ascii_uppercase(), value.clone());
                }
            }
        }
        scoped.insert(mapped, tags);
    }
    let mut specs = Vec::new();
    let mut options = Vec::new();
    for &index in &input.selected {
        let track = if index == input.video {
            &vt
        } else {
            &tracks[index]
        };
        let encoding = if index == input.video {
            mkv::Encoding::Ffv1V1 {
                width: u32::try_from(track.width).map_err(|_| "video width overflow")?,
                height: u32::try_from(track.height).map_err(|_| "video height overflow")?,
            }
        } else if track.codec == "A_OPUS" {
            mkv::Encoding::Opus {
                configuration: &track.codec_private,
            }
        } else {
            mkv::Encoding::Aac {
                configuration: &track.codec_private,
                sample_rate: u32::try_from(track.sample_rate).map_err(|_| "audio rate overflow")?,
                channels: u16::try_from(track.channels).map_err(|_| "audio channels overflow")?,
            }
        };
        specs.push(mkv::TrackSpec {
            encoding,
            name: &track.name,
            language: &track.language,
        });
        options.push(mkv::TrackOptions {
            codec_delay_ns: track.codec_delay_ns,
            default_duration_ns: track.default_duration_ns,
            video: (index == input.video).then(|| mkv::VideoMetadata {
                pixel_aspect: track.pixel_aspect(),
                colour: Some(track.colour),
                hdr: track.hdr,
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    let mut schedule = Vec::new();
    schedule
        .try_reserve_exact(
            input
                .reader
                .packets
                .len()
                .checked_add(video.packets.len())
                .ok_or("packet schedule overflow")?,
        )
        .map_err(|_| "packet schedule allocation failed")?;

    for (mapped, &index) in input.selected.iter().enumerate() {
        let packets = if index == input.video {
            &video.packets
        } else {
            &input.reader.packets
        };
        let number = if index == input.video {
            vt.number
        } else {
            tracks[index].number
        };
        for (packet, p) in packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == number)
        {
            schedule.push((
                i128::from(p.pts_ns) - i128::from(options[mapped].codec_delay_ns),
                mapped,
                packet,
            ));
        }
    }
    schedule.sort_unstable();
    let mut copied = 0;
    crate::owned_matroska_remux::publish(destination, o, |output| {
        let metadata = mkv::FileMetadata {
            tags: video.tags.clone(),
            chapters: video.chapters.clone(),
        };
        let mut writer = mkv::PacketWriter::new_with_text_metadata(
            output,
            &specs,
            &options,
            &metadata,
            &video.metadata,
            &scoped,
        )
        .map_err(|e| e.to_string())?;
        for &(_, mapped, index) in &schedule {
            check(o)?;
            let is_video = input.selected[mapped] == input.video;
            let reader = if is_video {
                &mut video
            } else {
                &mut input.reader
            };
            let packet = reader.packets[index].clone();
            let bytes = reader.read_packet(index).map_err(|e| e.to_string())?;
            writer
                .write_packet_with_options(
                    mapped,
                    u64::try_from(packet.pts_ns).map_err(|_| "negative packet timestamp")?,
                    packet
                        .duration_ns
                        .unwrap_or(options[mapped].default_duration_ns),
                    packet.keyframe,
                    &bytes,
                    mkv::PacketOptions {
                        discard_padding_ns: packet.discard_padding_ns,
                        invisible: packet.invisible,
                    },
                )
                .map_err(|e| e.to_string())?;
            if !is_video {
                copied += 1;
            }
            if let Some(hook) = &o.progress {
                hook.emit(writer.event());
            }
        }
        writer.finish().map_err(|e| e.to_string())
    })?;
    Ok(Some(LosslessStats {
        backend: "fvid",
        decoded_frames: consumed,
        video_packets: video.packets.len() as u64,
        copied_packets: copied,
        video_frames: decoded.video_frames,
        seek_used: false,
        trimmed_audio_sample_frames: 0,
        pixel_format: decoded.pixel_format,
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: t.vertical_flip,
        horizontal_flip: t.horizontal_flip,
    }))
}
