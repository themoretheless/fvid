//! Canonical bare FLAC header/index inspection; no compressed PCM decoding.
use fvid_media_info::{MediaInfo, StreamInfo};
use std::path::Path;
pub fn probe_flac(path: &Path) -> Result<MediaInfo, String> {
    if !crate::owned_raw_flac::recognizes(path)? {
        return Err("input does not have a bare FLAC audio signature".into());
    }
    let source = crate::owned_raw_flac::open(path, &Default::default())?;
    let comments = crate::owned_flac_layout::vorbis_comments(source.metadata_blocks())
        .map_err(|e| e.to_string())?;
    let (metadata, stream_metadata) = crate::owned_flac_layout::probe_metadata_from_entries(
        comments.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    );
    let rate = source.sample_rate;
    let audible = source.samples();
    let coded = source.samples();
    let duration = i64::try_from(audible).map_err(|_| "FLAC duration exceeds API range")?;
    let duration_us = i64::try_from(u128::from(audible) * 1_000_000 / u128::from(rate))
        .map_err(|_| "FLAC duration exceeds API range")?;
    let payload = source
        .frames
        .iter()
        .try_fold(0u128, |sum, frame| sum.checked_add(frame.size as u128))
        .ok_or("FLAC byte count overflow")?;
    let average = |bytes: u128, samples: u64| {
        if samples == 0 {
            None
        } else {
            i64::try_from(bytes * 8 * u128::from(rate) / u128::from(samples)).ok()
        }
    };
    let file_bytes = u128::from(std::fs::metadata(path).map_err(|e| e.to_string())?.len());
    let codec = "flac";
    Ok(MediaInfo {
        path: path.into(),
        format: "flac".into(),
        start_us: Some(0),
        duration_us: Some(duration_us),
        bit_rate: average(file_bytes, audible),
        metadata,
        chapters: Vec::new(),
        streams: vec![StreamInfo {
            index: 0,
            media_type: "audio".into(),
            codec: codec.into(),
            time_base: [1, rate as i32],
            start: Some(0),
            duration: Some(duration),
            bit_rate: average(payload, coded),
            average_frame_rate: [0, 1],
            profile: None,
            level: None,
            disposition: 0,
            metadata: stream_metadata,
            width: 0,
            height: 0,
            pixel_format: -1,
            sample_rate: rate as i32,
            channels: i32::from(source.channels),
            video_delay: 0,
            extradata_bytes: source.stream_info.len(),
        }],
    })
}
