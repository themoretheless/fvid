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
/// Admission validates all inputs before creating output. Primed AAC requires
/// per-segment priming transport and retains its existing route until implemented.
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
        let plans = reader
            .tracks()
            .iter()
            .map(|t| mp4_matroska::plan(t, reader.movie_timescale(), cancel))
            .collect::<Result<Vec<_>>>()?;
        if plans.iter().any(|p| p.options.codec_delay_ns != 0) {
            return Ok(None);
        }
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
                    .ok_or_else(|| invalid("concat padding exceeds interval"))?;
                duration = duration.max(end);
            }
        }
        if duration == 0 {
            return Err(invalid("concat segment has no presented media"));
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
            let pts = segment
                .offset
                .checked_add(packet.pts)
                .ok_or_else(|| invalid("concat packet timestamp overflow"))?;
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
