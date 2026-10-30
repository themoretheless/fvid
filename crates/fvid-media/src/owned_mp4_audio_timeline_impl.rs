fn mp4_write_tail_silence(output: &mut impl std::io::Write, frames: u64, channels: u16, control: &DecodeProgress<'_>) -> Result<()> {
    let mut bytes = frames.checked_mul(u64::from(channels) * Mp4TimelineDecoder::SAMPLE_BYTES as u64)
        .ok_or_else(|| invalid("MP4 tail silence size overflow"))?;
    let zeros = [0u8; 4096];
    while bytes != 0 {
        control.check()?;
        let count = bytes.min(zeros.len() as u64) as usize;
        output.write_all(&zeros[..count])?;
        bytes -= count as u64;
    }
    Ok(())
}
pub(crate) fn mp4_active_samples(
    track: &Mp4NormalizationTrack,
    interval: Option<(Duration, Duration)>,
    max_packets: Option<u64>,
) -> usize {
    let media_edits = track.edits.iter().filter(|e| e.media_time >= 0).count();
    if media_edits > 1 {
        return track.samples.len();
    }
    let mut active = max_packets
        .map(|m| usize::try_from(m).unwrap_or(usize::MAX))
        .unwrap_or(track.samples.len())
        .min(track.samples.len());
    if let Some((_, end)) = interval
        && track.timescale > 0
        && (track.edits.is_empty()
            || (track.edits.len() == 1 && track.edits[0].media_time == 0))
    {
        if end.is_zero() {
            return 0;
        }
        let scale = u128::from(track.timescale);
        if let Some(end_scaled) = end.as_nanos().checked_mul(scale) {
            let mut interval_samples = 0usize;
            while interval_samples < active {
                let Some(sample) = track.samples.get(interval_samples) else {
                    break;
                };
                if sample.pts >= 0
                    && (sample.pts as u128).saturating_mul(1_000_000_000) >= end_scaled
                {
                    break;
                }
                interval_samples += 1;
            }
            active = active.min(interval_samples);
        }
    }
    active
}

