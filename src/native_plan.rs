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
        if transform.channels.is_some_and(|n| n != i32::from(info.channels)) { info.validate_rematrix().map_err(|e|e.to_string())?; }
        info.decode_interval(transform.interval.map(|(a,b)|(std::time::Duration::from_micros(a as u64),std::time::Duration::from_micros(b as u64)))).map_err(|e|e.to_string())?;
        (0,info.sample_rate,info.channels,info.codec(),"FVid packed WAVE PCM conversion to interleaved float32".to_owned())
    } else {
        let info=crate::native_media::audio_source_info_selected(source,selected).map_err(|e|e.to_string())?;
        (info.stream_index,info.sample_rate,info.channels,info.codec.into(),format!("FVid owned {} decoder to interleaved float PCM",match info.codec {"aac"=>"AAC-LC","alac"=>"ALAC",_=>"PCM"}))
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
            "atomic float32 PCM export to .f32le, .wav, .mka or .mkv; never overwrite an existing destination"
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
    let mp4 = crate::container::mp4::recognizes_prefix(&signature[..count]);
    let adts = crate::container::adts::header(&signature[..count]).is_some();
    let matroska=count>=4 && signature[..4]==[0x1a,0x45,0xdf,0xa3];
    if !mp4 && !adts && !matroska { return Ok(None); }
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let detail = if matroska {
        crate::container::matroska_copy::inspect(&mut input,false,None).map_err(|e|e.to_string())?;
        "FVid identity Matroska copy; retain all EBML bytes including attachments and unknown metadata"
    } else if mp4 {
        crate::container::mp4_relocate::fast_start(&mut input, &mut std::io::sink()).map_err(|e| e.to_string())?;
        "FVid MP4 fast-start relocation; initialized fragmented MP4 is copied unchanged"
    } else {
        "FVid ADTS AAC packet copy into MP4 or Matroska; preserve encoder priming"
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
            if matroska {"owned Matroska output: .mkv, or .mka for audio-only input; no stream or metadata edits".into()} else if adts {"owned ADTS output: .mp4/.m4a or .mka/.mkv; all packets retained".into()} else {"owned MP4 output requires .mp4 or .m4a; all streams retained".into()},
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

/// Validate each ADTS segment independently and describe packet concatenation.
pub fn concat_adts(sources: &[std::path::PathBuf]) -> Result<MediaPlan> {
    if !(2..=256).contains(&sources.len()) {return Err("concat requires 2..=256 inputs".into());}
    let mut asc=None;
    let mut packets=0usize;
    for source in sources {
        let file=std::fs::File::open(source).map_err(|e|e.to_string())?;
        let mut reader=crate::container::adts::StreamReader::open(std::io::BufReader::new(file)).map_err(|e|e.to_string())?;
        let config=reader.configuration().asc;
        if asc.is_some_and(|v|v!=config) {return Err("ADTS concat requires identical AAC configurations".into());}
        asc=Some(config);
        while reader.next_packet().map_err(|e|e.to_string())?.is_some() {
            packets+=1;
            if packets>crate::container::mp4::Limits::default().samples {return Err("AAC remux sample index exceeds limit".into());}
        }
    }
    Ok(MediaPlan {
        command:"concat".into(),input:sources[0].clone(),inputs:sources.to_vec(),
        streams:vec![PlanStream {index:0,media_type:"audio".into(),codec:"aac".into(),disposition:"copy".into()}],
        steps:vec![PlanStep {action:"copy".into(),detail:format!("append {packets} AAC packets into one MP4 or Matroska track without decoding")},
            PlanStep {action:"publish".into(),detail:"write container timing/index; sync and publish without overwriting".into()}],
        graph:None,notes:vec!["backend: fvid; output requires .mp4/.m4a or .mka/.mkv".into(),
            "ADTS encoder priming and padding remain in every segment; no gapless trimming".into()],
    })
}

/// Describe ADTS-to-WAVE interval decoding, including exact retained sample count.
pub fn trim_adts(source: &std::path::Path, from: i64, to: i64) -> Result<MediaPlan> {
    if from < 0 || to <= from {return Err("trim requires 0 <= from < to".into());}
    let input=std::io::BufReader::new(std::fs::File::open(source).map_err(|e|e.to_string())?);
    let info=crate::native_media::inspect_adts(input).map_err(|e|e.to_string())?;
    let boundary=|us:i64| (us as u128 * u128::from(info.sample_rate)).div_ceil(1_000_000).min(u128::from(info.sample_frames)) as u64;
    let frames=boundary(to)-boundary(from);
    if frames==0 {return Err("audio interval contains no samples".into());}
    let mut plan=decode_audio(source,&AudioDecodeTransform {interval:Some((from,to)),..Default::default()})?;
    plan.command="trim".into();
    plan.steps.last_mut().ok_or("missing write step")?.detail=format!("publish {frames} float32 PCM sample frames to .wav without overwriting");
    plan.notes.push("decode AAC pre-roll from the beginning; ADTS encoder priming remains part of its timeline".into());
    Ok(plan)
}

/// Require either a single AAC stream or an explicit AAC stream selection.
/// A PCM destination must not silently discard other container tracks.
pub(crate) fn aac_trim_selection(source: &std::path::Path, selected: Option<usize>) -> Result<usize> {
    let info=crate::native_probe::probe(source)?;
    let index=match selected {
        Some(index)=>index,
        None if info.streams.len()==1=>info.streams[0].index,
        None=>return Err("AAC-to-WAVE trim requires an explicit audio stream when other tracks exist".into()),
    };
    if !info.streams.iter().any(|s|s.index==index && s.media_type=="audio" && s.codec=="aac") {
        return Err("selected trim stream is not AAC audio".into());
    }
    Ok(index)
}

/// Plan AAC interval export using source edits and trim metadata.
pub fn audio_trim_selection(source: &std::path::Path, selected: Option<usize>) -> Result<usize> {
    let info = crate::native_probe::probe(source)?;
    let index = match selected {
        Some(index) => index,
        None if info.streams.len() == 1 => info.streams[0].index,
        None => return Err("audio-to-WAVE trim requires an explicit audio stream when other tracks exist".into()),
    };
    crate::native_media::audio_source_info_selected(source, Some(index)).map_err(|e| e.to_string())?;
    Ok(index)
}

pub fn trim_audio(source: &std::path::Path, from: i64, to: i64, selected: Option<usize>) -> Result<MediaPlan> {
    if crate::native_media::is_aac_trim_source(source, selected).map_err(|e| e.to_string())? {
        return trim_aac(source, from, to, selected);
    }
    let index = audio_trim_selection(source, selected)?;
    let mut plan = decode_audio_selected(source, &AudioDecodeTransform {
        interval: Some((from, to)), ..Default::default()
    }, Some(index))?;
    plan.command = "trim".into();
    plan.steps.last_mut().ok_or("missing write step")?.detail = "publish selected audio as float32 PCM .wav without overwriting".into();
    Ok(plan)
}

pub fn trim_aac(source: &std::path::Path, from: i64, to: i64, selected: Option<usize>) -> Result<MediaPlan> {
    let index=aac_trim_selection(source,selected)?;
    if crate::native_export::is_adts_source(source).map_err(|e|e.to_string())? {
        return trim_adts(source,from,to);
    }
    let mut plan=decode_audio_selected(source,&AudioDecodeTransform {interval:Some((from,to)),..Default::default()},Some(index))?;
    plan.command="trim".into();
    plan.steps.last_mut().ok_or("missing write step")?.detail="publish selected audio as float32 PCM .wav without overwriting".into();
    plan.notes.push("only explicitly selected audio is exported; source presentation edits and decoder pre-roll apply".into());
    Ok(plan)
}

/// Metadata-only plan for decoded, constant-rate video concatenation to Y4M.
pub fn concat_y4m(sources: &[std::path::PathBuf], selected: Option<usize>) -> Result<MediaPlan> {
    if !(2..=256).contains(&sources.len()) {return Err("concat requires 2..=256 inputs".into());}
    for source in sources {crate::native_export::validate_video_selection(source,selected).map_err(|e|e.to_string())?;}
    Ok(MediaPlan {
        command:"concat".into(),input:sources[0].clone(),inputs:sources.to_vec(),
        streams:vec![PlanStream {index:selected.unwrap_or(0),media_type:"video".into(),codec:"rawvideo".into(),disposition:"decode".into()}],
        steps:vec![PlanStep {action:"decode".into(),detail:"decode each source independently through FVid and append presentation frames".into()},
            PlanStep {action:"write".into(),detail:"require matching frame rate, geometry, bit depth, chroma, range and pixel aspect; publish Y4M without overwriting".into()}],
        graph:None,notes:vec!["backend: fvid; output .y4m only, no external codecs".into(),
            "metadata-only plan; exact frame timing and decoded layout are validated during execution".into(),
            "timestamps restart at zero in each segment and become contiguous in output; audio is not exported".into()],
    })
}

/// Plan decoded video interval export with an explicit Y4M output contract.
pub fn trim_y4m(source: &std::path::Path, from: i64, to: i64, selected: Option<usize>) -> Result<MediaPlan> {
    if from < 0 || to <= from {return Err("trim requires 0 <= from < to".into());}
    crate::native_export::validate_video_selection(source,selected).map_err(|e|e.to_string())?;
    Ok(MediaPlan {
        command:"trim".into(),input:source.to_owned(),inputs:vec![source.to_owned()],
        streams:vec![PlanStream {index:selected.unwrap_or(0),media_type:"video".into(),codec:"rawvideo".into(),disposition:"decode".into()}],
        steps:vec![PlanStep {action:"decode".into(),detail:"FVid software decode including reference pre-roll".into()},
            PlanStep {action:"trim".into(),detail:format!("retain frame presentation starts in [{from}, {to}) microseconds from the first presented frame")},
            PlanStep {action:"write".into(),detail:"require constant contiguous selected frame timing; publish Y4M without overwriting".into()}],
        graph:None,notes:vec!["backend: fvid; output .y4m only; no audio is exported".into(),
            "metadata-only plan; decoded frames, nonempty interval and exact timing are validated during execution".into()],
    })
}

/// Metadata-only plan for the same admission used by owned lossless export.
pub fn transcode_lossless(source: &std::path::Path, transform: &crate::media_info::LosslessTransform) -> Result<MediaPlan> {
    if crate::native_lossless::supports_overlay(transform) {
        let spec=transform.overlay.as_ref().unwrap();let mut remaining=transform.clone();remaining.overlay=None;
        let spatial=transcode_lossless(source,&remaining)?;
        let mut plan=overlay(source,&spec.path,i64::from(spec.x),i64::from(spec.y))?;
        let mut index=1;
        for step in spatial.steps.iter().filter(|s|s.action=="geometry") {plan.steps.insert(index,step.clone());index+=1;}
        index+=1;
        for step in spatial.steps.iter().filter(|s|s.action=="filter") {plan.steps.insert(index,step.clone());index+=1;}
        plan.command="transcode-lossless".into();return Ok(plan);
    }
    if !crate::native_lossless::supports(transform) || !crate::native_lossless::eligible(source).map_err(|e|e.to_string())? {
        return Err("request is not supported by the owned lossless planner".into());
    }
    let (geometry,filters)=crate::native_lossless::configuration(transform).map_err(|e|e.to_string())?;
    let streams=if crate::native_lossless_y4m::is_source(source).map_err(|e|e.to_string())? {
        vec![PlanStream {index:0,media_type:"video".into(),codec:"rawvideo".into(),disposition:"primary_video".into()}]
    } else {
        let info=crate::native_probe::probe(source)?;
        info.streams.iter().map(|stream| PlanStream {
            index:stream.index,media_type:stream.media_type.clone(),codec:stream.codec.clone(),
            disposition:if stream.media_type=="video" {"primary_video"} else {"copy"}.into(),
        }).collect()
    };
    let mut steps=vec![PlanStep{action:"decode".into(),detail:"FVid owned demuxer and video decoder; retain supported companion audio packets without audio re-encoding".into()}];
    if !geometry.is_identity() {steps.push(PlanStep{action:"geometry".into(),detail:format!("crop {:?}; horizontal flip {}; vertical flip {}; transpose {:?}; pad {:?}; scale {:?}; normalize stored rotation first",geometry.crop,geometry.horizontal_flip,geometry.vertical_flip,geometry.transpose,geometry.pad,geometry.scale)});}
    if !filters.is_empty() {
        for (name,args) in [("avgblur",&transform.avgblur),("boxblur",&transform.boxblur),
            ("negate",&transform.negate),("sobel",&transform.sobel),("prewitt",&transform.prewitt),
            ("roberts",&transform.roberts),("kirsch",&transform.kirsch),("scharr",&transform.scharr),
            ("pixelize",&transform.pixelize),("dilation",&transform.dilation),("erosion",&transform.erosion),("chromashift",&transform.chromashift)] {
            if let Some(args)=args {steps.push(PlanStep{action:"filter".into(),detail:format!("FVid {name}={args}; operate on sample planes at source precision")});}
        }
    }
    steps.push(PlanStep{action:"encode".into(),detail:"FVid lossless FFV1 v1 range encoder; retain video timing and display metadata".into()});
    steps.push(PlanStep{action:"write".into(),detail:"FVid Matroska muxer; atomic publication without overwrite; cancellation removes temporary output".into()});
    Ok(MediaPlan {command:"transcode-lossless".into(),input:source.to_path_buf(),inputs:vec![source.to_path_buf()],streams,steps,graph:None,
        notes:vec!["backend: fvid; no external demuxer, decoder, encoder or muxer".into(),"metadata-only plan: frame geometry, filter plane compatibility and payload correctness are validated during execution".into()]})
}

/// Plan owned loudness measurement without decoding or claiming true peak.
pub fn loudness(source: &std::path::Path, selected: Option<usize>, weights: Option<&[f64]>) -> Result<MediaPlan> {
    let mut plan=decode_audio_selected(source,&Default::default(),selected)?;
    let defaults;
    let weights=match weights {
        Some(weights)=>weights,
        None=>{defaults=crate::native_pcm::loudness_channel_weights(source,selected).map_err(|e|e.to_string())?;&defaults},
    };
    let (rate,channels)=if crate::native_pcm::is_wave(source).map_err(|e|e.to_string())? {
        let info=crate::native_pcm::inspect(&mut std::fs::File::open(source).map_err(|e|e.to_string())?,None).map_err(|e|e.to_string())?;
        (info.sample_rate,info.channels)
    } else {
        let info=crate::native_media::audio_source_info_selected(source,selected).map_err(|e|e.to_string())?;
        (info.sample_rate,info.channels)
    };
    if weights.len()!=usize::from(channels) {return Err("channel weights must match selected audio stream".into());}
    crate::native_pcm::LoudnessMeter::new(rate,weights)?;
    plan.command="loudness".into();
    plan.steps.retain(|step|step.action!="write");
    plan.steps.push(PlanStep {action:"analyze".into(),detail:format!("owned K-weighting, gated integrated LUFS, 3-second LRA and unweighted sample peak; channel energy weights {weights:?}")});
    plan.notes.push("Silence and incomplete windows return null measurements; true peak is not measured".into());
    Ok(plan)
}

/// Metadata-only two-pass constant-gain normalization plan.
pub fn normalize_loudness(source: &std::path::Path, selected: Option<usize>, weights: Option<&[f64]>, target: crate::native_pcm::NormalizeTarget) -> Result<MediaPlan> {
    target.validate()?;
    let mut plan=loudness(source,selected,weights)?;
    plan.command="normalize-loudness".into();
    plan.steps.push(PlanStep {action:"gain".into(),detail:format!("constant gain to {} LUFS, limited by sample-peak ceiling {} dBFS; reject gain above 64",target.integrated_lufs,target.sample_peak_dbfs)});
    plan.steps.push(PlanStep {action:"decode".into(),detail:"second owned decode pass, preserving rate, channels and dynamics".into()});
    plan.steps.push(PlanStep {action:"write".into(),detail:"atomic float WAVE or Matroska PCM publication; reject existing destination".into()});
    plan.notes.push("Gain is computed during execution; silent/short input fails; peak limiting may prevent reaching the LUFS target; no true-peak ceiling".into());
    Ok(plan)
}


/// Validate compressed concat compatibility and describe the owned Matroska route.
/// Plan Matroska concat for a concrete destination, including audio-only admission.
pub fn concat_matroska_to(sources: &[std::path::PathBuf], destination: &std::path::Path) -> Result<Option<MediaPlan>> {
    let audio_only=match destination.extension().and_then(|s|s.to_str()) {
        Some("mka")=>true,Some("mkv")=>false,_=>return Ok(None),
    };
    let Some(plan)=concat_mp4_matroska(sources)? else {return Ok(None);};
    if audio_only && crate::native_probe::probe(&sources[0])?.streams.iter().any(|s|s.media_type!="audio") {
        return Err(".mka concat output requires audio-only input".into());
    }
    Ok(Some(plan))
}

pub fn concat_mp4_matroska(sources: &[std::path::PathBuf]) -> Result<Option<MediaPlan>> {
    if crate::container::mp4_concat::open(sources,None).map_err(|e|e.to_string())?.is_none() {
        if !crate::native_audio_mix::concat_eligible(sources).map_err(|e|e.to_string())? {return Ok(None);}
        return Ok(Some(MediaPlan {command:"concat".into(),input:sources[0].clone(),inputs:sources.to_vec(),
            streams:crate::native_probe::probe(&sources[0])?.streams.into_iter().map(|s|PlanStream {
                index:s.index,media_type:s.media_type,codec:s.codec,disposition:"decode_to_pcm".into(),
            }).collect(),graph:None,
            steps:vec![PlanStep {action:"decode".into(),detail:"decode each single audio source independently through owned codecs, retaining its audible samples".into()},
                PlanStep {action:"concat".into(),detail:"append float32 samples without resampling or channel remapping".into()},
                PlanStep {action:"write".into(),detail:"owned PCM Matroska .mka/.mkv, atomic publication without overwrite".into()}],
            notes:vec!["backend: fvid; matching sample rates and channel counts required".into(),"decoded PCM output; tags, chapters and compressed packets are not retained; one segment of temporary disk space required".into()]}));
    }
    let info=crate::native_probe::probe(&sources[0])?;
    Ok(Some(MediaPlan {command:"concat".into(),input:sources[0].clone(),inputs:sources.to_vec(),
        streams:info.streams.into_iter().map(|s|PlanStream {index:s.index,media_type:s.media_type,codec:s.codec,disposition:"copy".into()}).collect(),
        steps:vec![PlanStep {action:"copy".into(),detail:"FVid AVC/HEVC/AAC packet copy; preserve decode order and shift each segment by its presented duration".into()},
            PlanStep {action:"mux".into(),detail:"FVid Matroska muxer; retain first-input track/file metadata and offset all input chapters".into()},
            PlanStep {action:"publish".into(),detail:"Atomic publication without overwriting; completion only after publication".into()}],graph:None,
        notes:vec!["backend: fvid; output .mkv or audio-only .mka".into(),"AAC priming is retained through global CodecDelay and per-segment signed DiscardPadding".into()]}))
}

/// Describe owned opaque overlay while retaining supported main audio tracks.
pub fn overlay(source:&std::path::Path,foreground:&std::path::Path,x:i64,y:i64)->Result<MediaPlan> {
    if !crate::native_export::overlay_eligible(source).map_err(|e|e.to_string())? {return Err("owned overlay requires a supported owned lossless main input".into());}
    let info=crate::native_probe::probe(source)?;let second=crate::native_probe::probe(foreground)?;
    if second.streams.iter().filter(|s|s.media_type=="video").count()!=1 {return Err("foreground requires exactly one video track".into());}
    Ok(MediaPlan {command:"overlay".into(),input:source.to_owned(),inputs:vec![source.to_owned(),foreground.to_owned()],
        streams:info.streams.into_iter().map(|s|PlanStream {index:s.index,media_type:s.media_type.clone(),codec:s.codec,
            disposition:if s.media_type=="video" {"decode_overlay"}else{"copy"}.into()}).collect(),graph:None,
        steps:vec![PlanStep {action:"decode".into(),detail:"owned main and foreground software decoders; normalize stored display transforms".into()},
            PlanStep {action:"overlay".into(),detail:format!("opaque sample-plane overlay at {x},{y}; align first presentation origins; retain latest foreground at each main PTS; hold foreground EOF")},
            PlanStep {action:"encode".into(),detail:"owned lossless FFV1 at main sample depth; preserve main timing and supported AAC/Opus packets".into()},
            PlanStep {action:"write".into(),detail:"owned Matroska .mkv; atomic no-overwrite publication".into()}],
        notes:vec!["backend: fvid; no external decoder, encoder or muxer".into(),"matching sample depth, colour encoding/range and sampling required; chroma placement must align; validated while decoding".into(),"foreground audio is not part of the overlay; main supported companion tracks are retained".into()]})
}
