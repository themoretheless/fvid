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
    if !supports(sources, destination, options) {
        return crate::owned_wave_remux::concat(sources, destination, options);
    }
    let readers = open_readers(sources, options)?;
    let mut stats = crate::owned_matroska_remux::publish(destination, options, |output| {
        crate::owned_matroska::concat_adts(
            readers,
            output,
            options.cancel.as_ref(),
            options.progress.as_ref(),
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
    let mut readers = Vec::with_capacity(sources.len());
    for source in sources {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        let file = File::open(source).map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("ADTS concat requires regular files".into());
        }
        readers.push(
            crate::owned_aac::adts::StreamReader::open_with_packet_limit(
                BufReader::new(file),
                options.max_packet_bytes,
            )
            .map_err(|e| e.to_string())?,
        );
    }
    Ok(readers)
}

/// Validate the complete compatible sequence without output or execution progress.
pub fn plan_concat(
    sources: &[PathBuf],
    options: &CopyOptions,
) -> Result<fvid_media_info::MediaPlan, String> {
    use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
    if !supports(sources, Path::new("planned.mka"), options) {
        return crate::owned_wave_plan::plan_concat(sources, options);
    }
    let mut sequence = crate::owned_aac::adts::SequenceReader::new(open_readers(sources, options)?)
        .map_err(|e| e.to_string())?;
    let config = sequence.configuration();
    let mut packets = 0u64;
    let mut bytes = 0u64;
    loop {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        let Some(packet) = sequence.next_packet().map_err(|e| e.to_string())? else {
            break;
        };
        packets = packets.checked_add(1).ok_or("ADTS packet count overflow")?;
        bytes = bytes
            .checked_add(packet.len() as u64)
            .ok_or("ADTS payload count overflow")?;
    }
    Ok(MediaPlan {
        command: "concat".into(), input: sources[0].clone(), inputs: sources.to_vec(),
        streams: vec![PlanStream { index: 0, media_type: "audio".into(), codec: "aac".into(), disposition: "copy".into() }],
        steps: vec![
            PlanStep { action: "concat".into(), detail: format!("copy {packets} unchanged AAC packets, {bytes} payload bytes across {} segments; {} Hz, {} channels", sources.len(), config.sample_rate, config.channels) },
            PlanStep { action: "mux".into(), detail: "write one Matroska AAC track with continuous nanosecond packet clock".into() },
            PlanStep { action: "publish".into(), detail: "publish completed output without overwriting; remove temporary output on failure or cancellation".into() },
        ], graph: None,
        notes: vec!["backend: owned ADTS/Matroska; no external demuxer or muxer".into(), "output must be .mkv or .mka; destination/publication checked during execution".into(), "identical AAC configurations required; packet byte limit excludes ADTS headers".into()],
    })
}
