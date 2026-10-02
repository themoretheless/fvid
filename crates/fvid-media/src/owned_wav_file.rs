//! Direct IEEE float WAV writer (no lavf muxer).
type Result<T> = std::result::Result<T, String>;
use std::io::Write;
use std::path::Path;

/// Write IEEE float little-endian WAV without lavf mux overhead.
pub fn write_wav_f32le(
    destination: &Path,
    sample_rate: i32,
    channels: i32,
    samples: &[f32],
) -> Result<()> {
    write_wav_f32le_checked(destination, sample_rate, channels, samples, || Ok(()))
}

pub(crate) fn write_wav_f32le_checked<F: FnMut() -> Result<()>>(
    destination: &Path,
    sample_rate: i32,
    channels: i32,
    samples: &[f32],
    check: F,
) -> Result<()> {
    write_float_wave::<4, _>(
        destination,
        sample_rate,
        channels,
        samples.len(),
        samples.iter().map(|sample| sample.to_le_bytes()),
        0,
        &[],
        check,
    )
}
/// Write IEEE float64 WAV, retaining all sample bits.
pub fn write_wav_f64le(
    destination: &Path,
    sample_rate: i32,
    channels: i32,
    samples: &[f64],
) -> Result<()> {
    write_wav_f64le_checked(destination, sample_rate, channels, samples, || Ok(()))
}
pub(crate) fn write_wav_f64le_checked<F: FnMut() -> Result<()>>(
    destination: &Path,
    sample_rate: i32,
    channels: i32,
    samples: &[f64],
    check: F,
) -> Result<()> {
    write_float_wave::<8, _>(
        destination,
        sample_rate,
        channels,
        samples.len(),
        samples.iter().map(|sample| sample.to_le_bytes()),
        0,
        &[],
        check,
    )
}
pub(crate) fn write_wav_f32le_with_side_data_checked<F: FnMut() -> Result<()>>(
    destination: &Path,
    sample_rate: i32,
    channels: i32,
    samples: &[f32],
    mask: u32,
    info_chunks: &[u8],
    check: F,
) -> Result<()> {
    write_float_wave::<4, _>(
        destination,
        sample_rate,
        channels,
        samples.len(),
        samples.iter().map(|s| s.to_le_bytes()),
        mask,
        info_chunks,
        check,
    )
}
pub(crate) fn write_wav_f64le_with_side_data_checked<F: FnMut() -> Result<()>>(
    destination: &Path,
    sample_rate: i32,
    channels: i32,
    samples: &[f64],
    mask: u32,
    info_chunks: &[u8],
    check: F,
) -> Result<()> {
    write_float_wave::<8, _>(
        destination,
        sample_rate,
        channels,
        samples.len(),
        samples.iter().map(|s| s.to_le_bytes()),
        mask,
        info_chunks,
        check,
    )
}
fn write_float_wave<const WIDTH: usize, F: FnMut() -> Result<()>>(
    destination: &Path,
    sample_rate: i32,
    channels: i32,
    count: usize,
    samples: impl Iterator<Item = [u8; WIDTH]>,
    mask: u32,
    info_chunks: &[u8],
    mut check: F,
) -> Result<()> {
    check()?;
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if sample_rate <= 0 || !(1..=64).contains(&channels) {
        return Err("invalid wav rate/channel count".into());
    }
    if count % channels as usize != 0 {
        return Err("incomplete WAV channel frame".into());
    }
    let data_bytes = (count as u64)
        .checked_mul(WIDTH as u64)
        .ok_or("wav data size overflow")?;
    if mask != 0 && mask.count_ones() != channels as u32 {
        return Err("invalid WAV channel mask".into());
    }
    let extensible = mask != 0 || (WIDTH == 8 && channels > 2);
    let header_bytes = if extensible { 80u64 } else { 44 };
    let riff_size = data_bytes
        .checked_add(header_bytes - 8)
        .and_then(|n| n.checked_add(info_chunks.len() as u64))
        .ok_or("wav riff size overflow")?;
    if riff_size > u32::MAX as u64 || data_bytes > u32::MAX as u64 {
        return Err("wav payload exceeds 4 GiB".into());
    }
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = None;
    for attempt in 0..100 {
        let path = directory.join(format!(".fvid-wav-{}-{attempt}.tmp", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => {
                temporary = Some(path);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    let temporary = temporary.ok_or("cannot reserve temporary output")?;
    let write = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        let block_align = (channels as u16)
            .checked_mul(WIDTH as u16)
            .ok_or("wav block align overflow")?;
        let byte_rate = (sample_rate as u32)
            .checked_mul(u32::from(block_align))
            .ok_or("wav byte rate overflow")?;
        let mut header = [0u8; 44];
        header[0..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&(riff_size as u32).to_le_bytes());
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[16..20].copy_from_slice(&16u32.to_le_bytes());
        header[20..22].copy_from_slice(&3u16.to_le_bytes()); // IEEE float
        header[22..24].copy_from_slice(&(channels as u16).to_le_bytes());
        header[24..28].copy_from_slice(&(sample_rate as u32).to_le_bytes());
        header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
        header[32..34].copy_from_slice(&block_align.to_le_bytes());
        header[34..36].copy_from_slice(&((WIDTH * 8) as u16).to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&(data_bytes as u32).to_le_bytes());
        let mut header = if extensible {
            crate::owned_wav::float_wav_header_with_precision(
                sample_rate as u32,
                channels as u16,
                count as u64 / channels as u64,
                mask,
                (WIDTH * 8) as u16,
            )?
        } else {
            header.to_vec()
        };
        header[4..8].copy_from_slice(&(riff_size as u32).to_le_bytes());
        file.write_all(&header).map_err(|e| e.to_string())?;
        let mut output = std::io::BufWriter::new(file);
        for (index, sample) in samples.enumerate() {
            if index % 16384 == 0 {
                check()?;
            }
            output.write_all(&sample).map_err(|e| e.to_string())?;
        }
        check()?;
        output.write_all(info_chunks).map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(err) = write {
        let _ = std::fs::remove_file(&temporary);
        return Err(err);
    }
    if let Err(error) = check() {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    // A hard link atomically refuses a concurrently created destination.
    // Never fall back to rename, which can overwrite it on Unix.
    let result =
        std::fs::hard_link(&temporary, destination).map_err(|e| format!("publish WAV: {e}"));
    let _ = std::fs::remove_file(&temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_float_wav_preserves_bits_and_refuses_overwrite() {
        let directory = std::env::temp_dir().join(format!("fvid-owned-wav-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("stereo.wav");
        let _ = std::fs::remove_file(&path);
        let samples = [0.25, -0.5, f32::from_bits(0x7fc01234), -0.0];
        write_wav_f32le(&path, 48000, 2, &samples).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8,
            bytes.len()
        );
        assert_eq!(
            &bytes[44..],
            samples
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect::<Vec<_>>()
        );
        assert!(write_wav_f32le(&path, 48000, 2, &[1., 1.]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let invalid = directory.join("invalid.wav");
        assert!(write_wav_f32le(&invalid, 48000, 2, &[1.]).is_err());
        assert!(!invalid.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod double_tests {
    use super::*;
    #[test]
    fn writer_retains_all_ieee_double_bits_and_sizes() {
        let dir =
            std::env::temp_dir().join(format!("fvid-double-wav-writer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("samples.wav");
        let samples = [
            0.12345678901234567,
            -0.0,
            f64::from_bits(1),
            f64::from_bits(0x7ff8000012345678),
        ];
        write_wav_f64le(&path, 48000, 2, &samples).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + samples.len() * 8);
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 64);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 32);
        for (sample, actual) in samples.iter().zip(bytes[44..].chunks_exact(8)) {
            assert_eq!(actual, sample.to_le_bytes());
        }
        assert!(write_wav_f64le(&path, 48000, 2, &samples).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
