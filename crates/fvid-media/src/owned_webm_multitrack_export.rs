//! Filter the single video track and retain selected AAC/Opus packet timelines.
#[path = "owned_webm_audio_window.rs"]
mod audio_window;
#[path = "owned_webm_pcm_window.rs"]
mod pcm_window;
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
    trimmed_audio_sample_frames: u64,
    pcm_windows: Vec<usize>,
    video_origin: i64,
}
fn open(path: &Path, options: &CopyOptions) -> Result<Reader> {
    let limits = webm::Limits::default();
    let mut reader = webm::WebmReader::open(
        BufReader::new(File::open(path).map_err(|e| e.to_string())?),
        limits,
    )
    .map_err(|e| e.to_string())?;
    reader.scan_all().map_err(|e| e.to_string())?;
    reader.restrict_packet_bytes(options.max_packet_bytes);
    Ok(reader)
}
fn check(o: &CopyOptions) -> Result<()> {
    if o.cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(o)
}
fn qualify(path: &Path, t: &LosslessTransform, o: &CopyOptions) -> Result<Option<Input>> {
    let mut prefix = [0; 4];
    if !File::open(path)
        .is_ok_and(|mut f| f.read_exact(&mut prefix).is_ok() && prefix == [0x1a, 0x45, 0xdf, 0xa3])
    {
        return Ok(None);
    }
    if crate::owned_lossless::request(t).is_none() {
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
    // The shared video pipeline bakes stored crop/rotation before filtering;
    // companion audio retains its original timestamps independently.
    if !matches!(
        track.codec.as_str(),
        "V_FFV1" | "V_VP9" | "V_AV1" | "V_MPEGH/ISO/HEVC" | "V_MPEG4/ISO/AVC"
    ) {
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
            .is_some_and(|&d| d & !1 != 0)
        {
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
            if reader
                .packet_payload_size(index)
                .map_err(|e| e.to_string())?
                > o.max_packet_bytes
            {
                return Err("WebM packet exceeds budget or has empty lace frame".into());
            }
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
                        crate::owned_opus_packet::duration_ns_with_header(&track.codec_private,&bytes).map_err(|e| e.to_string())?
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
    crate::owned_lossless::metadata(o)?;
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
    let mut trimmed_audio_sample_frames = 0u64;
    let mut pcm_windows=Vec::new();
    let mut video_origin=0;
    if let Some(window) = t.interval {
        let number=reader.tracks[video].number;
        let origin=reader.packets.iter().filter(|p|p.track==number && !p.invisible).map(|p|p.pts_ns).min().ok_or("input has no visible video")?;
        video_origin=origin;
        for &index in &selected {
            if index==video {continue;}
            match audio_window::apply(&mut reader,index,origin,window,o)? {
                Some(removed)=>trimmed_audio_sample_frames=trimmed_audio_sample_frames.checked_add(removed).ok_or("trimmed sample count overflow")?,
                None=>pcm_windows.push(index),
            }
        }
    }
    Ok(Some(Input {
        reader,
        video,
        selected,
        trimmed_audio_sample_frames,
        pcm_windows,
        video_origin,
    }))
}
pub(crate) fn plan_selection(
    source: &Path,
    t: &LosslessTransform,
    o: &CopyOptions,
) -> Result<Option<(usize, Vec<usize>)>> {
    let Some(input) = qualify(source, t, o)? else {
        return Ok(None);
    };
    let Some(request) = crate::owned_lossless::request(t) else {
        return Ok(None);
    };
    if matches!(
        crate::owned_video_decode::try_webm(source, &request),
        Ok(None)
    ) {
        return Ok(None);
    }
    let mut stage = o.clone();
    stage.streams.clear();
    stage.stream_metadata_set.clear();
    stage.stream_metadata_delete.clear();
    let enforce_encoded_budget =
        o.max_packet_bytes != CopyOptions::default().max_packet_bytes;
    let mut check_encoded =
        |view: crate::owned_video_decode::FrameView<'_>| -> Result<()> {
            crate::owned_lossless::check_encoded_ffv1(view, o.max_packet_bytes)
        };
    let Some((stats, _)) = crate::owned_video_decode::decode_webm(
        source,
        &request,
        if enforce_encoded_budget {
            Some(&mut check_encoded)
        } else {
            None
        },
        Some(&stage),
    )? else {
        return Ok(None);
    };
    if stats.video_frames == 0 {
        return Err("Matroska has no selected frames".into());
    }
    Ok(Some((input.video, input.selected)))
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
    let mut private = crate::owned_video_temporary::Directory::new()?;
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
    let mut vt = video.tracks[0].clone();
    if vt.language.is_empty() {
        vt.language = video
            .track_languages
            .get(&vt.number)
            .or_else(|| video.track_legacy_languages.get(&vt.number))
            .cloned()
            .unwrap_or_else(|| "eng".into());
    }
    let mut pcm_readers=BTreeMap::new();
    if let Some(window)=t.interval {
        for &index in &input.pcm_windows {
            let track=input.reader.tracks[index].clone();
            let first=input.reader.packets.iter().find(|p|p.track==track.number).ok_or("audio track has no packets")?.pts_ns;
            let prepared=pcm_window::prepare(source,index,&track,first,input.video_origin,window,o,&mut private)?;
            let total=pcm_window::presentation_samples(&mut input.reader,index,prepared.rate,o)?;
            input.trimmed_audio_sample_frames=input.trimmed_audio_sample_frames.checked_add(total.saturating_sub(prepared.sample_frames)).ok_or("trimmed sample count overflow")?;
            let pcm=open(&prepared.path,o)?;
            let mut replacement=pcm.tracks[0].clone();replacement.number=track.number;
            input.reader.tracks[index]=replacement;
            pcm_readers.insert(index,pcm);
        }
    }
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
        tags.retain(|k, v| {
            !k.is_empty()
                && k.len() <= 128
                && v.len() <= 1024
                && !k.contains('\0')
                && !v.contains('\0')
        });

        if index != input.video {
            for (_, key, value) in o.stream_metadata_set.iter().filter(|(i, _, _)| *i == index) {
                tags.retain(|k, _| !k.eq_ignore_ascii_case(key));
                if key.eq_ignore_ascii_case("title") {
                    tracks[index].name = value.clone();
                } else if key.eq_ignore_ascii_case("language") {
                    tracks[index].language = if value.is_empty() {
                        "und".into()
                    } else {
                        value.clone()
                    };
                } else if !value.is_empty() {
                    tags.insert(key.to_ascii_uppercase(), value.clone());
                }
            }
            for (_, key) in o.stream_metadata_delete.iter().filter(|(i, _)| *i == index) {
                tags.retain(|k, _| !k.eq_ignore_ascii_case(key));
                if key.eq_ignore_ascii_case("title") {
                    tracks[index].name.clear();
                } else if key.eq_ignore_ascii_case("language") {
                    tracks[index].language = "und".into();
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
        } else if track.codec == "A_PCM/FLOAT/IEEE" {
            mkv::Encoding::PcmFloat32 {
                sample_rate:u32::try_from(track.sample_rate).map_err(|_|"PCM rate overflow")?,
                channels:u16::try_from(track.channels).map_err(|_|"PCM channel count overflow")?,
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
        } else if let Some(pcm)=pcm_readers.get(&index) {
            &pcm.packets
        } else {
            &input.reader.packets
        };
        let number = if index == input.video {
            vt.number
        } else if let Some(pcm)=pcm_readers.get(&index) {
            pcm.tracks[0].number
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
        let enabled: Vec<_> = input
            .selected
            .iter()
            .map(|&idx| input.reader.track_enabled(tracks[idx].number))
            .collect();
        let defaults: Vec<_> = input
            .selected
            .iter()
            .map(|&idx| {
                input
                    .reader
                    .track_dispositions
                    .get(&tracks[idx].number)
                    .copied()
                    .unwrap_or(1)
                    & 1
                    != 0
            })
            .collect();
        let mut writer = mkv::PacketWriter::new_with_track_preferences(
            output,
            &specs,
            &options,
            &metadata,
            &video.metadata,
            &scoped,
            &enabled,
            &defaults,
        )
        .map_err(|e| e.to_string())?;
        for &(_, mapped, index) in &schedule {
            check(o)?;
            let source_index=input.selected[mapped];
            let is_video = source_index == input.video;
            let is_pcm=pcm_readers.contains_key(&source_index);
            let reader = if is_video {
                &mut video
            } else if let Some(pcm)=pcm_readers.get_mut(&source_index) {
                pcm
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
            if !is_video && !is_pcm {
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
        trimmed_audio_sample_frames: input.trimmed_audio_sample_frames,
        pixel_format: decoded.pixel_format,
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: t.vertical_flip,
        horizontal_flip: t.horizontal_flip,
    }))
}
