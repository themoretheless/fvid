fn check(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(CancelFlag::is_cancelled) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
pub(crate) fn inspect<R: Read + Seek>(
    input: &mut R,
    audio_only: bool,
    cancel: Option<&CancelFlag>,
) -> Result<Vec<(u64, usize)>> {
    inspect_with_packet_limit(input, audio_only, cancel, 64 << 20)
}
pub(crate) fn inspect_with_packet_limit<R: Read + Seek>(
    input: &mut R, audio_only: bool, cancel: Option<&CancelFlag>, packet_bytes: usize,
) -> Result<Vec<(u64, usize)>> {
    if packet_bytes == 0 { return Err(invalid("packet byte limit must be positive")); }
    check(cancel)?;
    input.seek(SeekFrom::Start(0))?;
    let mut reader = WebmReader::open(
        input,
        Limits {
            packet_bytes,
            ..Default::default()
        },
    )?;
    loop {
        check(cancel)?;
        if !reader.scan_more()? {
            break;
        }
    }
    if reader.tracks.is_empty() || reader.packets.is_empty() {
        return Err(invalid("Matroska copy requires tracks and media packets"));
    }
    if audio_only && reader.tracks.iter().any(|track| track.kind != 2) {
        return Err(invalid(
            ".mka output requires audio-only Matroska input; no tracks are dropped",
        ));
    }
    let mut ends = Vec::new();
    ends.try_reserve_exact(reader.packets.len())
        .map_err(|_| invalid("cannot allocate Matroska copy index"))?;
    for packet in reader.packets {
        check(cancel)?;
        let end = packet
            .offset
            .checked_add(
                u64::try_from(packet.size).map_err(|_| invalid("Matroska packet size overflow"))?,
            )
            .ok_or_else(|| invalid("Matroska packet end overflow"))?;
        ends.push((end, packet.size));
    }
    ends.sort_unstable_by_key(|p| p.0);
    Ok(ends)
}

/// Copy a validated container without reconstructing or discarding EBML fields.
/// Progress counts complete media packets, not container overhead; publication
/// and the final `done` notification belong to the caller.
pub fn copy<R: Read + Seek, W: Write>(
    input: &mut R,
    output: &mut W,
    audio_only: bool,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    copy_with_packet_limit(input, output, audio_only, cancel, progress, 64 << 20)
}

/// Identity copy with a caller-selected maximum encoded packet size.
pub fn copy_with_packet_limit<R: Read + Seek, W: Write>(
    input: &mut R, output: &mut W, audio_only: bool,
    cancel: Option<&CancelFlag>, progress: Option<&ProgressHook>, packet_bytes: usize,
) -> Result<ProgressEvent> {
    check(cancel)?;
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    if let Some(hook) = progress {
        hook.emit(event);
    }
    let packets = inspect_with_packet_limit(input, audio_only, cancel, packet_bytes)?;
    let length = input.seek(SeekFrom::End(0))?;
    if packets.last().is_some_and(|(end, _)| *end > length) {
        return Err(invalid("Matroska packet exceeds source length"));
    }
    input.seek(SeekFrom::Start(0))?;
    let mut buffer = buffer(64 << 10)?;
    let mut copied = 0u64;
    let mut next = 0usize;
    while copied < length {
        check(cancel)?;
        let count = (length - copied).min(buffer.len() as u64) as usize;
        input.read_exact(&mut buffer[..count])?;
        output.write_all(&buffer[..count])?;
        copied += count as u64;
        while next < packets.len() && packets[next].0 <= copied {
            event.packets += 1;
            event.payload_bytes = event
                .payload_bytes
                .checked_add(packets[next].1 as u64)
                .ok_or_else(|| invalid("Matroska payload count overflow"))?;
            next += 1;
        }
        if let Some(hook) = progress {
            hook.emit(event);
        }
    }
    check(cancel)?;
    let mut tail = [0];
    if input.read(&mut tail)? != 0 {
        return Err(invalid("Matroska source length changed during copy"));
    }
    Ok(event)
}
