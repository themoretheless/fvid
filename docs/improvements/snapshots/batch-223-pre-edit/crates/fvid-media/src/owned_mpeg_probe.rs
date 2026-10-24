//! Header/index inspection of canonical bare MPEG Layer I/II/III framing.
//! Durations use audible sample coordinates. This does not qualify every
//! encoded payload or decode PCM, except bounded free-format framing probes.
use crate::owned_mpeg_audio::{LAYER_I, LAYER_II};
use fvid_media_info::{MediaInfo, StreamInfo};
use std::{collections::BTreeMap, path::Path};
pub fn probe_mpeg(path: &Path) -> Result<MediaInfo, String> {
    if !crate::owned_raw_mp3::recognizes(path)? {
        return Err("input does not have a bare MPEG audio signature".into());
    }
    let source = crate::owned_raw_mp3::open(path, &Default::default())?;
    let rate = source.sample_rate;
    let audible = source.presentation_samples();
    let coded = source.samples();
    let duration = i64::try_from(audible).map_err(|_| "MPEG duration exceeds API range")?;
    let duration_us = i64::try_from(u128::from(audible) * 1_000_000 / u128::from(rate))
        .map_err(|_| "MPEG duration exceeds API range")?;
    let payload = source
        .frames
        .iter()
        .try_fold(0u128, |sum, frame| sum.checked_add(frame.size as u128))
        .ok_or("MPEG byte count overflow")?;
    let average = |bytes: u128, samples: u64| {
        if samples == 0 {
            None
        } else {
            i64::try_from(bytes * 8 * u128::from(rate) / u128::from(samples)).ok()
        }
    };
    let file_bytes = u128::from(std::fs::metadata(path).map_err(|e| e.to_string())?.len());
    let codec = match source.tag {
        LAYER_I => "mp1",
        LAYER_II => "mp2",
        _ => "mp3",
    };
    Ok(MediaInfo {
        path: path.into(),
        format: "mp3".into(),
        start_us: Some(0),
        duration_us: Some(duration_us),
        bit_rate: average(file_bytes, audible),
        metadata: BTreeMap::new(),
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
            metadata: BTreeMap::new(),
            width: 0,
            height: 0,
            pixel_format: -1,
            sample_rate: rate as i32,
            channels: i32::from(source.channels),
            video_delay: 0,
            extradata_bytes: 0,
        }],
    })
}
