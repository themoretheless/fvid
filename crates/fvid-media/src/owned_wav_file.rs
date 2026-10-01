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
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if sample_rate <= 0 || !(1..=64).contains(&channels) {
        return Err("invalid wav rate/channel count".into());
    }
    if samples.len() % channels as usize != 0 {
        return Err("incomplete WAV channel frame".into());
    }
    let data_bytes = (samples.len() as u64)
        .checked_mul(4)
        .ok_or("wav data size overflow")?;
    let riff_size = data_bytes.checked_add(36).ok_or("wav riff size overflow")?;
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
            .checked_mul(4)
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
        header[34..36].copy_from_slice(&32u16.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&(data_bytes as u32).to_le_bytes());
        file.write_all(&header).map_err(|e| e.to_string())?;
        let mut output = std::io::BufWriter::new(file);
        for sample in samples {
            output
                .write_all(&sample.to_le_bytes())
                .map_err(|e| e.to_string())?;
        }
        output.flush().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(err) = write {
        let _ = std::fs::remove_file(&temporary);
        return Err(err);
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
