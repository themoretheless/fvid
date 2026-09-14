//! Atomic-ish output publication: prefer hard link, fall back to rename.
use std::fs;
use std::io;
use std::path::Path;

/// Publish `temporary` as `destination` without clobbering an existing file.
///
/// Prefers `hard_link` then unlinks the temporary name. When hard links are
/// unavailable (cross-volume paths, FAT, some network filesystems), falls back
/// to `rename`. Rename still respects a prior no-clobber check but is not the
/// same atomic visibility guarantee as a same-volume hard-link publish.
///
/// On success the temporary path no longer names the payload (removed or renamed).
pub fn publish_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    match fs::hard_link(temporary, destination) {
        Ok(()) => {
            let _ = fs::remove_file(temporary);
            Ok(())
        }
        Err(hard_link_err) => fs::rename(temporary, destination).map_err(|rename_err| {
            io::Error::new(
                rename_err.kind(),
                format!(
                    "publish failed (hard_link: {hard_link_err}; rename: {rename_err})"
                ),
            )
        }),
    }
}
