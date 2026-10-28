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
        && options.max_controlled_bytes != Some(0)
        && crate::owned_budget::check_rss_budget(options).is_ok()
        && options.stream_metadata_set.iter().all(|(_, key, value)| {
            track_key(key)
                && !value.contains('\0')
                && value.len() <= 1024
                && (!key.eq_ignore_ascii_case("language") || value.len() <= 128)
        })
        && options.stream_metadata_delete.iter().all(|(_, key)| track_key(key))
        && options.metadata_set.len().saturating_add(options.metadata_delete.len()) <= 64
        && options.metadata_set.iter().all(|(key, value)| {
            !key.is_empty()
                && key.len() <= 128
                && !key.contains('\0')
                && !value.contains('\0')
                && (crate::owned_file_tags::FileTags::supports_key(key) || value.len() <= 1024)
        })
        && options.metadata_delete.iter().all(|key| {
            !key.is_empty() && key.len() <= 128 && !key.contains('\0')
        })
}
fn track_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && !key.contains('\0')
        && !matches!(
            key.to_ascii_lowercase().as_str(),
            "rotate" | "stereo_mode" | "alpha_mode"
        )
}
fn track_policy(options: &CopyOptions, selected: &[usize]) -> bool {
    options.stream_metadata_set.iter().all(|(index, _, _)| selected.contains(index))
        && options.stream_metadata_delete.iter().all(|(index, _)| selected.contains(index))
        && selected.iter().all(|index| {
            options.stream_metadata_set.iter().filter(|(i, _, _)| i == index).count()
                .saturating_add(options.stream_metadata_delete.iter().filter(|(i, _)| i == index).count()) <= 64
        })
}
fn track_metadata<'a>(
    options: &'a CopyOptions,
    selected: &[usize],
) -> (
    Vec<crate::owned_mp4_matroska::TrackMetadataOverride<'a>>,
    std::collections::BTreeMap<usize, std::collections::BTreeMap<String, String>>,
) {
    let mut overrides = Vec::new();
    let mut scoped: std::collections::BTreeMap<usize, std::collections::BTreeMap<String, String>> =
        std::collections::BTreeMap::new();
    for (output_index, &index) in selected.iter().enumerate() {
        let mut edit = crate::owned_mp4_matroska::TrackMetadataOverride {
            index,
            name: None,
            language: None,
        };
        let mut tags = std::collections::BTreeMap::<String, String>::new();
        for (_, key, value) in options.stream_metadata_set.iter().filter(|(i, _, _)| *i == index) {
            if key.eq_ignore_ascii_case("title") {
                edit.name = Some(value);
            } else if key.eq_ignore_ascii_case("language") {
                edit.language = Some(value);
            } else {
                tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
                if !value.is_empty() {
                    tags.insert(key.to_ascii_uppercase(), value.clone());
                }
            }
        }
        for (_, key) in options.stream_metadata_delete.iter().filter(|(i, _)| *i == index) {
            if key.eq_ignore_ascii_case("title") {
                edit.name = Some("");
            } else if key.eq_ignore_ascii_case("language") {
                edit.language = Some("");
            } else {
                tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
            }
        }
        if edit.name.is_some() || edit.language.is_some() {
            overrides.push(edit);
        }
        if !tags.is_empty() {
            scoped.insert(output_index, tags);
        }
    }
    (overrides, scoped)
}
fn metadata<R: std::io::Read + std::io::Seek>(
    reader: &crate::owned_mp4::Mp4Reader<R>,
    options: &CopyOptions,
) -> (
    crate::owned_matroska::FileMetadata,
    std::collections::BTreeMap<String, String>,
) {
    let mut metadata = crate::owned_matroska::FileMetadata::from_mp4(reader);
    let mut text = std::collections::BTreeMap::<String, String>::new();
    let encoder = reader.encoder().trim();
    if !encoder.is_empty() && encoder.len() <= 1024 && !encoder.contains('\0') {
        text.insert("ENCODER".into(), encoder.to_owned());
    }
    for (key, value) in &options.metadata_set {
        text.retain(|name, _| !name.eq_ignore_ascii_case(key));
        if !metadata.tags.set(key, value) && !value.is_empty() {
            text.insert(key.to_ascii_uppercase(), value.clone());
        }
    }
    for key in &options.metadata_delete {
        metadata.tags.set(key, "");
        text.retain(|name, _| !name.eq_ignore_ascii_case(key));
    }
    (metadata, text)
}
fn open(
    source: &Path,
    options: &CopyOptions,
) -> crate::owned_mp4::Result<crate::owned_mp4::Mp4Reader<BufReader<File>>> {
    let file = File::open(source)?;
    if !file.metadata()?.is_file() {
        return Err(crate::owned_mp4::invalid("MP4 remux requires a regular file"));
    }
    let mut limits = crate::owned_mp4::Limits::default();
    if let Some(limit) = options.max_controlled_bytes {
        limits.metadata_bytes = limits.metadata_bytes.min(limit);
    }
    let mut reader = crate::owned_mp4::Mp4Reader::open(BufReader::new(file), limits)?;
    reader.restrict_packet_bytes(
        options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
    );
    Ok(reader)
}
fn validate_selected_prefix_packet_budget<R: std::io::Read + std::io::Seek>(
    reader: &crate::owned_mp4::Mp4Reader<R>,
    selected: &[usize],
    options: &CopyOptions,
) -> Result<(u64, u64)> {
    let plans = selected
        .iter()
        .map(|&index| {
            let track = &reader.tracks()[index];
            crate::owned_mp4_matroska::plan(
                track,
                reader.movie_timescale(),
                options.cancel.as_ref(),
            )
            .map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>>>()?;
    if let Some(mem_limit) = options.max_controlled_bytes {
        let (file_meta, text_tags) = metadata(reader, options);
        let (overrides, scoped_tags) = track_metadata(options, selected);
        let mut raw_meta_bytes = 0usize;
        for v in [
            &file_meta.tags.title,
            &file_meta.tags.artist,
            &file_meta.tags.album,
            &file_meta.tags.comment,
            &file_meta.tags.date,
        ] {
            if !v.is_empty() {
                raw_meta_bytes = raw_meta_bytes.saturating_add(v.len()).saturating_add(32);
            }
        }
        for c in &file_meta.chapters {
            raw_meta_bytes = raw_meta_bytes.saturating_add(c.title.len()).saturating_add(64);
        }
        for (k, v) in &text_tags {
            raw_meta_bytes = raw_meta_bytes
                .saturating_add(k.len())
                .saturating_add(v.len())
                .saturating_add(32);
        }
        for o in &overrides {
            if let Some(n) = o.name {
                raw_meta_bytes = raw_meta_bytes.saturating_add(n.len()).saturating_add(16);
            }
            if let Some(l) = o.language {
                raw_meta_bytes = raw_meta_bytes.saturating_add(l.len()).saturating_add(16);
            }
        }
        for entries in scoped_tags.values() {
            for (k, v) in entries {
                raw_meta_bytes = raw_meta_bytes
                    .saturating_add(k.len())
                    .saturating_add(v.len())
                    .saturating_add(32);
            }
        }
        let base_bytes = 16384usize.saturating_add(raw_meta_bytes.saturating_mul(8));
        let plan_bytes = plans.iter().try_fold(base_bytes, |sum, p| {
            // Edits can shorten the initialized plan without releasing its
            // reserved timing storage. Admit the allocation that remains live.
            p.packets
                .capacity()
                .checked_mul(std::mem::size_of::<crate::owned_mp4_matroska::PacketTime>())
                .and_then(|n| sum.checked_add(n))
                .ok_or("MP4 packet plan memory overflow")
        })?;
        if plan_bytes > mem_limit {
            return Err("controlled memory budget exceeded".into());
        }
    }
    let mut queue = std::collections::BinaryHeap::new();
    for (track, plan) in plans.iter().enumerate() {
        if let Some(first) = plan.packets.first() {
            queue.push(std::cmp::Reverse((first.dts, selected[track], track, 0usize)));
        }
    }
    let mut packets = 0u64;
    let mut payload = 0u64;
    while let Some(std::cmp::Reverse((_, source_track, track, index))) = queue.pop() {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        if options.max_packets.is_some_and(|limit| packets >= limit) {
            break;
        }
        let sample = reader.tracks()[source_track]
            .samples
            .get(index)
            .ok_or("missing MP4 sample")?;
        if (sample.size as usize) > options.max_packet_bytes {
            return Err("MP4 packet exceeds budget".into());
        }
        if options
            .max_controlled_bytes
            .is_some_and(|limit| (sample.size as usize).saturating_add(16384) > limit)
        {
            return Err("controlled memory budget exceeded".into());
        }
        packets = packets.checked_add(1).ok_or("MP4 packet count overflow")?;
        payload = payload
            .checked_add(u64::from(sample.size))
            .ok_or("MP4 byte count overflow")?;
        if let Some(next) = plans[track].packets.get(index + 1) {
            queue.push(std::cmp::Reverse((
                next.dts,
                source_track,
                track,
                index + 1,
            )));
        }
    }
    Ok((packets, payload))
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
    let reader = match open(source, options) {
        Ok(reader) => reader,
        // Recognized MP4 parse/resource failures belong to this route. Keep
        // unsupported sample entries available to other owned adapters.
        Err(error) => return !error.is_unsupported(),
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
    if options.max_packets == Some(0) {
        return Err("Matroska output requires at least one packet".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if !supports(source, destination, options) {
        return Err("request is not an owned MP4 Matroska remux".into());
    }
    let mut reader = open(source, options).map_err(|e| e.to_string())?;
    let (metadata, text) = metadata(&reader, options);
    let selected = crate::owned_mp4_matroska::selection(reader.tracks().len(), &options.streams).map_err(|e| e.to_string())?;
    validate_selected_prefix_packet_budget(&reader, &selected, options)?;
    let (tracks, scoped) = track_metadata(options, &selected);
    crate::owned_matroska_remux::publish(destination, options, |output| {
        crate::owned_mp4_matroska::write_selected_with_text_metadata_overrides(
            &mut reader,
            output,
            options.cancel.as_ref(),
            options.progress.as_ref(),
            options.max_packets,
            &options.streams,
            crate::owned_mp4_matroska::RemuxMetadata { file: &metadata, tracks: &tracks },
            &text,
            &scoped,
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
    if options.max_packets == Some(0) {
        return Err("Matroska output requires at least one packet".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if !supports(source, Path::new("planned.mkv"), options) {
        return Err("request is not an owned MP4 Matroska remux".into());
    }
    let reader = open(source, options).map_err(|e| e.to_string())?;
    let selected = crate::owned_mp4_matroska::selection(reader.tracks().len(), &options.streams).map_err(|e| e.to_string())?;
    let (packets, payload) = validate_selected_prefix_packet_budget(&reader, &selected, options)?;
    let mut notes = vec![
        "backend: owned MP4/Matroska; no external demuxer or muxer".into(),
        "destination must be .mkv or audio-only .mka; publication and packet payload reads are verified during execution".into(),
        "selected original stream indexes retain requested output order; optional global packet cap retains a DTS-interleaved prefix; known container tags and track title/language are deleted before assignments; other track tag edits and aggregate/RSS policies remain unsupported".into(),
    ];
    if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
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
        notes,
    })
}
