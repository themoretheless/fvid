//! Streaming PCM WAVE remux, preserving encoded sample bits and fmt parameters.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::CopyStats;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
fn policies(options: &CopyOptions) -> bool {
    (options.streams.is_empty() || options.streams == [0])
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
        && options
            .metadata_set
            .iter()
            .all(|(key, _)| crate::owned_wave_metadata::tag_for_key(key).is_ok())
        && options
            .metadata_delete
            .iter()
            .all(|key| crate::owned_wave_metadata::tag_for_key(key).is_ok())
}
pub(crate) fn supports(source: &Path, destination: &Path, options: &CopyOptions) -> bool {
    destination.extension().and_then(|s| s.to_str()) == Some("wav")
        && policies(options)
        && std::fs::File::open(source)
            .is_ok_and(|mut file| crate::owned_wave_inspect::inspect(&mut file, None).is_ok())
}
/// Packet limits count frame-aligned I/O blocks, at most 4096 sample frames.
/// No samples are decoded, resampled or requantized, including non-finite floats.
pub fn remux(source: &Path, destination: &Path, options: &CopyOptions) -> Result<CopyStats> {
    if !policies(options) {
        return Err(
            "owned WAVE remux does not implement requested stream/metadata policies".into(),
        );
    }
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err("owned WAVE remux requires .wav output".into());
    }
    if options.metadata_set.len() + options.metadata_delete.len() > 64
        || options.metadata_set.iter().any(|(_, v)| v.contains('\0'))
    {
        return Err("invalid WAVE metadata mutations".into());
    }
    let check = || -> Result<()> {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        crate::owned_budget::check_rss_budget(options)
    };
    let emit = |event| {
        if let Some(hook) = &options.progress {
            hook.emit(event);
        }
    };
    check()?;
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    emit(event);
    check()?;
    let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let info = crate::owned_wave_inspect::inspect(&mut input, options.cancel.as_ref())
        .map_err(|e| e.to_string())?;
    let frame = usize::from(info.block);
    let capacity = (4096 * frame).min(options.max_packet_bytes / frame * frame);
    if capacity == 0 {
        return Err("PCM packet limit cannot hold one sample frame".into());
    }
    let size = u128::from(info.data_bytes).min(
        options
            .max_packets
            .map(|n| u128::from(n) * capacity as u128)
            .unwrap_or(u128::from(info.data_bytes)),
    ) as u32;
    if size == 0 && info.data_bytes != 0 {
        return Err("no WAVE samples selected".into());
    }
    let metadata_bytes =
        crate::owned_wave_inspect::info_chunks_bytes(&mut input, &info, options.cancel.as_ref())
            .map_err(|e| e.to_string())?;
    let assignments = options.metadata_set.iter().try_fold(0usize, |n, (_, v)| {
        n.checked_add(v.len())
            .and_then(|n| n.checked_add(10))
            .ok_or("metadata size overflow")
    })?;
    let estimate = metadata_bytes
        .checked_add(assignments)
        .and_then(|n| n.checked_add(12))
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_add(capacity))
        .and_then(|n| n.checked_add(16384))
        .ok_or("WAVE memory estimate overflow")?;
    if options
        .max_controlled_bytes
        .is_some_and(|max| estimate > max)
    {
        return Err(format!(
            "controlled memory budget exceeded: need {estimate} bytes"
        ));
    }
    check()?;
    let metadata =
        crate::owned_wave_inspect::info_chunks(&mut input, &info, options.cancel.as_ref())
            .map_err(|e| e.to_string())?;
    let metadata = crate::owned_wave_metadata::edit_info_chunks(
        &metadata,
        &options.metadata_delete,
        &options.metadata_set,
    )?;
    // Preserve fmt bytes verbatim, including extensible valid bits, speaker mask
    // and subformat GUID. Inspection has already qualified every chunk.
    input.seek(SeekFrom::Start(12)).map_err(|e| e.to_string())?;
    let mut fmt = Vec::new();
    let mut fact = false;
    while input.stream_position().map_err(|e| e.to_string())? < info.end {
        check()?;
        let mut header = [0; 8];
        input.read_exact(&mut header).map_err(|e| e.to_string())?;
        let bytes = u32::from_le_bytes(header[4..].try_into().unwrap());
        if &header[..4] == b"fmt " {
            if !matches!(bytes, 16 | 18 | 40) {
                return Err("WAVE fmt changed during remux".into());
            }
            fmt.extend_from_slice(&header);
            let start = fmt.len();
            fmt.resize(start + bytes as usize, 0);
            input
                .read_exact(&mut fmt[start..])
                .map_err(|e| e.to_string())?;
            input
                .seek(SeekFrom::Current(i64::from(bytes % 2)))
                .map_err(|e| e.to_string())?;
        } else {
            fact |= &header[..4] == b"fact";
            input
                .seek(SeekFrom::Current(i64::from(bytes) + i64::from(bytes % 2)))
                .map_err(|e| e.to_string())?;
        }
    }
    let fact_bytes = if fact { 12 } else { 0 };
    let riff = 4u64
        + fmt.len() as u64
        + fact_bytes
        + 8
        + u64::from(size)
        + u64::from(size % 2)
        + metadata.len() as u64;
    let riff = u32::try_from(riff).map_err(|_| "WAVE exceeds RIFF size limit")?;
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = None;
    for attempt in 0..100 {
        let path = directory.join(format!(
            ".fvid-wave-remux-{}-{attempt}.tmp",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                temporary = Some((path, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    let (temporary, mut output) = temporary.ok_or("cannot reserve WAVE output")?;
    let write = (|| -> Result<()> {
        check()?;
        output
            .write_all(b"RIFF")
            .and_then(|_| output.write_all(&riff.to_le_bytes()))
            .and_then(|_| output.write_all(b"WAVE"))
            .and_then(|_| output.write_all(&fmt))
            .map_err(|e| e.to_string())?;
        if fact {
            output
                .write_all(b"fact\x04\x00\x00\x00")
                .and_then(|_| output.write_all(&(size / u32::from(info.block)).to_le_bytes()))
                .map_err(|e| e.to_string())?;
        }
        output
            .write_all(b"data")
            .and_then(|_| output.write_all(&size.to_le_bytes()))
            .map_err(|e| e.to_string())?;
        input
            .seek(SeekFrom::Start(info.data_offset))
            .map_err(|e| e.to_string())?;
        let mut buffer = vec![0; capacity];
        let mut remaining = size as usize;
        while remaining > 0 {
            check()?;
            let count = remaining.min(capacity);
            input
                .read_exact(&mut buffer[..count])
                .and_then(|_| output.write_all(&buffer[..count]))
                .map_err(|e| e.to_string())?;
            remaining -= count;
            event.packets += 1;
            event.payload_bytes += count as u64;
            emit(event);
            check()?;
        }
        if size % 2 != 0 {
            output.write_all(&[0]).map_err(|e| e.to_string())?;
        }
        output
            .write_all(&metadata)
            .and_then(|_| output.flush())
            .map_err(|e| e.to_string())?;
        check()?;
        std::fs::hard_link(&temporary, destination).map_err(|e| format!("publish WAVE: {e}"))?;
        Ok(())
    })();
    drop(output);
    let _ = std::fs::remove_file(&temporary);
    write?;
    emit(ProgressEvent {
        done: true,
        ..event
    });
    Ok(CopyStats {
        packets: event.packets,
        payload_bytes: event.payload_bytes,
        segments: 1,
        backend: "owned streaming WAVE remux",
        fvid_payload_copies: event.packets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn payload(path: &Path) -> (crate::owned_wave_inspect::WaveInfo, Vec<u8>) {
        let raw = std::fs::read(path).unwrap();
        let mut file = std::io::Cursor::new(&raw);
        let info = crate::owned_wave_inspect::inspect(&mut file, None).unwrap();
        (
            info,
            raw[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize]
                .to_vec(),
        )
    }
    #[test]
    fn public_remux_preserves_integer_storage_valid_bits_and_float_nan_payloads() {
        let dir = std::env::temp_dir().join(format!("fvid-wave-remux-bits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for bits in [8u16, 16, 24, 32] {
            let source = dir.join(format!("source-{bits}.wav"));
            let width = usize::from(bits / 8);
            let raw: Vec<u8> = (0..15 * width).map(|i| (i * 73 + 17) as u8).collect();
            crate::owned_wav_file::write_wav_integer_le(&source, 48000, 1, bits, &raw, 4).unwrap();
            if bits == 24 {
                let mut bytes = std::fs::read(&source).unwrap();
                bytes[38..40].copy_from_slice(&20u16.to_le_bytes());
                std::fs::write(&source, bytes).unwrap();
            }
            let output = dir.join(format!("copy-{bits}.wav"));
            assert!(supports(&source, &output, &CopyOptions::default()));
            let stats = crate::remux(&source, &output, &CopyOptions::default()).unwrap();
            let (before, _) = payload(&source);
            let (after, actual) = payload(&output);
            assert_eq!(actual, raw);
            assert_eq!(after.bits_per_sample, before.bits_per_sample);
            assert_eq!(after.valid_bits, before.valid_bits);
            assert_eq!(after.channel_mask, 4);
            assert_eq!(stats.payload_bytes, raw.len() as u64);
            assert_eq!(stats.backend, "owned streaming WAVE remux");
            assert!(crate::remux(&source, &output, &CopyOptions::default()).is_err());
            let limited = dir.join(format!("limited-{bits}.wav"));
            let stats = crate::remux(
                &source,
                &limited,
                &CopyOptions {
                    max_packet_bytes: 3 * width,
                    max_packets: Some(2),
                    metadata_set: vec![("title".into(), "own remux".into())],
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(stats.packets, 2);
            let (info, actual) = payload(&limited);
            assert_eq!(info.sample_frames, 6);
            assert_eq!(actual, raw[..6 * width]);
            assert_eq!(
                crate::owned_probe::probe_wave(&limited)
                    .unwrap()
                    .metadata
                    .get("title")
                    .map(String::as_str),
                Some("own remux")
            );
        }
        for double in [false, true] {
            let source = dir.join(format!("nan-{double}.wav"));
            let output = dir.join(format!("nan-copy-{double}.wav"));
            if double {
                crate::owned_wav_file::write_wav_f64le(
                    &source,
                    48000,
                    1,
                    &[f64::from_bits(0x7ff8000012345678), -0.0, f64::INFINITY],
                )
                .unwrap();
            } else {
                crate::owned_wav_file::write_wav_f32le(
                    &source,
                    48000,
                    1,
                    &[f32::from_bits(0x7fc01234), -0.0, f32::INFINITY],
                )
                .unwrap();
            }
            crate::remux(&source, &output, &CopyOptions::default()).unwrap();
            assert_eq!(payload(&source).1, payload(&output).1);
        }
        let empty = dir.join("empty.wav");
        let empty_copy = dir.join("empty-copy.wav");
        crate::owned_wav_file::write_wav_integer_le(&empty, 48000, 1, 16, &[], 0).unwrap();
        let stats = crate::remux(&empty, &empty_copy, &CopyOptions::default()).unwrap();
        assert_eq!(stats.packets, 0);
        assert_eq!(payload(&empty_copy).0.sample_frames, 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn budgets_and_cancellation_refuse_publication_and_done_is_after_publish() {
        let dir =
            std::env::temp_dir().join(format!("fvid-wave-remux-control-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.wav");
        crate::owned_wav_file::write_wav_integer_le(&source, 48000, 1, 8, &[1, 2, 3, 4, 5], 0)
            .unwrap();
        let rejected = dir.join("budget.wav");
        assert!(crate::remux(
            &source,
            &rejected,
            &CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("controlled memory"));
        assert!(!rejected.exists());
        let cancel = fvid_control::CancelFlag::default();
        let flag = cancel.clone();
        let cancelled = dir.join("cancelled.wav");
        let options = CopyOptions {
            cancel: Some(cancel),
            max_packet_bytes: 1,
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                assert!(!event.done);
                if event.packets == 2 {
                    flag.cancel();
                }
            })),
            ..Default::default()
        };
        assert!(crate::remux(&source, &cancelled, &options)
            .unwrap_err()
            .contains("cancelled"));
        assert!(!cancelled.exists());
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-wave-remux")));
        let output = dir.join("success.wav");
        let published = output.clone();
        let options = CopyOptions {
            max_controlled_bytes: Some(64 * 1024),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                if event.done {
                    assert!(published.exists());
                    assert_eq!(payload(&published).1, [1, 2, 3, 4, 5]);
                }
            })),
            ..Default::default()
        };
        crate::remux(&source, &output, &options).unwrap();
        let large = dir.join("large.wav");
        let large_copy = dir.join("large-copy.wav");
        crate::owned_wav_file::write_wav_integer_le(&large, 48000, 1, 8, &vec![128; 131072], 0)
            .unwrap();
        let stats = crate::remux(
            &large,
            &large_copy,
            &CopyOptions {
                max_packet_bytes: 1024,
                max_controlled_bytes: Some(20 * 1024),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.payload_bytes, 131072);
        assert_eq!(stats.packets, 128);
        assert_eq!(payload(&large_copy).1, vec![128; 131072]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
