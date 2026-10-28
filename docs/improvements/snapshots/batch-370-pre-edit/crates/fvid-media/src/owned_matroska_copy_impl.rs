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
    inspect_with_packet_and_memory_limits(input, audio_only, cancel, packet_bytes, None)
}
pub(crate) fn inspect_with_packet_and_memory_limits<R: Read + Seek>(
    input: &mut R,
    audio_only: bool,
    cancel: Option<&CancelFlag>,
    packet_bytes: usize,
    max_controlled_bytes: Option<usize>,
) -> Result<Vec<(u64, usize)>> {
    if packet_bytes == 0 { return Err(invalid("packet byte limit must be positive")); }
    if max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    check(cancel)?;
    input.seek(SeekFrom::Start(0))?;
    let effective_packet_bytes = packet_bytes.min(max_controlled_bytes.unwrap_or(usize::MAX));
    let memory_packets = max_controlled_bytes
        .map(|limit| limit / std::mem::size_of::<Packet>());
    let mut limits = Limits {
        packet_bytes: effective_packet_bytes,
        ..Default::default()
    };
    if let Some(maximum) = memory_packets {
        limits.packets = limits.packets.min(maximum);
    }
    let map_packet_limit = |error: Error| {
        if memory_packets.is_some_and(|max| max < Limits::default().packets)
            && error
                .to_string()
                .contains("WebM packet count exceeds limit")
        {
            invalid("controlled memory budget exceeded: Matroska packet index limit")
        } else {
            error
        }
    };
    let mut reader = WebmReader::open(input, limits).map_err(|e| map_packet_limit(e.into()))?;
    loop {
        check(cancel)?;
        if !reader.scan_more().map_err(|e| map_packet_limit(e.into()))? {
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
    let index_bytes = reader
        .packets
        .len()
        .checked_mul(std::mem::size_of::<(u64, usize)>())
        .and_then(|n| n.checked_add(64 << 10))
        .ok_or_else(|| invalid("Matroska copy index overflow"))?;
    if max_controlled_bytes.is_some_and(|limit| index_bytes > limit) {
        return Err(invalid("controlled memory budget exceeded"));
    }
    let mut ends = Vec::new();
    ends.try_reserve_exact(reader.packets.len())
        .map_err(|_| invalid("cannot allocate Matroska copy index"))?;
    for packet in reader.packets {
        check(cancel)?;
        if max_controlled_bytes.is_some_and(|limit| packet.size > limit) {
            return Err(invalid("controlled memory budget exceeded"));
        }
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
    copy_with_packet_and_memory_limits(input, output, audio_only, cancel, progress, packet_bytes, None)
}

pub fn copy_with_packet_and_memory_limits<R: Read + Seek, W: Write>(
    input: &mut R,
    output: &mut W,
    audio_only: bool,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    packet_bytes: usize,
    max_controlled_bytes: Option<usize>,
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
    let packets = inspect_with_packet_and_memory_limits(
        input,
        audio_only,
        cancel,
        packet_bytes,
        max_controlled_bytes,
    )?;
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
    let tail_bytes = loop {
        check(cancel)?;
        match input.read(&mut tail) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => break result?,
        }
    };
    if tail_bytes != 0 {
        return Err(invalid("Matroska source length changed during copy"));
    }
    Ok(event)
}
