//! Owned ADTS to Matroska remux; compressed AAC packets remain unchanged.
use fvid_control::CopyOptions;
use fvid_media_info::CopyStats;
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
pub(crate) fn adts_matroska_policies(options: &CopyOptions) -> bool {
    options.max_controlled_bytes != Some(0)
        && crate::owned_budget::check_rss_budget(options).is_ok()
        && options.max_packet_bytes > 0
        && crate::owned_matroska::extract_adts_matroska_metadata(Some(options)).is_ok()
}
pub(crate) fn supports(source: &Path, destination: &Path, options: &CopyOptions) -> bool {
    if !(options.streams.is_empty() || options.streams == [0])
        || !adts_matroska_policies(options)
        || !matches!(
            destination.extension().and_then(|s| s.to_str()),
            Some("mkv" | "mka")
        )
    {
        return false;
    }
    let mut prefix = [0; 7];
    File::open(source).is_ok_and(|mut f| {
        f.read_exact(&mut prefix).is_ok() && crate::owned_aac::adts::header(&prefix).is_some()
    })
}
pub fn remux(
    source: &Path,
    destination: &Path,
    options: &CopyOptions,
) -> Result<CopyStats, String> {
    crate::owned_budget::check_rss_budget(options)?;
    if !supports(source, destination, options) {
        return Err("request is not an owned ADTS Matroska remux".into());
    }
    if options.max_packets == Some(0) {
        return Err("Matroska AAC output requires at least one packet".into());
    }
    let meta_bytes = crate::owned_matroska::estimate_adts_matroska_metadata_bytes(Some(options))
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
    let file = File::open(source).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("ADTS remux requires a regular file".into());
    }
    let reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
        BufReader::new(file),
        options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
    )
    .map_err(|e| e.to_string())?;
    crate::owned_matroska_remux::publish(destination, options, |output| {
        crate::owned_matroska::write_adts_with_options(
            reader,
            output,
            options.cancel.as_ref(),
            options.progress.as_ref(),
            options.max_packets,
            Some(options),
        )
        .map_err(|e| e.to_string())
    })
}
/// Validate packet framing and describe owned remux without output or progress.
pub fn plan_remux(
    source: &Path,
    options: &CopyOptions,
) -> Result<fvid_media_info::MediaPlan, String> {
    use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
    crate::owned_budget::check_rss_budget(options)?;
    if !supports(source, Path::new("planned.mka"), options) {
        return Err("request is not an owned ADTS Matroska remux".into());
    }
    if options.max_packets == Some(0) {
        return Err("Matroska AAC output requires at least one packet".into());
    }
    let meta_bytes = crate::owned_matroska::estimate_adts_matroska_metadata_bytes(Some(options))
        .map_err(|e| e.to_string())?;
    if options
        .max_controlled_bytes
        .is_some_and(|limit| 16384usize.saturating_add(meta_bytes) > limit)
    {
        return Err("controlled memory budget exceeded".into());
    }
    let check = || {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            Err("media operation cancelled".to_string())
        } else {
            crate::owned_budget::check_rss_budget(options)
        }
    };
    check()?;
    let file = File::open(source).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("ADTS remux requires a regular file".into());
    }
    let mut reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
        BufReader::new(file),
        options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
    )
    .map_err(|e| e.to_string())?;
    let config = reader.configuration();
    let asc = reader.audio_specific_config().to_vec();
    let mut negotiated_rate = None;
    let mut packets = 0u64;
    let mut bytes = 0u64;
    loop {
        check()?;
        if options.max_packets.is_some_and(|limit| packets >= limit) {
            break;
        }
        let Some(packet) = reader.next_packet().map_err(|e| e.to_string())? else {
            break;
        };
        if negotiated_rate.is_none() {
            negotiated_rate = crate::owned_aac::adts::probe_output_rate(&packet, &asc)
                .map_err(|e| e.to_string())?;
        }
        packets = packets.checked_add(1).ok_or("ADTS packet count overflow")?;
        bytes = bytes
            .checked_add(packet.len() as u64)
            .ok_or("ADTS payload count overflow")?;
    }
    check()?;
    let output_rate = negotiated_rate.unwrap_or(config.sample_rate);
    let mut steps = vec![
        PlanStep {
            action: "copy".into(),
            detail: format!(
                "copy {packets} unchanged AAC packets, {bytes} payload bytes; {} Hz, {} channels",
                output_rate, config.channels
            ),
        },
        PlanStep {
            action: "mux".into(),
            detail: "write Matroska AAC track with AudioSpecificConfig and nanosecond packet timestamps".into(),
        },
    ];
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
        detail: "publish completed output without overwriting; remove temporary file on error or cancellation".into(),
    });
    let mut notes = vec![
        "backend: owned ADTS/Matroska; no external demuxer or muxer".into(),
        "output must be .mkv or .mka; destination and publication checked during execution".into(),
        "unedited all-track request; packet byte limit counts AAC payload, excluding ADTS headers".into(),
    ];
    if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {} packets; controlled memory limit: {} bytes; RSS limit: {} bytes",
            options.max_packet_bytes,
            options
                .max_packets
                .map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
            options
                .max_controlled_bytes
                .map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
            options
                .max_rss_bytes
                .map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
        ));
    }
    Ok(MediaPlan {
        command: "remux".into(),
        input: source.into(),
        inputs: vec![source.into()],
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
