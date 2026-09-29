//! Plans for owned media operations; no external demuxer or codec dependency.
pub use fvid_media_info::{AudioDecodeTransform, MediaPlan, PlanStep, PlanStream};
type Result<T> = std::result::Result<T, String>;

/// Plan AAC-LC or packed WAVE PCM extraction through owned readers.
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
    let wave=crate::native_pcm::is_wave(source).map_err(|e|e.to_string())?;
    let (index,input_rate,input_channels,codec,decode_detail)=if wave {
        if selected.is_some_and(|n|n!=0) {return Err("WAVE has only stream 0".into());}
        let mut file=std::fs::File::open(source).map_err(|e|e.to_string())?;
        let info=crate::native_pcm::inspect(&mut file,None).map_err(|e|e.to_string())?;
        info.validate_decode().map_err(|e|e.to_string())?;
        info.decode_interval(transform.interval.map(|(a,b)|(std::time::Duration::from_micros(a as u64),std::time::Duration::from_micros(b as u64)))).map_err(|e|e.to_string())?;
        (0,info.sample_rate,info.channels,info.codec(),"FVid packed WAVE PCM conversion to interleaved float32".to_owned())
    } else {
        let info=crate::native_media::aac_source_info_selected(source,selected).map_err(|e|e.to_string())?;
        (info.stream_index,info.sample_rate,info.channels,"aac".into(),"FVid owned AAC-LC decoder to interleaved float PCM".into())
    };
    let channels = transform.channels.unwrap_or(i32::from(input_channels));
    if channels != i32::from(input_channels) && !matches!(channels, 1 | 2) {
        return Err("native audio channel conversion supports mono or stereo output".into());
    }
    let rate = transform.sample_rate.unwrap_or(input_rate as i32);
    let mut steps = vec![PlanStep {
        action: "decode".into(),
        detail: decode_detail,
    }];
    if let Some((from, to)) = transform.interval {
        steps.push(PlanStep { action: "trim".into(),
            detail: format!("presentation window [{from}, {to}) µs; round up to source sample grid; pre-roll only for predictive codecs") });
    }
    if channels != i32::from(input_channels) {
        steps.push(PlanStep { action: "rematrix".into(),
            detail: format!("FVid channel conversion {} → {channels}; center/surround -3 dB, omit LFE when downmixing", input_channels) });
    }
    if let Some(gain) = transform.volume.filter(|&gain| gain != 1.0) {
        steps.push(PlanStep {
            action: "volume".into(),
            detail: format!("unclipped linear gain {gain} after channel conversion"),
        });
    }
    if rate != input_rate as i32 {
        steps.push(PlanStep {
            action: "resample".into(),
            detail: format!(
                "FVid windowed-sinc resampler {} Hz → {rate} Hz",
                input_rate
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
        streams: vec![PlanStream { index: index, media_type: "audio".into(),
            codec, disposition: "decode".into() }],
        steps, graph: None,
        notes: vec![
            "backend: fvid; no external codec or resampler".into(),
            if wave {"WAVE sample starts are selected directly; no decoder pre-roll; invalid float samples or padding bits fail during execution".into()} else {"MP4 edits and Matroska trim metadata apply; ADTS retains encoder priming".into()},
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

/// Plan owned MP4 output. A dry run validates relocation without publishing a file.
/// Unknown input signatures return None for the caller's remaining format dispatch.
pub fn remux(source: &std::path::Path) -> Result<Option<MediaPlan>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut input = std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
    let mut signature = [0; 8];
    let mut count = 0;
    while count < signature.len() {
        let n = input.read(&mut signature[count..]).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        count += n;
    }
    let mp4 = count == 8 && &signature[4..8] == b"ftyp";
    let adts = crate::container::adts::header(&signature[..count]).is_some();
    if !mp4 && !adts { return Ok(None); }
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let detail = if mp4 {
        crate::container::mp4_relocate::fast_start(&mut input, &mut std::io::sink()).map_err(|e| e.to_string())?;
        "FVid MP4 fast-start relocation; initialized fragmented MP4 is copied unchanged"
    } else {
        "FVid ADTS AAC packet copy into MP4; preserve encoder priming and write sample tables"
    };
    let info = crate::native_probe::probe(source)?;
    Ok(Some(MediaPlan {
        command: "remux".into(), input: source.to_owned(), inputs: vec![source.to_owned()],
        streams: info.streams.into_iter().map(|s| PlanStream {
            index: s.index, media_type: s.media_type, codec: s.codec, disposition: "copy".into(),
        }).collect(),
        steps: vec![PlanStep { action: "copy".into(), detail: detail.into() },
            PlanStep { action: "publish".into(), detail: "flush and sync temporary output, then publish without overwriting".into() }],
        graph: None,
        notes: vec!["backend: fvid; no decode/encode or external demuxer".into(),
            "owned output requires .mp4 or .m4a; all streams retained".into(),
            "input structure scanned without writing an output; execution revalidates the current source".into()],
    }))
}

/// Plan sample-preserving WAVE concatenation using the execution validator.
pub fn concat_wave(sources: &[std::path::PathBuf]) -> Result<MediaPlan> {
    let (info,frames) = crate::native_pcm::concat_info(sources).map_err(|e|e.to_string())?;
    Ok(MediaPlan {
        command: "concat".into(), input: sources[0].clone(), inputs: sources.to_vec(),
        streams: vec![PlanStream {index:0,media_type:"audio".into(),codec:info.codec(),disposition:"copy".into()}],
        steps: vec![PlanStep {action:"copy".into(),detail:format!("append {frames} PCM sample frames at {} Hz without conversion",info.sample_rate)},
            PlanStep {action:"publish".into(),detail:"rewrite RIFF/data/fact lengths; sync and publish without overwriting".into()}],
        graph:None,notes:vec!["backend: fvid; no external demuxer or codec".into(),"retain first input metadata; all PCM formats and channel masks must match".into()],
    })
}
