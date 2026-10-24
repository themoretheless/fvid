//! Atomic MP4-to-Matroska packet copy through owned demuxing and muxing.
use fvid_control::CopyOptions;
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
fn policies(options: &CopyOptions) -> bool {
    options.max_packet_bytes > 0
        && options.max_controlled_bytes.is_none()
        && options.max_rss_bytes.is_none()
        && options.stream_metadata_set.iter().all(|(_, key, value)| track_key(key) && !value.contains('\0'))
        && options.stream_metadata_delete.iter().all(|(_, key)| track_key(key))
        && options.metadata_set.len().saturating_add(options.metadata_delete.len()) <= 64
        && options.metadata_set.iter().all(|(key, value)| {
            !key.contains('\0') && !value.contains('\0')
                && crate::owned_file_tags::FileTags::supports_key(key)
        })
        && options.metadata_delete.iter().all(|key| {
            !key.contains('\0') && crate::owned_file_tags::FileTags::supports_key(key)
        })
}
fn track_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("title") || key.eq_ignore_ascii_case("language")
}
fn track_policy(options: &CopyOptions, selected: &[usize]) -> bool {
    options.stream_metadata_set.iter().all(|(index, _, _)| selected.contains(index))
        && options.stream_metadata_delete.iter().all(|(index, _)| selected.contains(index))
        && selected.iter().all(|index| {
            options.stream_metadata_set.iter().filter(|(i, _, _)| i == index).count()
                .saturating_add(options.stream_metadata_delete.iter().filter(|(i, _)| i == index).count()) <= 64
        })
}
fn track_metadata<'a>(options: &'a CopyOptions, selected: &[usize]) -> Vec<crate::owned_mp4_matroska::TrackMetadataOverride<'a>> {
    selected.iter().filter_map(|&index| {
        let mut edit = crate::owned_mp4_matroska::TrackMetadataOverride { index, name: None, language: None };
        for (_, key) in options.stream_metadata_delete.iter().filter(|(i, _)| *i == index) {
            if key.eq_ignore_ascii_case("title") { edit.name = Some(""); } else { edit.language = Some(""); }
        }
        for (_, key, value) in options.stream_metadata_set.iter().filter(|(i, _, _)| *i == index) {
            if key.eq_ignore_ascii_case("title") { edit.name = Some(value); } else { edit.language = Some(value); }
        }
        (edit.name.is_some() || edit.language.is_some()).then_some(edit)
    }).collect()
}
fn metadata<R: std::io::Read + std::io::Seek>(
    reader: &crate::owned_mp4::Mp4Reader<R>, options: &CopyOptions,
) -> crate::owned_matroska::FileMetadata {
    let mut metadata = crate::owned_matroska::FileMetadata::from_mp4(reader);
    for key in &options.metadata_delete { metadata.tags.set(key, ""); }
    for (key, value) in &options.metadata_set { metadata.tags.set(key, value); }
    metadata
}
fn open(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_mp4::Mp4Reader<BufReader<File>>> {
    let file = File::open(source).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("MP4 remux requires a regular file".into());
    }
    crate::owned_mp4::Mp4Reader::open(
        BufReader::new(file),
        crate::owned_mp4::Limits {
            packet_bytes: options.max_packet_bytes,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn supports(source: &Path, destination: &Path, options: &CopyOptions) -> bool {
    if !policies(options)
        || !matches!(
            destination.extension().and_then(|s| s.to_str()),
            Some("mkv" | "mka")
        )
    {
        return false;
    }
    let mut prefix = [0; 8];
    if !File::open(source).is_ok_and(|mut f| {
        f.read_exact(&mut prefix).is_ok() && crate::owned_mp4::recognizes_prefix(&prefix)
    }) {
        return false;
    }
    let Ok(reader) = open(source, options) else {
        return false;
    };
    let Ok(selected) = crate::owned_mp4_matroska::selection(reader.tracks().len(), &options.streams) else {
        return false;
    };
    if !track_policy(options, &selected) { return false; }
    if !crate::owned_mp4_matroska::eligible(&reader)
        || (destination.extension().and_then(|s| s.to_str()) == Some("mka")
            && selected.iter().any(|&index| reader.tracks()[index].handler != *b"soun"))
    {
        return false;
    }
    selected.iter().all(|&index| {
        let t = &reader.tracks()[index];
        crate::owned_mp4_matroska::plan(t, reader.movie_timescale(), options.cancel.as_ref())
            .is_ok()
            && crate::owned_mp4_matroska::spec(t).is_ok()
    })
}
/// Copy supported AVC/HEVC/AAC packets unchanged with their presentation edits.
/// Known container tags and track title/language may be deleted/set; aggregate/RSS policies
/// remain outside this owned route.
pub fn remux(
    source: &Path,
    destination: &Path,
    options: &CopyOptions,
) -> Result<fvid_media_info::CopyStats> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if !supports(source, destination, options) {
        return Err("request is not an owned MP4 Matroska remux".into());
    }
    let mut reader = open(source, options)?;
    let metadata = metadata(&reader, options);
    let selected = crate::owned_mp4_matroska::selection(reader.tracks().len(), &options.streams).map_err(|e| e.to_string())?;
    let tracks = track_metadata(options, &selected);
    crate::owned_matroska_remux::publish(destination, options, |output| {
        crate::owned_mp4_matroska::write_selected_with_metadata_overrides(
            &mut reader,
            output,
            options.cancel.as_ref(),
            options.progress.as_ref(),
            options.max_packets,
            &options.streams,
            crate::owned_mp4_matroska::RemuxMetadata { file: &metadata, tracks: &tracks },
        )
        .map_err(|e| e.to_string())
    })
}

/// Metadata-only MP4 packet-copy plan; no output or progress callbacks.
pub fn plan_remux(source: &Path, options: &CopyOptions) -> Result<fvid_media_info::MediaPlan> {
    use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if !supports(source, Path::new("planned.mkv"), options) {
        return Err("request is not an owned MP4 Matroska remux".into());
    }
    let reader = open(source, options)?;
    let mut packets = 0u64;
    let mut payload = 0u64;
    let selected = crate::owned_mp4_matroska::selection(reader.tracks().len(), &options.streams).map_err(|e| e.to_string())?;
    let plans = selected.iter().map(|&index| {
        let track = &reader.tracks()[index];
        crate::owned_mp4_matroska::plan(track, reader.movie_timescale(), options.cancel.as_ref())
            .map_err(|e| e.to_string())
    }).collect::<Result<Vec<_>>>()?;
    let mut queue = std::collections::BinaryHeap::new();
    for (track, plan) in plans.iter().enumerate() {
        if let Some(first) = plan.packets.first() {
            queue.push(std::cmp::Reverse((first.dts, selected[track], track, 0usize)));
        }
    }
    while let Some(std::cmp::Reverse((_, source_track, track, index))) = queue.pop() {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        if options.max_packets.is_some_and(|limit| packets >= limit) { break; }
        let sample = reader.tracks()[source_track].samples.get(index).ok_or("missing MP4 sample")?;
        packets = packets.checked_add(1).ok_or("MP4 packet count overflow")?;
        payload = payload.checked_add(u64::from(sample.size)).ok_or("MP4 byte count overflow")?;
        if let Some(next) = plans[track].packets.get(index + 1) {
            queue.push(std::cmp::Reverse((next.dts, source_track, track, index + 1)));
        }
    }
    Ok(MediaPlan {
        command: "remux".into(), input: source.into(), inputs: vec![source.into()],
        streams: selected.iter().map(|&index| { let track = &reader.tracks()[index]; PlanStream {
            index, media_type: if track.handler == *b"vide" { "video" } else { "audio" }.into(),
            codec: match &track.codec { b"avc1" | b"avc3" => "h264", b"hvc1" | b"hev1" => "hevc", _ => "aac" }.into(), disposition: "copy".into(),
        }}).collect(),
        steps: vec![
            PlanStep { action: "demux".into(), detail: "owned indexed MP4 reader with bounded metadata and packet payloads".into() },
            PlanStep { action: "timestamps".into(), detail: "preserve decode order; interleave edited DTS; retain B-frame presentation timing, AAC delay and signed tail padding".into() },
            PlanStep { action: "copy".into(), detail: format!("copy {packets} compressed packets, {payload} payload bytes unchanged into owned Matroska") },
            PlanStep { action: "metadata".into(), detail: format!("retain file tags/chapters and video colour/HDR/display metadata; delete {} and set {} container tags; delete {} and set {} track title/language fields by original source index", options.metadata_delete.len(), options.metadata_set.len(), options.stream_metadata_delete.len(), options.stream_metadata_set.len()) },
            PlanStep { action: "publish".into(), detail: "publish complete output without overwriting; remove temporary file on error or cancellation".into() },
        ], graph: None,
        notes: vec!["backend: owned MP4/Matroska; no external demuxer or muxer".into(), "destination must be .mkv or audio-only .mka; publication and packet payload reads are verified during execution".into(), "selected original stream indexes retain requested output order; optional global packet cap retains a DTS-interleaved prefix; known container tags and track title/language are deleted before assignments; other track tag edits and aggregate/RSS policies remain unsupported".into()],
    })
}
