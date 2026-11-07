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
    index_and_meta_bytes: usize,
}
fn open_from_file(file: File, options: &CopyOptions) -> Result<Reader> {
    let mut limits = webm::Limits::default();
    let memory_packets = options
        .max_controlled_bytes
        .filter(|&bytes| bytes > 0)
        .map(|bytes| bytes / std::mem::size_of::<webm::Packet>());
    if let Some(maximum) = memory_packets {
        limits.packets = limits.packets.min(maximum);
    }
    let map_packet_limit = |error: crate::owned_webm::Error| {
        if memory_packets.is_some_and(|max| max < webm::Limits::default().packets)
            && error
                .to_string()
                .contains("WebM packet count exceeds limit")
        {
            "controlled memory budget exceeded: Matroska packet index limit".to_owned()
        } else {
            error.to_string()
        }
    };
    let mut reader = webm::WebmReader::open(
        BufReader::new(file),
        limits,
    )
    .map_err(map_packet_limit)?;
    reader.scan_all().map_err(map_packet_limit)?;
    let effective_packet_limit = options
        .max_controlled_bytes
        .filter(|&limit| limit > 0)
        .map_or(options.max_packet_bytes, |limit| {
            options.max_packet_bytes.min(limit)
        });
    reader.restrict_packet_bytes(effective_packet_limit);
    Ok(reader)
}
fn open(path: &Path, options: &CopyOptions) -> Result<Reader> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    open_from_file(file, options)
}
fn check(o: &CopyOptions) -> Result<()> {
    if o.cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    if o.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if o.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if o.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(o)
}
fn check_multitrack_matroska_metadata_budget(
    reader: &Reader,
    video: usize,
    selected: &[usize],
    interval: Option<(i64, i64)>,
    options: &CopyOptions,
) -> Result<usize> {
    let vtrack = &reader.tracks[video];
    let mut description = mkv::VideoTrackDescription {
        name: vtrack.name.clone(),
        language: reader
            .track_languages
            .get(&vtrack.number)
            .cloned()
            .unwrap_or_else(|| "eng".into()),
        legacy_language: reader
            .track_legacy_languages
            .get(&vtrack.number)
            .cloned()
            .unwrap_or_else(|| "eng".into()),
        disposition: reader
            .track_dispositions
            .get(&vtrack.number)
            .copied()
            .unwrap_or(1),
    };
    let mut combined_track_tags = BTreeMap::new();
    let mut audio_desc_bytes = 0usize;
    for &index in selected {
        let track = &reader.tracks[index];
        let mut audio_name = track.name.clone();
        let mut audio_lang = if !track.language.is_empty() {
            track.language.clone()
        } else {
            reader
                .track_languages
                .get(&track.number)
                .or_else(|| reader.track_legacy_languages.get(&track.number))
                .cloned()
                .unwrap_or_else(|| "eng".into())
        };
        let mut tags = reader.track_metadata.get(&0).cloned().unwrap_or_default();
        if let Some(uid) = reader.track_uids.get(&track.number) {
            if let Some(scoped) = reader.track_metadata.get(uid) {
                tags.extend(scoped.clone());
            }
        }
        tags.retain(|k, v| {
            !k.is_empty()
                && k.len() <= 128
                && v.len() <= 1024
                && !k.contains('\0')
                && !v.contains('\0')
        });
        for (_, key, value) in options
            .stream_metadata_set
            .iter()
            .filter(|(i, _, _)| *i == index)
        {
            tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
            if index == video {
                if !crate::owned_ffv1_export::edit_track_description(&mut description, key, value)
                    && !value.is_empty()
                {
                    tags.insert(key.to_ascii_uppercase(), value.clone());
                }
            } else if key.eq_ignore_ascii_case("title") {
                audio_name = value.clone();
            } else if key.eq_ignore_ascii_case("language") {
                audio_lang = if value.is_empty() {
                    "und".into()
                } else {
                    value.clone()
                };
            } else if !value.is_empty() {
                tags.insert(key.to_ascii_uppercase(), value.clone());
            }
        }
        for (_, key) in options
            .stream_metadata_delete
            .iter()
            .filter(|(i, _)| *i == index)
        {
            tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
            if index == video {
                crate::owned_ffv1_export::edit_track_description(&mut description, key, "");
            } else if key.eq_ignore_ascii_case("title") {
                audio_name.clear();
            } else if key.eq_ignore_ascii_case("language") {
                audio_lang = "und".into();
            }
        }
        if tags.len() > 256 {
            return Err("Matroska text tag count exceeds limit".into());
        }
        if index != video {
            audio_desc_bytes = audio_desc_bytes
                .saturating_add(audio_name.len())
                .saturating_add(audio_lang.len().saturating_sub(3));
        }
        for (k, v) in tags {
            combined_track_tags.insert(format!("{index}:{k}"), v);
        }
    }
    let origin = reader
        .packets
        .iter()
        .filter(|p| p.track == vtrack.number && !p.invisible)
        .map(|p| p.pts_ns)
        .min()
        .unwrap_or(0);
    let mut file = mkv::FileMetadata {
        tags: reader.tags.clone(),
        chapters: crate::owned_ffv1_export::export_chapters(&reader.chapters, origin, interval)?,
    };
    let mut text_tags = reader.metadata.clone();
    if !reader.writing_app.is_empty()
        && reader.writing_app != "FVid"
        && !reader.writing_app.contains('\0')
        && reader.writing_app.len() <= 1024
        && !text_tags.keys().any(|k| k.eq_ignore_ascii_case("encoder"))
    {
        text_tags.insert("ENCODER".into(), reader.writing_app.clone());
    }
    text_tags.retain(|k, v| {
        !k.is_empty()
            && k.len() <= 128
            && v.len() <= 1024
            && !k.contains('\0')
            && !v.contains('\0')
    });
    for (key, value) in &options.metadata_set {
        text_tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
        if !file.tags.set(key, value) && !value.is_empty() {
            text_tags.insert(key.to_ascii_uppercase(), value.clone());
        }
    }
    for key in &options.metadata_delete {
        file.tags.set(key, "");
        text_tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
    }
    if text_tags.len() > 256 {
        return Err("Matroska text tag count exceeds limit".into());
    }
    let meta_working_bytes = crate::owned_lossless::estimate_matroska_metadata_working_bytes(
        &file,
        &text_tags,
        &combined_track_tags,
        Some(&description),
    )
    .saturating_add(audio_desc_bytes.saturating_mul(8));
    if options
        .max_controlled_bytes
        .filter(|&limit| limit > 0)
        .is_some_and(|limit| meta_working_bytes > limit)
    {
        return Err("Matroska metadata working set exceeds controlled memory budget".into());
    }
    Ok(meta_working_bytes)
}
fn qualify(path: &Path, t: &LosslessTransform, o: &CopyOptions) -> Result<Option<Input>> {
    use std::io::{Seek, SeekFrom};
    check(o)?;
    if crate::owned_lossless::request(t).is_none() {
        return Ok(None);
    }
    let Ok(mut file) = File::open(path) else {
        return Ok(None);
    };
    let mut prefix = [0; 4];
    if file.read_exact(&mut prefix).is_err() || prefix != [0x1a, 0x45, 0xdf, 0xa3] {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut reader = open_from_file(file, o)?;
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
            let packet_size = reader
                .packet_payload_size(index)
                .map_err(|e| e.to_string())?;
            if packet_size > o.max_packet_bytes {
                return Err("WebM packet exceeds budget or has empty lace frame".into());
            }
            if o.max_controlled_bytes
                .is_some_and(|limit| limit > 0 && packet_size > limit)
            {
                return Err("WebM packet exceeds controlled memory budget".into());
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
    let mut selected_options = o.clone();
    selected_options
        .stream_metadata_set
        .retain(|(i, _, _)| selected.contains(i));
    selected_options
        .stream_metadata_delete
        .retain(|(i, _)| selected.contains(i));
    crate::owned_lossless::check_options_matroska_metadata_budget(&selected_options)?;
    let meta_working_bytes =
        check_multitrack_matroska_metadata_budget(&reader, video, &selected, t.interval, o)?;
    let index_bytes = reader
        .estimated_index_payload_bytes()
        .map_err(|e| e.to_string())?;
    let index_and_meta_bytes = index_bytes.saturating_add(meta_working_bytes);
    if let Some(limit) = o.max_controlled_bytes.filter(|&limit| limit > 0) {
        if index_and_meta_bytes > limit {
            return Err("controlled memory budget exceeded: Matroska index and metadata".into());
        }
        for &i in &selected {
            let track_number = reader.tracks[i].number;
            for (index, p) in reader.packets.iter().enumerate() {
                if p.track == track_number {
                    let packet_size = reader
                        .packet_payload_size(index)
                        .map_err(|e| e.to_string())?;
                    if index_and_meta_bytes.saturating_add(packet_size) > limit {
                        return Err("WebM packet exceeds controlled memory budget".into());
                    }
                }
            }
        }
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
        index_and_meta_bytes,
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
    let mut stage = o.clone();
    stage.streams.clear();
    stage.stream_metadata_set.clear();
    stage.stream_metadata_delete.clear();
    let enforce_encoded_budget =
        o.max_packet_bytes != CopyOptions::default().max_packet_bytes
            || o.max_controlled_bytes.is_some();
    let mut max_encoded_video_packet_bytes = 0usize;
    let mut check_encoded =
        |view: crate::owned_video_decode::FrameView<'_>| -> Result<()> {
            let encoded_len = crate::owned_lossless::encode_ffv1_len_with_controlled(
                view,
                o.max_packet_bytes,
                o.max_controlled_bytes,
                input.index_and_meta_bytes,
            )?;
            max_encoded_video_packet_bytes = max_encoded_video_packet_bytes.max(encoded_len);
            Ok(())
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
    if let Some(limit) = o.max_controlled_bytes {
        let mut max_packet_bytes = max_encoded_video_packet_bytes;
        for &i in &input.selected {
            if i == input.video {
                continue;
            }
            let track_number = input.reader.tracks[i].number;
            for (index, p) in input.reader.packets.iter().enumerate() {
                if p.track == track_number {
                    max_packet_bytes = max_packet_bytes.max(
                        input
                            .reader
                            .packet_payload_size(index)
                            .map_err(|e| e.to_string())?,
                    );
                }
            }
        }
        let memory_packets = (limit / std::mem::size_of::<webm::Packet>())
            .min(webm::Limits::default().packets);
        if (stats.video_frames as usize) > memory_packets {
            return Err("controlled memory budget exceeded: Matroska packet index limit".into());
        }
        let mut video_packets_cap = 0usize;
        while video_packets_cap < stats.video_frames as usize {
            video_packets_cap = video_packets_cap
                .saturating_mul(2)
                .max(4)
                .min(memory_packets);
        }
        let mut video_tag_overhead = 0usize;
        for (key, value) in &o.metadata_set {
            if !crate::owned_file_tags::FileTags::supports_key(key) && !value.is_empty() {
                video_tag_overhead = video_tag_overhead
                    .saturating_add(4096)
                    .saturating_add(key.len())
                    .saturating_add(value.len());
            } else if !value.is_empty() {
                video_tag_overhead = video_tag_overhead.saturating_add(value.len());
            }
        }
        let mut has_video_scoped_tag = false;
        for (index, key, value) in &o.stream_metadata_set {
            if *index == input.video
                && !key.eq_ignore_ascii_case("title")
                && !key.eq_ignore_ascii_case("language")
                && !key.eq_ignore_ascii_case("default")
                && !key.eq_ignore_ascii_case("forced")
                && !key.eq_ignore_ascii_case("hearing_impaired")
                && !key.eq_ignore_ascii_case("visual_impaired")
                && !key.eq_ignore_ascii_case("original")
                && !key.eq_ignore_ascii_case("comment")
                && !value.is_empty()
            {
                has_video_scoped_tag = true;
                video_tag_overhead = video_tag_overhead
                    .saturating_add(4096)
                    .saturating_add(key.len())
                    .saturating_add(value.len());
            }
        }
        if has_video_scoped_tag {
            video_tag_overhead = video_tag_overhead.saturating_add(4096);
        }
        let video_index_bytes = (3usize * 4096)
            .saturating_add(4 * std::mem::size_of::<webm::Track>())
            .saturating_add(video_packets_cap.saturating_mul(std::mem::size_of::<webm::Packet>()))
            .saturating_add(16)
            .saturating_add(video_tag_overhead);
        let schedule_len = input
            .reader
            .packets
            .len()
            .checked_add(stats.video_frames as usize)
            .ok_or("packet schedule overflow")?;
        let schedule_bytes = schedule_len
            .checked_mul(std::mem::size_of::<(i128, usize, usize)>())
            .ok_or("packet schedule overflow")?;
        if input
            .index_and_meta_bytes
            .saturating_add(video_index_bytes)
            .saturating_add(schedule_bytes)
            .saturating_add(max_packet_bytes)
            > limit
        {
            return Err("controlled memory budget exceeded: Matroska packet schedule limit".into());
        }
    }
    Ok(Some((input.video, input.selected)))
}
pub(crate) fn supports(source: &Path, t: &LosslessTransform, o: &CopyOptions) -> bool {
    if check(o).is_err() {
        return false;
    }
    match qualify(source, t, o) {
        Ok(Some(_)) => !matches!(
            crate::owned_video_decode::decode_webm(
                source,
                &crate::owned_lossless::request(t).unwrap(),
                None,
                Some(o),
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
    if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err("lossless export requires FFV1 in .mkv".into());
    }
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    let Some(mut input) = qualify(source, t, o)? else {
        return Ok(None);
    };
    let request = crate::owned_lossless::request(t).unwrap();
    if matches!(
        crate::owned_video_decode::decode_webm(source, &request, None, Some(o)),
        Ok(None)
    ) {
        return Ok(None);
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
    let stage_base_bytes = if o.max_controlled_bytes.is_some() {
        crate::owned_lossless::check_source_matroska_metadata_budget(
            source,
            request.interval,
            &stage,
        )?
    } else {
        0
    };
    let extra_base_bytes = input.index_and_meta_bytes.saturating_sub(stage_base_bytes);
    let (decoded, _, consumed) = crate::owned_ffv1_export::export_processed(
        source,
        &video_path,
        &request,
        &stage,
        None,
        extra_base_bytes,
    )?;
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
    let mut pcm_index_bytes = 0usize;
    let video_index_bytes = if o.max_controlled_bytes.is_some() {
        video
            .estimated_index_payload_bytes()
            .map_err(|e| e.to_string())?
    } else {
        0
    };
    if let Some(window)=t.interval {
        for &index in &input.pcm_windows {
            let track=input.reader.tracks[index].clone();
            let first=input.reader.packets.iter().find(|p|p.track==track.number).ok_or("audio track has no packets")?.pts_ns;
            let base_working_bytes = input
                .index_and_meta_bytes
                .saturating_add(video_index_bytes)
                .saturating_add(pcm_index_bytes);
            let prepared=pcm_window::prepare(source,index,&track,first,input.video_origin,window,o,base_working_bytes,&mut private)?;
            let total=pcm_window::presentation_samples(&mut input.reader,index,prepared.rate,o)?;
            input.trimmed_audio_sample_frames=input.trimmed_audio_sample_frames.checked_add(total.saturating_sub(prepared.sample_frames)).ok_or("trimmed sample count overflow")?;
            let pcm=open(&prepared.path,o)?;
            if o.max_controlled_bytes.is_some() {
                pcm_index_bytes = pcm_index_bytes.saturating_add(
                    pcm.estimated_index_payload_bytes()
                        .map_err(|e| e.to_string())?,
                );
            }
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
    let schedule_len = input
        .reader
        .packets
        .len()
        .checked_add(video.packets.len())
        .ok_or("packet schedule overflow")?;
    if let Some(limit) = o.max_controlled_bytes {
        let schedule_bytes = schedule_len
            .checked_mul(std::mem::size_of::<(i128, usize, usize)>())
            .ok_or("packet schedule overflow")?;
        if input
            .index_and_meta_bytes
            .saturating_add(video_index_bytes)
            .saturating_add(pcm_index_bytes)
            .saturating_add(schedule_bytes)
            > limit
        {
            return Err("controlled memory budget exceeded: Matroska packet schedule limit".into());
        }
    }
    schedule
        .try_reserve_exact(schedule_len)
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
            if let Some(limit) = o.max_controlled_bytes.filter(|&limit| limit > 0) {
                let schedule_bytes =
                    schedule.len().saturating_mul(std::mem::size_of::<(i128, usize, usize)>());
                if input
                    .index_and_meta_bytes
                    .saturating_add(video_index_bytes)
                    .saturating_add(pcm_index_bytes)
                    .saturating_add(schedule_bytes)
                    .saturating_add(bytes.len())
                    > limit
                {
                    return Err("controlled memory budget exceeded: Matroska packet schedule limit".into());
                }
            }
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
