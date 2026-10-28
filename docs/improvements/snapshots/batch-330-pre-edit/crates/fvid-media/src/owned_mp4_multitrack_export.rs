//! Own FFV1 video filtering plus byte-preserving AAC packet copy in source order.
use crate::owned_video_temporary::Directory;
use crate::{owned_matroska as mkv, owned_mp4::Mp4Reader};
use fvid_control::CopyOptions;
use fvid_media_info::{LosslessStats, LosslessTransform};
use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
type Reader = Mp4Reader<BufReader<File>>;
struct Input {
    reader: Reader,
    video: usize,
    count: u64,
    selected: Vec<usize>,
}
fn qualify(
    source: &Path,
    transform: &LosslessTransform,
    options: &CopyOptions,
) -> Result<Option<Input>> {
    if !crate::owned_mp4_video_bridge::recognizes(source) {
        return Ok(None);
    }
    if crate::owned_lossless::request(transform).is_none() {
        return Ok(None);
    }
    check(options)?;
    let reader = match Mp4Reader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        crate::owned_mp4::Limits {
            packet_bytes: options.max_controlled_bytes.map_or(
                options.max_packet_bytes,
                |limit| options.max_packet_bytes.min(limit),
            ),
            metadata_bytes: crate::owned_mp4::Limits::default()
                .metadata_bytes
                .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
            ..Default::default()
        },
    ) {
        Ok(reader) => reader,
        Err(e) if e.is_unsupported() => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let tracks = reader.tracks();
    if !(2..=126).contains(&tracks.len()) || !reader.refused().is_empty() {
        return Ok(None);
    }
    let videos: Vec<_> = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.handler == *b"vide")
        .map(|(i, _)| i)
        .collect();
    if videos.len() != 1 {
        return Ok(None);
    }
    let video = videos[0];
    let selected = crate::owned_mp4_matroska::selection(tracks.len(), &options.streams)
        .map_err(|e| e.to_string())?;
    if !selected.contains(&video) {
        return Err("lossless export requires at least one selected video stream".into());
    }

    if !matches!(tracks[video].rotation, 0 | 90 | 180 | 270)
        || !matches!(&tracks[video].codec, b"avc1" | b"avc3" | b"hvc1" | b"hev1")
    {
        return Ok(None);
    }
    for (i, track) in tracks.iter().enumerate() {
        if i != video
            && (track.handler != *b"soun"
                || track.codec != *b"mp4a"
                || !(track.edits.is_empty()
                    || (track.edits.len() == 1 && track.edits[0].media_time >= 0)))
        {
            return Ok(None);
        }
    }
    if options
        .stream_metadata_set
        .iter()
        .any(|(i, _, _)| *i >= tracks.len())
        || options
            .stream_metadata_delete
            .iter()
            .any(|(i, _)| *i >= tracks.len())
    {
        return Err("metadata refers to missing input track".into());
    }
    crate::owned_lossless::metadata(options)?;
    let mut normalized = options.clone();
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
    crate::owned_lossless::check_options_matroska_metadata_budget(options)?;
    let count = selected.iter().try_fold(0u64, |n, &i| {
        let t = &tracks[i];
        n.checked_add(t.samples.len() as u64)
            .ok_or("MP4 input packet count overflow")
    })?;
    if options.max_packets.is_some_and(|limit| count > limit) {
        return Err("MP4 input packet count exceeds limit".into());
    }
    let effective_packet_limit = options
        .max_controlled_bytes
        .map_or(options.max_packet_bytes, |limit| {
            options.max_packet_bytes.min(limit)
        });
    for &i in &selected {
        let t = &tracks[i];
        let exceeds = match &t.samples {
            crate::owned_mp4::SampleIndex::Expanded(samples) => {
                samples.iter().any(|s| s.size as usize > effective_packet_limit)
            }
            crate::owned_mp4::SampleIndex::Uniform(u) => {
                u.count > 0 && u.bytes_per_frame as usize > effective_packet_limit
            }
        };
        if exceeds {
            return Err("MP4 packet exceeds budget".into());
        }
    }
    Ok(Some(Input {
        reader,
        video,
        count,
        selected,
    }))
}
pub(crate) fn plan_selection(
    source: &Path,
    transform: &LosslessTransform,
    options: &CopyOptions,
) -> Result<Option<(usize, Vec<usize>)>> {
    let Some(input) = qualify(source, transform, options)? else {
        return Ok(None);
    };
    check(options)?;
    let Some(bridge) =
        crate::owned_mp4_video_bridge::prepare_video_controlled(source, Some(options))?
    else {
        return Ok(None);
    };
    let Some(request) = crate::owned_lossless::request(transform) else {
        return Ok(None);
    };
    if !crate::owned_ffv1_export::supports(&bridge.path, &request) {
        return Ok(None);
    }
    crate::owned_lossless::check_source_matroska_metadata_budget(
        &bridge.path,
        request.interval,
        options,
    )?;
    let mut stage = options.clone();
    stage.streams.clear();
    stage.max_packets = None;
    stage.stream_metadata_set.clear();
    stage.stream_metadata_delete.clear();
    let enforce_encoded_budget =
        options.max_packet_bytes != CopyOptions::default().max_packet_bytes
            || options.max_controlled_bytes.is_some();
    let mut check_encoded =
        |view: crate::owned_video_decode::FrameView<'_>| -> Result<()> {
            crate::owned_lossless::check_encoded_ffv1_with_controlled(
                view,
                options.max_packet_bytes,
                options.max_controlled_bytes,
            )
        };
    let Some((stats, _)) = crate::owned_video_decode::decode_webm(
        &bridge.path,
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
pub(crate) fn supports(
    source: &Path,
    transform: &LosslessTransform,
    options: &CopyOptions,
) -> bool {
    match qualify(source, transform, options) {
        Ok(Some(_)) => {
            match crate::owned_mp4_video_bridge::prepare_video_controlled(source, Some(options)) {
                Ok(Some(bridge)) => {
                    crate::owned_lossless::request(transform).is_some_and(|request| {
                        crate::owned_ffv1_export::supports(&bridge.path, &request)
                    })
                }
                Err(_) => true,
                Ok(None) => false,
            }
        }
        Err(_) => true,
        Ok(None) => false,
    }
}
fn check(options: &CopyOptions) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)
}
pub(crate) fn try_export(
    source: &Path,
    destination: &Path,
    transform: &LosslessTransform,
    options: &CopyOptions,
) -> Result<Option<LosslessStats>> {
    let Some(mut input) = qualify(source, transform, options)? else {
        return Ok(None);
    };
    check(options)?;
    if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err("lossless export requires FFV1 in .mkv".into());
    }
    if std::fs::symlink_metadata(destination).is_ok() {
        return Err("output already exists".into());
    }
    let Some(bridge) =
        crate::owned_mp4_video_bridge::prepare_video_controlled(source, Some(options))?
    else {
        return Ok(None);
    };
    if !crate::owned_lossless::request(transform)
        .is_some_and(|request| crate::owned_ffv1_export::supports(&bridge.path, &request))
    {
        return Ok(None);
    }
    let private = Directory::new()?;
    let video_path = private.0.join("video.mkv");
    let mut stage = options.clone();
    stage.progress = None;
    stage.streams.clear();
    stage.max_packets = None;
    stage.stream_metadata_set = options
        .stream_metadata_set
        .iter()
        .filter(|(i, _, _)| *i == input.video)
        .map(|(_, k, v)| (0, k.clone(), v.clone()))
        .collect();
    stage.stream_metadata_delete = options
        .stream_metadata_delete
        .iter()
        .filter(|(i, _)| *i == input.video)
        .map(|(_, k)| (0, k.clone()))
        .collect();
    let mut stats = crate::owned_lossless::transcode_lossless(
        &bridge.path,
        &video_path,
        transform.clone(),
        &stage,
    )?;
    drop(bridge);
    let mut limits = crate::owned_webm::Limits::default();
    limits.packet_bytes = limits.packet_bytes.min(options.max_packet_bytes);
    let mut video = crate::owned_webm::WebmReader::open(
        BufReader::new(File::open(&video_path).map_err(|e| e.to_string())?),
        limits,
    )
    .map_err(|e| e.to_string())?;
    video.restrict_packet_bytes(options.max_packet_bytes);
    video.scan_all().map_err(|e| e.to_string())?;
    let mut vt = video
        .tracks
        .first()
        .ok_or("filtered video has no track")?
        .clone();
    if vt.language.is_empty() {
        vt.language = video
            .track_languages
            .get(&vt.number)
            .or_else(|| video.track_legacy_languages.get(&vt.number))
            .cloned()
            .unwrap_or_else(|| "und".into());
    }
    let metadata = mkv::FileMetadata {
        tags: video.tags.clone(),
        chapters: video.chapters.clone(),
    };
    let mut tracks = input.reader.tracks().to_vec();
    let mut scoped = BTreeMap::new();
    if let Some(uid) = video.track_uids.get(&vt.number) {
        if let Some(tags) = video.track_metadata.get(uid) {
            scoped.insert(input.video, tags.clone());
        }
    }
    for (i, key, value) in &options.stream_metadata_set {
        if *i == input.video {
            continue;
        }
        if key.eq_ignore_ascii_case("title") {
            tracks[*i].name = value.clone();
        } else if key.eq_ignore_ascii_case("language") {
            tracks[*i].language = if value.is_empty() {
                "und".into()
            } else {
                value.clone()
            };
        } else {
            let tags = scoped.entry(*i).or_insert_with(BTreeMap::new);
            tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
            if !value.is_empty() {
                tags.insert(key.to_ascii_uppercase(), value.clone());
            }
        }
    }
    for (i, key) in &options.stream_metadata_delete {
        if *i == input.video {
            continue;
        }
        if key.eq_ignore_ascii_case("title") {
            tracks[*i].name.clear();
        } else if key.eq_ignore_ascii_case("language") {
            tracks[*i].language = "und".into();
        } else if let Some(tags) = scoped.get_mut(i) {
            tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
        }
    }
    let mut plans = (0..tracks.len()).map(|_| None).collect::<Vec<_>>();
    let mut specs = Vec::new();
    let mut track_options = Vec::new();
    let mut trimmed_audio_sample_frames = 0u64;
    for &i in &input.selected {
        let track = &tracks[i];
        if i == input.video {
            specs.push(mkv::TrackSpec {
                encoding: mkv::Encoding::Ffv1V1 {
                    width: u32::try_from(vt.width).map_err(|_| "video width overflow")?,
                    height: u32::try_from(vt.height).map_err(|_| "video height overflow")?,
                },
                name: &vt.name,
                language: &vt.language,
            });
            track_options.push(mkv::TrackOptions {
                video: Some(mkv::VideoMetadata {
                    crop: vt.crop.map(|v| v as u32),
                    pixel_aspect: vt.pixel_aspect(),
                    colour: Some(vt.colour),
                    hdr: vt.hdr,
                }),
                default_duration_ns: vt.default_duration_ns,
                ..Default::default()
            });
        } else {
            let plan = crate::owned_mp4_matroska::plan_window(
                track,
                input.reader.movie_timescale(),
                transform.interval,
                options.cancel.as_ref(),
            )
            .map_err(|e| e.to_string())?;
            trimmed_audio_sample_frames = trimmed_audio_sample_frames
                .checked_add(plan.trimmed_audio_sample_frames)
                .ok_or("trimmed sample count overflow")?;
            track_options.push(plan.options);
            specs.push(crate::owned_mp4_matroska::spec(track).map_err(|e| e.to_string())?);
            plans[i] = Some(plan);
        }
    }
    let scoped = scoped
        .into_iter()
        .filter_map(|(source, tags)| {
            input
                .selected
                .iter()
                .position(|&i| i == source)
                .map(|output| (output, tags))
        })
        .collect();
    let mut output_indices = vec![None; tracks.len()];
    for (output, &source) in input.selected.iter().enumerate() {
        output_indices[source] = Some(output);
    }
    let mut packets = Vec::new();
    packets
        .try_reserve_exact(
            video
                .packets
                .len()
                .checked_add(input.count as usize)
                .ok_or("packet schedule overflow")?,
        )
        .map_err(|_| "packet schedule allocation failed")?;
    for (index, packet) in video.packets.iter().enumerate() {
        if packet.pts_ns < 0 || packet.duration_ns.unwrap_or(vt.default_duration_ns) == 0 {
            return Err("invalid filtered video timing".into());
        }
        packets.push((i128::from(packet.pts_ns), input.video, index));
    }
    for (track, plan) in plans.iter().enumerate() {
        if let Some(plan) = plan {
            for (index, packet) in plan.packets.iter().enumerate() {
                packets.push((packet.dts, track, index));
            }
        }
    }
    packets.sort_unstable();
    let enabled: Vec<_> = input.selected.iter().map(|&index|
        crate::owned_mp4_matroska::effective_track_enabled(&input.reader,index)).collect();
    let defaults: Vec<_> = input.selected.iter().map(|&index|
        crate::owned_mp4_matroska::effective_track_default(&input.reader,index)).collect();
    let mut copied = 0;
    let event = crate::owned_matroska_remux::publish(destination, options, |output| {
        let mut writer = mkv::PacketWriter::new_with_track_preferences(
            output,
            &specs,
            &track_options,
            &metadata,
            &video.metadata,
            &scoped,
            &enabled,
            &defaults,
        )
        .map_err(|e| e.to_string())?;
        let mut payload = Vec::new();
        for &(_, track, index) in &packets {
            check(options)?;
            let mapped = output_indices[track].ok_or("missing selected output track")?;
            if track == input.video {
                let packet = video.packets[index].clone();
                let data = video.read_packet(index).map_err(|e| e.to_string())?;
                writer
                    .write_packet(
                        mapped,
                        packet.pts_ns as u64,
                        packet.duration_ns.unwrap_or(vt.default_duration_ns),
                        true,
                        &data,
                    )
                    .map_err(|e| e.to_string())?;
            } else {
                let packet = &plans[track].as_ref().unwrap().packets[index];
                input
                    .reader
                    .read_packet(track, index, &mut payload)
                    .map_err(|e| e.to_string())?;
                let sync = tracks[track]
                    .samples
                    .get(index)
                    .ok_or("missing MP4 audio sample")?
                    .sync;
                writer
                    .write_packet_with_options(
                        mapped,
                        packet.pts,
                        packet.duration,
                        sync,
                        &payload,
                        packet.options,
                    )
                    .map_err(|e| e.to_string())?;
                copied += 1;
            }
            if let Some(hook) = &options.progress {
                hook.emit(writer.event());
            }
        }
        writer.finish().map_err(|e| e.to_string())
    })?;
    stats.decoded_frames = input.reader.tracks()[input.video].samples.len() as u64;
    stats.copied_packets = copied;
    stats.trimmed_audio_sample_frames = trimmed_audio_sample_frames;
    stats.video_packets = video.packets.len() as u64;
    std::hint::black_box(event);
    Ok(Some(stats))
}
