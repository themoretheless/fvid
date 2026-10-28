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
    let mut options = fvid_media::CopyOptions::default();
    if let Some(index) = selected {
        options.streams = vec![index];
    }
    decode_audio_with_options(source, transform, &options)
}

pub fn decode_audio_with_options(
    source: &std::path::Path,
    transform: &AudioDecodeTransform,
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    let selected = options.streams.first().copied();
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
    // Reuse the library's dynamic-description planner and explicit PCE
    // destination rules; entry 1 is not necessarily the active decoder setup.
    {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(source).map_err(|e|e.to_string())?;
        let mut prefix = [0;8];
        if file.read_exact(&mut prefix).is_ok() && crate::container::mp4::recognizes_prefix(&prefix) {
            file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
            let limits = crate::container::mp4::Limits {
                metadata_bytes: crate::container::mp4::Limits::default()
                    .metadata_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                packet_bytes: options
                    .max_packet_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                ..Default::default()
            };
            let reader = crate::container::mp4::Mp4Reader::open_indexed(file, limits).map_err(|e|e.to_string())?;
            let index = crate::native_media::mp4_audio_index(&reader,selected).map_err(|e|e.to_string())?;
            let track = &reader.tracks()[index];
            if track.ensure_constant_description().is_err()
                || transform.channels.is_some_and(|n|track.description_for_sample(0).is_some_and(|d|d.channels>2 && n!=i32::from(d.channels)))
            {
                let mut forwarded=options.clone();
                forwarded.streams=selected.into_iter().collect();
                forwarded.metadata_set.clear();
                forwarded.metadata_delete.clear();
                forwarded.stream_metadata_set.clear();
                forwarded.stream_metadata_delete.clear();
                return fvid_media::plan_decode_audio(source,transform,&forwarded);
            }
        }
    }
    let wave=crate::native_pcm::is_wave(source).map_err(|e|e.to_string())?;
    let (index,input_rate,input_channels,codec,decode_detail)=if wave {
        if selected.is_some_and(|n|n!=0) {return Err("WAVE has only stream 0".into());}
        let mut file=std::fs::File::open(source).map_err(|e|e.to_string())?;
        let info=crate::native_pcm::inspect(&mut file,options.cancel.as_ref()).map_err(|e|e.to_string())?;
        info.validate_decode().map_err(|e|e.to_string())?;
        let frame_bytes = usize::from(info.frame_bytes());
        if frame_bytes == 0
            || (65536usize.min(options.max_packet_bytes) / frame_bytes * frame_bytes) == 0
        {
            return Err("packet byte limit is smaller than one PCM sample frame".into());
        }
        if transform.channels.is_some_and(|n| n != i32::from(info.channels)) { info.validate_rematrix().map_err(|e|e.to_string())?; }
        info.decode_interval(transform.interval.map(|(a,b)|(std::time::Duration::from_micros(a as u64),std::time::Duration::from_micros(b as u64)))).map_err(|e|e.to_string())?;
        (0,info.sample_rate,info.channels,info.codec(),"FVid packed WAVE PCM conversion to interleaved float32".to_owned())
    } else {
        let interval = transform.interval.map(|(a, b)| {
            (
                std::time::Duration::from_micros(a as u64),
                std::time::Duration::from_micros(b as u64),
            )
        });
        let info=crate::native_media::audio_source_info_selected_with_interval_and_options(source,selected,interval,options).map_err(|e|e.to_string())?;
        if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes {
            validate_encoded_audio_packet_budget(source, info.stream_index, interval, options)?;
        }
        if transform.channels.is_some_and(|n| n != i32::from(info.channels)) {
            if info.codec.starts_with("pcm_") && info.channel_mask.is_none() && info.channels > 2 {
                return Err("PCM channel conversion requires a known speaker layout".into());
            }
            if !(1..=8).contains(&info.channels) {
                return Err("native audio rematrixing supports 1..=8 input channels".into());
            }
            if info.channel_mask.is_some_and(|mask| {
                !((info.channels <= 2 && mask == 0)
                    || (mask != 0
                        && fvid_media::owned_pcm_channels::rematrix_layout_supported(
                            info.channels,
                            mask,
                        )))
            }) {
                return Err("native audio rematrixing requires a supported speaker layout".into());
            }
        }
        (info.stream_index,info.sample_rate,info.channels,info.codec.into(),format!("FVid owned {} decoder to interleaved float PCM",match info.codec {"aac"=>"AAC-LC","alac"=>"ALAC",_=>"PCM"}))
    };
    let channels = transform.channels.unwrap_or(i32::from(input_channels));
    if channels != i32::from(input_channels)
        && !((input_channels <= 8 && matches!(channels, 1 | 2))
            || (input_channels <= 2 && (1..=8).contains(&channels)))
    {
        return Err("native audio channel conversion supports mono/stereo downmix or 1..=8 channel upmix from mono/stereo".into());
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
    let mut notes = vec![
        "backend: fvid; no external codec or resampler".into(),
        if wave {
            "WAVE sample starts are selected directly; no decoder pre-roll; invalid float samples or padding bits fail during execution".into()
        } else {
            "MP4 edits and Matroska trim metadata apply; ADTS retains encoder priming".into()
        },
        "metadata-only plan: packet contents and timeline consistency are verified during execution".into(),
    ];
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {} packets; controlled memory limit: {} bytes; RSS limit: {} bytes",
            options.max_packet_bytes,
            options.max_packets.map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
            options.max_controlled_bytes.map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
            options.max_rss_bytes.map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
        ));
    }
    Ok(MediaPlan {
        command: "decode-audio".into(), input: source.to_path_buf(), inputs: vec![source.to_path_buf()],
        streams: vec![PlanStream { index, media_type: "audio".into(),
            codec, disposition: "decode".into() }],
        steps, graph: None,
        notes,
    })
}

