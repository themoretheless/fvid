//! Atomic MP4-to-Matroska packet copy through owned demuxing and muxing.
use fvid_control::CopyOptions;
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
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
    if !(options.streams.is_empty() && crate::owned_matroska_remux::unedited_policies(options))
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
    if !crate::owned_mp4_matroska::eligible(&reader)
        || (destination.extension().and_then(|s| s.to_str()) == Some("mka")
            && reader.tracks().iter().any(|t| t.handler != *b"soun"))
    {
        return false;
    }
    reader.tracks().iter().all(|t| {
        crate::owned_mp4_matroska::plan(t, reader.movie_timescale(), options.cancel.as_ref())
            .is_ok()
            && crate::owned_mp4_matroska::spec(t).is_ok()
    })
}
/// Copy supported AVC/HEVC/AAC packets unchanged with their presentation edits.
/// Unsupported stream edits and aggregate/RSS policies remain explicit refusals.
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
    crate::owned_matroska_remux::publish(destination, options, |output| {
        crate::owned_mp4_matroska::write_limited(
            &mut reader,
            output,
            options.cancel.as_ref(),
            options.progress.as_ref(),
            options.max_packets,
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
    let plans = reader.tracks().iter().map(|track| {
        crate::owned_mp4_matroska::plan(track, reader.movie_timescale(), options.cancel.as_ref())
            .map_err(|e| e.to_string())
    }).collect::<Result<Vec<_>>>()?;
    let mut queue = std::collections::BinaryHeap::new();
    for (track, plan) in plans.iter().enumerate() {
        if let Some(first) = plan.packets.first() {
            queue.push(std::cmp::Reverse((first.dts, track, 0usize)));
        }
    }
    while let Some(std::cmp::Reverse((_, track, index))) = queue.pop() {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        if options.max_packets.is_some_and(|limit| packets >= limit) { break; }
        let sample = reader.tracks()[track].samples.get(index).ok_or("missing MP4 sample")?;
        packets = packets.checked_add(1).ok_or("MP4 packet count overflow")?;
        payload = payload.checked_add(u64::from(sample.size)).ok_or("MP4 byte count overflow")?;
        if let Some(next) = plans[track].packets.get(index + 1) {
            queue.push(std::cmp::Reverse((next.dts, track, index + 1)));
        }
    }
    Ok(MediaPlan {
        command: "remux".into(), input: source.into(), inputs: vec![source.into()],
        streams: reader.tracks().iter().enumerate().map(|(index, track)| PlanStream {
            index, media_type: if track.handler == *b"vide" { "video" } else { "audio" }.into(),
            codec: match &track.codec { b"avc1" | b"avc3" => "h264", b"hvc1" | b"hev1" => "hevc", _ => "aac" }.into(), disposition: "copy".into(),
        }).collect(),
        steps: vec![
            PlanStep { action: "demux".into(), detail: "owned indexed MP4 reader with bounded metadata and packet payloads".into() },
            PlanStep { action: "timestamps".into(), detail: "preserve decode order; interleave edited DTS; retain B-frame presentation timing, AAC delay and signed tail padding".into() },
            PlanStep { action: "copy".into(), detail: format!("copy {packets} compressed packets, {payload} payload bytes unchanged into owned Matroska") },
            PlanStep { action: "metadata".into(), detail: "retain track names/languages, file tags/chapters and video colour/HDR/display metadata".into() },
            PlanStep { action: "publish".into(), detail: "publish complete output without overwriting; remove temporary file on error or cancellation".into() },
        ], graph: None,
        notes: vec!["backend: owned MP4/Matroska; no external demuxer or muxer".into(), "destination must be .mkv or audio-only .mka; publication and packet payload reads are verified during execution".into(), "all supported tracks represented; optional global packet cap retains a DTS-interleaved prefix; stream/tag edits and aggregate/RSS policies remain unsupported".into()],
    })
}
