//! Owned ADTS to Matroska remux; compressed AAC packets remain unchanged.
use fvid_control::CopyOptions;
use fvid_media_info::CopyStats;
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
pub(crate) fn supports(source: &Path, destination: &Path, options: &CopyOptions) -> bool {
    if !crate::owned_matroska_remux::policies(options)
        || !matches!(
            destination.extension().and_then(|s| s.to_str()),
            Some("mkv" | "mka")
        )
    {
        return false;
    }
    let mut prefix = [0; 7];
    File::open(source).is_ok_and(|mut f| {
        f.read_exact(&mut prefix).is_ok() && crate::owned_aac::adts::header(&prefix).is_some()
    })
}
pub fn remux(
    source: &Path,
    destination: &Path,
    options: &CopyOptions,
) -> Result<CopyStats, String> {
    if !supports(source, destination, options) {
        return Err("request is not an owned ADTS Matroska remux".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    let file = File::open(source).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("ADTS remux requires a regular file".into());
    }
    let reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
        BufReader::new(file),
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    crate::owned_matroska_remux::publish(destination, options, |output| {
        crate::owned_matroska::write_adts(
            reader,
            output,
            options.cancel.as_ref(),
            options.progress.as_ref(),
        )
        .map_err(|e| e.to_string())
    })
}
