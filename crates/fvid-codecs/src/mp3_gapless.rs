//! Owned bounded Xing/Info and revision-zero LAME gapless metadata parsing.
use crate::{Result, invalid, mp3_frame::Header};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gapless {
    pub encoder_delay: u16,
    pub end_padding: u16,
}
impl Gapless {
    /// Coded PCM coordinates. Decoder latency moves from the tail to the head;
    /// a negative tail trim is clamped, as in continuous/no-gap encoder output.
    pub fn window(self, coded_samples: u64, decoder_delay: u16) -> Result<std::ops::Range<u64>> {
        let start = u64::from(self.encoder_delay) + u64::from(decoder_delay);
        let tail = u64::from(self.end_padding.saturating_sub(decoder_delay));
        let end = coded_samples
            .checked_sub(tail)
            .ok_or_else(|| invalid("MP3 gapless padding exceeds coded samples"))?;
        if start > end {
            return Err(invalid("MP3 gapless delay exceeds presentation end"));
        }
        Ok(start..end)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub frames: Option<u32>,
    pub bytes: Option<u32>,
    pub gapless: Option<Gapless>,
}
/// Recognize both historical fixed-side offsets and CRC-aware writer offsets.
pub fn tag_offset(packet: &[u8]) -> Result<Option<usize>> {
    let h = Header::parse(packet)?;
    if h.frame_bytes != packet.len() {
        return Err(invalid("MP3 info packet length mismatch"));
    }
    let base = 4 + h.side_bytes();
    for offset in [base, base + if h.crc { 2 } else { 0 }] {
        if packet
            .get(offset..offset + 4)
            .is_some_and(|v| v == b"Xing" || v == b"Info")
        {
            return Ok(Some(offset));
        }
    }
    Ok(None)
}
fn checksum(bytes: &[u8], clear: Option<std::ops::Range<usize>>) -> u16 {
    let mut crc = 0u16;
    for (i, &byte) in bytes.iter().enumerate() {
        crc ^= u16::from(if clear.as_ref().is_some_and(|r| r.contains(&i)) {
            0
        } else {
            byte
        });
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xa001 } else { 0 };
        }
    }
    crc
}
pub fn parse(packet: &[u8]) -> Result<Option<Info>> {
    let Some(offset) = tag_offset(packet)? else {
        return Ok(None);
    };
    let read = |at: usize, n: usize| {
        packet
            .get(at..at + n)
            .ok_or_else(|| invalid("truncated MP3 info fields"))
    };
    let flags = u32::from_be_bytes(read(offset + 4, 4)?.try_into().unwrap());
    if flags & !15 != 0 {
        return Err(invalid("unknown MP3 info field flags"));
    }
    let mut at = offset + 8;
    let mut frames = None;
    let mut bytes = None;
    for (flag, size) in [(1, 4), (2, 4), (4, 100), (8, 4)] {
        if flags & flag == 0 {
            continue;
        }
        let value = read(at, size)?;
        if flag == 1 {
            frames = Some(u32::from_be_bytes(value.try_into().unwrap()));
        }
        if flag == 2 {
            bytes = Some(u32::from_be_bytes(value.try_into().unwrap()));
        }
        at += size;
    }
    let mut info = Info {
        frames,
        bytes,
        gapless: None,
    };
    let Some(tag) = packet.get(at..at + 36) else {
        return Ok(Some(info));
    };
    if tag[9] >> 4 != 0
        || !tag[..9]
            .iter()
            .all(|&v| v == 0 || v.is_ascii_graphic() || v == b' ')
        || !tag[..9].iter().any(u8::is_ascii_alphabetic)
    {
        return Ok(Some(info));
    }
    let stored = u16::from_be_bytes(tag[34..36].try_into().unwrap());
    let standard = checksum(&packet[..at + 34], None) == stored;
    // Some Lavc/Lavf writers checksum a fixed 190-byte prefix, with the tag
    // CRC slot still zero. This is a verified on-disk format compatibility rule.
    let fixed = flags == 15
        && (tag.starts_with(b"Lavc") || tag.starts_with(b"Lavf"))
        && packet
            .get(..190)
            .is_some_and(|p| checksum(p, Some(at + 34..at + 36)) == stored);
    if standard || fixed {
        info.gapless = Some(Gapless {
            encoder_delay: (u16::from(tag[21]) << 4) | u16::from(tag[22] >> 4),
            end_padding: (u16::from(tag[22] & 15) << 8) | u16::from(tag[23]),
        });
    }
    Ok(Some(info))
}
