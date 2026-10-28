//! Metadata-only planning for owned container remux.
use fvid_control::CopyOptions;
use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
use std::{fs::File, path::Path};
pub(crate) fn supports(source: &Path, options: &CopyOptions) -> bool {
    crate::owned_mp4_remux::supports(source, Path::new("planned.mkv"), options)
        || crate::owned_adts_remux::supports(source, Path::new("planned.mka"), options)
        || crate::owned_matroska_remux::supports(source, Path::new("planned.mkv"), options)
}
pub fn plan_remux(source: &Path, options: &CopyOptions) -> Result<MediaPlan, String> {
    crate::owned_budget::check_rss_budget(options)?;
    if crate::owned_mp4_remux::supports(source, Path::new("planned.mkv"), options) {
        return crate::owned_mp4_remux::plan_remux(source, options);
    }
    if crate::owned_adts_remux::supports(source, Path::new("planned.mka"), options) {
        return crate::owned_adts_remux::plan_remux(source, options);
    }
    if !supports(source, options) {
        if !crate::owned_wave_inspect::is_wave(source).unwrap_or(false) {
            if options.max_packet_bytes == 0 {
                return Err("packet byte limit must be positive".into());
            }
            if options.max_controlled_bytes == Some(0) {
                return Err("controlled memory limit must be positive".into());
            }
            if File::open(source).is_ok_and(|mut f| {
                use std::io::Read;
                let mut prefix = [0u8; 8];
                f.read_exact(&mut prefix).is_ok() && crate::owned_mp4::recognizes_prefix(&prefix)
            }) {
                return crate::owned_mp4_remux::plan_remux(source, options);
            }
            if File::open(source).is_ok_and(|mut f| {
                use std::io::Read;
                let mut prefix = [0u8; 4];
                f.read_exact(&mut prefix).is_ok() && prefix == [0x1a, 0x45, 0xdf, 0xa3]
            }) {
                return Err("request is not an owned Matroska identity remux".into());
            }
            if crate::owned_adts_export::recognizes(source).unwrap_or(false) {
                return crate::owned_adts_remux::plan_remux(source, options);
            }
        }
        return crate::owned_wave_plan::plan_remux(source, options);
    }
    let mut input = File::open(source).map_err(|e| e.to_string())?;
    if !input.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Matroska remux requires a regular file".into());
    }
    let packets = crate::owned_matroska_copy::inspect_with_packet_and_memory_limits(
        &mut input,
        false,
        options.cancel.as_ref(),
        options.max_packet_bytes,
        options.max_controlled_bytes,
    )
    .map_err(|e| e.to_string())?;
    let payload_bytes = packets.iter().try_fold(0u64, |sum, (_, size)| {
        sum.checked_add(*size as u64)
            .ok_or("Matroska payload count overflow")
    })?;
    let info = crate::owned_webm_probe::probe_webm_with_packet_limit(
        source,
        options.max_packet_bytes.min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
    )?;
    let mut notes = vec![
        "backend: owned Matroska; no external demuxer or muxer".into(),
        "output must be .mkv or audio-only .mka; destination-specific validation and publication occur during execution".into(),
        "identity request only: track/tag edits, packet-count caps and memory/RSS policies require further implementation".into(),
    ];
    if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes, options.max_controlled_bytes, options.max_rss_bytes
        ));
    }
    Ok(MediaPlan {
        command: "remux".into(), input: source.into(), inputs: vec![source.into()],
        streams: info.streams.into_iter().map(|stream| PlanStream { index: stream.index, media_type: stream.media_type, codec: stream.codec, disposition: "copy".into() }).collect(),
        steps: vec![
            PlanStep { action: "copy".into(), detail: format!("preserve every EBML byte; {} media packets, {payload_bytes} payload bytes", packets.len()) },
            PlanStep { action: "metadata".into(), detail: "retain all original metadata and attachments without reconstruction".into() },
            PlanStep { action: "publish".into(), detail: "publish completed output without overwriting; remove temporary file on error or cancellation".into() },
        ], graph: None,
        notes,
    })
}