pub(crate) fn decode_mp4_audio_reader_controlled<R: std::io::Read + std::io::Seek>(
    reader: Mp4TimelineReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
    selected: Option<usize>,
    control: &mut DecodeProgress<'_>,
) -> Result<AudioDecodeStats> {
    decode_mp4_audio_reader_targeted(reader,output,interval,selected,control,None)
}
pub(crate) fn decode_mp4_audio_reader_targeted<R: std::io::Read + std::io::Seek>(
    mut reader: Mp4TimelineReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
    selected: Option<usize>,
    control: &mut DecodeProgress<'_>,
    target: Option<(u32,u16,u32)>,
) -> Result<AudioDecodeStats> {
    control.check()?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    let index = mp4_audio_index(&reader, selected)?;
    let track = reader.tracks()[index].clone();
    let initial_description = track.samples.get(0).ok_or_else(|| invalid("MP4 audio has no samples"))?.description_index;
    let initial_setup = track.decoder_setup(initial_description)?;
    let max_packets = control.packet_limit();
    let active_samples = mp4_active_samples(&track, interval, max_packets);
    let mut ps = vec![false; track.sample_descriptions.len()];
    // Probe each used configuration independently; never feed packets from a
    // different setup into an AAC entropy/history state.
    for (i, flag) in ps.iter_mut().enumerate() {
        if (0..active_samples).any(|n| track.samples.get(n).is_some_and(|s| s.description_index as usize == i+1)) {
            *flag = negotiate_mp4_description_ps_with_interval(&mut reader, index, (i+1) as u32, interval, max_packets, || control.check())?;
        }
    }
    let make_decoder = |id: u32| -> Result<Mp4TimelineDecoder> {
        let setup = track.decoder_setup(id)?;
        if ps[(id-1) as usize] { Mp4TimelineDecoder::with_in_band_ps(&setup) } else { Mp4TimelineDecoder::new(&setup) }
    };
    let mut decoder = make_decoder(initial_description)?;
    let rate = decoder.sample_rate();
    let channels = decoder.channels();
    if track.timescale == 0 || initial_setup.sample_rate != rate || (!ps[(initial_description-1) as usize] && initial_setup.channels != channels) {
        return Err(invalid("MP4 audio export requires valid clock and matching audio geometry"));
    }
    // Detect the need for normalization before caller-owned output.
    let mut checked = initial_description;
    // Even a prefix with one decoder setup can belong to a longer track whose
    // endpoint is fractional on that setup's grid. The integer schedule would
    // reject the whole duration before reaching the requested short interval.
    let fractional_extent = track.edits.is_empty()
        && (u128::from(track.duration) * u128::from(rate)) % u128::from(track.timescale) != 0;
    let mut normalize = target.is_some() || track.timescale % rate != 0 || fractional_extent
        || track.has_declared_gaps();
    for i in 0..active_samples {
        control.check()?;
        let id = track.samples.get(i).ok_or_else(|| invalid("missing audio sample"))?.description_index;
        if track.sample_descriptions[(id - 1) as usize] != track.sample_descriptions[(checked - 1) as usize] {
            let next = make_decoder(id)?;
            if next.sample_rate() != rate || next.channels() != channels || next.normalization_mask() != decoder.normalization_mask() {
                normalize = true;
            }
            checked = id;
        }
    }
    if normalize {
        drop(initial_setup);
        return decode_mp4_audio_normalized(reader,output,interval,index,track,initial_description,ps,decoder,target,control);
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
    let edit_clock = track.resolved_edit_clock(reader.movie_timescale())?;
    let presentation = Mp4AudioSchedule::new(
        edit_clock.edits(),
        track.duration,
        track.timescale,
        edit_clock.timescale,
        rate,
    )?;
    let declared_tail = track.declared_empty_tail()?.map(|tail| -> Result<(u64, u64)> {
        let boundary = |ticks| -> Result<u64> {
            u64::try_from(mp4_export_clock::Time::new(ticks, u128::from(track.timescale))?.ceil_frames(rate)?)
                .map_err(|_| invalid("MP4 tail sample position overflow"))
        };
        Ok((boundary(tail.start)?, boundary(tail.end)?))
    }).transpose()?;
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
    // number of edits. Checkpoints also retain their active setup identity.
    let mut checkpoint: Option<(
        usize,
        u32,
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
        decoder = make_decoder(initial_description)?;
        let mut active_description = initial_description;
        let mut first_sample = 0;
        let mut expected = None;
        let mut pending_window = std::collections::VecDeque::new();
        if let Some((index, description, start, previous, state, pending)) = &checkpoint
            && *start <= from
        {
            decoder = make_decoder(*description)?;
            if decoder.restore_checkpoint(state)? {
                active_description = *description;
                first_sample = *index;
                expected = *previous;
                pending_window = pending.clone();
            }
        }
        let mut captured = false;
        let mut written = 0u64;
        let mut sample_index = first_sample;
        loop {
            control.check()?;
            let terminal = sample_index == track.samples.len() || control.packet_limit_reached();
            let changing = !terminal && {
                let id = track.samples.get(sample_index).ok_or_else(|| invalid("missing audio sample"))?.description_index;
                track.sample_descriptions[(id-1) as usize] != track.sample_descriptions[(active_description-1) as usize]
            };
            let (start, duration, samples) = if terminal || (changing && !pending_window.is_empty()) {
                let Some(samples) = decoder.finish_delayed()? else {
                    if !pending_window.is_empty() { return Err(invalid("delayed MP4 audio left an undrained source window")); }
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
                if changing {
                    decoder = make_decoder(sample.description_index)?;
                    active_description = sample.description_index;
                }
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
                        checkpoint = Some((sample_index, active_description, start, expected, state, pending_window.clone()));
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
            let position = from.checked_add(written).ok_or_else(|| invalid("MP4 tail position overflow"))?;
            if sample_index == track.samples.len()
                && declared_tail.is_some_and(|(start, end)| position >= start && to <= end) {
                mp4_write_tail_silence(output, length - written, channels, control)?;
                written = length;
            } else {
                return Err(invalid("audio edit extends outside available samples"));
            }
        }
        stats.sample_frames += written;
    }
    if stats.sample_frames == 0 {
        return Err(invalid("MP4 audio edit contains no samples"));
    }
    Ok(stats)
}

#[allow(dead_code)] // Precise PCM instantiation shares this implementation too.
pub(crate) fn negotiate_mp4_ps<R: std::io::Read + std::io::Seek>(
    reader: &mut Mp4TimelineReader<R>, index: usize, max_packets: Option<u64>, check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    negotiate_mp4_ps_with_interval(reader, index, None, max_packets, check)
}

#[allow(dead_code)] // Precise PCM instantiation shares this implementation too.
pub(crate) fn negotiate_mp4_ps_with_interval<R: std::io::Read + std::io::Seek>(
    reader: &mut Mp4TimelineReader<R>, index: usize, interval: Option<(Duration, Duration)>, max_packets: Option<u64>, check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let Some(first) = reader.tracks()[index].samples.get(0) else { return Ok(false); };
    negotiate_mp4_description_ps_with_interval(reader, index, first.description_index, interval, max_packets, check)
}

fn negotiate_mp4_description_ps_with_interval<R: std::io::Read + std::io::Seek>(
    reader: &mut Mp4TimelineReader<R>, index: usize, description: u32, interval: Option<(Duration, Duration)>, max_packets: Option<u64>, mut check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let setup = reader.tracks()[index].decoder_setup(description)?;
    if setup.codec != *b"mp4a" { return Ok(false); }
    let asc = mp4_probe_config(&setup.configuration)?;
    let Ok(mut probe) = InBandPsProbe::new(asc, setup.sample_rate) else { return Ok(false); };
    let active_samples = mp4_active_samples(&reader.tracks()[index], interval, max_packets);
    let mut packet = Vec::new();
    for sample in 0..active_samples {
        check()?;
        if reader.tracks()[index].description_for_sample(sample) != reader.tracks()[index].sample_descriptions.get((description-1) as usize) {
            probe.reset();
            continue;
        }
        reader.read_packet(index, sample, &mut packet)?;
        if probe.read(&packet).is_err() { probe.reset(); continue; }
        if probe.ps_detected() { return Ok(true); }
    }
    Ok(false)
}

include!("owned_mp4_audio_normalized_impl.rs");

#[allow(dead_code)] // Precise PCM instantiation shares this implementation too.
pub(crate) fn mp4_normalized_geometry<R:std::io::Read+std::io::Seek>(reader:&mut Mp4TimelineReader<R>,index:usize,max_packets:Option<u64>)->Result<Option<(u32,u16,u32)>> {
    mp4_normalized_geometry_with_interval(reader,index,None,max_packets)
}
#[allow(dead_code)]
pub(crate) fn mp4_normalized_geometry_with_interval<R:std::io::Read+std::io::Seek>(reader:&mut Mp4TimelineReader<R>,index:usize,interval:Option<(Duration,Duration)>,max_packets:Option<u64>)->Result<Option<(u32,u16,u32)>> {
    mp4_normalized_geometry_controlled_with_interval(reader,index,interval,max_packets,||Ok(()))
}
#[allow(dead_code)] // Precise PCM instantiation shares this implementation too.
pub(crate) fn mp4_normalized_geometry_controlled<R:std::io::Read+std::io::Seek>(reader:&mut Mp4TimelineReader<R>,index:usize,max_packets:Option<u64>,check:impl FnMut()->Result<()>)->Result<Option<(u32,u16,u32)>> {
    mp4_normalized_geometry_controlled_with_interval(reader,index,None,max_packets,check)
}
#[allow(dead_code)] // Precise PCM instantiation shares this implementation too.
pub(crate) fn mp4_normalized_geometry_controlled_with_interval<R:std::io::Read+std::io::Seek>(reader:&mut Mp4TimelineReader<R>,index:usize,interval:Option<(Duration,Duration)>,max_packets:Option<u64>,mut check:impl FnMut()->Result<()>)->Result<Option<(u32,u16,u32)>> {
    let track=&reader.tracks()[index];
    let first=track.samples.get(0).ok_or_else(||invalid("MP4 audio has no samples"))?.description_index;
    let setup=track.decoder_setup(first)?;
    let ps=negotiate_mp4_description_ps_with_interval(reader,index,first,interval,max_packets,&mut check)?;
    let decoder=if ps {Mp4TimelineDecoder::with_in_band_ps(&setup)?}else{Mp4TimelineDecoder::new(&setup)?};
    let initial=(decoder.sample_rate(),decoder.channels(),decoder.normalization_mask());
    let active_samples=mp4_active_samples(&reader.tracks()[index],interval,max_packets);
    for i in 0..reader.tracks()[index].sample_descriptions.len() {
        check()?;
        let track=&reader.tracks()[index];
        if !(0..active_samples).any(|n|track.samples.get(n).is_some_and(|s|s.description_index as usize==i+1)) {continue;}
        let setup=track.decoder_setup((i+1)as u32)?;
        let ps=negotiate_mp4_description_ps_with_interval(reader,index,(i+1)as u32,interval,max_packets,&mut check)?;
        let next=if ps {Mp4TimelineDecoder::with_in_band_ps(&setup)?}else{Mp4TimelineDecoder::new(&setup)?};
        if (next.sample_rate(),next.channels(),next.normalization_mask())!=initial {return Ok(Some(initial));}
    }
    Ok(None)
}
