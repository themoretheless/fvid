use std::fs::File;
// Owned strict RIFF/WAVE PCM inspection, shared by probe and editing.
use fvid_control::CancelFlag;
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};
type Result<T> = std::io::Result<T>;
fn invalid(s: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, s)
}
#[derive(Debug, Clone, Copy)]
pub struct WaveInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub float: bool,
    pub valid_bits: u16,
    pub channel_mask: u32,
    pub data_offset: u64,
    pub sample_frames: u64,
    pub block: u16,
    pub data_bytes: u32,
    pub end: u64,
}

impl WaveInfo {
    /// Codec name for the stored PCM precision; no decoded-width conversion.
    pub fn codec(&self) -> String {
        if self.float {
            format!("pcm_f{}le", self.bits_per_sample)
        } else if self.bits_per_sample == 8 {
            "pcm_u8".into()
        } else {
            format!("pcm_s{}le", self.bits_per_sample)
        }
    }
}

pub fn is_wave(path: &Path) -> std::io::Result<bool> {
    let mut file = File::open(path)?;
    let mut prefix = [0; 12];
    match file.read_exact(&mut prefix) {
        Ok(()) => {
            Ok(matches!(&prefix[..4], b"RIFF" | b"RIFX" | b"RF64") && &prefix[8..] == b"WAVE")
        }
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}
fn check(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(|c| c.is_cancelled()) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
fn chunk<R: Read + Seek>(input: &mut R, at: u64, end: u64) -> Result<([u8; 4], u32, u64)> {
    if end - at < 8 {
        return Err(invalid("truncated WAVE chunk header"));
    }
    input.seek(SeekFrom::Start(at))?;
    let mut head = [0; 8];
    input.read_exact(&mut head)?;
    let size = u32::from_le_bytes(head[4..].try_into().unwrap());
    let next = at + 8 + u64::from(size) + u64::from(size & 1);
    if next > end {
        return Err(invalid("WAVE chunk exceeds RIFF extent"));
    }
    Ok((head[..4].try_into().unwrap(), size, next))
}
fn format(bytes: &[u8]) -> Result<(u32, u16, u16, bool, u16, u16, u32)> {
    if !matches!(bytes.len(), 16 | 18 | 40) {
        return Err(invalid("unsupported WAVE format extension"));
    }
    let u16at = |n| u16::from_le_bytes(bytes[n..n + 2].try_into().unwrap());
    let u32at = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().unwrap());
    let (mut tag, channels, rate, byte_rate, block, bits) =
        (u16at(0), u16at(2), u32at(4), u32at(8), u16at(12), u16at(14));
    let (mut valid_bits, mut channel_mask) = (bits, 0);
    if tag == 0xfffe {
        if bytes.len() != 40
            || u16at(16) != 22
            || bytes[28..40] != [0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]
        {
            return Err(invalid("invalid extensible WAVE format"));
        }
        let sub = u32at(24);
        if sub != 1 && sub != 3 {
            return Err(invalid("WAVE trim requires uncompressed PCM"));
        }
        tag = sub as u16;
        let valid = u16at(18);
        let mask = u32at(20);
        valid_bits = valid;
        channel_mask = mask;
        if valid == 0
            || valid > bits
            || (tag == 3 && valid != bits)
            || (mask != 0 && mask.count_ones() != u32::from(channels))
        {
            return Err(invalid("invalid WAVE valid bits or channel mask"));
        }
    } else if bytes.len() != 16 && (bytes.len() != 18 || u16at(16) != 0) {
        return Err(invalid("invalid PCM format extension"));
    }
    if !((tag == 1 && matches!(bits, 8 | 16 | 24 | 32)) || (tag == 3 && matches!(bits, 32 | 64))) {
        return Err(invalid(
            "WAVE trim requires packed integer or IEEE float PCM",
        ));
    }
    if channels == 0
        || rate == 0
        || u32::from(channels) * u32::from(bits / 8) != u32::from(block)
        || rate.checked_mul(u32::from(block)) != Some(byte_rate)
    {
        return Err(invalid("inconsistent WAVE sample geometry"));
    }
    Ok((
        rate,
        channels,
        bits,
        tag == 3,
        block,
        valid_bits,
        channel_mask,
    ))
}

/// Strict RIFF bounds; only metadata safe to retain unchanged is admitted.
/// Timed cue/loop/broadcast metadata needs its own retiming before it can be copied.
pub fn inspect<R: Read + Seek>(input: &mut R, cancel: Option<&CancelFlag>) -> Result<WaveInfo> {
    inspect_impl(input, cancel, true)
}
/// Read PCM geometry without copying or retiming opaque ancillary chunks.
/// All chunk extents and audio geometry remain strictly validated.
pub(crate) fn inspect_probe<R: Read + Seek>(input: &mut R) -> Result<WaveInfo> {
    inspect_impl(input, None, false)
}
fn inspect_impl<R: Read + Seek>(
    input: &mut R,
    cancel: Option<&CancelFlag>,
    preserve: bool,
) -> Result<WaveInfo> {
    check(cancel)?;
    let length = input.seek(SeekFrom::End(0))?;
    input.seek(SeekFrom::Start(0))?;
    let mut head = [0; 12];
    input.read_exact(&mut head)?;
    if &head[..4] != b"RIFF" || &head[8..] != b"WAVE" {
        return Err(invalid(
            "native PCM trim requires RIFF/WAVE (RF64 and RIFX are not implemented)",
        ));
    }
    let end = u64::from(u32::from_le_bytes(head[4..8].try_into().unwrap())) + 8;
    if end != length || end < 12 {
        return Err(invalid("RIFF extent disagrees with file length"));
    }
    let (mut fmt, mut data, mut fact) = (None, None, None);
    let mut at = 12;
    let mut data_offset = 0;
    while at < end {
        check(cancel)?;
        let (tag, size, next) = chunk(input, at, end)?;
        match &tag {
            b"fmt " => {
                if fmt.is_some() || !matches!(size, 16 | 18 | 40) {
                    return Err(invalid("duplicate or unsupported WAVE format chunk"));
                }
                let mut bytes = [0; 40];
                input.read_exact(&mut bytes[..size as usize])?;
                fmt = Some(format(&bytes[..size as usize])?);
            }
            b"data" => {
                data_offset = at + 8;
                if data.replace(size).is_some() {
                    return Err(invalid("multiple WAVE data chunks are not implemented"));
                }
            }
            b"fact" => {
                if fact.is_some() || size != 4 {
                    return Err(invalid("duplicate or unsupported WAVE fact chunk"));
                }
                let mut bytes = [0; 4];
                input.read_exact(&mut bytes)?;
                fact = Some(u32::from_le_bytes(bytes));
            }
            b"LIST" => {
                let mut kind = [0; 4];
                if size < 4 {
                    return Err(invalid("short WAVE LIST chunk"));
                }
                input.read_exact(&mut kind)?;
                if preserve && &kind != b"INFO" {
                    return Err(invalid(
                        "native PCM trim requires retiming for non-INFO WAVE lists",
                    ));
                }
            }
            b"JUNK" | b"PAD " => (),
            _ if preserve => {
                return Err(invalid(&format!(
                    "native PCM trim cannot yet preserve WAVE chunk {:?}",
                    String::from_utf8_lossy(&tag)
                )));
            }
            _ => (),
        }
        at = next;
    }
    let (rate, channels, bits, float, block, valid_bits, channel_mask) =
        fmt.ok_or_else(|| invalid("missing WAVE format"))?;
    let data_bytes = data.ok_or_else(|| invalid("missing WAVE data"))?;
    if data_bytes % u32::from(block) != 0 {
        return Err(invalid("partial WAVE sample frame"));
    }
    let frames = u64::from(data_bytes) / u64::from(block);
    if fact.is_some_and(|n| u64::from(n) != frames) {
        return Err(invalid("WAVE fact count disagrees with PCM payload"));
    }
    Ok(WaveInfo {
        sample_rate: rate,
        channels,
        bits_per_sample: bits,
        float,
        sample_frames: frames,
        valid_bits,
        channel_mask,
        data_offset,
        block,
        data_bytes,
        end,
    })
}

/// Retain non-timed INFO lists exactly, including unknown entries and padding.
/// The caller must supply the inspection result for this reader.
pub fn info_chunks<R: Read + Seek>(
    input: &mut R,
    info: &WaveInfo,
    cancel: Option<&CancelFlag>,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut at = 12;
    while at < info.end {
        check(cancel)?;
        let (tag, size, next) = chunk(input, at, info.end)?;
        if &tag == b"LIST" {
            let total = usize::try_from(next - at)
                .map_err(|_| invalid("WAVE metadata exceeds address space"))?;
            output
                .try_reserve_exact(total)
                .map_err(|_| invalid("cannot allocate WAVE metadata"))?;
            let first = output.len();
            output.resize(first + total, 0);
            input.seek(SeekFrom::Start(at))?;
            input.read_exact(&mut output[first..])?;
            if size < 4 || &output[first + 8..first + 12] != b"INFO" {
                return Err(invalid("WAVE metadata is not an INFO list"));
            }
        }
        at = next;
    }
    Ok(output)
}

/// INFO chunk storage required by an export, counted without allocating payloads.
pub fn info_chunks_bytes<R: Read + Seek>(
    input: &mut R,
    info: &WaveInfo,
    cancel: Option<&CancelFlag>,
) -> Result<usize> {
    let mut total = 0usize;
    let mut at = 12;
    while at < info.end {
        check(cancel)?;
        let (tag, size, next) = chunk(input, at, info.end)?;
        if &tag == b"LIST" {
            if size < 4 {
                return Err(invalid("short WAVE INFO list"));
            }
            let mut kind = [0; 4];
            input.read_exact(&mut kind)?;
            if &kind != b"INFO" {
                return Err(invalid("WAVE metadata is not an INFO list"));
            }
            total = total
                .checked_add(
                    usize::try_from(next - at)
                        .map_err(|_| invalid("WAVE metadata exceeds address space"))?,
                )
                .ok_or_else(|| invalid("WAVE metadata size overflow"))?;
        }
        at = next;
    }
    Ok(total)
}
