//! Atomic file publication for byte-preserving Matroska identity remux.
use fvid_control::CopyOptions;
use fvid_media_info::CopyStats;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub(crate) fn policies(o: &CopyOptions) -> bool {
    o.streams.is_empty() && o.max_packets.is_none() && unedited_policies(o)
}
pub(crate) fn unedited_policies(o: &CopyOptions) -> bool {
    o.metadata_set.is_empty()
        && o.metadata_delete.is_empty()
        && o.stream_metadata_set.is_empty()
        && o.stream_metadata_delete.is_empty()
        && o.max_controlled_bytes.is_none()
        && o.max_rss_bytes.is_none()
        && o.max_packet_bytes > 0
}
pub(crate) fn supports(source: &Path, destination: &Path, options: &CopyOptions) -> bool {
    if !policies(options)
        || !matches!(
            destination.extension().and_then(|s| s.to_str()),
            Some("mkv" | "mka")
        )
    {
        return false;
    }
    let mut bytes = [0; 4];
    File::open(source)
        .is_ok_and(|mut f| f.read_exact(&mut bytes).is_ok() && bytes == [0x1a, 0x45, 0xdf, 0xa3])
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
/// Preserve every source byte. Requests requiring track selection or edits are refused.
pub fn remux(
    source: &Path,
    destination: &Path,
    options: &CopyOptions,
) -> Result<CopyStats, String> {
    if !supports(source, destination, options) {
        return Err("request is not an owned Matroska identity remux".into());
    }
    if std::fs::symlink_metadata(destination).is_ok() {
        return Err("output already exists".into());
    }
    let check = || {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            Err("media operation cancelled".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let mut input = File::open(source).map_err(|e| e.to_string())?;
    if !input.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Matroska remux requires a regular file".into());
    }
    let audio_only = destination.extension().and_then(|s| s.to_str()) == Some("mka");
    crate::owned_matroska_copy::inspect_with_packet_limit(
        &mut input,
        audio_only,
        options.cancel.as_ref(),
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    publish(destination, options, |output| {
        crate::owned_matroska_copy::copy_with_packet_limit(
            &mut input,
            output,
            audio_only,
            options.cancel.as_ref(),
            options.progress.as_ref(),
            options.max_packet_bytes,
        )
        .map_err(|e| e.to_string())
    })
}
pub(crate) fn publish(
    destination: &Path,
    options: &CopyOptions,
    write: impl FnOnce(&mut File) -> Result<fvid_control::ProgressEvent, String>,
) -> Result<CopyStats, String> {
    if std::fs::symlink_metadata(destination).is_ok() {
        return Err("output already exists".into());
    }
    let check = || {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            Err("media operation cancelled".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (temporary, mut output) = loop {
        let path = parent.join(format!(
            ".fvid-matroska-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut open = OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(0o600);
        }
        match open.open(&path) {
            Ok(file) => break (Temporary(path), file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    };
    let mut event = write(&mut output)?;
    output.flush().map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    check()?;
    std::fs::hard_link(&temporary.0, destination).map_err(|e| e.to_string())?;
    drop(output);
    drop(temporary);
    event.done = true;
    if let Some(progress) = &options.progress {
        progress.emit(event);
    }
    Ok(CopyStats {
        packets: event.packets,
        payload_bytes: event.payload_bytes,
        segments: 1,
        backend: "owned Matroska",
        fvid_payload_copies: event.packets,
    })
}
