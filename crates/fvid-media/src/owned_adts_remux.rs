//! Owned ADTS to Matroska remux; compressed AAC packets remain unchanged.
use fvid_control::CopyOptions;
use fvid_media_info::CopyStats;
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
pub(crate) fn supports(source: &Path, destination: &Path, options: &CopyOptions) -> bool {
    if !crate::owned_matroska_remux::policies(options)
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
    if !supports(source, destination, options) {
        return Err("request is not an owned ADTS Matroska remux".into());
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
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    crate::owned_matroska_remux::publish(destination, options, |output| {
        crate::owned_matroska::write_adts(
            reader,
            output,
            options.cancel.as_ref(),
            options.progress.as_ref(),
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
    if !supports(source, Path::new("planned.mka"), options) {
        return Err("request is not an owned ADTS Matroska remux".into());
    }
    let check = || {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            Err("media operation cancelled".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let file = File::open(source).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("ADTS remux requires a regular file".into());
    }
    let mut reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
        BufReader::new(file),
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let config = reader.configuration();
    let mut packets = 0u64;
    let mut bytes = 0u64;
    loop {
        check()?;
        let Some(packet) = reader.next_packet().map_err(|e| e.to_string())? else {
            break;
        };
        packets = packets.checked_add(1).ok_or("ADTS packet count overflow")?;
        bytes = bytes
            .checked_add(packet.len() as u64)
            .ok_or("ADTS payload count overflow")?;
    }
    check()?;
    Ok(MediaPlan {
        command: "remux".into(), input: source.into(), inputs: vec![source.into()],
        streams: vec![PlanStream { index: 0, media_type: "audio".into(), codec: "aac".into(), disposition: "copy".into() }],
        steps: vec![
            PlanStep { action: "copy".into(), detail: format!("copy {packets} unchanged AAC packets, {bytes} payload bytes; {} Hz, {} channels", config.sample_rate, config.channels) },
            PlanStep { action: "mux".into(), detail: "write Matroska AAC track with AudioSpecificConfig and nanosecond packet timestamps".into() },
            PlanStep { action: "publish".into(), detail: "publish completed output without overwriting; remove temporary file on error or cancellation".into() },
        ], graph: None,
        notes: vec!["backend: owned ADTS/Matroska; no external demuxer or muxer".into(), "output must be .mkv or .mka; destination and publication checked during execution".into(), "unedited all-track request; packet byte limit counts AAC payload, excluding ADTS headers".into()],
    })
}
