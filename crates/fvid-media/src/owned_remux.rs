//! Owned container remux dispatch.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
pub fn remux(
    source: &Path,
    destination: &Path,
    options: &fvid_control::CopyOptions,
) -> Result<fvid_media_info::CopyStats, String> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if let Ok(mut file) = File::open(source) {
        let mut sig = [0u8; 12];
        let mut read = 0usize;
        while read < sig.len() {
            match file.read(&mut sig[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let is_wave = read == 12
            && matches!(&sig[..4], b"RIFF" | b"RIFX" | b"RF64")
            && &sig[8..12] == b"WAVE";
        if !is_wave {
            if read >= 8 && crate::owned_mp4::recognizes_prefix(&sig[..8]) {
                let _ = file.seek(SeekFrom::Start(0));
                return crate::owned_mp4_remux::remux_from_file(file, destination, options);
            }
            if options.max_packets == Some(0) {
                return Err("packet work limit requires at least one packet".into());
            }
            if read >= 4 && sig[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
                let _ = file.seek(SeekFrom::Start(0));
                return crate::owned_matroska_remux::remux_from_file(file, destination, options);
            }
            if read >= 7 && crate::owned_aac::adts::header(&sig[..7]).is_some() {
                let _ = file.seek(SeekFrom::Start(0));
                return crate::owned_adts_remux::remux_from_file(file, destination, options);
            }
        } else if options.max_packets == Some(0) {
            return Err("packet work limit requires at least one packet".into());
        }
    } else if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    crate::owned_wave_remux::remux(source, destination, options)
}
