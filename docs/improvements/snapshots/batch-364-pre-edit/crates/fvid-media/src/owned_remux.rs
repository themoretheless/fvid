//! Owned container remux dispatch.
use std::path::Path;
pub fn remux(
    source: &Path,
    destination: &Path,
    options: &fvid_control::CopyOptions,
) -> Result<fvid_media_info::CopyStats, String> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if crate::owned_mp4_remux::supports(source, destination, options) {
        return crate::owned_mp4_remux::remux(source, destination, options);
    }
    if crate::owned_adts_remux::supports(source, destination, options) {
        return crate::owned_adts_remux::remux(source, destination, options);
    }
    if crate::owned_matroska_remux::supports(source, destination, options) {
        return crate::owned_matroska_remux::remux(source, destination, options);
    }
    if !crate::owned_wave_inspect::is_wave(source).unwrap_or(false) {
        if options.max_packet_bytes == 0 {
            return Err("packet byte limit must be positive".into());
        }
        if options.max_packets == Some(0) {
            return Err("packet work limit requires at least one packet".into());
        }
        if options.max_controlled_bytes == Some(0) {
            return Err("controlled memory limit must be positive".into());
        }
        crate::owned_budget::check_rss_budget(options)?;
        if std::fs::File::open(source).is_ok_and(|mut f| {
            use std::io::Read;
            let mut prefix = [0u8; 8];
            f.read_exact(&mut prefix).is_ok() && crate::owned_mp4::recognizes_prefix(&prefix)
        }) {
            return crate::owned_mp4_remux::remux(source, destination, options);
        }
        if std::fs::File::open(source).is_ok_and(|mut f| {
            use std::io::Read;
            let mut prefix = [0u8; 4];
            f.read_exact(&mut prefix).is_ok() && prefix == [0x1a, 0x45, 0xdf, 0xa3]
        }) {
            return crate::owned_matroska_remux::remux(source, destination, options);
        }
        if crate::owned_adts_export::recognizes(source).unwrap_or(false) {
            return crate::owned_adts_remux::remux(source, destination, options);
        }
    }
    crate::owned_wave_remux::remux(source, destination, options)
}
