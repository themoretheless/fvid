//! Native planar Y4M export. No foreign decoder, encoder or muxer is used.
use crate::{
    Result, invalid,
    playback_native::{NativeReader, RawFrame},
};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Export a constant-rate planar video. Variable timing/geometry is rejected
/// rather than silently retimed. The destination is created only on success;
/// an existing destination (including a symlink) is never replaced.
pub fn export_y4m(source: &Path, destination: &Path) -> Result<u64> {
    export_y4m_interval(source, destination, None)
}

/// Export frames with presentation starts in [from, to), decoding reference
/// pre-roll through the owned codec. The resulting Y4M starts at time zero.
pub fn export_y4m_interval(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
) -> Result<u64> {
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("export interval requires from < to"));
    }
    let mut reader = NativeReader::software(BufReader::new(File::open(source)?), usize::MAX)?;
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (temporary, file) = (0..100)
        .find_map(|_| {
            let path = directory.join(format!(
                ".fvid-y4m-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Some(Ok((Temporary(path), file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .ok_or_else(|| invalid("cannot reserve Y4M output"))??;
    let mut output = BufWriter::new(file);
    let mut previous: Option<(u128, u128, u128)> = None;
    let mut layout = None;
    let mut count = 0;
    while let Some(frame) = reader.read_frame_raw()? {
        let (start, end, scale) = reader
            .frame_interval()
            .ok_or_else(|| invalid("missing frame timing"))?;
        if let Some((from, to)) = interval {
            let stamp = product(start, 1_000_000_000)?;
            if stamp >= product(to.as_nanos(), u128::from(scale))? {
                break;
            }
            if stamp < product(from.as_nanos(), u128::from(scale))? {
                continue;
            }
        }
        let duration = end
            .checked_sub(start)
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid("invalid frame duration"))?;
        if let Some((old_end, old_duration, old_scale)) = previous {
            if product(start, old_scale)? != product(old_end, u128::from(scale))?
                || product(duration, old_scale)? != product(old_duration, u128::from(scale))?
            {
                return Err(invalid(
                    "Y4M export requires constant contiguous frame timing",
                ));
            }
        }
        previous = Some((end, duration, u128::from(scale)));
        let [width, height] = reader.dimensions();
        let rotation = reader.rotation();
        let (chroma, depth) = match &frame {
            RawFrame::Avc { picture, .. }
                if width % 2 == 0
                    && height % 2 == 0
                    && picture.cb.len()
                        == picture.coded_width.div_ceil(2) * picture.coded_height.div_ceil(2)
                    && picture.cr.len() == picture.cb.len() =>
            {
                ("420", picture.bit_depth)
            }
            RawFrame::Planar8(p) if p.chroma_width == width && p.chroma_height == height => {
                ("444", 8)
            }
            RawFrame::Planar8(p)
                if p.chroma_width == width.div_ceil(2) && p.chroma_height == height =>
            {
                ("422", 8)
            }
            RawFrame::Planar8(p)
                if p.chroma_width == width.div_ceil(2) && p.chroma_height == height.div_ceil(2) =>
            {
                ("420", 8)
            }
            _ => return Err(invalid("Y4M export requires supported planar frames")),
        };
        let full = match &frame {
            RawFrame::Avc { colour, .. } => colour.full,
            RawFrame::Planar8(p) => p.colour.full,
            _ => false,
        };
        let current = (width, height, chroma, depth, full);
        if let Some(first) = layout {
            if first != current {
                return Err(invalid(
                    "Y4M export requires fixed frame geometry and depth",
                ));
            }
        } else {
            layout = Some(current);
            let divisor = gcd(u128::from(scale), duration);
            let tag = if depth == 8 {
                chroma.to_string()
            } else {
                format!("{chroma}p{depth}")
            };
            // MP4 parsing already expresses aspect after the display rotation.
            let (an, ad) = reader.pixel_aspect();
            writeln!(
                output,
                "YUV4MPEG2 W{width} H{height} F{}:{} Ip A{an}:{ad} C{tag} XCOLORRANGE={}",
                u128::from(scale) / divisor,
                duration / divisor,
                if full { "FULL" } else { "LIMITED" }
            )?;
        }
        output.write_all(b"FRAME\n")?;
        match frame {
            RawFrame::Avc { picture, .. } if rotation != 0 => {
                let (w, h) = picture.dimensions();
                let bytes = if picture.bit_depth == 8 { 1 } else { 2 };
                let mut planar = Vec::new();
                picture.write_planar(&mut planar)?;
                let mut offset = 0;
                for (pw, ph) in [(w, h), (w / 2, h / 2), (w / 2, h / 2)] {
                    let end = offset + pw * ph * bytes;
                    output.write_all(&crate::playback_native::rotate_plane(
                        &planar[offset..end], pw, ph, rotation, bytes,
                    ))?;
                    offset = end;
                }
            }
            RawFrame::Avc { picture, .. } => picture.write_planar(&mut output)?,
            RawFrame::Planar8(p) => {
                output.write_all(&p.y)?;
                output.write_all(&p.cb)?;
                output.write_all(&p.cr)?;
            }
            _ => unreachable!(),
        }
        count += 1;
    }
    if count == 0 {
        return Err(invalid("input has no decoded video frames"));
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    // Both paths share a directory. Unlike rename, hard_link cannot clobber a
    // destination created concurrently; unsupported filesystems return an error.
    std::fs::hard_link(&temporary.0, destination)?;
    Ok(count)
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn product(a: u128, b: u128) -> Result<u128> {
    a.checked_mul(b)
        .ok_or_else(|| invalid("Y4M timestamp overflow"))
}

/// Export ADTS AAC to raw f32le or float WAV, publishing only a complete decode.
pub fn export_aac_pcm(source: &Path, destination: &Path) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_interval(source, destination, None)
}

/// Export a half-open interval from the ADTS decoded sample timeline.
pub fn export_aac_pcm_interval(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
) -> Result<crate::native_media::AudioDecodeStats> {
    use std::io::{Read, Seek, SeekFrom};
    let wav = match destination.extension().and_then(|s| s.to_str()) {
        Some("wav") => true,
        Some("f32le") => false,
        _ => return Err(invalid("native AAC PCM output requires .f32le or .wav extension")),
    };
    let limits = crate::container::adts::Limits::default();
    let mut data = Vec::new();
    File::open(source)?.take(limits.file_bytes as u64 + 1).read_to_end(&mut data)?;
    if data.len() > limits.file_bytes {
        return Err(invalid("AAC input exceeds container byte limit"));
    }
    let directory = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary, file) = (0..100).find_map(|_| {
        let path = directory.join(format!(".fvid-pcm-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => Some(Ok((Temporary(path), file))),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(e) => Some(Err(e)),
        }
    }).ok_or_else(|| invalid("cannot reserve PCM output"))??;
    let mut output = BufWriter::new(file);
    if wav { output.write_all(&[0; 80])?; }
    let stats = if data.get(4..8) == Some(b"ftyp") {
        crate::native_media::decode_mp4_aac_pcm_interval(&data, &mut output, interval)?
    } else {
        crate::native_media::decode_aac_pcm_interval(&data, &mut output, &limits, interval)?
    };
    if wav {
        let header = float_wav_header(&stats)?;
        output.seek(SeekFrom::Start(0))?;
        output.write_all(&header)?;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    std::fs::hard_link(&temporary.0, destination)?;
    Ok(stats)
}

fn float_wav_header(stats: &crate::native_media::AudioDecodeStats) -> Result<Vec<u8>> {
    let mask: u32 = match stats.channels {
        1 => 0x4, 2 => 0x3, 3 => 0x7, 4 => 0x107, 5 => 0x37, 6 => 0x3f,
        _ => return Err(invalid("unsupported WAV channel layout")),
    };
    let align = stats.channels * 4;
    let bytes = stats.sample_frames.checked_mul(u64::from(align))
        .and_then(|n| u32::try_from(n).ok()).filter(|n| *n <= u32::MAX - 72)
        .ok_or_else(|| invalid("WAV exceeds RIFF size limit"))?;
    let rate = stats.sample_rate.checked_mul(u32::from(align))
        .ok_or_else(|| invalid("WAV byte rate overflow"))?;
    let mut header = Vec::with_capacity(80);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(bytes + 72).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&40u32.to_le_bytes());
    header.extend_from_slice(&0xfffeu16.to_le_bytes());
    header.extend_from_slice(&stats.channels.to_le_bytes());
    header.extend_from_slice(&stats.sample_rate.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&align.to_le_bytes());
    header.extend_from_slice(&32u16.to_le_bytes());
    header.extend_from_slice(&22u16.to_le_bytes());
    header.extend_from_slice(&32u16.to_le_bytes());
    header.extend_from_slice(&mask.to_le_bytes());
    header.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
    header.extend_from_slice(b"fact");
    header.extend_from_slice(&4u32.to_le_bytes());
    header.extend_from_slice(&(stats.sample_frames as u32).to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&bytes.to_le_bytes());
    Ok(header)
}
