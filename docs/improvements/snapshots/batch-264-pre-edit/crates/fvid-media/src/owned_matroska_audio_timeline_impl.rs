pub(crate) fn decode_matroska_audio_reader_controlled<R: std::io::Read + std::io::Seek>(
    mut reader: MatroskaTimelineReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
    selected: Option<usize>,
    control: &mut DecodeProgress<'_>,
) -> Result<AudioDecodeStats> {
    control.check()?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    reader.scan_all()?;
    let index = matroska_audio_index(&reader, selected)?;
    let track = reader.tracks[index].clone();
    let in_band_ps = negotiate_matroska_ps(&mut reader, index, || control.check())?;
    let mut decoder = if in_band_ps { MatroskaTimelineDecoder::with_in_band_ps(&track)? } else { MatroskaTimelineDecoder::from_matroska(&track)? };
    let rate = decoder.sample_rate();
    let channels = decoder.channels();
    let exact_clock=matches!(track.codec.as_str(),"A_AAC"|"A_PCM/INT/LIT"|"A_PCM/INT/BIG"|"A_PCM/FLOAT/IEEE"|"A_MPEG/L1"|"A_MPEG/L2"|"A_MPEG/L3"|"A_FLAC"|"A_OPUS"|"A_ALAC");
    if track.sample_rate != u64::from(rate) || (!in_band_ps && track.channels != u64::from(channels)) {
        return Err(invalid(
            "Matroska audio geometry disagrees with configuration",
        ));
    }
    let boundary = |ns: u128| -> Result<u64> {
        let samples = ns
            .checked_mul(u128::from(rate))
            .ok_or_else(|| invalid("audio time overflow"))?
            .div_ceil(1_000_000_000);
        u64::try_from(samples).map_err(|_| invalid("audio sample position overflow"))
    };
    let (from, to) = match interval {
        Some((from, to)) => (boundary(from.as_nanos())?, boundary(to.as_nanos())?),
        None => (0, u64::MAX),
    };
    // Muxers express integral sample counts in rounded nanoseconds. Restore
    // the nearest sample for this metadata; user interval boundaries still ceil.
    let trim_samples = |ns: u64| -> Result<u64> {
        u64::try_from((u128::from(ns) * u128::from(rate) + 500_000_000) / 1_000_000_000)
            .map_err(|_| invalid("audio trim length overflow"))
    };
    let total_delay = trim_samples(track.codec_delay_ns)?;
    let mut delay = total_delay;
    let mut leading_padding = 0u64;
    let mut position = 0u64;
    let mut origin = None;
    // Coded audio timestamp quantization applies to the full coded window, before
    // signed padding. Audible position must not be used as that clock.
    let mut sample_epoch: Option<i64> = None;
    let mut coded_samples = 0u64;
    let mut sample_epoch_grid = None;
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let mut pending_packet: std::collections::VecDeque<_> = reader.packets.iter().take(0).cloned().enumerate().collect();
    let mut index = 0;
    loop {
        control.check()?;
        if position >= to {
            break;
        }
        let terminal = index == reader.packets.len() || control.packet_limit_reached();
        let (packet_index, packet, samples) = if terminal {
            let Some(samples) = decoder.finish()? else {
                break;
            };
            let packet = pending_packet
                .pop_front()
                .ok_or_else(|| invalid("delayed Matroska audio has no source packet"))?;
            (packet.0, packet.1, samples)
        } else {
            let packet = reader.packets[index].clone();
            if packet.track != track.number {
                index += 1;
                continue;
            }
            let source_index=index;
            let encoded = reader.read_packet(index)?;
            let samples = decoder.decode(&encoded)?;
            control.packet(encoded.len())?;
            index += 1;
            if decoder.delayed() {
                pending_packet.push_back((source_index,packet));
                if pending_packet.len() > 3 { return Err(invalid("delayed Matroska source lookahead exceeded")); }
                let Some(samples) = samples else {
                    continue;
                };
                let packet = pending_packet.pop_front()
                    .ok_or_else(|| invalid("delayed Matroska audio source identity mismatch"))?;
                (packet.0, packet.1, samples)
            } else {
                (
                    source_index,
                    packet,
                    samples.ok_or_else(|| invalid("immediate Matroska audio returned no PCM"))?,
                )
            }
        };
        if !samples.len().is_multiple_of(usize::from(channels)) {
            return Err(invalid("invalid Matroska decoded PCM stride"));
        }
        let frames = (samples.len() / usize::from(channels)) as u64;
        let timestamp = if exact_clock {
            let unit = 1_000_000_000i128;
            let grid=reader.packet_timestamp_grid(packet_index);
            let matches=if let (Some(current),Some(epoch))=(grid,sample_epoch_grid) {
                aac_quantized_grid_matches(current,epoch,coded_samples,rate,
                    reader.timestamp_scale_ns(),reader.track_timestamp_scale(track.number).unwrap_or(1.0))
            } else {sample_epoch.is_some_and(|epoch|aac_quantized_sample_matches(
                packet.pts_ns,epoch,coded_samples,rate,reader.timestamp_scale_ns()))};
            if !matches {
                sample_epoch = Some(packet.pts_ns);
                sample_epoch_grid = grid;
                coded_samples = 0;
            }
            let rate = i128::from(rate);
            let projected = i128::from(sample_epoch.unwrap()) * rate + i128::from(coded_samples) * unit;
            coded_samples = coded_samples.checked_add(frames)
                .ok_or_else(|| invalid("audio coded sample clock overflow"))?;
            (projected + unit / 2).div_euclid(unit) - i128::from(total_delay)
        } else {
            let base = *origin.get_or_insert(packet.pts_ns);
            ((i128::from(packet.pts_ns) - i128::from(base)) * i128::from(rate)
                + 500_000_000).div_euclid(1_000_000_000) - i128::from(total_delay)
        };
        // Once coded audio timing has been resolved, explicit trims and gaps
        // are exact sample windows, even when smaller than a container tick.
        let precision = if exact_clock { 0 } else {
            (u128::from(reader.timestamp_scale_ns()) * u128::from(rate) / 1_000_000_000) as u64
        };
        let padding = trim_samples(packet.discard_padding_ns.unsigned_abs())?;
        if padding > frames {
            return Err(invalid("Matroska discard padding exceeds audio packet"));
        }
        let head = if packet.discard_padding_ns < 0 {
            padding
        } else {
            0
        };
        let tail = if packet.discard_padding_ns > 0 {
            padding
        } else {
            0
        };
        let skip_delay = if exact_clock {
            timestamp.saturating_neg().max(0).min(i128::from(frames)) as u64
        } else {
            let skipped = delay.min(frames);
            delay -= skipped;
            skipped
        };
        let first = head.max(skip_delay);
        if !exact_clock && position == 0 && stats.sample_frames == 0 {
            // Leading padding can consume several complete laced packets.
            // Accumulate its origin until audible output starts, rather than
            // reinserting the remaining discarded prefix as timestamp silence.
            leading_padding = leading_padding.checked_add(head.saturating_sub(skip_delay))
                .ok_or_else(|| invalid("Matroska leading padding overflow"))?;
        }
        let end = frames - tail;
        if first > end {
            return Err(invalid("Matroska audio trimming overlaps"));
        }
        let available = end - first;
        stats.decoded_frames += 1;
        if available == 0 {
            continue;
        }
        let audible_start = timestamp + i128::from(first) - i128::from(leading_padding);
        let audible_start =
            if audible_start < 0 && audible_start.unsigned_abs() <= u128::from(precision) {
                0
            } else {
                u64::try_from(audible_start)
                    .map_err(|_| invalid("negative presented Matroska audio timestamp"))?
            };
        if audible_start < position && position - audible_start > precision {
            return Err(invalid("overlapping presented Matroska audio intervals"));
        }
        if audible_start > position && audible_start - position > precision {
            // A gap is silence in the presentation timeline, not a request to
            // concatenate subsequent sample bytes earlier than their timestamp.
            let gap_begin = from.max(position);
            let gap_end = to.min(audible_start);
            if gap_end > gap_begin {
                let mut frames = gap_end - gap_begin;
                let zero = [0u8; 4096];
                let frame_bytes = usize::from(channels) * MatroskaTimelineDecoder::SAMPLE_BYTES;
                let chunk_frames = zero.len() / frame_bytes;
                while frames > 0 {
                    control.check()?;
                    let count = frames.min(chunk_frames as u64) as usize;
                    output.write_all(&zero[..count * frame_bytes])?;
                    frames -= count as u64;
                    stats.sample_frames += count as u64;
                }
            }
            position = audible_start;
        }
        let begin = from.saturating_sub(position).min(available);
        let end = to.saturating_sub(position).min(available).max(begin);
        let width = usize::from(channels);
        for sample in &samples[(first + begin) as usize * width..(first + end) as usize * width] {
            output.write_all(&sample.to_le_bytes())?;
        }
        stats.sample_frames += end - begin;
        position = position
            .checked_add(available)
            .ok_or_else(|| invalid("audio timeline overflow"))?;
    }
    if stats.sample_frames == 0 {
        return Err(invalid("Matroska audio interval contains no samples"));
    }
    Ok(stats)
}

pub(crate) fn negotiate_matroska_ps<R: std::io::Read + std::io::Seek>(
    reader: &mut MatroskaTimelineReader<R>, index: usize, mut check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let track = reader.tracks[index].clone();
    if track.codec != "A_AAC" { return Ok(false); }
    let Ok(rate) = u32::try_from(track.sample_rate) else { return Ok(false); };
    let Ok(mut probe) = InBandPsProbe::new(&track.codec_private, rate) else { return Ok(false); };
    reader.scan_all()?;
    for packet_index in 0..reader.packets.len() {
        check()?;
        if reader.packets[packet_index].track != track.number { continue; }
        let packet = reader.read_packet(packet_index)?;
        if probe.read(&packet).is_err() { probe.reset(); continue; }
        if probe.ps_detected() { return Ok(true); }
    }
    Ok(false)
}
