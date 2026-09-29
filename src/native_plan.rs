//! Plans for owned media operations; no external demuxer or codec dependency.
pub use fvid_media_info::{AudioDecodeTransform, MediaPlan, PlanStep, PlanStream};
type Result<T> = std::result::Result<T, String>;

/// Plan AAC-LC extraction from owned ADTS, MP4 or Matroska readers.
/// Packet contents and timeline consistency are checked during execution.
pub fn decode_audio(
    source: &std::path::Path,
    transform: &AudioDecodeTransform,
) -> Result<MediaPlan> {
    decode_audio_selected(source, transform, None)
}

/// Plan one explicitly selected zero-based container stream.
pub fn decode_audio_selected(
    source: &std::path::Path,
    transform: &AudioDecodeTransform,
    selected: Option<usize>,
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
    let info = crate::native_media::aac_source_info_selected(source, selected)
        .map_err(|e| e.to_string())?;
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

/// Metadata-only plan for exact RIFF/WAVE PCM slicing. The same parser and
/// sample-boundary calculation are used by `native_pcm::trim_wave`.
pub fn trim_pcm(
    source: &std::path::Path,
    from: i64,
    to: i64,
    selected: Option<usize>,
) -> Result<MediaPlan> {
    if selected.is_some_and(|n| n != 0) {
        return Err("WAVE has only stream 0".into());
    }
    let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let info = crate::native_pcm::inspect(&mut input, None).map_err(|e| e.to_string())?;
    let range = info.interval(from, to).map_err(|e| e.to_string())?;
    let frames = range.end - range.start;
    Ok(MediaPlan {
        command:"trim-pcm".into(),input:source.to_path_buf(),inputs:vec![source.to_path_buf()],
        streams:vec![PlanStream{index:0,media_type:"audio".into(),codec:info.codec(),disposition:"trim_pcm".into()}],
        steps:vec![
            PlanStep{action:"interval".into(),detail:format!("exact sample range [{}, {}) at {} Hz; {frames} retained frames, {} PCM bytes; requested [{from}, {to}) µs, clipped at EOF",range.start,range.end,info.sample_rate,frames*u64::from(info.frame_bytes()))},
            PlanStep{action:"trim".into(),detail:format!("FVid RIFF reader copies unchanged {}-bit samples in {} channels through aligned blocks of at most 64 KiB; no decoder",info.bits_per_sample,info.channels)},
            PlanStep{action:"metadata".into(),detail:"preserve fmt, INFO, JUNK and PAD chunks; rewrite RIFF/data lengths and fact sample count; reject unsupported timed metadata".into()},
            PlanStep{action:"write".into(),detail:"publish .wav atomically without overwriting an existing path; errors and cancellation remove the temporary output".into()},
        ],graph:None,
        notes:vec!["backend: fvid; no external demuxer, decoder or muxer".into(),
            "metadata-only plan: RIFF geometry and interval are validated; payload reading, output permissions and publication are checked during execution".into()],
    })
}
