pub fn header_channels(header: &[u8]) -> Result<u8> {
    // Share the identification parser with the native single/multistream
    // decoder; transport must preserve valid channel mappings and versions.
    let head = fvid_opus::OpusHead::parse(header).map_err(|e| invalid(&e.to_string()))?;
    Ok(head.channel_count)
}

pub fn pre_skip_ns(header: &[u8]) -> Result<u64> {
    header_channels(header)?;
    Ok(u64::from(u16::from_le_bytes([header[10], header[11]])) * 1_000_000_000 / 48_000)
}

/// Read multistream boundaries from the identification header before timing.
pub fn duration_ns_with_header(header: &[u8], packet: &[u8]) -> Result<u64> {
    let head=fvid_opus::OpusHead::parse(header).map_err(|e|invalid(&e.to_string()))?;
    if head.stream_count==1 {return duration_ns(packet);}
    let samples=fvid_opus::multistream_packet_samples(packet,usize::from(head.stream_count))
        .map_err(|e|invalid(&e.to_string()))?;
    Ok(samples as u64*1_000_000_000/48_000)
}

/// Validate packet framing and return its decoded duration, without inspecting entropy data.
pub fn duration_ns(packet: &[u8]) -> Result<u64> {
    let toc = *packet.first().ok_or_else(|| invalid("empty Opus packet"))?;
    let config = toc >> 3;
    let frame_us = if config < 12 {
        [10_000, 20_000, 40_000, 60_000][usize::from(config & 3)]
    } else if config < 16 {
        [10_000, 20_000][usize::from(config & 1)]
    } else {
        [2_500, 5_000, 10_000, 20_000][usize::from(config & 3)]
    };
    let mut offset = 1;
    let mut end = packet.len();
    let code = toc & 3;
    let (count, vbr) = match code {
        0 => (1, false),
        1 => (2, false),
        2 => (2, true),
        _ => {
            let control = *packet
                .get(offset)
                .ok_or_else(|| invalid("missing Opus frame count"))?;
            offset += 1;
            if control & 64 != 0 {
                let mut padding = 0usize;
                loop {
                    let byte = *packet
                        .get(offset)
                        .ok_or_else(|| invalid("truncated Opus padding"))?;
                    offset += 1;
                    padding = padding
                        .checked_add(if byte == 255 { 254 } else { usize::from(byte) })
                        .ok_or_else(|| invalid("Opus padding overflow"))?;
                    if byte != 255 {
                        break;
                    }
                }
                end = end
                    .checked_sub(padding)
                    .filter(|&n| n >= offset)
                    .ok_or_else(|| invalid("Opus padding exceeds packet"))?;
            }
            (usize::from(control & 63), control & 128 != 0)
        }
    };
    if count == 0 || frame_us * count > 120_000 {
        return Err(invalid("invalid Opus packet duration"));
    }
    if vbr {
        let mut sizes = 0usize;
        for _ in 1..count {
            let first = *packet
                .get(offset)
                .filter(|_| offset < end)
                .ok_or_else(|| invalid("truncated Opus frame length"))?;
            offset += 1;
            let size = if first < 252 {
                usize::from(first)
            } else {
                let second = *packet
                    .get(offset)
                    .filter(|_| offset < end)
                    .ok_or_else(|| invalid("truncated Opus frame length"))?;
                offset += 1;
                usize::from(first) + 4 * usize::from(second)
            };
            sizes += size;
        }
        if sizes > end.saturating_sub(offset) || end - offset - sizes > 1275 {
            return Err(invalid("invalid Opus frame lengths"));
        }
    } else if !(end - offset).is_multiple_of(count) || (end - offset) / count > 1275 {
        return Err(invalid("invalid Opus CBR frame lengths"));
    }
    Ok((frame_us * count) as u64 * 1000)
}
