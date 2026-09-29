//! Owned single-track AAC Matroska muxing. No codec or external muxer is used.
//! Mapping: https://www.matroska.org/technical/codec_specs.html#a_aac
use crate::{Result, invalid};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::io::{Read, Seek, SeekFrom, Write};
fn size(value: u64) -> Result<Vec<u8>> {
    for width in 1..=8 {
        if value < (1u64 << (7 * width)) - 1 {
            let encoded = (value | (1u64 << (7 * width))).to_be_bytes();
            return Ok(encoded[8 - width..].to_vec());
        }
    }
    Err(invalid("Matroska element exceeds size range"))
}
fn head(output: &mut impl Write, id: u32, length: u64) -> Result<()> {
    let bytes = id.to_be_bytes();
    let start = bytes
        .iter()
        .position(|&b| b != 0)
        .ok_or_else(|| invalid("zero EBML ID"))?;
    output.write_all(&bytes[start..])?;
    output.write_all(&size(length)?)?;
    Ok(())
}
fn element(id: u32, data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    head(&mut out, id, data.len() as u64)?;
    out.extend_from_slice(data);
    Ok(out)
}
fn uint(id: u32, value: u64) -> Result<Vec<u8>> {
    let b = value.to_be_bytes();
    element(id, &b[b.iter().position(|&v| v != 0).unwrap_or(7)..])
}

/// Stream strict ADTS packets into Matroska. Per-packet clusters avoid signed
/// block timestamp overflow at a 1 ns clock. No cue table/index is accumulated.
/// Caller must discard partial output after error and publish only on success.
pub fn write_adts<R: Read, W: Write + Seek>(
    mut input: super::adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let check = || {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            Err(invalid("media operation cancelled"))
        } else {
            Ok(())
        }
    };
    check()?;
    if output.stream_position()? != 0 {
        return Err(invalid("Matroska output must start at zero"));
    }
    let config = input.configuration();
    let samples = u64::from(crate::codec::config::AacConfig::parse(&config.asc)?.frame_samples);
    let ebml = [
        uint(0x4286, 1)?,
        uint(0x42f7, 1)?,
        uint(0x42f2, 4)?,
        uint(0x42f3, 8)?,
        element(0x4282, b"matroska")?,
        uint(0x4287, 4)?,
        uint(0x4285, 2)?,
    ]
    .concat();
    output.write_all(&element(0x1a45dfa3, &ebml)?)?;
    output.write_all(&0x18538067u32.to_be_bytes())?;
    let segment_size = output.stream_position()?;
    output.write_all(&[1, 255, 255, 255, 255, 255, 255, 255])?;
    let info = element(
        0x1549a966,
        &[
            uint(0x2ad7b1, 1)?,
            element(0x4d80, b"FVid")?,
            element(0x5741, b"FVid")?,
            element(0x4489, &0f64.to_be_bytes())?,
        ]
        .concat(),
    )?;
    let duration_offset = output.stream_position()? + info.len() as u64 - 8;
    output.write_all(&info)?;
    let audio = element(
        0xe1,
        &[
            element(0xb5, &f64::from(config.sample_rate).to_be_bytes())?,
            uint(0x9f, u64::from(config.channels))?,
        ]
        .concat(),
    )?;
    let track = element(
        0xae,
        &[
            uint(0xd7, 1)?,
            uint(0x73c5, 1)?,
            uint(0x83, 2)?,
            uint(0x9c, 0)?,
            element(0x86, b"A_AAC")?,
            element(0x63a2, &config.asc)?,
            audio,
        ]
        .concat(),
    )?;
    output.write_all(&element(0x1654ae6b, &track)?)?;
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    let time = |packet: u64| -> Result<u64> {
        u64::try_from(
            u128::from(packet) * u128::from(samples) * 1_000_000_000
                / u128::from(config.sample_rate),
        )
        .map_err(|_| invalid("Matroska timestamp overflow"))
    };
    if let Some(h) = progress {
        h.emit(event);
    }
    loop {
        check()?;
        let Some(packet) = input.next_packet()? else {
            break;
        };
        let next = event
            .packets
            .checked_add(1)
            .ok_or_else(|| invalid("packet count overflow"))?;
        let start = time(event.packets)?;
        let end = time(next)?;
        let timestamp = uint(0xe7, start)?;
        let duration = uint(0x9b, end - start)?;
        let block = 4 + packet.len() as u64;
        let group = 1 + size(block)?.len() as u64 + block + duration.len() as u64;
        let cluster = timestamp.len() as u64 + 1 + size(group)?.len() as u64 + group;
        head(output, 0x1f43b675, cluster)?;
        output.write_all(&timestamp)?;
        head(output, 0xa0, group)?;
        head(output, 0xa1, block)?;
        output.write_all(&[0x81, 0, 0, 0])?;
        output.write_all(&packet)?;
        output.write_all(&duration)?;
        event.packets = next;
        event.payload_bytes = event
            .payload_bytes
            .checked_add(packet.len() as u64)
            .ok_or_else(|| invalid("payload count overflow"))?;
        if let Some(h) = progress {
            h.emit(event);
        }
    }
    check()?;
    if event.packets == 0 {
        return Err(invalid("no AAC packets"));
    }
    let end = output.stream_position()?;
    let length = end - segment_size - 8;
    if length >= (1u64 << 56) - 1 {
        return Err(invalid("Matroska segment exceeds size range"));
    }
    output.seek(SeekFrom::Start(segment_size))?;
    output.write_all(&(length | (1u64 << 56)).to_be_bytes())?;
    output.seek(SeekFrom::Start(duration_offset))?;
    output.write_all(&(time(event.packets)? as f64).to_be_bytes())?;
    output.seek(SeekFrom::Start(end))?;
    check()?;
    Ok(event)
}
