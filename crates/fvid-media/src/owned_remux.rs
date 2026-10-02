//! Owned container remux dispatch.
use std::path::Path;
pub fn remux(
    source: &Path,
    destination: &Path,
    options: &fvid_control::CopyOptions,
) -> Result<fvid_media_info::CopyStats, String> {
    if crate::owned_matroska_remux::supports(source, destination, options) {
        crate::owned_matroska_remux::remux(source, destination, options)
    } else {
        crate::owned_wave_remux::remux(source, destination, options)
    }
}