fn validate_encoded_audio_packet_budget(
    source: &std::path::Path,
    stream_index: usize,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    options: &fvid_media::CopyOptions,
) -> Result<()> {
    use std::io::{BufReader, Read, Seek, SeekFrom};
    let mut input = BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
    let mut prefix = [0; 8];
    input.read_exact(&mut prefix).map_err(|e| e.to_string())?;
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    if crate::container::mp4::recognizes_prefix(&prefix) {
        let reader = crate::container::mp4::Mp4Reader::open(
            input,
            crate::container::mp4::Limits {
                metadata_bytes: crate::container::mp4::Limits::default()
                    .metadata_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                packet_bytes: options
                    .max_packet_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        if let Some(track) = reader.tracks().get(stream_index) {
            let max_samples = crate::native_media::mp4_active_samples(track, interval, options.max_packets);
            let exceeds = match &track.samples {
                crate::container::mp4::SampleIndex::Expanded(samples) => samples
                    .iter()
                    .take(max_samples)
                    .any(|s| (s.size as usize) > options.max_packet_bytes),
                crate::container::mp4::SampleIndex::Uniform(uniform) => {
                    uniform.count > 0
                        && max_samples > 0
                        && (uniform.bytes_per_frame as usize) > options.max_packet_bytes
                }
            };
            if exceeds {
                return Err("MP4 packet exceeds budget".into());
            }
        }
        return Ok(());
    }
    if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let mut limits = crate::container::webm::Limits::default();
        let memory_packets = options
            .max_controlled_bytes
            .map(|bytes| bytes / std::mem::size_of::<crate::container::webm::Packet>());
        if let Some(maximum) = memory_packets {
            limits.packets = limits.packets.min(maximum);
        }
        let map_packet_limit = |error: crate::Error| {
            if memory_packets
                .is_some_and(|max| max < crate::container::webm::Limits::default().packets)
                && error
                    .to_string()
                    .contains("WebM packet count exceeds limit")
            {
                "controlled memory budget exceeded: Matroska packet index limit".to_owned()
            } else {
                error.to_string()
            }
        };
        let mut reader = crate::container::webm::WebmReader::open(input, limits)
            .map_err(map_packet_limit)?;
        reader.scan_all().map_err(map_packet_limit)?;
        let Some(track) = reader.tracks.get(stream_index) else {
            return Err("selected audio stream is absent".into());
        };
        let track_number = track.number;
        let active_limit = crate::native_media::matroska_active_track_packets(
            &reader,
            stream_index,
            interval,
            options.max_packets,
        );
        let mut track_packets = 0usize;
        for (packet_index, packet) in reader.packets.iter().enumerate() {
            if packet.track == track_number {
                if track_packets >= active_limit {
                    break;
                }
                if reader
                    .packet_payload_size(packet_index)
                    .map_err(|e| e.to_string())?
                    > options.max_packet_bytes
                {
                    return Err("WebM packet exceeds budget".into());
                }
                track_packets += 1;
            }
        }
        return Ok(());
    }
    crate::native_media::inspect_adts_with_interval_and_options(input, interval, options)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Metadata-only plan for exact RIFF/WAVE PCM slicing. The same parser and
/// sample-boundary calculation are used by `native_pcm::trim_wave`.
pub fn trim_pcm(
    source: &std::path::Path,
    from: i64,
    to: i64,
    selected: Option<usize>,
) -> Result<MediaPlan> {
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    trim_pcm_with_options(source, from, to, &options)
}

pub fn trim_pcm_with_options(
    source: &std::path::Path,
    from: i64,
    to: i64,
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
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
    fvid_media::owned_budget::check_rss_budget(options)?;
    if options.streams.iter().any(|&n| n != 0) {
        return Err("WAVE has only stream 0".into());
    }
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return fvid_media::plan_trim_pcm(source, from, to, options);
    }
    let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let info =
        crate::native_pcm::inspect(&mut input, options.cancel.as_ref()).map_err(|e| e.to_string())?;
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
    remux_with_options(source, &Default::default())
}

pub fn remux_with_options(
    source: &std::path::Path,
    options: &fvid_media::CopyOptions,
) -> Result<Option<MediaPlan>> {
    use std::io::{Read, Seek, SeekFrom};
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
    fvid_media::owned_budget::check_rss_budget(options)?;
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
        crate::container::matroska_copy::inspect_with_packet_and_memory_limits(
            &mut input,
            false,
            options.cancel.as_ref(),
            options.max_packet_bytes,
            options.max_controlled_bytes,
        )
        .map_err(|e| e.to_string())?;
        "FVid identity Matroska copy; retain all EBML bytes including attachments and unknown metadata"
    } else if mp4 {
        if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
            || options.max_controlled_bytes.is_some()
        {
            let file = std::fs::File::open(source).map_err(|e| e.to_string())?;
            let effective_packet_bytes = options
                .max_packet_bytes
                .min(options.max_controlled_bytes.unwrap_or(usize::MAX));
            let reader = crate::container::mp4::Mp4Reader::open(
                std::io::BufReader::new(file),
                crate::container::mp4::Limits {
                    packet_bytes: effective_packet_bytes,
                    metadata_bytes: crate::container::mp4::Limits::default().metadata_bytes
                        .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            for track in reader.tracks() {
                if (0..track.samples.len()).any(|i| {
                    track
                        .samples
                        .get(i)
                        .is_some_and(|sample| usize::try_from(sample.size).unwrap_or(usize::MAX) > options.max_packet_bytes)
                }) {
                    return Err("MP4 packet exceeds budget".into());
                }
                if let Some(mem_limit) = options.max_controlled_bytes
                    && (0..track.samples.len()).any(|i| {
                        track
                            .samples
                            .get(i)
                            .is_some_and(|sample| usize::try_from(sample.size).unwrap_or(usize::MAX) > mem_limit)
                    })
                {
                    return Err("controlled memory budget exceeded".into());
                }
            }
        }
        crate::container::mp4_relocate::fast_start_with_memory_limit(
            &mut input,
            &mut std::io::sink(),
            options.cancel.as_ref(),
            None,
            options.max_controlled_bytes,
        )
        .map_err(|e| e.to_string())?;
        "FVid MP4 fast-start relocation; initialized fragmented MP4 is copied unchanged"
    } else {
        let meta_bytes =
            crate::container::matroska_write::estimate_adts_matroska_metadata_bytes(Some(options))
                .map_err(|e| e.to_string())?;
        if options
            .max_controlled_bytes
            .is_some_and(|limit| 16384usize.saturating_add(meta_bytes) > limit)
        {
            return Err("controlled memory budget exceeded".into());
        }
        let mut reader = crate::container::adts::StreamReader::open_with_packet_limit(
            input,
            options
                .max_packet_bytes
                .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
        )
        .map_err(|e| e.to_string())?;
        let mut packets = 0usize;
        loop {
            if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err("media operation cancelled".into());
            }
            fvid_media::owned_budget::check_rss_budget(options)?;
            if options
                .max_packets
                .is_some_and(|limit| packets as u64 >= limit)
            {
                break;
            }
            let Some(_) = reader.next_packet().map_err(|e| e.to_string())? else {
                break;
            };
            packets += 1;
            if packets > crate::container::mp4::Limits::default().samples {
                return Err("AAC remux sample index exceeds limit".into());
            }
            if options.max_controlled_bytes.is_some_and(|limit| {
                packets.saturating_mul(32).saturating_add(16384).saturating_add(meta_bytes) > limit
            }) {
                return Err("controlled memory budget exceeded".into());
            }
        }
        "FVid ADTS AAC packet copy into MP4 or Matroska; preserve encoder priming"
    };
    let info = if matroska {
        crate::native_probe::matroska_with_options(source, options)?
    } else if adts {
        let mut probe_opts = options.clone();
        probe_opts.max_controlled_bytes = None;
        probe_opts.max_packets = None;
        crate::native_probe::probe_with_options(source, &probe_opts)?
    } else {
        crate::native_probe::probe_with_options(source, options)?
    };
    let mut notes = vec![
        "backend: fvid; no decode/encode or external demuxer".into(),
        if matroska {"owned Matroska output: .mkv, or .mka for audio-only input; no stream or metadata edits".into()} else if adts {"owned ADTS output: .mp4/.m4a or .mka/.mkv; all packets retained".into()} else {"owned MP4 output requires .mp4 or .m4a; all streams retained".into()},
        "input structure scanned without writing an output; execution revalidates the current source".into(),
    ];
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    let mut steps = vec![
        PlanStep { action: "copy".into(), detail: detail.into() },
        PlanStep { action: "publish".into(), detail: "flush and sync temporary output, then publish without overwriting".into() },
    ];
    if matroska {
        steps.push(PlanStep {
            action: "budget".into(),
            detail: format!("FVid maximum encoded packet size: {} bytes", options.max_packet_bytes),
        });
    }
    Ok(Some(MediaPlan {
        command: "remux".into(), input: source.to_owned(), inputs: vec![source.to_owned()],
        streams: info.streams.into_iter().map(|s| PlanStream {
            index: s.index, media_type: s.media_type, codec: s.codec, disposition: "copy".into(),
        }).collect(),
        steps,
        graph: None,
        notes,
    }))
}

pub fn remux_adts_with_options(
    source: &std::path::Path,
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("ADTS remux requires at least one packet".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    let meta_bytes =
        crate::container::matroska_write::estimate_adts_matroska_metadata_bytes(Some(options))
            .map_err(|e| e.to_string())?;
    if options
        .max_controlled_bytes
        .is_some_and(|limit| 16384usize.saturating_add(meta_bytes) > limit)
    {
        return Err("controlled memory budget exceeded".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    let file = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut reader = crate::container::adts::StreamReader::open_with_packet_limit(
        std::io::BufReader::new(file),
        options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
    )
    .map_err(|e| e.to_string())?;
    let mut packets = 0usize;
    loop {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        if options.max_packets.is_some_and(|limit| packets as u64 >= limit) {
            break;
        }
        let Some(_) = reader.next_packet().map_err(|e| e.to_string())? else {
            break;
        };
        packets += 1;
        if packets > crate::container::mp4::Limits::default().samples {
            return Err("AAC remux sample index exceeds limit".into());
        }
        if options.max_controlled_bytes.is_some_and(|limit| {
            packets.saturating_mul(32).saturating_add(16384).saturating_add(meta_bytes) > limit
        }) {
            return Err("controlled memory budget exceeded".into());
        }
    }
    let mut notes = vec![
        "backend: fvid; no decode/encode or external demuxer".into(),
        "owned ADTS output: .mp4/.m4a or .mka/.mkv; all packets retained".into(),
        "input structure scanned without writing an output; execution revalidates the current source".into(),
    ];
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    let detail = if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_rss_bytes.is_some()
        || !options.streams.is_empty()
    {
        format!("FVid ADTS AAC packet copy ({packets} AAC packets) into MP4 or Matroska; preserve encoder priming")
    } else {
        "FVid ADTS AAC packet copy into MP4 or Matroska; preserve encoder priming".into()
    };
    let mut steps = vec![PlanStep {
        action: "copy".into(),
        detail,
    }];
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        steps.push(PlanStep {
            action: "metadata".into(),
            detail: "apply Matroska container and stream 0 metadata mutations".into(),
        });
    }
    steps.push(PlanStep {
        action: "publish".into(),
        detail: "flush and sync temporary output, then publish without overwriting".into(),
    });
    Ok(MediaPlan {
        command: "remux".into(),
        input: source.to_owned(),
        inputs: vec![source.to_owned()],
        streams: vec![PlanStream {
            index: 0,
            media_type: "audio".into(),
            codec: "aac".into(),
            disposition: "copy".into(),
        }],
        steps,
        graph: None,
        notes,
    })
}

/// Plan sample-preserving WAVE concatenation using the execution validator.
pub fn concat_wave(sources: &[std::path::PathBuf]) -> Result<MediaPlan> {
    concat_wave_with_options(sources, &Default::default())
}

pub fn concat_wave_with_options(
    sources: &[std::path::PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
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
    fvid_media::owned_budget::check_rss_budget(options)?;
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return fvid_media::owned_wave_plan::plan_concat(sources, options);
    }
    let (info, frames) =
        crate::native_pcm::concat_info_with_cancel(sources, options.cancel.as_ref())
            .map_err(|e| e.to_string())?;
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
    concat_adts_with_options(sources, &Default::default())
}

pub fn concat_adts_with_options(
    sources: &[std::path::PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
    if !(2..=256).contains(&sources.len()) {return Err("concat requires 2..=256 inputs".into());}
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("ADTS concat requires at least one packet".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    let meta_bytes =
        crate::container::matroska_write::estimate_adts_matroska_metadata_bytes(Some(options))
            .map_err(|e| e.to_string())?;
    let base_bytes = sources.len().saturating_mul(8192).saturating_add(16384);
    if options
        .max_controlled_bytes
        .is_some_and(|limit| base_bytes.saturating_add(meta_bytes) > limit)
    {
        return Err("controlled memory budget exceeded".into());
    }
    let mut readers = Vec::with_capacity(sources.len());
    for source in sources {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        let file = std::fs::File::open(source).map_err(|e| e.to_string())?;
        let reader = crate::container::adts::StreamReader::open_with_packet_limit(
            std::io::BufReader::new(file),
            options
                .max_packet_bytes
                .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
        )
        .map_err(|e| e.to_string())?;
        readers.push(reader);
    }
    let mut sequence =
        crate::container::adts::SequenceReader::new(readers).map_err(|e| e.to_string())?;
    let mut packets = 0usize;
    loop {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        if options.max_packets.is_some_and(|limit| packets as u64 >= limit) {
            break;
        }
        let Some(_) = sequence.next_packet().map_err(|e| e.to_string())? else {
            break;
        };
        packets += 1;
        if packets > crate::container::mp4::Limits::default().samples {
            return Err("AAC remux sample index exceeds limit".into());
        }
        if options.max_controlled_bytes.is_some_and(|limit| {
            base_bytes
                .saturating_add(packets.saturating_mul(32))
                .saturating_add(meta_bytes)
                > limit
        }) {
            return Err("controlled memory budget exceeded".into());
        }
    }
    let mut notes = vec![
        "backend: fvid; output requires .mp4/.m4a or .mka/.mkv".into(),
        "ADTS encoder priming and padding remain in every segment; no gapless trimming".into(),
    ];
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    let mut steps = vec![PlanStep {
        action: "copy".into(),
        detail: format!("append {packets} AAC packets into one MP4 or Matroska track without decoding"),
    }];
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        steps.push(PlanStep {
            action: "metadata".into(),
            detail: "apply Matroska container and stream 0 metadata mutations".into(),
        });
    }
    steps.push(PlanStep {
        action: "publish".into(),
        detail: "write container timing/index; sync and publish without overwriting".into(),
    });
    Ok(MediaPlan {
        command:"concat".into(),input:sources[0].clone(),inputs:sources.to_vec(),
        streams:vec![PlanStream {index:0,media_type:"audio".into(),codec:"aac".into(),disposition:"copy".into()}],
        steps,
        graph:None,notes,
    })
}

/// Describe ADTS-to-WAVE interval decoding, including exact retained sample count.
pub fn trim_adts(source: &std::path::Path, from: i64, to: i64) -> Result<MediaPlan> {
    trim_adts_with_options(source, from, to, &Default::default())
}

pub fn trim_adts_with_options(
    source: &std::path::Path,
    from: i64,
    to: i64,
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
    if from < 0 || to <= from {return Err("trim requires 0 <= from < to".into());}
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    let interval = (
        std::time::Duration::from_micros(from as u64),
        std::time::Duration::from_micros(to as u64),
    );
    let input=std::io::BufReader::new(std::fs::File::open(source).map_err(|e|e.to_string())?);
    let info=crate::native_media::inspect_adts_with_interval_and_options(input, Some(interval), options).map_err(|e|e.to_string())?;
    let boundary=|us:i64| (us as u128 * u128::from(info.sample_rate)).div_ceil(1_000_000).min(u128::from(info.sample_frames)) as u64;
    let frames=boundary(to)-boundary(from);
    if frames==0 {return Err("audio interval contains no samples".into());}
    let mut plan=decode_audio_with_options(source,&AudioDecodeTransform {interval:Some((from,to)),..Default::default()},options)?;
    plan.command="trim".into();
    plan.steps.last_mut().ok_or("missing write step")?.detail=format!("publish {frames} float32 PCM sample frames to .wav without overwriting");
    plan.notes.push("decode AAC pre-roll from the beginning; ADTS encoder priming remains part of its timeline".into());
    Ok(plan)
}

/// Require either a single AAC stream or an explicit AAC stream selection.
/// A PCM destination must not silently discard other container tracks.
pub(crate) fn aac_trim_selection(source: &std::path::Path, selected: Option<usize>) -> Result<usize> {
    aac_trim_selection_with_options(source, selected, &fvid_media::CopyOptions::default())
}

pub(crate) fn aac_trim_selection_with_options(
    source: &std::path::Path,
    selected: Option<usize>,
    options: &fvid_media::CopyOptions,
) -> Result<usize> {
    let info=crate::native_probe::probe_with_options(source, options)?;
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
    audio_trim_selection_with_options(source, selected, &fvid_media::CopyOptions::default())
}

pub fn audio_trim_selection_with_options(
    source: &std::path::Path,
    selected: Option<usize>,
    options: &fvid_media::CopyOptions,
) -> Result<usize> {
    audio_trim_selection_with_interval_and_options(source, selected, None, options)
}

pub fn audio_trim_selection_with_interval_and_options(
    source: &std::path::Path,
    selected: Option<usize>,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    options: &fvid_media::CopyOptions,
) -> Result<usize> {
    let info = crate::native_probe::probe_with_options(source, options)?;
    let index = match selected {
        Some(index) => index,
        None if info.streams.len() == 1 => info.streams[0].index,
        None => return Err("audio-to-WAVE trim requires an explicit audio stream when other tracks exist".into()),
    };
    crate::native_media::audio_source_info_selected_with_interval_and_options(source, Some(index), interval, options).map_err(|e| e.to_string())?;
    Ok(index)
}

pub fn trim_audio(source: &std::path::Path, from: i64, to: i64, selected: Option<usize>) -> Result<MediaPlan> {
    let mut options = fvid_media::CopyOptions::default();
    if let Some(index) = selected {
        options.streams = vec![index];
    }
    trim_audio_with_options(source, from, to, &options)
}

pub fn trim_audio_with_options(
    source: &std::path::Path,
    from: i64,
    to: i64,
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
    let selected = options.streams.first().copied();
    let interval = (from >= 0 && to > from).then(|| {
        (
            std::time::Duration::from_micros(from as u64),
            std::time::Duration::from_micros(to as u64),
        )
    });
    if crate::native_media::is_aac_trim_source_with_interval_and_options(source, selected, interval, options).map_err(|e| e.to_string())? {
        return trim_aac_with_options(source, from, to, options);
    }
    let index = audio_trim_selection_with_interval_and_options(source, selected, interval, options)?;
    let mut scoped = options.clone();
    scoped.streams = vec![index];
    let mut plan = decode_audio_with_options(source, &AudioDecodeTransform {
        interval: Some((from, to)), ..Default::default()
    }, &scoped)?;
    plan.command = "trim".into();
    plan.steps.last_mut().ok_or("missing write step")?.detail = "publish selected audio as float32 PCM .wav without overwriting".into();
    Ok(plan)
}

pub fn trim_aac(source: &std::path::Path, from: i64, to: i64, selected: Option<usize>) -> Result<MediaPlan> {
    let mut options = fvid_media::CopyOptions::default();
    if let Some(index) = selected {
        options.streams = vec![index];
    }
    trim_aac_with_options(source, from, to, &options)
}

pub fn trim_aac_with_options(
    source: &std::path::Path,
    from: i64,
    to: i64,
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
    let selected = options.streams.first().copied();
    let index=aac_trim_selection_with_options(source,selected,options)?;
    if crate::native_export::is_adts_source(source).map_err(|e|e.to_string())? {
        return trim_adts_with_options(source,from,to,options);
    }
    let mut scoped = options.clone();
    scoped.streams = vec![index];
    let mut plan=decode_audio_with_options(source,&AudioDecodeTransform {interval:Some((from,to)),..Default::default()},&scoped)?;
    plan.command="trim".into();
    plan.steps.last_mut().ok_or("missing write step")?.detail="publish selected audio as float32 PCM .wav without overwriting".into();
    plan.notes.push("only explicitly selected audio is exported; source presentation edits and decoder pre-roll apply".into());
    Ok(plan)
}

/// Metadata-only plan for decoded, constant-rate video concatenation to Y4M.
pub fn concat_y4m(sources: &[std::path::PathBuf], selected: Option<usize>) -> Result<MediaPlan> {
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    concat_y4m_with_options(sources, &options)
}

pub fn concat_y4m_with_options(
    sources: &[std::path::PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
    if !(2..=256).contains(&sources.len()) {return Err("concat requires 2..=256 inputs".into());}
    crate::native_export::validate_y4m_options(options).map_err(|e| e.to_string())?;
    let selected = options.streams.first().copied();
    for source in sources {
        crate::native_export::validate_video_selection_with_options(source, selected, options).map_err(|e| e.to_string())?;
    }
    let mut remaining_packets = options.max_packets;
    let mut active_sources = 0usize;
    for source in sources {
        crate::native_export::validate_video_packet_budget_with_geometry_and_limit(
            source,
            &Default::default(),
            options,
            remaining_packets,
        )
        .map_err(|e| e.to_string())?;
        active_sources += 1;
        if let Some(rem) = remaining_packets {
            if let Some(seg_frames) = crate::native_export::y4m_segment_frame_upper_bound_with_options(source, selected, options)
                .map_err(|e| e.to_string())?
            {
                let next_rem = rem.saturating_sub(seg_frames);
                remaining_packets = Some(next_rem);
                if next_rem == 0 {
                    break;
                }
            } else {
                remaining_packets = None;
            }
        }
    }
    crate::native_export::validate_y4m_concat_compatibility_with_options(
        &sources[..active_sources.max(1)],
        options,
    )
    .map_err(|e| e.to_string())?;
    if options.max_packets == Some(0) {
        return Err("input has no decoded video frames".into());
    }
    let mut notes = vec!["backend: fvid; output .y4m only, no external codecs".into(),
        "metadata-only plan; exact frame timing and decoded layout are validated during execution".into(),
        "timestamps restart at zero in each segment and become contiguous in output; audio is not exported".into()];
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    Ok(MediaPlan {
        command:"concat".into(),input:sources[0].clone(),inputs:sources.to_vec(),
        streams:vec![PlanStream {index:selected.unwrap_or(0),media_type:"video".into(),codec:"rawvideo".into(),disposition:"decode".into()}],
        steps:vec![PlanStep {action:"decode".into(),detail:"decode each source independently through FVid and append presentation frames".into()},
            PlanStep {action:"write".into(),detail:"require matching frame rate, geometry, bit depth, chroma, range and pixel aspect; publish Y4M without overwriting".into()}],
        graph:None,notes,
    })
}

/// Plan decoded video interval export with an explicit Y4M output contract.
pub fn trim_y4m(source: &std::path::Path, from: i64, to: i64, selected: Option<usize>) -> Result<MediaPlan> {
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    trim_y4m_with_options(source, from, to, &options)
}

pub fn trim_y4m_with_options(
    source: &std::path::Path,
    from: i64,
    to: i64,
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
    if from < 0 || to <= from {return Err("trim requires 0 <= from < to".into());}
    crate::native_export::validate_y4m_options(options).map_err(|e| e.to_string())?;
    let selected = options.streams.first().copied();
    crate::native_export::validate_video_selection_with_options(source, selected, options).map_err(|e| e.to_string())?;
    crate::native_export::validate_video_packet_budget_with_geometry_interval_and_limit(
        source,
        &Default::default(),
        options,
        Some((
            std::time::Duration::from_micros(from as u64),
            std::time::Duration::from_micros(to as u64),
        )),
        true,
        options.max_packets,
    )
    .map_err(|e| e.to_string())?;
    if options.max_packets == Some(0) {
        return Err("input has no decoded video frames".into());
    }
    let mut notes = vec!["backend: fvid; output .y4m only; no audio is exported".into(),
        "metadata-only plan; decoded frames, nonempty interval and exact timing are validated during execution".into()];
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    Ok(MediaPlan {
        command:"trim".into(),input:source.to_owned(),inputs:vec![source.to_owned()],
        streams:vec![PlanStream {index:selected.unwrap_or(0),media_type:"video".into(),codec:"rawvideo".into(),disposition:"decode".into()}],
        steps:vec![PlanStep {action:"decode".into(),detail:"FVid software decode including reference pre-roll".into()},
            PlanStep {action:"trim".into(),detail:format!("retain frame presentation starts in [{from}, {to}) microseconds from the first presented frame")},
            PlanStep {action:"write".into(),detail:"require constant contiguous selected frame timing; publish Y4M without overwriting".into()}],
        graph:None,notes,
    })
}

/// Plan decoded video export with optional interval, geometry, pixel filters, and budgets to Y4M.
pub fn export_y4m_with_options(
    source: &std::path::Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    options: &fvid_media::CopyOptions,
) -> Result<MediaPlan> {
    if let Some((from, to)) = interval
        && from >= to
    {
        return Err("export interval requires both --from and --to with from < to".into());
    }
    crate::native_export::validate_y4m_options(options).map_err(|e| e.to_string())?;
    let selected = options.streams.first().copied();
    crate::native_export::validate_video_selection_with_options(source, selected, options).map_err(|e| e.to_string())?;
    crate::native_export::validate_video_packet_budget_with_geometry_filters_interval_and_limit(
        source,
        geometry,
        filters,
        options,
        interval,
        false,
        options.max_packets,
    )
    .map_err(|e| e.to_string())?;
    if options.max_packets == Some(0) {
        return Err("input has no decoded video frames".into());
    }
    let mut steps = vec![PlanStep {
        action: "decode".into(),
        detail: "FVid software decode including reference pre-roll".into(),
    }];
    if let Some((from, to)) = interval {
        steps.push(PlanStep {
            action: "trim".into(),
            detail: format!(
                "retain frame presentation starts in [{}, {}) microseconds",
                from.as_micros(),
                to.as_micros()
            ),
        });
    }
    if !geometry.is_identity() {
        steps.push(PlanStep {
            action: "geometry".into(),
            detail: format!(
                "crop {:?}; horizontal flip {}; vertical flip {}; transpose {:?}; rotate {:?}; pad {:?}; scale {:?}",
                geometry.crop,
                geometry.horizontal_flip,
                geometry.vertical_flip,
                geometry.transpose,
                geometry.rotate,
                geometry.pad,
                geometry.scale
            ),
        });
    }
    if !filters.is_empty() {
        steps.push(PlanStep {
            action: "filter".into(),
            detail: "owned pixel filters in media option order".into(),
        });
    }
    steps.push(PlanStep {
        action: "write".into(),
        detail: "require constant contiguous frame timing; publish Y4M without overwriting".into(),
    });
    let mut notes = vec![
        "backend: fvid; output .y4m only; no audio is exported".into(),
        "metadata-only plan; decoded frames, nonempty interval and exact timing are validated during execution".into(),
    ];
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    Ok(MediaPlan {
        command: "export-y4m".into(),
        input: source.to_owned(),
        inputs: vec![source.to_owned()],
        streams: vec![PlanStream {
            index: selected.unwrap_or(0),
            media_type: "video".into(),
            codec: "rawvideo".into(),
            disposition: "decode".into(),
        }],
        steps,
        graph: None,
        notes,
    })
}

/// Metadata-only plan for the same admission used by owned lossless export.
pub fn transcode_lossless(source: &std::path::Path, transform: &crate::media_info::LosslessTransform) -> Result<MediaPlan> {
    if crate::native_lossless::supports_overlay(transform) {
        let spec=transform.overlay.as_ref().unwrap();let mut remaining=transform.clone();remaining.overlay=None;
        let (geometry,filters)=crate::native_lossless::configuration(&remaining).map_err(|e|e.to_string())?;
        let spatial=transcode_lossless(source,&remaining)?;
        let mut plan=overlay_with_geometry_and_filters(source,&spec.path,i64::from(spec.x),i64::from(spec.y),&geometry,&filters)?;
        let mut index=1;
        for step in spatial.steps.iter().filter(|s|s.action=="geometry") {plan.steps.insert(index,step.clone());index+=1;}
        index+=1;
        for step in spatial.steps.iter().filter(|s|s.action=="filter") {plan.steps.insert(index,step.clone());index+=1;}
        plan.command="transcode-lossless".into();return Ok(plan);
    }
    if (!crate::native_lossless::supports(transform) && !crate::native_lossless::supports_framestep(transform))
        || !crate::native_lossless::eligible(source).map_err(|e|e.to_string())? {
        return Err("request is not supported by the owned lossless planner".into());
    }
    let mut spatial_transform=transform.clone();
    spatial_transform.framestep=None;
    let (geometry,filters)=crate::native_lossless::configuration(&spatial_transform).map_err(|e|e.to_string())?;
    if crate::native_lossless_y4m::eligible(source).map_err(|e|e.to_string())? {
        crate::native_lossless_y4m::validate_first_frame(source, &geometry, &filters, false)
            .map_err(|e| e.to_string())?;
    } else {
        crate::native_lossless::validate_first_mp4_frame(source, &geometry, &filters, false)
            .map_err(|e| e.to_string())?;
    }
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
    if !geometry.is_identity() {steps.push(PlanStep{action:"geometry".into(),detail:format!("crop {:?}; horizontal flip {}; vertical flip {}; transpose {:?}; rotate {:?}; pad {:?}; scale {:?}; normalize stored rotation first",geometry.crop,geometry.horizontal_flip,geometry.vertical_flip,geometry.transpose,geometry.rotate,geometry.pad,geometry.scale)});}
    if !filters.is_empty() {
        for (name,args) in [("tmix",&transform.tmix),("hqdn3d",&transform.hqdn3d),("eq",&transform.eq),("unsharp",&transform.unsharp),("hue",&transform.hue),
            ("gblur",&transform.gblur),("avgblur",&transform.avgblur),("boxblur",&transform.boxblur),
            ("negate",&transform.negate),("sobel",&transform.sobel),("prewitt",&transform.prewitt),
            ("roberts",&transform.roberts),("kirsch",&transform.kirsch),("scharr",&transform.scharr),
            ("smartblur",&transform.smartblur),("sab",&transform.sab),("bilateral",&transform.bilateral),
            ("cas",&transform.cas),("vignette",&transform.vignette),("curves",&transform.curves),
            ("colorbalance",&transform.colorbalance),("colorlevels",&transform.colorlevels),("colorchannelmixer",&transform.colorchannelmixer),
            ("monochrome",&transform.monochrome),("grayworld",&transform.grayworld),("drawbox",&transform.drawbox),("drawgrid",&transform.drawgrid),
            ("lagfun",&transform.lagfun),("bitplanenoise",&transform.bitplanenoise),("deband",&transform.deband),("gradfun",&transform.gradfun),
            ("lenscorrection",&transform.lenscorrection),("pixelize",&transform.pixelize),("removegrain",&transform.removegrain),
            ("yaepblur",&transform.yaepblur),("vibrance",&transform.vibrance),("dilation",&transform.dilation),("erosion",&transform.erosion),
            ("colorize",&transform.colorize),("exposure",&transform.exposure),("chromashift",&transform.chromashift),
            ("colorcontrast",&transform.colorcontrast),("colorcorrect",&transform.colorcorrect),("shuffleplanes",&transform.shuffleplanes),
            ("lutyuv",&transform.lutyuv),("colorhold",&transform.colorhold),("fade",&transform.fade),("perspective",&transform.perspective)] {
            if let Some(args)=args {steps.push(PlanStep{action:"filter".into(),detail:format!("FVid {name}={args}; operate on sample planes at source precision")});}
        }
    }
    if let Some(args)=&transform.framestep {
        steps.push(PlanStep{action:"filter".into(),detail:format!("FVid framestep={args}; decode every reference frame and emit selected presentation frames")});
    }
    steps.push(PlanStep{action:"encode".into(),detail:"FVid lossless FFV1 v1 range encoder; retain video timing and display metadata".into()});
    steps.push(PlanStep{action:"write".into(),detail:"FVid Matroska muxer; atomic publication without overwrite; cancellation removes temporary output".into()});
    Ok(MediaPlan {command:"transcode-lossless".into(),input:source.to_path_buf(),inputs:vec![source.to_path_buf()],streams,steps,graph:None,
        notes:vec!["backend: fvid; no external demuxer, decoder, encoder or muxer".into(),"metadata-only plan: frame geometry, filter plane compatibility and payload correctness are validated during execution".into()]})
}

/// Plan owned loudness measurement without decoding or claiming true peak.
pub fn loudness(source: &std::path::Path, selected: Option<usize>, weights: Option<&[f64]>) -> Result<MediaPlan> {
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    loudness_with_options(source, &options, weights)
}

pub fn loudness_with_options(
    source: &std::path::Path,
    options: &fvid_media::CopyOptions,
    weights: Option<&[f64]>,
) -> Result<MediaPlan> {
    if options.streams.len() > 1
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err("owned loudness plan requires at most one audio stream and no metadata edits".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("loudness requires at least one packet".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    let selected = options.streams.first().copied();
    let mut plan = if options.max_controlled_bytes.is_none()
        && (crate::native_pcm::is_wave(source).map_err(|e| e.to_string())?
            || crate::native_media::audio_source_info_selected_with_options(source, selected, options).is_ok())
    {
        decode_audio_with_options(source, &Default::default(), options)?
    } else {
        fvid_media::plan_decode_audio(source, &Default::default(), options)?
    };
    let defaults;
    let weights=match weights {
        Some(weights)=>weights,
        None=>{defaults=crate::native_pcm::loudness_channel_weights_with_options(source,options).map_err(|e|e.to_string())?;&defaults},
    };
    let (rate,channels)=if crate::native_pcm::is_wave(source).map_err(|e|e.to_string())? {
        let info=crate::native_pcm::inspect(&mut std::fs::File::open(source).map_err(|e|e.to_string())?,options.cancel.as_ref()).map_err(|e|e.to_string())?;
        let frame = usize::from(info.frame_bytes());
        let capacity = 65536usize.min(options.max_packet_bytes) / frame * frame;
        if capacity == 0 {
            return Err("packet byte limit is smaller than one PCM sample frame".into());
        }
        (info.sample_rate,info.channels)
    } else if options.max_controlled_bytes.is_none()
        && let Ok(info) = crate::native_media::audio_source_info_selected_with_options(source, selected, options)
    {
        if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes {
            validate_encoded_audio_packet_budget(source, info.stream_index, None, options)?;
        }
        (info.sample_rate, info.channels)
    } else {
        fvid_media::owned_audio_plan::source_geometry(source, options)?
    };
    if weights.len()!=usize::from(channels) {return Err("channel weights must match selected audio stream".into());}
    crate::native_pcm::LoudnessMeter::new(rate,weights)?;
    plan.command="loudness".into();
    plan.steps.retain(|step|step.action!="write");
    plan.steps.push(PlanStep {action:"analyze".into(),detail:format!("owned K-weighting, gated integrated LUFS, 3-second LRA and unweighted sample peak; channel energy weights {weights:?}")});
    plan.notes.push("Silence and incomplete windows return null measurements; true peak is not measured".into());
    if (options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some())
        && !plan
            .notes
            .iter()
            .any(|n| n.starts_with("encoded packet limit:"))
    {
        plan.notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    Ok(plan)
}

/// Metadata-only two-pass constant-gain normalization plan.
pub fn normalize_loudness(source: &std::path::Path, selected: Option<usize>, weights: Option<&[f64]>, target: crate::native_pcm::NormalizeTarget) -> Result<MediaPlan> {
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    normalize_loudness_with_options(source, &options, weights, target)
}

pub fn normalize_loudness_with_options(
    source: &std::path::Path,
    options: &fvid_media::CopyOptions,
    weights: Option<&[f64]>,
    target: crate::native_pcm::NormalizeTarget,
) -> Result<MediaPlan> {
    target.validate()?;
    let has_meta = !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty();
    let wave_meta_only = options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
        && fvid_media::owned_wave_metadata::edit_info_chunks(
            &[],
            &options.metadata_delete,
            &options.metadata_set,
        )
        .is_ok();
    if has_meta && !wave_meta_only {
        crate::native_export::pcm_matroska::validate_matroska_pcm_metadata(Some(options))
            .map_err(|e| e.to_string())?;
    }
    let mut loudness_options = options.clone();
    loudness_options.metadata_set.clear();
    loudness_options.metadata_delete.clear();
    loudness_options.stream_metadata_set.clear();
    loudness_options.stream_metadata_delete.clear();
    let mut plan=loudness_with_options(source,&loudness_options,weights)?;
    if has_meta && options.max_controlled_bytes.is_some() {
        if let Ok((_, channels)) =
            fvid_media::owned_audio_plan::source_geometry(source, &loudness_options)
        {
            crate::native_export::pcm_matroska::check_output_memory_budget(
                source,
                channels,
                65536,
                options,
                if wave_meta_only { None } else { Some(true) },
            )
            .map_err(|e| e.to_string())?;
        }
    }
    plan.command="normalize-loudness".into();
    plan.steps.push(PlanStep {action:"gain".into(),detail:format!("constant gain to {} LUFS, limited by sample-peak ceiling {} dBFS; reject gain above 64",target.integrated_lufs,target.sample_peak_dbfs)});
    plan.steps.push(PlanStep {action:"decode".into(),detail:"second owned decode pass, preserving rate, channels and dynamics".into()});
    if has_meta {
        let detail = if wave_meta_only {
            format!(
                "WAVE INFO or Matroska container edits: set {}, delete {}",
                options.metadata_set.len(),
                options.metadata_delete.len()
            )
        } else {
            "apply Matroska container and stream 0 metadata mutations".into()
        };
        plan.steps.push(PlanStep {
            action: "metadata".into(),
            detail,
        });
    }
    plan.steps.push(PlanStep {action:"write".into(),detail:"atomic float WAVE or Matroska PCM publication; reject existing destination".into()});
    plan.notes.push("Gain is computed during execution; silent/short input fails; peak limiting may prevent reaching the LUFS target; no true-peak ceiling".into());
    Ok(plan)
}


/// Validate compressed concat compatibility and describe the owned Matroska route.
/// Plan Matroska concat for a concrete destination, including audio-only admission.
pub fn concat_matroska_to(sources: &[std::path::PathBuf], destination: &std::path::Path) -> Result<Option<MediaPlan>> {
    concat_matroska_to_with_options(sources, destination, &Default::default())
}

pub fn concat_matroska_to_with_options(
    sources: &[std::path::PathBuf],
    destination: &std::path::Path,
    options: &fvid_media::CopyOptions,
) -> Result<Option<MediaPlan>> {
    let (audio_only, wav_only) = match destination.extension().and_then(|s| s.to_str()) {
        Some("mka") => (true, false),
        Some("mkv") => (false, false),
        Some("wav") => (true, true),
        _ => return Ok(None),
    };
    if wav_only {
        crate::native_audio_mix::validate_output_controls(options, Some(false))
            .map_err(|e| e.to_string())?;
    }
    let Some(plan) = concat_mp4_matroska_with_options(sources, options)? else {
        return Ok(None);
    };
    if wav_only && plan.streams.iter().any(|s| s.disposition != "decode_to_pcm") {
        return Err(".wav concat output requires sample-domain audio or compatible PCM WAVE inputs".into());
    }
    if audio_only && plan.streams.iter().any(|s| s.media_type != "audio") {
        return Err(".mka concat output requires audio-only input".into());
    }
    Ok(Some(plan))
}

pub fn concat_mp4_matroska(sources: &[std::path::PathBuf]) -> Result<Option<MediaPlan>> {
    concat_mp4_matroska_with_options(sources, &Default::default())
}

pub fn concat_mp4_matroska_with_options(
    sources: &[std::path::PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<Option<MediaPlan>> {
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    if crate::container::mp4_concat::open_with_options(sources, options)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        let mut default_opts = fvid_media::CopyOptions::default();
        default_opts.cancel = options.cancel.clone();
        if !crate::native_audio_mix::concat_eligible_with_options(sources, &default_opts)
            .map_err(|e| e.to_string())?
        {
            return Ok(None);
        }
        crate::native_audio_mix::validate_output_controls(options, None)
            .map_err(|e| e.to_string())?;
        crate::native_audio_mix::validate_concat_segments_with_options(sources, options)
            .map_err(|e| e.to_string())?;
        let mut notes = vec![
            "backend: fvid; matching sample rates and channel counts required".into(),
            "decoded PCM output; tags, chapters and compressed packets are not retained; one segment of temporary disk space required".into(),
        ];
        if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
            || options.max_packets.is_some()
            || options.max_controlled_bytes.is_some()
            || options.max_rss_bytes.is_some()
        {
            notes.push(format!(
                "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
                options.max_packet_bytes, options.max_packets, options.max_controlled_bytes, options.max_rss_bytes
            ));
        }
        let mut steps = vec![
            PlanStep {action:"decode".into(),detail:"decode each single audio source independently through owned codecs, retaining its audible samples".into()},
            PlanStep {action:"concat".into(),detail:"append float32 samples without resampling or channel remapping".into()},
        ];
        if !options.metadata_set.is_empty()
            || !options.metadata_delete.is_empty()
            || !options.stream_metadata_set.is_empty()
            || !options.stream_metadata_delete.is_empty()
        {
            steps.push(PlanStep {
                action: "metadata".into(),
                detail: "apply container and stream metadata mutations".into(),
            });
        }
        steps.push(PlanStep {action:"write".into(),detail:"owned float32 PCM .wav or Matroska .mka/.mkv, atomic publication without overwrite".into()});
        return Ok(Some(MediaPlan {command:"concat".into(),input:sources[0].clone(),inputs:sources.to_vec(),
            streams:crate::native_probe::probe_with_options(&sources[0], options)?.streams.into_iter().map(|s|PlanStream {
                index:s.index,media_type:s.media_type,codec:s.codec,disposition:"decode_to_pcm".into(),
            }).collect(),graph:None,
            steps,
            notes}));
    }
    let info=crate::native_probe::probe_with_options(&sources[0], options)?;
    let selected = crate::container::mp4_matroska::selection(info.streams.len(), &options.streams)
        .map_err(|e| e.to_string())?;
    if options.metadata_set.len() + options.metadata_delete.len() > 64 {
        return Err("too many metadata mutations".into());
    }
    if options.stream_metadata_set.len() + options.stream_metadata_delete.len() > 64 {
        return Err("too many stream metadata mutations".into());
    }
    for (key, value) in &options.metadata_set {
        let key = key.trim();
        if key.is_empty()
            || key.len() > 128
            || value.len() > 1024
            || key.contains('\0')
            || value.contains('\0')
        {
            return Err("invalid metadata tag".into());
        }
    }
    for key in &options.metadata_delete {
        let key = key.trim();
        if key.is_empty() || key.len() > 128 || key.contains('\0') {
            return Err("invalid metadata key".into());
        }
    }
    for (index, key, value) in &options.stream_metadata_set {
        if !selected.contains(index) {
            return Err("stream metadata index out of range".into());
        }
        let key = key.trim();
        if key.is_empty()
            || key.len() > 128
            || value.len() > 1024
            || key.contains('\0')
            || value.contains('\0')
            || matches!(
                key.to_ascii_lowercase().as_str(),
                "rotate" | "stereo_mode" | "alpha_mode"
            )
            || (key.eq_ignore_ascii_case("language") && value.len() > 128)
        {
            return Err("invalid stream metadata tag".into());
        }
    }
    for (index, key) in &options.stream_metadata_delete {
        if !selected.contains(index) {
            return Err("stream metadata index out of range".into());
        }
        let key = key.trim();
        if key.is_empty()
            || key.len() > 128
            || key.contains('\0')
            || matches!(
                key.to_ascii_lowercase().as_str(),
                "rotate" | "stereo_mode" | "alpha_mode"
            )
        {
            return Err("invalid stream metadata key".into());
        }
    }
    let mut notes = vec![
        "backend: fvid; output .mkv or audio-only .mka".into(),
        "AAC priming is retained through global CodecDelay and per-segment signed DiscardPadding".into(),
    ];
    if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    let streams: Vec<_> = selected
        .iter()
        .map(|&idx| {
            let s = &info.streams[idx];
            PlanStream {
                index: s.index,
                media_type: s.media_type.clone(),
                codec: s.codec.clone(),
                disposition: "copy".into(),
            }
        })
        .collect();
    let mut steps = vec![
        PlanStep {action:"copy".into(),detail:"FVid AVC/HEVC/AAC packet copy; preserve decode order and shift each segment by its presented duration".into()},
        PlanStep {action:"mux".into(),detail:"FVid Matroska muxer; retain first-input track/file metadata and offset all input chapters".into()},
    ];
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        steps.push(PlanStep {
            action: "metadata".into(),
            detail: "apply container and stream metadata mutations".into(),
        });
    }
    steps.push(PlanStep {action:"publish".into(),detail:"Atomic publication without overwriting; completion only after publication".into()});
    Ok(Some(MediaPlan {command:"concat".into(),input:sources[0].clone(),inputs:sources.to_vec(),
        streams,
        steps,graph:None,
        notes}))
}

/// Describe owned opaque overlay while retaining supported main audio tracks.
pub fn overlay(source:&std::path::Path,foreground:&std::path::Path,x:i64,y:i64)->Result<MediaPlan> {
    overlay_with_geometry(source,foreground,x,y,&crate::native_geometry::VideoGeometry::default())
}

pub fn overlay_with_geometry(source:&std::path::Path,foreground:&std::path::Path,x:i64,y:i64,geometry:&crate::native_geometry::VideoGeometry)->Result<MediaPlan> {
    overlay_with_geometry_and_filters(source,foreground,x,y,geometry,&Default::default())
}

pub fn overlay_with_geometry_and_filters(
    source: &std::path::Path,
    foreground: &std::path::Path,
    x: i64,
    y: i64,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
) -> Result<MediaPlan> {
    if !crate::native_export::overlay_eligible(source).map_err(|e|e.to_string())? {return Err("owned overlay requires a supported owned lossless main input".into());}
    let info=crate::native_probe::probe(source)?;let second=crate::native_probe::probe(foreground)?;
    if second.streams.iter().filter(|s|s.media_type=="video").count()!=1 {return Err("foreground requires exactly one video track".into());}
    crate::native_export::TimedOverlay::validate_pair_with_geometry_and_filters(source,foreground,x,y,geometry,filters).map_err(|e|e.to_string())?;
    Ok(MediaPlan {command:"overlay".into(),input:source.to_owned(),inputs:vec![source.to_owned(),foreground.to_owned()],
        streams:info.streams.into_iter().map(|s|PlanStream {index:s.index,media_type:s.media_type.clone(),codec:s.codec,
            disposition:if s.media_type=="video" {"decode_overlay"}else{"copy"}.into()}).collect(),graph:None,
        steps:vec![PlanStep {action:"decode".into(),detail:"owned main and foreground software decoders; normalize stored display transforms".into()},
            PlanStep {action:"overlay".into(),detail:format!("opaque sample-plane overlay at {x},{y}; align first presentation origins; retain latest foreground at each main PTS; hold foreground EOF")},
            PlanStep {action:"encode".into(),detail:"owned lossless FFV1 at main sample depth; preserve main timing and supported AAC/Opus packets".into()},
            PlanStep {action:"write".into(),detail:"owned Matroska .mkv; atomic no-overwrite publication".into()}],
        notes:vec!["backend: fvid; no external decoder, encoder or muxer".into(),"sample depth converted to main precision; matching colour encoding/range and sampling required; chroma placement must align; validated while decoding".into(),"foreground audio is not part of the overlay; main supported companion tracks are retained".into()]})
}

/// Read-only plan for owned geometry, timed overlay and pixel-filter decoding.
/// Frame-dependent compatibility is validated by execution, not by probe alone.
pub fn decode_overlay(source: &std::path::Path, foreground: &std::path::Path, x: i64, y: i64) -> Result<MediaPlan> {
    decode_overlay_with_geometry(source, foreground, x, y, &crate::native_geometry::VideoGeometry::default())
}

pub fn decode_overlay_with_geometry(source: &std::path::Path, foreground: &std::path::Path, x: i64, y: i64, geometry: &crate::native_geometry::VideoGeometry) -> Result<MediaPlan> {
    decode_overlay_with_geometry_and_filters(source, foreground, x, y, geometry, &Default::default())
}

pub fn decode_overlay_with_geometry_and_filters(
    source: &std::path::Path,
    foreground: &std::path::Path,
    x: i64,
    y: i64,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
) -> Result<MediaPlan> {
    let info = crate::native_probe::probe(source)?;
    let second = crate::native_probe::probe(foreground)?;
    if !info.streams.iter().any(|s| s.media_type == "video")
        || second.streams.iter().filter(|s| s.media_type == "video").count() != 1 {
        return Err("decode overlay requires main video and exactly one foreground video track".into());
    }
    crate::native_export::TimedOverlay::validate_pair_with_geometry_and_filters(source, foreground, x, y, geometry, filters).map_err(|e| e.to_string())?;
    Ok(MediaPlan {
        command: "decode".into(), input: source.to_owned(), inputs: vec![source.to_owned(), foreground.to_owned()],
        streams: info.streams.into_iter().map(|s| PlanStream { index: s.index, media_type: s.media_type.clone(), codec: s.codec,
            disposition: if s.media_type == "video" { "decode_overlay" } else { "discard" }.into() }).collect(),
        steps: vec![
            PlanStep { action: "decode".into(), detail: "owned main and foreground software decoders; materialize stored display transforms".into() },
            PlanStep { action: "overlay".into(), detail: format!("opaque overlay at {x},{y}; align first presentation origins before interval selection; hold latest foreground and repeat at EOF") },
            PlanStep { action: "discard".into(), detail: "discard processed video samples; report main frame count and output geometry".into() },
        ], graph: None,
        notes: vec!["backend: fvid; no external decoder, filter or muxer".into(), "depth converted to main precision; colour/range, sampling, display aspect and HDR compatibility validated during decoding".into()],
    })
}
