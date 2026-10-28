//! Owned file concat dispatch: PCM WAVE or compatible ADTS into Matroska.
use fvid_control::CopyOptions;
use fvid_media_info::CopyStats;
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};
pub(crate) fn supports(sources: &[PathBuf], destination: &Path, options: &CopyOptions) -> bool {
    (2..=256).contains(&sources.len())
        && sources
            .iter()
            .all(|source| crate::owned_adts_remux::supports(source, destination, options))
}
pub fn concat(
    sources: &[PathBuf],
    destination: &Path,
    options: &CopyOptions,
) -> Result<CopyStats, String> {
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if !supports(sources, destination, options) {
        if sources
            .first()
            .is_some_and(|p| !crate::owned_wave_inspect::is_wave(p).unwrap_or(false))
        {
            return Err("request is not an owned ADTS Matroska concat".into());
        }
        return crate::owned_wave_remux::concat(sources, destination, options);
    }
    let readers = open_readers(sources, options)?;
    let mut stats = crate::owned_matroska_remux::publish(destination, options, |output| {
        crate::owned_matroska::concat_adts_with_options(
            readers,
            output,
            options.cancel.as_ref(),
            options.progress.as_ref(),
            options.max_packets,
            Some(options),
        )
        .map_err(|e| e.to_string())
    })?;
    stats.segments = sources.len();
    Ok(stats)
}

fn open_readers(
    sources: &[PathBuf],
    options: &CopyOptions,
) -> Result<Vec<crate::owned_aac::adts::StreamReader<BufReader<File>>>, String> {
    let meta_bytes = crate::owned_matroska::estimate_adts_matroska_metadata_bytes(Some(options))
        .map_err(|e| e.to_string())?;
    if options.max_controlled_bytes.is_some_and(|limit| {
        sources
            .len()
            .saturating_mul(8192)
            .saturating_add(16384)
            .saturating_add(meta_bytes)
            > limit
    }) {
        return Err("controlled memory budget exceeded".into());
    }
    let mut readers = Vec::with_capacity(sources.len());
    for source in sources {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        let file = File::open(source).map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("ADTS concat requires regular files".into());
        }
        let reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
            BufReader::new(file),
            options
                .max_packet_bytes
                .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
        )
        .map_err(|e| e.to_string())?;
        readers.push(reader);
    }
    Ok(readers)
}

/// Validate the selected compatible packet prefix without output or execution progress.
pub fn plan_concat(
    sources: &[PathBuf],
    options: &CopyOptions,
) -> Result<fvid_media_info::MediaPlan, String> {
    use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if !supports(sources, Path::new("planned.mka"), options) {
        if sources
            .first()
            .is_some_and(|p| !crate::owned_wave_inspect::is_wave(p).unwrap_or(false))
        {
            return Err("request is not an owned ADTS Matroska concat".into());
        }
        return crate::owned_wave_plan::plan_concat(sources, options);
    }
    let meta_bytes = crate::owned_matroska::estimate_adts_matroska_metadata_bytes(Some(options))
        .map_err(|e| e.to_string())?;
    let base_bytes = sources.len().saturating_mul(8192).saturating_add(16384);
    let mut sequence = crate::owned_aac::adts::SequenceReader::new(open_readers(sources, options)?)
        .map_err(|e| e.to_string())?;
    let config = sequence.configuration();
    let asc = sequence.audio_specific_config().to_vec();
    let mut negotiated_rate = None;
    let mut packets = 0u64;
    let mut bytes = 0u64;
    loop {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        crate::owned_budget::check_rss_budget(options)?;
        if options.max_packets.is_some_and(|limit| packets >= limit) {
            break;
        }
        let Some(packet) = sequence.next_packet().map_err(|e| e.to_string())? else {
            break;
        };
        if negotiated_rate.is_none() {
            negotiated_rate = crate::owned_aac::adts::probe_output_rate(&packet, &asc)
                .map_err(|e| e.to_string())?;
        }
        packets = packets.checked_add(1).ok_or("ADTS packet count overflow")?;
        if options.max_controlled_bytes.is_some_and(|limit| {
            base_bytes
                .saturating_add((packets as usize).saturating_mul(32))
                .saturating_add(meta_bytes)
                > limit
        }) {
            return Err("controlled memory budget exceeded".into());
        }
        bytes = bytes
            .checked_add(packet.len() as u64)
            .ok_or("ADTS payload count overflow")?;
    }
    let output_rate = negotiated_rate.unwrap_or(config.sample_rate);
    let mut steps = vec![
        PlanStep {
            action: "concat".into(),
            detail: format!(
                "copy {packets} unchanged AAC packets, {bytes} payload bytes across {} segments; {} Hz, {} channels",
                sources.len(),
                output_rate,
                config.channels
            ),
        },
        PlanStep {
            action: "mux".into(),
            detail: "write one Matroska AAC track with continuous nanosecond packet clock".into(),
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
        detail: "publish completed output without overwriting; remove temporary output on failure or cancellation".into(),
    });
    let mut notes = vec![
        "backend: owned ADTS/Matroska; no external demuxer or muxer".into(),
        "output must be .mkv or .mka; destination/publication checked during execution".into(),
        "identical AAC configurations required; packet byte limit excludes ADTS headers".into(),
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
        command: "concat".into(),
        input: sources[0].clone(),
        inputs: sources.to_vec(),
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
