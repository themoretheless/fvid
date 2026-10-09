pub(crate) fn decode_mp4_audio_reader_controlled<R: std::io::Read + std::io::Seek>(
    mut reader: Mp4TimelineReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
    selected: Option<usize>,
    control: &mut DecodeProgress<'_>,
) -> Result<AudioDecodeStats> {
    control.check()?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    let index = mp4_audio_index(&reader, selected)?;
    let track = reader.tracks()[index].clone();
    let in_band_ps = negotiate_mp4_ps(&mut reader, index, || control.check())?;
    let mut decoder = if in_band_ps { Mp4TimelineDecoder::with_in_band_ps(&track)? } else { Mp4TimelineDecoder::new(&track)? };
    let rate = decoder.sample_rate();
    let channels = decoder.channels();
    if track.timescale == 0 || track.sample_rate != rate || (!in_band_ps && track.channels != channels) {
        return Err(invalid(
            "MP4 audio export requires valid clock and matching audio geometry",
        ));
    }
    let sample_position = |ticks: u64| -> Result<u64> {
        let numerator = u128::from(ticks) * u128::from(rate);
        let denominator = u128::from(track.timescale);
        if numerator % denominator != 0 {
            return Err(invalid("MP4 audio timestamp is not aligned to a sample"));
        }
        u64::try_from(numerator / denominator)
            .map_err(|_| invalid("audio sample position overflow"))
    };
    let presentation = Mp4AudioSchedule::new(
        &track.edits,
        track.duration,
        track.timescale,
        reader.movie_timescale(),
        rate,
    )?;
    let timeline = presentation.sample_frames;
    let segments = presentation.segments;
    let (from, to) = if let Some((begin, end)) = interval {
        let boundary = |time: Duration| -> Result<u64> {
            let value = time
                .as_nanos()
                .checked_mul(u128::from(rate))
                .ok_or_else(|| invalid("audio interval overflow"))?
                .div_ceil(1_000_000_000);
            u64::try_from(value).map_err(|_| invalid("audio interval overflow"))
        };
        (boundary(begin)?, boundary(end)?.min(timeline))
    } else {
        (0, timeline)
    };
    if from >= to {
        return Err(invalid("audio interval contains no samples"));
    }
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let mut packet = Vec::new();
    // One source-boundary checkpoint bounds retained state regardless of the
    // number of edits. The stream and configuration never change in this call.
    let mut checkpoint: Option<(
        usize,
        u64,
        Option<u64>,
        Mp4AacCheckpoint,
        std::collections::VecDeque<(u64, u64)>,
    )> = None;
    for segment in segments {
        control.check()?;
        if control.packet_limit_reached() {
            break;
        }
        let segment_start = segment.presentation.start;
        let segment_end = segment.presentation.end;
        let source = segment.source_start;
        let begin = from.max(segment_start);
        let end = to.min(segment_end);
        if begin >= end {
            continue;
        }
        let length = end - begin;
        let Some(source) = source else {
            let zeros = [0u8; 4096];
            let mut bytes = length
                .checked_mul(u64::from(channels) * Mp4TimelineDecoder::SAMPLE_BYTES as u64)
                .ok_or_else(|| invalid("audio silence size overflow"))?;
            while bytes != 0 {
                control.check()?;
                let count = bytes.min(zeros.len() as u64) as usize;
                output.write_all(&zeros[..count])?;
                bytes -= count as u64;
            }
            stats.sample_frames += length;
            continue;
        };
        let from = source
            .checked_add(begin - segment_start)
            .ok_or_else(|| invalid("audio source edit overflow"))?;
        let to = from
            .checked_add(length)
            .ok_or_else(|| invalid("audio source edit overflow"))?;
        decoder.reset();
        let mut first_sample = 0;
        let mut expected = None;
        let mut pending_window = std::collections::VecDeque::new();
        if let Some((index, start, previous, state, pending)) = &checkpoint
            && *start <= from
            && decoder.restore_checkpoint(state)?
        {
            first_sample = *index;
            expected = *previous;
            pending_window = pending.clone();
        }
        let mut captured = false;
        let mut written = 0u64;
        let mut sample_index = first_sample;
        loop {
            control.check()?;
            let terminal = sample_index == track.samples.len() || control.packet_limit_reached();
            let (start, duration, samples) = if terminal {
                let Some(samples) = decoder.finish_delayed()? else {
                    break;
                };
                let (start, duration) = pending_window
                    .pop_front()
                    .ok_or_else(|| invalid("delayed MP4 audio has no source window"))?;
                (start, duration, samples)
            } else {
                let sample = track
                    .samples
                    .get(sample_index)
                    .ok_or_else(|| invalid("missing audio sample"))?;
                let start = sample_position(
                    u64::try_from(sample.pts).map_err(|_| invalid("negative audio timestamp"))?,
                )?;
                let duration = sample_position(u64::from(sample.duration))?;
                if expected.is_some_and(|value| value != start) {
                    return Err(invalid("non-contiguous MP4 audio timeline"));
                }
                if start >= to
                    && (!decoder.delayed() || pending_window.front().is_none_or(|(at, _)| *at >= to))
                {
                    break;
                }
                if !captured
                    && start
                        .checked_add(duration)
                        .ok_or_else(|| invalid("audio timestamp overflow"))?
                        > from
                {
                    if let Some(state) = decoder.checkpoint() {
                        checkpoint = Some((sample_index, start, expected, state, pending_window.clone()));
                    }
                    captured = true;
                }
                expected = Some(
                    start
                        .checked_add(duration)
                        .ok_or_else(|| invalid("audio timestamp overflow"))?,
                );
                reader.read_packet(index, sample_index, &mut packet)?;
                let samples = decoder.decode_timed(&packet,start,duration)?;
                control.packet(packet.len())?;
                sample_index += 1;
                if decoder.delayed() {
                    pending_window.push_back((start, duration));
                    if pending_window.len() > 3 { return Err(invalid("delayed MP4 source lookahead exceeded")); }
                    let Some(samples) = samples else {
                        continue;
                    };
                    let (start, duration) =
                        pending_window.pop_front().ok_or_else(|| invalid("delayed MP4 source identity mismatch"))?;
                    (start, duration, samples)
                } else {
                    (
                        start,
                        duration,
                        samples.ok_or_else(|| invalid("immediate MP4 audio returned no PCM"))?,
                    )
                }
            };
            if !samples.len().is_multiple_of(usize::from(channels)) {
                return Err(invalid("invalid MP4 decoded PCM stride"));
            }
            let frames = (samples.len() / usize::from(channels)) as u64;
            if duration == 0 || duration > frames {
                return Err(invalid(
                    "MP4 audio packet duration disagrees with decoded samples",
                ));
            }
            // The container assigns a presentation window to every packet.
            // Short windows trim decoded padding even in the interior; decoder
            // overlap state still consumes the complete compressed packet.
            let first = from.saturating_sub(start).min(duration) as usize;
            let last = to.saturating_sub(start).min(duration) as usize;
            for value in
                &samples[first * usize::from(channels)..last.max(first) * usize::from(channels)]
            {
                output.write_all(&value.to_le_bytes())?;
            }
            written += last.saturating_sub(first) as u64;
            stats.decoded_frames += 1;
            if written == length {
                break;
            }
        }
        if written != length && !control.packet_limit_reached() {
            return Err(invalid("audio edit extends outside available samples"));
        }
        stats.sample_frames += written;
    }
    if stats.sample_frames == 0 {
        return Err(invalid("MP4 audio edit contains no samples"));
    }
    Ok(stats)
}

pub(crate) fn negotiate_mp4_ps<R: std::io::Read + std::io::Seek>(
    reader: &mut Mp4TimelineReader<R>, index: usize, mut check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let track = reader.tracks()[index].clone();
    if track.codec != *b"mp4a" { return Ok(false); }
    let asc = mp4_probe_config(&track.configuration)?;
    let Ok(mut probe) = InBandPsProbe::new(asc, track.sample_rate) else { return Ok(false); };
    let mut packet = Vec::new();
    for sample in 0..track.samples.len() {
        check()?;
        reader.read_packet(index, sample, &mut packet)?;
        if probe.read(&packet).is_err() { probe.reset(); continue; }
        if probe.ps_detected() { return Ok(true); }
    }
    Ok(false)
}
