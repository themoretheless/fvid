//! Header/index inspection of canonical bare MPEG Layer I/II/III framing.
//! Durations use audible sample coordinates. This does not qualify every
//! encoded payload or decode PCM, except bounded free-format framing probes.
use crate::owned_mpeg_audio::{LAYER_I, LAYER_II};
use fvid_media_info::{MediaInfo, StreamInfo};
use std::{fs::File, path::Path};
pub fn probe_mpeg(path: &Path) -> Result<MediaInfo, String> {
    probe_mpeg_with_options(path, &Default::default())
}

pub fn probe_mpeg_with_options(
    path: &Path,
    options: &fvid_control::CopyOptions,
) -> Result<MediaInfo, String> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
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
    let source = crate::owned_raw_mp3::open(path, options)?;
    probe_mpeg_from_source(path, source, options)
}

pub fn probe_mpeg_from_file_with_options(
    path: &Path,
    file: File,
    options: &fvid_control::CopyOptions,
) -> Result<MediaInfo, String> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
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
    let source = crate::owned_raw_mp3::open_from_file(file, options)?;
    probe_mpeg_from_source(path, source, options)
}

fn probe_mpeg_from_source(
    path: &Path,
    source: crate::owned_mpeg_audio::Mp3,
    options: &fvid_control::CopyOptions,
) -> Result<MediaInfo, String> {
    let pres = source.presentation_window();
    let active_frames = source.frames.iter().take_while(|f| f.pts < pres.end).count();
    let packet_work = active_frames.saturating_add(usize::from(source.info_packet().is_some()));
    if options
        .max_packets
        .is_some_and(|max| (packet_work as u64) > max)
    {
        return Err("MPEG packet count exceeds packet limit".into());
    }
    let (metadata, stream_metadata) = source.probe_metadata();
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
    let file_bytes = u128::from(source.file_bytes() as u64);
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
            extradata_bytes: 0,
        }],
    })
}
