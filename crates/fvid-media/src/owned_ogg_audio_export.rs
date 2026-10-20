//! Shared controlled codec selection for Ogg file operations.
use crate::owned_ogg::Coding;
use crate::owned_ogg_opus_export::{CheckedRead, check, preflight};
use fvid_control::CopyOptions;
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
pub(crate) enum Backend {
    Vorbis,
    Opus,
    Joint,
}
pub(crate) fn coding(source: &Path, options: &CopyOptions) -> Result<Backend, String> {
    check(options)?;
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let (limits, _) = preflight(&mut file, options)?;
    let mut bytes = Vec::new();
    CheckedRead {
        input: BufReader::new(file),
        options,
    }
    .read_to_end(&mut bytes)
    .map_err(|e| e.to_string())?;
    let ogg = crate::owned_ogg::Ogg::parse(&bytes, &limits).map_err(|e| e.to_string())?;
    let catalog = crate::owned_ogg_audio_catalog::from_ogg(&ogg).map_err(|e| e.to_string())?;
    let selected = match options.streams.as_slice() {
        [] if catalog.len() == 1 => &catalog[0],
        [] => return Err("select exactly one Ogg audio stream".into()),
        [index] => catalog
            .iter()
            .find(|t| t.source_index == *index)
            .ok_or("selected Ogg audio source stream is absent")?,
        _ => return Err("select exactly one Ogg audio stream".into()),
    };
    check(options)?;
    let nth = catalog
        .iter()
        .position(|t| t.source_index == selected.source_index)
        .unwrap();
    if crate::owned_ogg_audio_catalog::needs_joint_program(&ogg, nth).map_err(|e| e.to_string())? {
        return Ok(Backend::Joint);
    }
    match selected.coding {
        Coding::Vorbis => Ok(Backend::Vorbis),
        Coding::Opus => Ok(Backend::Opus),
        _ => Err("unsupported Ogg audio source codec".into()),
    }
}
