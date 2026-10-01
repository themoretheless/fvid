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
    reader: Mp4Reader<R>,
    plans: Vec<TrackPlan>,
    offset: u64,
    duration: u64,
}
/// Admission validates all inputs and per-segment trim transport before output.
pub fn open(
    sources: &[PathBuf],
    cancel: Option<&CancelFlag>,
) -> Result<Option<Vec<Segment<BufReader<File>>>>> {
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
        let reader = Mp4Reader::open(BufReader::new(file), Default::default())?;
        if !mp4_matroska::eligible(&reader) {
            return Ok(None);
        }
        if let Some(first) = segments.first() {
            if reader.tracks().len() != first.reader.tracks().len()
                || !reader
                    .tracks()
                    .iter()
                    .zip(first.reader.tracks())
                    .all(|(a, b)| compatible(a, b))
            {
                return Ok(None);
            }
        }
        if reader
            .tracks()
            .iter()
            .any(|t| t.handler == *b"vide" && !t.samples.get(0).is_some_and(|s| s.sync))
        {
            return Ok(None);
        }
        let mut plans = reader
            .tracks()
            .iter()
            .map(|t| mp4_matroska::plan(t, reader.movie_timescale(), cancel))
            .collect::<Result<Vec<_>>>()?;
        let mut duration = 0;
        for p in &plans {
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
    check(cancel)?;
    let first = segments
        .first()
        .ok_or_else(|| invalid("concat requires input segments"))?;
    let tracks = first.reader.tracks().to_vec();
    let specs = tracks
        .iter()
        .map(mp4_matroska::spec)
        .collect::<Result<Vec<_>>>()?;
    let mut options: Vec<_> = first.plans.iter().map(|p| p.options).collect();
    for option in &mut options {
        option.default_duration_ns = 0;
    }
    let mut metadata = FileMetadata::from_mp4(&first.reader);
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
    let mut writer = PacketWriter::new_with_metadata(output, &specs, &options, &metadata)?;
    if let Some(hook) = progress {
        hook.emit(writer.event());
    }
    let mut payload = Vec::new();
    for segment in segments {
        let mut queue = BinaryHeap::new();
        for (track, plan) in segment.plans.iter().enumerate() {
            if let Some(packet) = plan.packets.first() {
                queue.push(Reverse((packet.dts, track, 0usize)));
            }
        }
        while let Some(Reverse((_, track, index))) = queue.pop() {
            check(cancel)?;
            let packet = &segment.plans[track].packets[index];
            segment.reader.read_packet(track, index, &mut payload)?;
            let pts = u64::try_from(
                i128::from(segment.offset)
                    + i128::from(packet.pts)
                    + i128::from(options[track].codec_delay_ns)
                    - i128::from(segment.plans[track].options.codec_delay_ns),
            )
            .map_err(|_| invalid("concat packet timestamp overflow"))?;
            let sync = segment.reader.tracks()[track]
                .samples
                .get(index)
                .ok_or_else(|| invalid("missing concat sample"))?
                .sync;
            writer.write_packet_with_options(
                track,
                pts,
                packet.duration,
                sync,
                &payload,
                packet.options,
            )?;
            if let Some(hook) = progress {
                hook.emit(writer.event());
            }
            if let Some(packet) = segment.plans[track].packets.get(index + 1) {
                queue.push(Reverse((packet.dts, track, index + 1)));
            }
        }
    }
    check(cancel)?;
    let event = writer.finish()?;
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
