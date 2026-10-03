//! Owned MP4 container descriptions and optional AVC SPS metadata, without libav.
use crate::owned_avc::configuration_profile_level as avc_probe_config;
use crate::owned_codec_config::aac_specific_config as aac_probe_config;
use crate::owned_mp4::Mp4Reader as Mp4ProbeReader;
use fvid_media_info::{ChapterInfo, MediaInfo, StreamInfo};
use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
fn ticks(value: u128, scale: u32, target: u32) -> Result<i64> {
    if scale == 0 {
        return Err("zero MP4 timescale".into());
    }
    i64::try_from(
        value
            .checked_mul(u128::from(target))
            .ok_or("MP4 duration overflow")?
            / u128::from(scale),
    )
    .map_err(|_| "MP4 duration exceeds probe API range".into())
}
fn tags(tags: &crate::owned_file_tags::FileTags) -> BTreeMap<String, String> {
    [
        ("title", &tags.title),
        ("artist", &tags.artist),
        ("album", &tags.album),
        ("genre", &tags.genre),
        ("date", &tags.date),
        ("comment", &tags.comment),
        ("track", &tags.track),
        ("album_artist", &tags.album_artist),
        ("disc", &tags.disc),
        ("publisher", &tags.publisher),
        ("copyright", &tags.copyright),
        ("description", &tags.description),
        ("rating", &tags.rating),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(key, value)| (key.into(), value.clone()))
    .collect()
}

fn mp4_unrepresented_error(error: &crate::owned_mp4::Error) -> bool {
    error.is_unsupported()
}
include!("owned_mp4_probe_impl.rs");
pub fn probe_mp4(path: &Path) -> Result<MediaInfo> {
    mp4(path)
}

/// Describe represented MP4/MOV tracks. `None` means the valid container
/// includes a sample entry this parser cannot yet describe; malformed input
/// returns an error and must not trigger another parser.
pub fn try_probe_mp4(path: &Path) -> Result<Option<MediaInfo>> {
    try_mp4(path)
}
