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
    let mut decoder = MatroskaTimelineDecoder::from_matroska(&track)?;
    let rate = decoder.sample_rate();
    let channels = decoder.channels();
    if track.sample_rate != u64::from(rate) || track.channels != u64::from(channels) {
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
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    for index in 0..reader.packets.len() {
        control.check()?;
        if control.packet_limit_reached() { break; }
        let packet = reader.packets[index].clone();
        if packet.track != track.number {
            continue;
        }
        if position >= to {
            break;
        }
        let base = *origin.get_or_insert(packet.pts_ns);
        let timestamp = ((i128::from(packet.pts_ns) - i128::from(base)) * i128::from(rate)
            + 500_000_000).div_euclid(1_000_000_000) - i128::from(total_delay);
        let precision = (u128::from(reader.timestamp_scale_ns()) * u128::from(rate)
            / 1_000_000_000) as u64;
        let encoded = reader.read_packet(index)?;
        let samples = decoder.decode(&encoded)?;
        let frames = (samples.len() / usize::from(channels)) as u64;
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
        let skip_delay = delay.min(frames);
        delay -= skip_delay;
        let first = head.max(skip_delay);
        if stats.decoded_frames == 0 {
            // Leading discarded samples establish the presentation origin;
            // reinserting them as a timestamp gap would undo DiscardPadding.
            leading_padding = head.saturating_sub(skip_delay);
        }
        let end = frames - tail;
        if first > end {
            return Err(invalid("Matroska audio trimming overlaps"));
        }
        let available = end - first;
        stats.decoded_frames += 1;
        control.packet(encoded.len())?;
        if available == 0 { continue; }
        let audible_start = timestamp + i128::from(first) - i128::from(leading_padding);
        let audible_start = if audible_start < 0 && audible_start.unsigned_abs() <= u128::from(precision) {
            0
        } else {
            u64::try_from(audible_start).map_err(|_| invalid("negative presented Matroska audio timestamp"))?
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
        position = position.checked_add(available).ok_or_else(||invalid("audio timeline overflow"))?;
    }
    if stats.sample_frames == 0 {
        return Err(invalid("Matroska audio interval contains no samples"));
    }
    Ok(stats)
}
