fn mp4_decode_source_packet<R:std::io::Read+std::io::Seek>(
    reader:&mut Mp4TimelineReader<R>, index:usize, sample:usize,
    decoder:&mut Mp4TimelineDecoder, group:Option<&Mp4Ep1Group>, packet:&mut Vec<u8>,
    pts:u64,duration:u64,edit:Option<usize>,control:&DecodeProgress<'_>,
)->Result<(Option<Vec<Mp4PcmScalar>>,usize)> {
    control.check()?;
    if let Some(group)=group && group.requires_esc(reader.tracks(),sample)? {
        let units=group.read_frame_in_edit_checked(reader,sample,edit,||control.check())?;
        let bytes=units.iter().try_fold(0usize,|n,u|n.checked_add(u.data.len()).ok_or_else(||invalid("EP1 MP4 group byte count overflow")))?;
        let input:Vec<_>=units.iter().map(|u|u.borrowed()).collect();
        control.check()?;
        Ok((decoder.decode_esc(&input)?,bytes))
    } else {
        reader.read_packet(index,sample,packet)?;
        Ok((decoder.decode_timed(packet,pts,duration)?,packet.len()))
    }
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
    let mut ps = vec![false; track.sample_descriptions.len()];
    // Probe each used configuration independently; never feed packets from a
    // different setup into an AAC entropy/history state.
    for (i, flag) in ps.iter_mut().enumerate() {
        if (0..track.samples.len()).any(|n| track.samples.get(n).is_some_and(|s| s.description_index as usize == i+1)) {
            *flag = negotiate_mp4_description_ps(&mut reader, index, (i+1) as u32, || control.check())?;
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
    let mut normalize = target.is_some() || track.timescale % rate != 0
        || (0..track.samples.len()).any(|i|track.samples.get(i).is_some_and(|s|s.pts<0));
    for i in 0..track.samples.len() {
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
    let ep1_group=Mp4Ep1Group::for_root(reader.tracks(),index)?;
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
    // number of edits. Checkpoints also retain their active setup identity.
    let mut checkpoint: Option<(
        usize,
        u32,
        u64,
        Option<u64>,
        Mp4AacCheckpoint,
        std::collections::VecDeque<(u64, u64)>,
        Option<usize>,
    )> = None;
    let contextual_od = reader.has_object_descriptor(track.id);
    for (edit_index, segment) in segments.into_iter().enumerate() {
        let root_edit = (!track.edits.is_empty()).then_some(edit_index);
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
        if let Some((index, description, start, previous, state, pending, saved_edit)) = &checkpoint
            && *start <= from
            && (!contextual_od || *saved_edit == root_edit)
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
                        checkpoint = Some((sample_index, active_description, start, expected, state, pending_window.clone(), root_edit));
                    }
                    captured = true;
                }
                expected = Some(
                    start
                        .checked_add(duration)
                        .ok_or_else(|| invalid("audio timestamp overflow"))?,
                );
                let (samples,bytes)=mp4_decode_source_packet(&mut reader,index,sample_index,&mut decoder,ep1_group.as_ref(),&mut packet,start,duration,root_edit,control)?;
                control.packet(bytes)?;
                sample_index += 1;
                if decoder.delayed() {
                    pending_window.try_reserve(1).map_err(|_| invalid("MP4 source window allocation failed"))?;
                    pending_window.push_back((start, duration));
                    if !decoder.spatial() && pending_window.len() > 3 { return Err(invalid("delayed MP4 source lookahead exceeded")); }
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
    reader: &mut Mp4TimelineReader<R>, index: usize, check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let Some(first) = reader.tracks()[index].samples.get(0) else { return Ok(false); };
    negotiate_mp4_description_ps(reader, index, first.description_index, check)
}

fn negotiate_mp4_description_ps<R: std::io::Read + std::io::Seek>(
    reader: &mut Mp4TimelineReader<R>, index: usize, description: u32, mut check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let setup = reader.tracks()[index].decoder_setup(description)?;
    if setup.codec != *b"mp4a" { return Ok(false); }
    let asc = mp4_probe_config(&setup.configuration)?;
    let Ok(mut probe) = InBandPsProbe::new(asc, setup.sample_rate) else { return Ok(false); };
    // Losing an initial protected SBR header can hide PS forever.
    // Preserve its error if no later PS discovery establishes dispatch.
    let ep1_group=Mp4Ep1Group::for_root(reader.tracks(),index)?;
    let retain_probe_error = probe.requires_in_band_ps() && (setup.channels == 2 || probe.requires_ep_recovery());
    let mut first_error = None;
    let mut packet = Vec::new();
    for sample in 0..reader.tracks()[index].samples.len() {
        check()?;
        if reader.tracks()[index].description_for_sample(sample) != reader.tracks()[index].sample_descriptions.get((description-1) as usize) {
            probe.reset();
            continue;
        }
        let result=if let Some(group)=&ep1_group && group.requires_esc(reader.tracks(),sample)? {
            let units=group.read_frame_in_edit_checked(reader,sample,reader.tracks()[index].edits.iter().position(|e|e.media_time>=0),||check())?;
            let input:Vec<_>=units.iter().map(|u|u.borrowed()).collect();
            probe.read_esc(&input)
        } else {reader.read_packet(index,sample,&mut packet)?;probe.read(&packet)};
        if let Err(error) = result {
            first_error.get_or_insert_with(|| error.to_string());
            probe.reset();
            continue;
        }
        if probe.ps_detected() { return Ok(true); }
    }
    if retain_probe_error && let Some(error) = first_error { return Err(invalid(&error)); }
    Ok(false)
}

include!("owned_mp4_audio_normalized_impl.rs");

#[allow(dead_code)] // Precise PCM instantiation shares this implementation too.
pub(crate) fn mp4_normalized_geometry<R:std::io::Read+std::io::Seek>(reader:&mut Mp4TimelineReader<R>,index:usize)->Result<Option<(u32,u16,u32)>> {
    mp4_normalized_geometry_controlled(reader,index,||Ok(()))
}
pub(crate) fn mp4_normalized_geometry_controlled<R:std::io::Read+std::io::Seek>(reader:&mut Mp4TimelineReader<R>,index:usize,mut check:impl FnMut()->Result<()>)->Result<Option<(u32,u16,u32)>> {
    let track=&reader.tracks()[index];
    let first=track.samples.get(0).ok_or_else(||invalid("MP4 audio has no samples"))?.description_index;
    let setup=track.decoder_setup(first)?;
    let ps=negotiate_mp4_description_ps(reader,index,first,&mut check)?;
    let decoder=if ps {Mp4TimelineDecoder::with_in_band_ps(&setup)?}else{Mp4TimelineDecoder::new(&setup)?};
    let initial=(decoder.sample_rate(),decoder.channels(),decoder.normalization_mask());
    for i in 0..reader.tracks()[index].sample_descriptions.len() {
        check()?;
        let track=&reader.tracks()[index];
        if !(0..track.samples.len()).any(|n|track.samples.get(n).is_some_and(|s|s.description_index as usize==i+1)) {continue;}
        let setup=track.decoder_setup((i+1)as u32)?;
        let ps=negotiate_mp4_description_ps(reader,index,(i+1)as u32,&mut check)?;
        let next=if ps {Mp4TimelineDecoder::with_in_band_ps(&setup)?}else{Mp4TimelineDecoder::new(&setup)?};
        if (next.sample_rate(),next.channels(),next.normalization_mask())!=initial {return Ok(Some(initial));}
    }
    Ok(None)
}
