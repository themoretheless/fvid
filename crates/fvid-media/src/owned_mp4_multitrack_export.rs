//! Own FFV1 video filtering plus byte-preserving AAC packet copy in source order.
use crate::{owned_matroska as mkv, owned_mp4::Mp4Reader};
use fvid_control::CopyOptions;
use fvid_media_info::{LosslessStats, LosslessTransform};
use std::{
    collections::BTreeMap,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
type Result<T> = std::result::Result<T, String>;
type Reader = Mp4Reader<BufReader<File>>;
struct Input {
    reader: Reader,
    video: usize,
    count: u64,
}
fn qualify(
    source: &Path,
    transform: &LosslessTransform,
    options: &CopyOptions,
) -> Result<Option<Input>> {
    if !crate::owned_mp4_video_bridge::recognizes(source) {
        return Ok(None);
    }
    if !options.streams.is_empty() || crate::owned_lossless::request(transform).is_none() {
        return Ok(None);
    }
    let mut normalized = options.clone();
    for (i, _, _) in &mut normalized.stream_metadata_set {
        *i = 0;
    }
    for (i, _) in &mut normalized.stream_metadata_delete {
        *i = 0;
    }
    if !crate::owned_lossless::policy(&normalized, true) {
        return Ok(None);
    }
    let reader = match Mp4Reader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        crate::owned_mp4::Limits {
            packet_bytes: options.max_packet_bytes,
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
    if tracks[video].rotation != 0
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
        return Ok(None);
    }
    let count = tracks.iter().try_fold(0u64, |n, t| {
        n.checked_add(t.samples.len() as u64)
            .ok_or("MP4 input packet count overflow")
    })?;
    if options.max_packets.is_some_and(|limit| count > limit) {
        return Err("MP4 input packet count exceeds limit".into());
    }
    Ok(Some(Input {
        reader,
        video,
        count,
    }))
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
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Result<Self> {
        loop {
            let path = std::env::temp_dir().join(format!(
                "fvid-mp4-multitrack-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("video.mkv"));
        let _ = std::fs::remove_dir(&self.0);
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
    Ok(())
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
    let mut video = crate::owned_webm::WebmReader::open(
        BufReader::new(File::open(&video_path).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    video.restrict_packet_bytes(options.max_packet_bytes);
    video.scan_all().map_err(|e| e.to_string())?;
    let vt = video
        .tracks
        .first()
        .ok_or("filtered video has no track")?
        .clone();
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
    for (i, key) in &options.stream_metadata_delete {
        if *i == input.video {
            continue;
        }
        if key.eq_ignore_ascii_case("title") {
            tracks[*i].name.clear();
        } else if key.eq_ignore_ascii_case("language") {
            tracks[*i].language = "und".into();
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
        } else if !value.is_empty() {
            scoped
                .entry(*i)
                .or_insert_with(BTreeMap::new)
                .insert(key.to_ascii_uppercase(), value.clone());
        }
    }
    let mut plans = Vec::new();
    let mut specs = Vec::new();
    let mut track_options = Vec::new();
    for (i, track) in tracks.iter().enumerate() {
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
            plans.push(None);
        } else {
            let plan = crate::owned_mp4_matroska::plan_window(
                track,
                input.reader.movie_timescale(),
                transform.interval,
                options.cancel.as_ref(),
            )
            .map_err(|e| e.to_string())?;
            track_options.push(plan.options);
            specs.push(crate::owned_mp4_matroska::spec(track).map_err(|e| e.to_string())?);
            plans.push(Some(plan));
        }
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
    let mut copied = 0;
    let event = crate::owned_matroska_remux::publish(destination, options, |output| {
        let mut writer = mkv::PacketWriter::new_with_text_metadata(
            output,
            &specs,
            &track_options,
            &metadata,
            &video.metadata,
            &scoped,
        )
        .map_err(|e| e.to_string())?;
        let mut payload = Vec::new();
        for &(_, track, index) in &packets {
            check(options)?;
            if track == input.video {
                let packet = video.packets[index].clone();
                let data = video.read_packet(index).map_err(|e| e.to_string())?;
                writer
                    .write_packet(
                        track,
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
                        track,
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
    stats.video_packets = video.packets.len() as u64;
    std::hint::black_box(event);
    Ok(Some(stats))
}
