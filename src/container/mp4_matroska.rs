//! Owned MP4 AVC/HEVC/AAC remux with presentation edits and DTS interleaving.
use super::{
    matroska_write::{
        self, Encoding, FileMetadata, PacketOptions, PacketWriter, TrackOptions, TrackSpec,
        VideoMetadata,
    },
    mp4::{Mp4Reader, Track},
};
use crate::{Result, invalid};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    io::{Read, Seek, Write},
};

/// Select only represented AVC/HEVC/AAC tracks with zero or one media edit.
/// Complex schedules remain the responsibility of the general media planner.
pub fn eligible<R: Read + Seek>(input: &Mp4Reader<R>) -> bool {
    (1..=126).contains(&input.tracks().len())
        && input.refused().is_empty()
        && input.tracks().iter().all(|t| {
            matches!(
                (&t.handler, &t.codec),
                (b"vide", b"avc1" | b"avc3" | b"hvc1" | b"hev1") | (b"soun", b"mp4a")
            ) && (t.edits.is_empty() || (t.edits.len() == 1 && t.edits[0].media_time >= 0))
        })
}
fn check(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(|c| c.is_cancelled()) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
pub(crate) struct PacketTime {
    pub(crate) pts: u64,
    pub(crate) duration: u64,
    pub(crate) dts: i128,
    pub(crate) options: PacketOptions,
}
pub(crate) struct TrackPlan {
    pub(crate) options: TrackOptions,
    pub(crate) packets: Vec<PacketTime>,
}
fn nanoseconds(ticks: i128, scale: u32) -> Result<i128> {
    if scale == 0 {
        return Err(invalid("zero MP4 track clock"));
    }
    ticks
        .checked_mul(1_000_000_000)
        .map(|n| n.div_euclid(i128::from(scale)))
        .ok_or_else(|| invalid("MP4 timestamp overflow"))
}
fn as_time(n: i128) -> Result<u64> {
    u64::try_from(n)
        .ok()
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or_else(|| invalid("Matroska timestamp overflow"))
}
pub(crate) fn plan(track: &Track, movie_scale: u32, cancel: Option<&CancelFlag>) -> Result<TrackPlan> {
    let mut packets = Vec::new();
    packets
        .try_reserve_exact(track.samples.len())
        .map_err(|_| invalid("cannot allocate Matroska packet timing"))?;
    if track.codec == *b"mp4a" {
        let audio = matroska_write::aac_packet_plan(track, movie_scale, cancel)?;
        for i in 0..audio.count {
            check(cancel)?;
            let (pts, duration, padding) = audio.packet(i)?;
            packets.push(PacketTime {
                pts,
                duration,
                dts: i128::from(pts) - i128::from(audio.delay),
                options: PacketOptions {
                    discard_padding_ns: padding,
                    invisible: false,
                },
            });
        }
        return Ok(TrackPlan {
            options: TrackOptions {
                codec_delay_ns: audio.delay,
                ..Default::default()
            },
            packets,
        });
    }
    let (begin, end) = match track.edits.as_slice() {
        [] => (0i128, None),
        [edit] if edit.media_time >= 0 && movie_scale != 0 => {
            let ticks = (u128::from(edit.duration) * u128::from(track.timescale))
                .div_ceil(u128::from(movie_scale));
            if ticks == 0 {
                return Err(invalid("empty MP4 video edit"));
            }
            let begin = i128::from(edit.media_time);
            (
                begin,
                Some(begin + i128::try_from(ticks).map_err(|_| invalid("MP4 edit overflow"))?),
            )
        }
        _ => {
            return Err(invalid(
                "MP4 video remux requires one contiguous media edit",
            ));
        }
    };
    // stts durations belong to decode order. With B frames and variable
    // cadence the displayed duration comes from the next presentation start,
    // matching the native MP4 video reader. Only the last picture uses stts.
    let mut presentation = Vec::new();
    presentation
        .try_reserve_exact(track.samples.len())
        .map_err(|_| invalid("cannot allocate presentation index"))?;
    let mut durations = Vec::new();
    durations
        .try_reserve_exact(track.samples.len())
        .map_err(|_| invalid("cannot allocate presentation durations"))?;
    for i in 0..track.samples.len() {
        check(cancel)?;
        let sample = track
            .samples
            .get(i)
            .ok_or_else(|| invalid("missing MP4 sample"))?;
        presentation.push((sample.pts, i));
        durations.push(i128::from(sample.duration));
    }
    presentation.sort_unstable();
    for pair in presentation.windows(2) {
        let span = i128::from(pair[1].0) - i128::from(pair[0].0);
        if span <= 0 {
            return Err(invalid("non-increasing MP4 presentation timestamps"));
        }
        durations[pair[0].1] = span;
    }
    let mut last_visible = None;
    for i in 0..track.samples.len() {
        check(cancel)?;
        let sample = track
            .samples
            .get(i)
            .ok_or_else(|| invalid("missing MP4 sample"))?;
        if sample.duration == 0 {
            return Err(invalid("zero MP4 video sample duration"));
        }
        let start = i128::from(sample.pts);
        let finish = start + durations[i];
        let clipped_start = start.max(begin);
        let clipped_end = end.map_or(finish, |end| finish.min(end));
        let invisible = clipped_start >= clipped_end;
        let (pts, duration) = if invisible {
            (
                as_time(nanoseconds((start - begin).max(0), track.timescale)?)?,
                as_time(nanoseconds(i128::from(sample.duration), track.timescale)?)?,
            )
        } else {
            let a = as_time(nanoseconds(clipped_start - begin, track.timescale)?)?;
            let b = as_time(nanoseconds(clipped_end - begin, track.timescale)?)?;
            last_visible = Some(i);
            (a, b - a)
        };
        if duration == 0 {
            return Err(invalid("MP4 video interval is shorter than one nanosecond"));
        }
        packets.push(PacketTime {
            pts,
            duration,
            dts: nanoseconds(i128::from(sample.dts) - begin, track.timescale)?,
            options: PacketOptions {
                invisible,
                ..Default::default()
            },
        });
    }
    // All dependencies precede a picture in decode order. Keep its prefix,
    // including references displayed beyond the edit end, but omit the tail
    // that is not needed to decode any retained picture.
    packets
        .truncate(last_visible.ok_or_else(|| invalid("MP4 edit contains no video pictures"))? + 1);
    let aspect = if matches!(track.rotation, 90 | 270) {
        (track.pixel_aspect.1, track.pixel_aspect.0)
    } else {
        track.pixel_aspect
    };
    let colour = track.colour;
    Ok(TrackPlan {
        packets,
        options: TrackOptions {
            rotation: track.rotation,
            video: Some(VideoMetadata {
                pixel_aspect: aspect,
                colour: (colour.primaries != 0
                    || colour.transfer != 0
                    || colour.matrix != 0
                    || colour.full_range)
                    .then_some(colour),
                hdr: track.hdr,
                ..Default::default()
            }),
            ..Default::default()
        },
    })
}

/// Remux every represented track. Payloads are copied unchanged, ordered by
/// edited DTS with per-track decode order retained. Timing/index memory is
/// separate from the one reusable packet buffer. Caller owns atomic publication.
pub fn write<R: Read + Seek, W: Write + Seek>(
    input: &mut Mp4Reader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    check(cancel)?;
    if !eligible(input) {
        return Err(invalid(
            "MP4 Matroska remux requires AVC/HEVC/AAC tracks with contiguous media edits",
        ));
    }
    let tracks = input.tracks().to_vec();
    let plans: Vec<_> = tracks
        .iter()
        .map(|t| plan(t, input.movie_timescale(), cancel))
        .collect::<Result<_>>()?;
    let specs: Vec<_> = tracks
        .iter()
        .map(|track| -> Result<_> {
            let encoding = match &track.codec {
                b"avc1" | b"avc3" => Encoding::Avc {
                    configuration: &track.configuration,
                    width: track.width.into(),
                    height: track.height.into(),
                },
                b"hvc1" | b"hev1" => Encoding::Hevc {
                    configuration: &track.configuration,
                    width: track.width.into(),
                    height: track.height.into(),
                },
                b"mp4a" => Encoding::Aac {
                    configuration: crate::codec::config::aac_specific_config(&track.configuration)?,
                    sample_rate: track.sample_rate,
                    channels: track.channels,
                },
                _ => return Err(invalid("unsupported MP4 Matroska track")),
            };
            Ok(TrackSpec {
                encoding,
                name: &track.name,
                language: &track.language,
            })
        })
        .collect::<Result<_>>()?;
    let options: Vec<_> = plans.iter().map(|p| p.options.clone()).collect();
    let mut writer =
        PacketWriter::new_with_metadata(output, &specs, &options, &FileMetadata::from_mp4(input))?;
    if let Some(hook) = progress {
        hook.emit(writer.event());
    }
    let mut queue = BinaryHeap::new();
    for (track, plan) in plans.iter().enumerate() {
        if let Some(first) = plan.packets.first() {
            queue.push(Reverse((first.dts, track, 0usize)));
        }
    }
    let mut payload = Vec::new();
    while let Some(Reverse((_, track, index))) = queue.pop() {
        check(cancel)?;
        let packet = &plans[track].packets[index];
        input.read_packet(track, index, &mut payload)?;
        let sync = tracks[track]
            .samples
            .get(index)
            .ok_or_else(|| invalid("missing MP4 sample"))?
            .sync;
        writer.write_packet_with_options(
            track,
            packet.pts,
            packet.duration,
            sync,
            &payload,
            packet.options,
        )?;
        if let Some(hook) = progress {
            hook.emit(writer.event());
        }
        if let Some(next) = plans[track].packets.get(index + 1) {
            queue.push(Reverse((next.dts, track, index + 1)));
        }
    }
    check(cancel)?;
    let event = writer.finish()?;
    check(cancel)?;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reordered_variable_cadence_uses_presentation_intervals() {
        let reader = Mp4Reader::open(
            std::io::Cursor::new(include_bytes!("../../tests/fixtures/video.mp4")),
            Default::default(),
        )
        .unwrap();
        let mut track = reader.tracks()[0].clone();
        let mut samples: Vec<_> = (0..track.samples.len())
            .map(|i| track.samples.get(i).unwrap())
            .collect();
        assert!(samples.len() >= 4);
        samples.truncate(4);
        for (sample, pts) in samples.iter_mut().zip([0, 100, 20, 65]) {
            sample.pts = pts;
            sample.duration = 10;
        }
        track.samples = super::super::mp4::SampleIndex::Expanded(samples);
        track.timescale = 1000;
        track.edits.clear();
        let plan = plan(&track, 1000, None).unwrap();
        assert_eq!(
            plan.packets
                .iter()
                .map(|p| (p.pts, p.duration))
                .collect::<Vec<_>>(),
            [
                (0, 20_000_000),
                (100_000_000, 10_000_000),
                (20_000_000, 45_000_000),
                (65_000_000, 35_000_000)
            ]
        );
    }
}
