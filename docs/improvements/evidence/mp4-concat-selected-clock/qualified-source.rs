//! Owned compressed MP4 concatenation into one Matroska stream per source track.
use super::{
    matroska_write::{FileMetadata, PacketWriter},
    mp4::{Mp4Reader, Track},
    mp4_matroska::{self, TrackPlan},
};
use crate::{Result, invalid};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    fs::File,
    io::{BufReader, Read, Seek, Write},
    path::PathBuf,
};

fn check(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(CancelFlag::is_cancelled) {
        return Err(invalid("media operation cancelled"));
    }
    Ok(())
}
fn compatible(a: &Track, b: &Track) -> bool {
    a.handler == b.handler
        && a.codec == b.codec
        && a.configuration == b.configuration
        && a.width == b.width
        && a.height == b.height
        && a.channels == b.channels
        && a.sample_rate == b.sample_rate
        && a.pixel_aspect == b.pixel_aspect
        && a.rotation == b.rotation
        && a.colour == b.colour
        && a.hdr == b.hdr
}
pub struct Segment<R> {
    pub(crate) reader: Mp4Reader<R>,
    plans: Vec<TrackPlan>,
    offset: u64,
    duration: u64,
}
/// Admission validates all inputs and per-segment trim transport before output.
pub fn open(
    sources: &[PathBuf],
    cancel: Option<&CancelFlag>,
) -> Result<Option<Vec<Segment<BufReader<File>>>>> {
    open_with_packet_limit(sources, cancel, super::mp4::Limits::default().packet_bytes)
}

pub fn open_with_packet_limit(
    sources: &[PathBuf],
    cancel: Option<&CancelFlag>,
    max_packet_bytes: usize,
) -> Result<Option<Vec<Segment<BufReader<File>>>>> {
    let mut options = fvid_media::CopyOptions::default();
    options.cancel = cancel.cloned();
    options.max_packet_bytes = max_packet_bytes;
    open_with_options(sources, &options)
}

