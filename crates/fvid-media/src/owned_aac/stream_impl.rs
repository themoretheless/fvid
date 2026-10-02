pub(crate) fn decode_adts_aac_reader_controlled<R: std::io::Read>(
    mut reader: AdtsStreamReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
    control: &mut DecodeProgress<'_>,
) -> Result<AudioDecodeStats> {
    control.check()?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    let config = reader.configuration();
    let boundary = |time: Duration| -> Result<u64> {
        let ticks = time.as_nanos().checked_mul(u128::from(config.sample_rate))
            .ok_or_else(|| invalid("audio interval overflow"))?;
        u64::try_from(ticks.div_ceil(1_000_000_000)).map_err(|_| invalid("audio interval overflow"))
    };
    let (from, to) = match interval {
        Some((from, to)) => (boundary(from)?, boundary(to)?),
        None => (0, u64::MAX),
    };
    let mut decoder = AdtsPacketDecoder::new(reader.audio_specific_config())?;
    let mut stats = AudioDecodeStats { sample_frames: 0, decoded_frames: 0,
        sample_rate: config.sample_rate, channels: config.channels };
    let channels = usize::from(config.channels);
    let mut position = 0u64;
    while position < to {
        control.check()?;
        if control.packet_limit_reached() { break; }
        let Some(packet) = reader.next_packet()? else { break; };
        let samples = decoder.decode(&packet)?;
        let frames = (samples.len() / channels) as u64;
        let end = position.checked_add(frames).ok_or_else(|| invalid("audio position overflow"))?;
        let first = from.saturating_sub(position).min(frames) as usize;
        let last = to.saturating_sub(position).min(frames) as usize;
        write_pcm_samples(output, &samples[first * channels..last.max(first) * channels])?;
        stats.sample_frames += last.saturating_sub(first) as u64;
        stats.decoded_frames += 1;
        control.packet(packet.len())?;
        position = end;
    }
    if stats.sample_frames == 0 { return Err(invalid("audio interval contains no samples")); }
    Ok(stats)
}

// Serialization scratch stays on the stack and does not grow with stream length.
fn write_pcm_samples(output: &mut impl std::io::Write, samples: &[f32]) -> Result<()> {
    let mut bytes = [0u8; 4096];
    for chunk in samples.chunks(bytes.len() / 4) {
        for (sample, target) in chunk.iter().zip(bytes.chunks_exact_mut(4)) {
            target.copy_from_slice(&sample.to_le_bytes());
        }
        output.write_all(&bytes[..chunk.len() * 4])?;
    }
    Ok(())
}
