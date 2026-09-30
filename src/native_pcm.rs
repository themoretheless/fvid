//! Owned streaming RIFF/WAVE PCM inspection, sample slicing and float conversion.
mod loudness;
pub use loudness::{IntegratedLoudness, LoudnessMeter, measure_file as measure_loudness_file};
mod k_weight;
pub use k_weight::KWeighting;
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

pub use fvid_media_info::PcmTrimStats;

#[derive(Debug, Clone, Copy)]
pub struct WaveInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub float: bool,
    pub valid_bits: u16,
    pub channel_mask: u32,
    data_offset: u64,
    pub sample_frames: u64,
    block: u16,
    data_bytes: u32,
    end: u64,
}

impl WaveInfo {
    /// Stored sample representation; trimming never converts these bytes.
    pub fn codec(&self) -> String {
        if self.float {
            format!("pcm_f{}le", self.bits_per_sample)
        } else if self.bits_per_sample == 8 {
            "pcm_u8".into()
        } else {
            format!("pcm_s{}le", self.bits_per_sample)
        }
    }
    pub fn frame_bytes(&self) -> u16 {
        self.block
    }
    /// Exact half-open sample range, clipped at EOF. Shared by plans and writes.
    pub fn interval(&self, from: i64, to: i64) -> Result<std::ops::Range<u64>> {
        if from < 0 || to <= from {
            return Err(invalid("PCM interval requires 0 <= from < to"));
        }
        let first = boundary(from, self.sample_rate)?.min(self.sample_frames);
        let last = boundary(to, self.sample_rate)?.min(self.sample_frames);
        if last <= first {
            return Err(invalid("no PCM samples in selected interval"));
        }
        Ok(first..last)
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
    let range = info.interval(from, to)?;
    let (first, last) = (range.start, range.end);
    let bytes = (last - first) * u64::from(info.block);
    let new_end = info.end - u64::from(info.data_bytes) - u64::from(info.data_bytes & 1)
        + bytes
        + (bytes & 1);
    write_wave(
        input.try_clone()?,
        info,
        destination,
        new_end,
        last - first,
        &mut [(
            input,
            info.data_offset + first * u64::from(info.block),
            bytes,
        )],
        cancel,
        progress,
    )
}

fn write_wave(
    mut input: File,
    info: WaveInfo,
    destination: &Path,
    new_end: u64,
    frames: u64,
    segments: &mut [(File, u64, u64)],
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<PcmTrimStats> {
    let bytes = frames * u64::from(info.block);
    if bytes > u64::from(u32::MAX)
        || new_end - 8 > u64::from(u32::MAX)
        || frames > u64::from(u32::MAX)
    {
        return Err(invalid(
            "concatenated WAVE exceeds RIFF size or sample count range",
        ));
    }
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
        sample_frames: frames,
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
            let capacity = buffer.len() / usize::from(info.block) * usize::from(info.block);
            for (source, offset, length) in segments.iter_mut() {
                source.seek(SeekFrom::Start(*offset))?;
                let mut remaining = *length;
                while remaining != 0 {
                    check(cancel)?;
                    let n = remaining.min(capacity as u64) as usize;
                    source.read_exact(&mut buffer[..n])?;
                    output.write_all(&buffer[..n])?;
                    remaining -= n as u64;
                    stats.packets += 1;
                    stats.payload_bytes += n as u64;
                    emit(&stats, false);
                }
            }
            if bytes & 1 != 0 {
                output.write_all(&[0])?;
            }
        } else if &tag == b"fact" {
            output.write_all(&size.to_le_bytes())?;
            output.write_all(&(frames as u32).to_le_bytes())?;
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

impl WaveInfo {
    pub(crate) fn decode_interval(
        &self,
        interval: Option<(std::time::Duration, std::time::Duration)>,
    ) -> Result<std::ops::Range<u64>> {
        if interval.is_some_and(|(a, b)| a >= b) {
            return Err(invalid("audio interval requires from < to"));
        }
        let boundary = |t: std::time::Duration| -> Result<u64> {
            let n = t
                .as_nanos()
                .checked_mul(u128::from(self.sample_rate))
                .ok_or_else(|| invalid("PCM interval overflow"))?
                .div_ceil(1_000_000_000);
            Ok(n.min(u128::from(self.sample_frames)) as u64)
        };
        let (first, last) = match interval {
            Some((a, b)) => (boundary(a)?, boundary(b)?),
            None => (0, self.sample_frames),
        };
        if last <= first {
            return Err(invalid("no PCM samples in selected interval"));
        }
        Ok(first..last)
    }
    /// Converting samples independently preserves interleaving without knowing
    /// speaker positions. Rematrixing has a separate layout requirement.
    pub(crate) fn validate_decode(&self) -> Result<()> {
        if !(1..=64).contains(&self.channels) {
            return Err(invalid("native PCM decoding supports 1..=64 channels"));
        }
        if self.sample_rate > i32::MAX as u32 {
            return Err(invalid("PCM sample rate exceeds media schema range"));
        }
        Ok(())
    }
    pub(crate) fn validate_rematrix(&self) -> Result<()> {
        self.validate_decode()?;
        let expected = match self.channels {
            1 => 4,
            2 => 3,
            3 => 7,
            4 => 0x107,
            5 => 0x37,
            6 => 0x3f,
            _ => {
                return Err(invalid(
                    "native PCM rematrixing supports 1..=6 input channels",
                ));
            }
        };
        if !(self.channel_mask == expected || (self.channels <= 2 && self.channel_mask == 0)) {
            return Err(invalid(
                "native PCM rematrixing requires a supported explicit channel layout",
            ));
        }
        Ok(())
    }
}

/// Read selected PCM sample starts in [from,to), rounding up to the sample grid.
/// PCM needs no decoder pre-roll; retained interleaved frames are converted in
/// bounded blocks. Floating NaN/Inf and nonzero integer padding bits are errors.
pub(crate) fn decode_reader<R: Read + Seek, W: Write>(
    mut input: R,
    info: WaveInfo,
    output: &mut W,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    control: &mut crate::native_media::DecodeProgress<'_>,
) -> Result<crate::native_media::AudioDecodeStats> {
    info.validate_decode()?;
    let range = info.decode_interval(interval)?;
    let (first, last) = (range.start, range.end);
    input.seek(SeekFrom::Start(
        info.data_offset + first * u64::from(info.block),
    ))?;
    let mut buffer = [0u8; 65536];
    let capacity = buffer.len() / usize::from(info.block) * usize::from(info.block);
    let mut remaining = (last - first) * u64::from(info.block);
    let sample_bytes = usize::from(info.bits_per_sample / 8);
    let mut blocks = 0;
    while remaining != 0 {
        control.check()?;
        let n = remaining.min(capacity as u64) as usize;
        input.read_exact(&mut buffer[..n])?;
        for bytes in buffer[..n].chunks_exact(sample_bytes) {
            let value = if info.float {
                if sample_bytes == 4 {
                    f32::from_le_bytes(bytes.try_into().unwrap())
                } else {
                    f64::from_le_bytes(bytes.try_into().unwrap()) as f32
                }
            } else {
                let mut packed = [0u8; 4];
                packed[..sample_bytes].copy_from_slice(bytes);
                let raw = u32::from_le_bytes(packed);
                let padding = info.bits_per_sample - info.valid_bits;
                if padding != 0 && raw & ((1u32 << padding) - 1) != 0 {
                    return Err(invalid("nonzero PCM padding bits"));
                }
                if sample_bytes == 1 {
                    (f32::from(bytes[0]) - 128.0) / 128.0
                } else {
                    let shift = 32 - u32::from(info.bits_per_sample);
                    let signed = ((raw << shift) as i32) >> shift;
                    (f64::from(signed) / (1u64 << (info.bits_per_sample - 1)) as f64) as f32
                }
            };
            if !value.is_finite() {
                return Err(invalid("non-finite PCM sample"));
            }
            output.write_all(&value.to_le_bytes())?;
        }
        remaining -= n as u64;
        blocks += 1;
        control.packet(n)?;
    }
    Ok(crate::native_media::AudioDecodeStats {
        sample_frames: last - first,
        decoded_frames: blocks,
        sample_rate: info.sample_rate,
        channels: info.channels,
    })
}

fn concat_inputs(
    sources: &[PathBuf],
    cancel: Option<&CancelFlag>,
) -> Result<(WaveInfo, u64, Vec<(File, u64, u64)>)> {
    if !(2..=256).contains(&sources.len()) {
        return Err(invalid("concat requires 2..=256 inputs"));
    }
    let mut template: Option<WaveInfo> = None;
    let mut segments = Vec::with_capacity(sources.len());
    let mut frames = 0u64;
    for source in sources {
        check(cancel)?;
        let mut input = File::open(source)?;
        let info = inspect(&mut input, cancel)?;
        if let Some(first) = template {
            if (
                first.sample_rate,
                first.channels,
                first.bits_per_sample,
                first.float,
                first.valid_bits,
                first.channel_mask,
            ) != (
                info.sample_rate,
                info.channels,
                info.bits_per_sample,
                info.float,
                info.valid_bits,
                info.channel_mask,
            ) {
                return Err(invalid(
                    "WAVE concat requires identical PCM formats and channel masks",
                ));
            }
        } else {
            template = Some(info);
        }
        frames = frames
            .checked_add(info.sample_frames)
            .ok_or_else(|| invalid("WAVE sample count overflow"))?;
        segments.push((input, info.data_offset, u64::from(info.data_bytes)));
    }
    let info = template.ok_or_else(|| invalid("missing WAVE input"))?;
    let bytes = frames
        .checked_mul(u64::from(info.block))
        .ok_or_else(|| invalid("WAVE payload size overflow"))?;
    let end = info.end - u64::from(info.data_bytes) - u64::from(info.data_bytes & 1)
        + bytes
        + (bytes & 1);
    if bytes > u64::from(u32::MAX) || frames > u64::from(u32::MAX) || end - 8 > u64::from(u32::MAX)
    {
        return Err(invalid(
            "concatenated WAVE exceeds RIFF size or sample count range",
        ));
    }
    Ok((info, frames, segments))
}

/// Validate all input headers and total RIFF size, without decoding samples.
pub fn concat_info(sources: &[PathBuf]) -> Result<(WaveInfo, u64)> {
    let (info, frames, _) = concat_inputs(sources, None)?;
    Ok((info, frames))
}

/// Append raw PCM frames with one aligned 64 KiB buffer. Retain the first input's
/// format and safe metadata; recompute RIFF/data/fact lengths. Never overwrite.
pub fn concat_wave(
    sources: &[PathBuf],
    destination: &Path,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<PcmTrimStats> {
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err(invalid("native WAVE concat output requires .wav"));
    }
    let (info, frames, mut segments) = concat_inputs(sources, cancel)?;
    let bytes = frames * u64::from(info.block);
    let end = info.end - u64::from(info.data_bytes) - u64::from(info.data_bytes & 1)
        + bytes
        + (bytes & 1);
    write_wave(
        segments[0].0.try_clone()?,
        info,
        destination,
        end,
        frames,
        &mut segments,
        cancel,
        progress,
    )
}
