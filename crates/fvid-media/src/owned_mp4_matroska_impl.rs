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
    let default_duration_ns=if track.handler==*b"vide" {
        packets.first().map(|p|p.duration).filter(|&d|d>0 && packets.iter().all(|p|p.duration==d)).unwrap_or(0)
    }else{0};
    Ok(TrackPlan {
        packets,
        options: TrackOptions {
            default_duration_ns,
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

pub(crate) fn spec(track: &Track) -> Result<TrackSpec<'_>> {
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
            configuration: aac_specific_config(&track.configuration)?,
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
    write_limited(input, output, cancel, progress, None)
}

/// Copy at most a global DTS-interleaved packet prefix; no payload is read past it.
pub fn write_limited<R: Read + Seek, W: Write + Seek>(
    input: &mut Mp4Reader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
) -> Result<ProgressEvent> {
    write_selected(input, output, cancel, progress, max_packets, &[])
}

pub(crate) fn selection(count: usize, requested: &[usize]) -> Result<Vec<usize>> {
    if count == 0 { return Err(invalid("MP4 has no streams")); }
    if requested.is_empty() { return Ok((0..count).collect()); }
    let mut selected = Vec::new();
    for &index in requested {
        if index >= count || selected.contains(&index) {
            return Err(invalid("invalid or duplicate MP4 stream index"));
        }
        selected.push(index);
    }
    Ok(selected)
}

/// Optional name/language replacements keyed by the original source index.
#[derive(Clone, Copy, Debug)]
pub struct TrackMetadataOverride<'a> {
    pub index: usize,
    pub name: Option<&'a str>,
    pub language: Option<&'a str>,
}
pub struct RemuxMetadata<'a> {
    pub file: &'a FileMetadata,
    pub tracks: &'a [TrackMetadataOverride<'a>],
}

/// Preserve requested output track order while copying a global source-DTS prefix.
pub fn write_selected<R: Read + Seek, W: Write + Seek>(
    input: &mut Mp4Reader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
    requested: &[usize],
) -> Result<ProgressEvent> {
    check(cancel)?;
    let metadata = FileMetadata::from_mp4(input);
    write_selected_with_metadata(input, output, cancel, progress, max_packets, requested, &metadata)
}

/// Packet copy with caller-prepared file tags and chapters.
pub fn write_selected_with_metadata<R: Read + Seek, W: Write + Seek>(
    input: &mut Mp4Reader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
    requested: &[usize],
    metadata: &FileMetadata,
) -> Result<ProgressEvent> {
    write_selected_with_metadata_overrides(input, output, cancel, progress, max_packets,
        requested, RemuxMetadata { file: metadata, tracks: &[] })
}

/// Replace represented track names/languages before emitting a container header.
/// Empty strings delete a field; absent values retain the source field.
pub fn write_selected_with_metadata_overrides<R: Read + Seek, W: Write + Seek>(
    input: &mut Mp4Reader<R>, output: &mut W,
    cancel: Option<&CancelFlag>, progress: Option<&ProgressHook>,
    max_packets: Option<u64>, requested: &[usize], metadata: RemuxMetadata<'_>,
) -> Result<ProgressEvent> {
    check(cancel)?;
    if !eligible(input) {
        return Err(invalid(
            "MP4 Matroska remux requires AVC/HEVC/AAC tracks with contiguous media edits",
        ));
    }
    let selected = selection(input.tracks().len(), requested)?;
    for (position, edit) in metadata.tracks.iter().enumerate() {
        if !selected.contains(&edit.index)
            || metadata.tracks[..position].iter().any(|previous| previous.index == edit.index)
            || edit.name.is_some_and(|value| value.contains('\0'))
            || edit.language.is_some_and(|value| value.contains('\0'))
        {
            return Err(invalid("invalid MP4 track metadata override"));
        }
    }
    let tracks: Vec<_> = selected.iter().map(|&index| {
        let mut track = input.tracks()[index].clone();
        if let Some(edit) = metadata.tracks.iter().find(|edit| edit.index == index) {
            if let Some(name) = edit.name { track.name = name.to_owned(); }
            if let Some(language) = edit.language { track.language = language.to_owned(); }
        }
        track
    }).collect();
    let plans: Vec<_> = tracks
        .iter()
        .map(|t| plan(t, input.movie_timescale(), cancel))
        .collect::<Result<_>>()?;
    let specs: Vec<_> = tracks.iter().map(spec).collect::<Result<_>>()?;
    let options: Vec<_> = plans.iter().map(|p| p.options.clone()).collect();
    let mut writer =
        PacketWriter::new_with_metadata(output, &specs, &options, metadata.file)?;
    if let Some(hook) = progress {
        hook.emit(writer.event());
    }
    let mut queue = BinaryHeap::new();
    for (track, plan) in plans.iter().enumerate() {
        if let Some(first) = plan.packets.first() {
            queue.push(Reverse((first.dts, selected[track], track, 0usize)));
        }
    }
    let mut payload = Vec::new();
    while let Some(Reverse((_, source_track, track, index))) = queue.pop() {
        check(cancel)?;
        if max_packets.is_some_and(|limit| writer.event().packets >= limit) {
            break;
        }
        let packet = &plans[track].packets[index];
        input.read_packet(source_track, index, &mut payload)?;
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
            queue.push(Reverse((next.dts, source_track, track, index + 1)));
        }
    }
    check(cancel)?;
    let event = if max_packets.is_some() { writer.finish_prefix()? } else { writer.finish()? };
    check(cancel)?;
    Ok(event)
}
