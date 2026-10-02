//! Owned file concat dispatch: PCM WAVE or compatible ADTS into Matroska.
use fvid_control::CopyOptions;
use fvid_media_info::CopyStats;
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};
pub(crate) fn supports(sources: &[PathBuf], destination: &Path, options: &CopyOptions) -> bool {
    (2..=256).contains(&sources.len())
        && sources
            .iter()
            .all(|source| crate::owned_adts_remux::supports(source, destination, options))
}
pub fn concat(
    sources: &[PathBuf],
    destination: &Path,
    options: &CopyOptions,
) -> Result<CopyStats, String> {
    if !supports(sources, destination, options) {
        return crate::owned_wave_remux::concat(sources, destination, options);
    }
    let mut readers = Vec::with_capacity(sources.len());
    for source in sources {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        let file = File::open(source).map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("ADTS concat requires regular files".into());
        }
        readers.push(
            crate::owned_aac::adts::StreamReader::open_with_packet_limit(
                BufReader::new(file),
                options.max_packet_bytes,
            )
            .map_err(|e| e.to_string())?,
        );
    }
    let mut stats = crate::owned_matroska_remux::publish(destination, options, |output| {
        crate::owned_matroska::concat_adts(
            readers,
            output,
            options.cancel.as_ref(),
            options.progress.as_ref(),
        )
        .map_err(|e| e.to_string())
    })?;
    stats.segments = sources.len();
    Ok(stats)
}
