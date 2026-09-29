//! Sample-exact, streaming RIFF/WAVE PCM trimming with no decoder or foreign muxer.
use crate::{
    Result, invalid,
    media_control::{CancelFlag, ProgressEvent, ProgressHook},
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy)]
pub struct PcmTrimStats {
    /// Number of aligned I/O blocks containing retained audio (at most 64 KiB each).
    pub packets: u64,
    pub sample_frames: u64,
    pub payload_bytes: u64,
    /// Additional PCM payload clones, excluding the bounded file I/O buffer.
    pub fvid_payload_copies: u64,
}
#[derive(Debug, Clone, Copy)]
pub struct WaveInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub float: bool,
    pub sample_frames: u64,
    block: u16,
    data_bytes: u32,
    end: u64,
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
fn format(bytes: &[u8]) -> Result<(u32, u16, u16, bool, u16)> {
    if !matches!(bytes.len(), 16 | 18 | 40) {
        return Err(invalid("unsupported WAVE format extension"));
    }
    let u16at = |n| u16::from_le_bytes(bytes[n..n + 2].try_into().unwrap());
    let u32at = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().unwrap());
    let (mut tag, channels, rate, byte_rate, block, bits) =
        (u16at(0), u16at(2), u32at(4), u32at(8), u16at(12), u16at(14));
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
    Ok((rate, channels, bits, tag == 3, block))
}

/// Strict RIFF bounds; only metadata safe to retain unchanged is admitted.
/// Timed cue/loop/broadcast metadata needs its own retiming before it can be copied.
pub fn inspect<R: Read + Seek>(input: &mut R, cancel: Option<&CancelFlag>) -> Result<WaveInfo> {
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
                if &kind != b"INFO" {
                    return Err(invalid(
                        "native PCM trim requires retiming for non-INFO WAVE lists",
                    ));
                }
            }
            b"JUNK" | b"PAD " => (),
            _ => {
                return Err(invalid(&format!(
                    "native PCM trim cannot yet preserve WAVE chunk {:?}",
                    String::from_utf8_lossy(&tag)
                )));
            }
        }
        at = next;
    }
    let (rate, channels, bits, float, block) = fmt.ok_or_else(|| invalid("missing WAVE format"))?;
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
        block,
        data_bytes,
        end,
    })
}
fn boundary(us: i64, rate: u32) -> Result<u64> {
    let value = i128::from(us) * i128::from(rate);
    if us < 0 || value % 1_000_000 != 0 {
        return Err(invalid("PCM time boundary is not exactly representable"));
    }
    u64::try_from(value / 1_000_000).map_err(|_| invalid("PCM timestamp overflow"))
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Retain sample starts in the exact half-open microsecond interval. Metadata
/// safe to copy is byte-preserved, `fact` and RIFF/data lengths are rewritten.
/// Cancellation or any error leaves no destination; existing paths are never replaced.
pub fn trim_wave(
    source: &Path,
    destination: &Path,
    from: i64,
    to: i64,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<PcmTrimStats> {
    if from < 0 || to <= from {
        return Err(invalid("PCM interval requires 0 <= from < to"));
    }
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err(invalid("native WAVE trim output requires .wav"));
    }
    check(cancel)?;
    let mut input = File::open(source)?;
    let info = inspect(&mut input, cancel)?;
    let first = boundary(from, info.sample_rate)?.min(info.sample_frames);
    let last = boundary(to, info.sample_rate)?.min(info.sample_frames);
    if last <= first {
        return Err(invalid("no PCM samples in selected interval"));
    }
    let bytes = (last - first) * u64::from(info.block);
    let new_end = info.end - u64::from(info.data_bytes) - u64::from(info.data_bytes & 1)
        + bytes
        + (bytes & 1);
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (temporary, mut output) = (0..100)
        .find_map(|_| {
            let path = directory.join(format!(
                ".fvid-wave-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Some(Ok((Temporary(path), file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .ok_or_else(|| invalid("cannot reserve WAVE output"))??;
    let mut stats = PcmTrimStats {
        packets: 0,
        sample_frames: last - first,
        payload_bytes: 0,
        fvid_payload_copies: 0,
    };
    let emit = |s: &PcmTrimStats, done| {
        if let Some(h) = progress {
            h.emit(ProgressEvent {
                packets: s.packets,
                payload_bytes: s.payload_bytes,
                done,
            });
        }
    };
    emit(&stats, false);
    check(cancel)?;
    output.write_all(b"RIFF")?;
    output.write_all(&((new_end - 8) as u32).to_le_bytes())?;
    output.write_all(b"WAVE")?;
    let mut buffer = [0u8; 65536];
    let mut at = 12;
    while at < info.end {
        check(cancel)?;
        let (tag, size, next) = chunk(&mut input, at, info.end)?;
        output.write_all(&tag)?;
        if &tag == b"data" {
            output.write_all(&(bytes as u32).to_le_bytes())?;
            input.seek(SeekFrom::Start(at + 8 + first * u64::from(info.block)))?;
            let mut remaining = bytes;
            let capacity = buffer.len() / usize::from(info.block) * usize::from(info.block);
            while remaining != 0 {
                check(cancel)?;
                let n = remaining.min(capacity as u64) as usize;
                input.read_exact(&mut buffer[..n])?;
                output.write_all(&buffer[..n])?;
                remaining -= n as u64;
                stats.packets += 1;
                stats.payload_bytes += n as u64;
                emit(&stats, false);
            }
            if bytes & 1 != 0 {
                output.write_all(&[0])?;
            }
        } else if &tag == b"fact" {
            output.write_all(&size.to_le_bytes())?;
            output.write_all(&((last - first) as u32).to_le_bytes())?;
        } else {
            output.write_all(&size.to_le_bytes())?;
            let mut remaining = u64::from(size) + u64::from(size & 1);
            while remaining != 0 {
                check(cancel)?;
                let n = remaining.min(buffer.len() as u64) as usize;
                input.read_exact(&mut buffer[..n])?;
                output.write_all(&buffer[..n])?;
                remaining -= n as u64;
            }
        }
        at = next;
    }
    check(cancel)?;
    output.flush()?;
    output.sync_all()?;
    drop(output);
    check(cancel)?;
    std::fs::hard_link(&temporary.0, destination)?;
    emit(&stats, true);
    Ok(stats)
}