pub fn open_with_options(
    sources: &[PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<Option<Vec<Segment<BufReader<File>>>>> {
    let cancel = options.cancel.as_ref();
    let max_packet_bytes = options.max_packet_bytes;
    if max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("Matroska output requires at least one packet"));
    }
    if !(2..=256).contains(&sources.len()) {
        return Err(invalid("concat requires 2..=256 inputs"));
    }
    let mut segments: Vec<Segment<BufReader<File>>> = Vec::new();
    let mut offset = 0u64;
    for source in sources {
        check(cancel)?;
        let mut file = File::open(source)?;
        let mut prefix = [0; 8];
        match file.read_exact(&mut prefix) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        if !super::mp4::recognizes_prefix(&prefix) {
            return Ok(None);
        }
        file.rewind()?;
        let mut reader = Mp4Reader::open(BufReader::new(file), super::mp4::Limits::default())?;
        reader.restrict_packet_bytes(
            max_packet_bytes.min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
        );
        if !mp4_matroska::eligible(&reader) {
            return Ok(None);
        }
        if let Some(first) = segments.first()
            && (reader.tracks().len() != first.reader.tracks().len()
                || !reader
                    .tracks()
                    .iter()
                    .zip(first.reader.tracks())
                    .all(|(a, b)| compatible(a, b)))
            {
                return Ok(None);
            }
        if reader
            .tracks()
            .iter()
            .any(|t| t.handler == *b"vide" && !t.samples.get(0).is_some_and(|s| s.sync))
        {
            return Ok(None);
        }
        let selected_tracks = mp4_matroska::selection(reader.tracks().len(), &options.streams)?;
        let mut plans = reader
            .tracks()
            .iter()
            .map(|t| mp4_matroska::plan(t, reader.movie_timescale(), cancel))
            .collect::<Result<Vec<_>>>()?;
        let mut duration = 0;
        // Only output tracks determine the selected segment's end. A longer
        // discarded track must not insert silence or a frozen-video gap.
        for &track in &selected_tracks {
            let p = &plans[track];
            for packet in &p.packets {
                if packet.options.invisible {
                    continue;
                }
                let end = packet
                    .pts
                    .checked_add(packet.duration)
                    .ok_or_else(|| invalid("concat interval overflow"))?;
                let end = end
                    .checked_sub(packet.options.discard_padding_ns.max(0) as u64)
                    .ok_or_else(|| invalid("concat padding exceeds interval"))?
                    .saturating_sub(p.options.codec_delay_ns);
                duration = duration.max(end);
            }
        }
        if duration == 0 {
            return Err(invalid("concat segment has no presented media"));
        }
        for (track, plan) in plans.iter_mut().enumerate() {
            let local_delay = plan.options.codec_delay_ns;
            let global_delay = segments.first().map_or(local_delay, |first| {
                first.plans[track].options.codec_delay_ns
            });
            if !transport_priming(plan, offset, global_delay, segments.is_empty())? {
                return Ok(None);
            }
        }
        let next = offset
            .checked_add(duration)
            .filter(|&n| n <= i64::MAX as u64)
            .ok_or_else(|| invalid("concat timeline overflow"))?;
        segments.push(Segment {
            reader,
            plans,
            offset,
            duration,
        });
        offset = next;
    }
    let selected = mp4_matroska::selection(segments[0].reader.tracks().len(), &options.streams)?;
    if let Some(mem_limit) = options.max_controlled_bytes {
        let mut plan_bytes = segments.len().saturating_mul(8192).saturating_add(16384);
        for segment in &segments {
            // Plans for unselected tracks are retained too: stream selection
            // controls output, not ownership of the segment's timing tables.
            for plan in &segment.plans {
                plan_bytes = plan
                    .packets
                    .capacity()
                    .checked_mul(std::mem::size_of::<mp4_matroska::PacketTime>())
                    .and_then(|n| plan_bytes.checked_add(n))
                    .ok_or_else(|| invalid("MP4 concat packet plan memory overflow"))?;
            }
        }
        if plan_bytes > mem_limit {
            return Err(invalid("controlled memory budget exceeded"));
        }
    }
    let mut packets = 0u64;
    'segments: for segment in &segments {
        let mut queue = BinaryHeap::new();
        for (pos, &source_track) in selected.iter().enumerate() {
            if let Some(packet) = segment.plans[source_track].packets.first() {
                queue.push(Reverse((packet.dts, pos, source_track, 0usize)));
            }
        }
        while let Some(Reverse((_, pos, source_track, index))) = queue.pop() {
            check(cancel)?;
            if options.max_packets.is_some_and(|limit| packets >= limit) {
                break 'segments;
            }
            let sample = segment.reader.tracks()[source_track]
                .samples
                .get(index)
                .ok_or_else(|| invalid("missing MP4 sample"))?;
            if (sample.size as usize) > max_packet_bytes {
                return Err(invalid("MP4 packet exceeds budget"));
            }
            if options
                .max_controlled_bytes
                .is_some_and(|limit| (sample.size as usize).saturating_add(16384) > limit)
            {
                return Err(invalid("controlled memory budget exceeded"));
            }
            packets = packets
                .checked_add(1)
                .ok_or_else(|| invalid("concat packet count overflow"))?;
            if let Some(packet) = segment.plans[source_track].packets.get(index + 1) {
                queue.push(Reverse((packet.dts, pos, source_track, index + 1)));
            }
        }
    }
    Ok(Some(segments))
}
fn transport_priming(
    plan: &mut TrackPlan,
    offset: u64,
    global_delay: u64,
    first: bool,
) -> Result<bool> {
    let local_delay = plan.options.codec_delay_ns;
    for packet in &mut plan.packets {
        let pts = i128::from(offset) + i128::from(packet.pts) + i128::from(global_delay)
            - i128::from(local_delay);
        if !(0..=i128::from(i64::MAX)).contains(&pts) {
            return Ok(false);
        }
        if !first {
            let head = local_delay.saturating_sub(packet.pts).min(packet.duration);
            if head > 0 {
                // One signed DiscardPadding cannot represent both ends of a
                // single partially retained packet. Retain existing routing.
                if packet.options.discard_padding_ns > 0 {
                    return Ok(false);
                }
                packet.options.discard_padding_ns =
                    -i64::try_from(head).map_err(|_| invalid("concat priming overflow"))?;
            }
        }
    }
    Ok(true)
}

/// Copy payloads unchanged, preserving per-track decode order and edited PTS.
/// Caller owns atomic publication and final completion notification.
pub fn write<R: Read + Seek, W: Write + Seek>(
    segments: &mut [Segment<R>],
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    write_limited(segments, output, cancel, progress, None)
}

pub fn write_limited<R: Read + Seek, W: Write + Seek>(
    segments: &mut [Segment<R>],
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
) -> Result<ProgressEvent> {
    let mut options = fvid_media::CopyOptions::default();
    options.cancel = cancel.cloned();
    options.progress = progress.cloned();
    options.max_packets = max_packets;
    write_with_options(segments, output, &options)
}

