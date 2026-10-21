//! Complete sample-frame assembly, shared by raw and Matroska packet routes.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ChannelInfo {
    pub channels: u16,
    pub mask: u32,
    streams: Option<u16>,
}
impl ChannelInfo {
    pub fn read(data: &[u8]) -> Result<Self> {
        if data.is_empty() || data.len() > 7 {
            return Err(invalid("invalid WavPack channel information length"));
        }
        let (channels, streams, mask_bytes) = if data.len() >= 6 {
            let channels = u16::from(data[0]) | ((u16::from(data[2]) & 15) << 8);
            let streams = u16::from(data[1]) | ((u16::from(data[2]) & 240) << 4);
            (channels + 1, Some(streams + 1), &data[3..])
        } else {
            (u16::from(data[0]), None, &data[1..])
        };
        let mask = mask_bytes
            .iter()
            .enumerate()
            .fold(0u32, |m, (i, &b)| m | (u32::from(b) << (i * 8)));
        if channels == 0
            || channels > MAX_CHANNELS
            || mask.count_ones() > u32::from(channels)
            || streams.is_some_and(|s| s > channels || u32::from(s) * 2 < u32::from(channels))
        {
            return Err(invalid("invalid WavPack declared channel layout"));
        }
        Ok(Self {
            channels,
            mask,
            streams,
        })
    }
}
pub(super) struct Frame<'a> {
    pub blocks: Vec<Block<'a>>,
    pub mask: Option<u32>,
}
impl<'a> Frame<'a> {
    pub fn parse(data: &'a [u8], channels: u16) -> Result<Self> {
        let mut blocks: Vec<Block<'a>> = Vec::new();
        let mut at = 0;
        let mut output = 0u16;
        let mut layout = None;
        while at < data.len() {
            let block = Block::parse(data, at)?;
            if (block.flags & INITIAL_BLOCK != 0) != blocks.is_empty() {
                return Err(invalid("WavPack initial block sequence mismatch"));
            }
            let next = at
                .checked_add(block.length)
                .ok_or_else(|| invalid("WavPack frame length overflow"))?;
            if block.flags & FINAL_BLOCK != 0 && next != data.len() {
                return Err(invalid(
                    "WavPack packet contains trailing bytes after final block",
                ));
            }
            if block.flags & FINAL_BLOCK == 0 && next == data.len() {
                return Err(invalid("WavPack frame is missing final block"));
            }
            if let Some(first) = blocks.first() {
                if block.block_samples != first.block_samples
                    || block.block_index != first.block_index
                {
                    return Err(invalid("WavPack component block sample window mismatch"));
                }
                if block.sample_rate != first.sample_rate
                    || block.bits_per_sample != first.bits_per_sample
                    || block.flags & (FLOAT | DSD_DATA) != first.flags & (FLOAT | DSD_DATA)
                    || block.version != first.version
                {
                    return Err(invalid("WavPack component block format mismatch"));
                }
                if first
                    .total_samples
                    .zip(block.total_samples)
                    .is_some_and(|(a, b)| a != b)
                {
                    return Err(invalid("WavPack component total sample count mismatch"));
                }
            }
            if let Some(info) = block.channel_info {
                if info.channels != channels || layout.is_some_and(|old| old != info) {
                    return Err(invalid(
                        "WavPack channel information disagrees with container or frame",
                    ));
                }
                layout = Some(info);
            }
            output = output
                .checked_add(block.channels)
                .ok_or_else(|| invalid("WavPack channel count overflow"))?;
            if output > channels {
                return Err(invalid(
                    "WavPack block's channels disagree with the container",
                ));
            }
            blocks
                .try_reserve(1)
                .map_err(|_| invalid("cannot allocate WavPack component index"))?;
            blocks.push(block);
            at = next;
        }
        if blocks.is_empty() {
            return Err(invalid("WavPack block tag or header missing"));
        }
        if output != channels {
            return Err(invalid(
                "WavPack block's channels disagree with the container",
            ));
        }
        if layout
            .is_some_and(|i: ChannelInfo| i.streams.is_some_and(|s| blocks.len() > usize::from(s)))
        {
            return Err(invalid(
                "WavPack frame exceeds declared component stream count",
            ));
        }
        let mask = layout.map(|i| i.mask);
        Ok(Self { blocks, mask })
    }
}
fn raw_header(
    out: &mut Vec<u8>,
    version: u16,
    samples: u32,
    flags: u32,
    crc: u32,
    body: &[u8],
) -> Result<()> {
    let size = body
        .len()
        .checked_add(24)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| invalid("Matroska WavPack block size overflow"))?;
    out.try_reserve(body.len() + 32)
        .map_err(|_| invalid("cannot allocate Matroska WavPack block"))?;
    out.extend_from_slice(b"wvpk");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&samples.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(body);
    Ok(())
}
pub(super) fn restore_matroska(data: &[u8], version: u16, channels: u16) -> Result<Vec<u8>> {
    if data.len() < 12 {
        return Err(invalid("truncated Matroska WavPack block header"));
    }
    let samples = le32(data, 0);
    let mut at = 4usize;
    let mut out = Vec::new();
    let mut count = 0;
    loop {
        let head = if channels > 2 { 12 } else { 8 };
        if at.checked_add(head).is_none_or(|n| n > data.len()) {
            return Err(invalid("truncated Matroska WavPack component header"));
        }
        let flags = le32(data, at);
        let crc = le32(data, at + 4);
        let length = if channels > 2 {
            le32(data, at + 8) as usize
        } else {
            data.len() - at - head
        };
        at += head;
        let end = at
            .checked_add(length)
            .filter(|&n| n <= data.len())
            .ok_or_else(|| invalid("Matroska WavPack component exceeds packet"))?;
        count += 1;
        if count > usize::from(channels) {
            return Err(invalid("too many Matroska WavPack component blocks"));
        }
        raw_header(&mut out, version, samples, flags, crc, &data[at..end])?;
        at = end;
        if at == data.len() {
            break;
        }
    }
    Ok(out)
}
