//! Plans for owned media operations; no external demuxer or codec dependency.
pub use fvid_media_info::{AudioDecodeTransform, MediaPlan, PlanStep, PlanStream};
type Result<T> = std::result::Result<T, String>;

/// Plan AAC-LC extraction from owned ADTS, MP4 or Matroska readers.
/// Packet contents and timeline consistency are checked during execution.
pub fn decode_audio(
    source: &std::path::Path,
    transform: &AudioDecodeTransform,
) -> Result<MediaPlan> {
    if transform
        .interval
        .is_some_and(|(from, to)| from < 0 || to <= from)
    {
        return Err("decode-audio interval requires 0 <= from < to".into());
    }
    if transform
        .sample_rate
        .is_some_and(|rate| !(8000..=384000).contains(&rate))
    {
        return Err("sample rate must be within 8000..=384000".into());
    }
    if transform
        .volume
        .is_some_and(|gain| !gain.is_finite() || !(0.0..=64.0).contains(&gain))
    {
        return Err("volume must be a finite linear gain within 0..=64".into());
    }
    let info = crate::native_media::aac_source_info(source).map_err(|e| e.to_string())?;
    let channels = transform.channels.unwrap_or(i32::from(info.channels));
    if channels != i32::from(info.channels) && !matches!(channels, 1 | 2) {
        return Err("native AAC channel conversion supports mono or stereo output".into());
    }
    let rate = transform.sample_rate.unwrap_or(info.sample_rate as i32);
    let mut steps = vec![PlanStep {
        action: "decode".into(),
        detail: "FVid owned AAC-LC decoder to interleaved float PCM".into(),
    }];
    if let Some((from, to)) = transform.interval {
        steps.push(PlanStep { action: "trim".into(),
            detail: format!("presentation window [{from}, {to}) µs; round up to source sample grid; decode pre-roll") });
    }
    if channels != i32::from(info.channels) {
        steps.push(PlanStep { action: "rematrix".into(),
            detail: format!("FVid channel conversion {} → {channels}; center/surround -3 dB, omit LFE when downmixing", info.channels) });
    }
    if let Some(gain) = transform.volume.filter(|&gain| gain != 1.0) {
        steps.push(PlanStep {
            action: "volume".into(),
            detail: format!("unclipped linear gain {gain} after channel conversion"),
        });
    }
    if rate != info.sample_rate as i32 {
        steps.push(PlanStep {
            action: "resample".into(),
            detail: format!(
                "FVid windowed-sinc resampler {} Hz → {rate} Hz",
                info.sample_rate
            ),
        });
    }
    steps.push(PlanStep {
        action: "write".into(),
        detail:
            "atomic float32 PCM export to .f32le or .wav; never overwrite an existing destination"
                .into(),
    });
    Ok(MediaPlan {
        command: "decode-audio".into(), input: source.to_path_buf(), inputs: vec![source.to_path_buf()],
        streams: vec![PlanStream { index: info.stream_index, media_type: "audio".into(),
            codec: "aac".into(), disposition: "decode".into() }],
        steps, graph: None,
        notes: vec![
            "backend: fvid; no external codec or resampler".into(),
            "MP4 edits and Matroska trim metadata apply; ADTS retains encoder priming".into(),
            "metadata-only plan: packet contents and timeline consistency are verified during execution".into(),
        ],
    })
}