pub fn write_with_options<R: Read + Seek, W: Write + Seek>(
    segments: &mut [Segment<R>],
    output: &mut W,
    options: &fvid_media::CopyOptions,
) -> Result<ProgressEvent> {
    let cancel = options.cancel.as_ref();
    let progress = options.progress.as_ref();
    let max_packets = options.max_packets;
    if max_packets == Some(0) {
        return Err(invalid("Matroska output requires at least one packet"));
    }
    check(cancel)?;
    let first = segments
        .first()
        .ok_or_else(|| invalid("concat requires input segments"))?;
    let selected = mp4_matroska::selection(first.reader.tracks().len(), &options.streams)?;
    if options.metadata_set.len() + options.metadata_delete.len() > 64 {
        return Err(invalid("too many metadata mutations"));
    }
    if options.stream_metadata_set.len() + options.stream_metadata_delete.len() > 64 {
        return Err(invalid("too many stream metadata mutations"));
    }
    let mut metadata = FileMetadata::from_mp4(&first.reader);
    let mut text = std::collections::BTreeMap::new();
    let encoder = first.reader.encoder().trim();
    if !encoder.is_empty() && encoder.len() <= 1024 && !encoder.contains('\0') {
        text.insert("ENCODER".into(), encoder.to_owned());
    }
    for (key, value) in &options.metadata_set {
        let key = key.trim();
        if key.is_empty()
            || key.len() > 128
            || value.len() > 1024
            || key.contains('\0')
            || value.contains('\0')
        {
            return Err(invalid("invalid metadata tag"));
        }
        text.retain(|k: &String, _| !k.eq_ignore_ascii_case(key));
        if !metadata.tags.set(key, value) && !value.is_empty() {
            text.insert(key.to_ascii_uppercase(), value.clone());
        }
    }
    for key in &options.metadata_delete {
        let key = key.trim();
        if key.is_empty() || key.len() > 128 || key.contains('\0') {
            return Err(invalid("invalid metadata key"));
        }
        metadata.tags.set(key, "");
        text.retain(|k, _| !k.eq_ignore_ascii_case(key));
    }
    let mut tracks: Vec<_> = selected
        .iter()
        .map(|&index| first.reader.tracks()[index].clone())
        .collect();
    let mut scoped: std::collections::BTreeMap<usize, std::collections::BTreeMap<String, String>> =
        std::collections::BTreeMap::new();
    for (index, key, value) in &options.stream_metadata_set {
        let Some(pos) = selected.iter().position(|&s| s == *index) else {
            return Err(invalid("stream metadata index out of range"));
        };
        let key = key.trim();
        if key.is_empty()
            || key.len() > 128
            || value.len() > 1024
            || key.contains('\0')
            || value.contains('\0')
            || matches!(
                key.to_ascii_lowercase().as_str(),
                "rotate" | "stereo_mode" | "alpha_mode"
            )
            || (key.eq_ignore_ascii_case("language") && value.len() > 128)
        {
            return Err(invalid("invalid stream metadata tag"));
        }
        if key.eq_ignore_ascii_case("title") {
            tracks[pos].name = value.clone();
        } else if key.eq_ignore_ascii_case("language") {
            tracks[pos].language = value.clone();
        } else {
            let map = scoped.entry(pos).or_default();
            map.retain(|k: &String, _| !k.eq_ignore_ascii_case(key));
            if !value.is_empty() {
                map.insert(key.to_ascii_uppercase(), value.clone());
            }
        }
    }
    for (index, key) in &options.stream_metadata_delete {
        let Some(pos) = selected.iter().position(|&s| s == *index) else {
            return Err(invalid("stream metadata index out of range"));
        };
        let key = key.trim();
        if key.is_empty()
            || key.len() > 128
            || key.contains('\0')
            || matches!(
                key.to_ascii_lowercase().as_str(),
                "rotate" | "stereo_mode" | "alpha_mode"
            )
        {
            return Err(invalid("invalid stream metadata key"));
        }
        if key.eq_ignore_ascii_case("title") {
            tracks[pos].name.clear();
        } else if key.eq_ignore_ascii_case("language") {
            tracks[pos].language.clear();
        } else if let Some(map) = scoped.get_mut(&pos) {
            map.retain(|k, _| !k.eq_ignore_ascii_case(key));
        }
    }
    let specs = tracks
        .iter()
        .map(mp4_matroska::spec)
        .collect::<Result<Vec<_>>>()?;
    let mut track_options: Vec<_> = selected.iter().map(|&i| first.plans[i].options).collect();
    for option in &mut track_options {
        option.default_duration_ns = 0;
    }
    metadata.chapters.clear();
    for segment in segments.iter() {
        for chapter in segment.reader.chapters() {
            if chapter.start_ns < segment.duration {
                metadata.chapters.push(super::webm::Chapter {
                    start_ns: segment
                        .offset
                        .checked_add(chapter.start_ns)
                        .ok_or_else(|| invalid("concat chapter overflow"))?,
                    end_ns: None,
                    title: chapter.title.clone(),
                });
            }
        }
    }
    let mut writer = PacketWriter::new_with_text_metadata(
        output,
        &specs,
        &track_options,
        &metadata,
        &text,
        &scoped,
    )?;
    if let Some(hook) = progress {
        hook.emit(writer.event());
    }
    let mut payload = Vec::new();
    'segments: for segment in segments {
        let mut queue = BinaryHeap::new();
        for (pos, &source_track) in selected.iter().enumerate() {
            if let Some(packet) = segment.plans[source_track].packets.first() {
                queue.push(Reverse((packet.dts, pos, source_track, 0usize)));
            }
        }
        while let Some(Reverse((_, pos, source_track, index))) = queue.pop() {
            check(cancel)?;
            if max_packets.is_some_and(|limit| writer.event().packets >= limit) {
                break 'segments;
            }
            let packet = &segment.plans[source_track].packets[index];
            segment.reader.read_packet(source_track, index, &mut payload)?;
            let pts = u64::try_from(
                i128::from(segment.offset)
                    + i128::from(packet.pts)
                    + i128::from(track_options[pos].codec_delay_ns)
                    - i128::from(segment.plans[source_track].options.codec_delay_ns),
            )
            .map_err(|_| invalid("concat packet timestamp overflow"))?;
            let sync = segment.reader.tracks()[source_track]
                .samples
                .get(index)
                .ok_or_else(|| invalid("missing concat sample"))?
                .sync;
            writer.write_packet_with_options(
                pos,
                pts,
                packet.duration,
                sync,
                &payload,
                packet.options,
            )?;
            if let Some(hook) = progress {
                hook.emit(writer.event());
            }
            if let Some(packet) = segment.plans[source_track].packets.get(index + 1) {
                queue.push(Reverse((packet.dts, pos, source_track, index + 1)));
            }
        }
    }
    check(cancel)?;
    let event = if max_packets.is_some() {
        writer.finish_prefix()?
    } else {
        writer.finish()?
    };
    check(cancel)?;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::super::{
        matroska_write::{PacketOptions, TrackOptions},
        mp4_matroska::PacketTime,
    };
    use super::*;
    fn plan(delay: u64, count: u64, tail: i64) -> TrackPlan {
        TrackPlan {
            trimmed_audio_sample_frames: 0,
            options: TrackOptions {
                codec_delay_ns: delay,
                ..Default::default()
            },
            packets: (0..count)
                .map(|i| PacketTime {
                    pts: i * 100,
                    duration: 100,
                    dts: i128::from(i * 100),
                    options: PacketOptions {
                        discard_padding_ns: if i + 1 == count { tail } else { 0 },
                        ..Default::default()
                    },
                })
                .collect(),
        }
    }
    #[test]
    fn delay_spanning_multiple_packets_becomes_per_packet_head_padding() {
        let mut value = plan(250, 4, 20);
        assert!(transport_priming(&mut value, 1000, 250, false).unwrap());
        assert_eq!(
            value
                .packets
                .iter()
                .map(|p| p.options.discard_padding_ns)
                .collect::<Vec<_>>(),
            [-100, -100, -50, 20]
        );
        assert_eq!(value.options.codec_delay_ns, 250);
    }
    #[test]
    fn a_second_packet_trimmed_at_both_ends_retains_existing_routing() {
        let mut value = plan(25, 1, 30);
        assert!(!transport_priming(&mut value, 100, 25, false).unwrap());
        assert!(transport_priming(&mut value, 0, 25, true).unwrap());
        assert_eq!(value.packets[0].options.discard_padding_ns, 30);
    }
    #[test]
    fn unrepresentable_negative_packet_time_is_not_admitted() {
        assert!(!transport_priming(&mut plan(250, 4, 0), 100, 0, false).unwrap());
    }
}
