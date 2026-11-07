//! Metadata-only planning for owned container remux.
use fvid_control::CopyOptions;
use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
pub(crate) fn supports(source: &Path, options: &CopyOptions) -> bool {
    crate::owned_mp4_remux::supports(source, Path::new("planned.mkv"), options)
        || crate::owned_adts_remux::supports(source, Path::new("planned.mka"), options)
        || crate::owned_matroska_remux::supports(source, Path::new("planned.mkv"), options)
}
pub fn plan_remux(source: &Path, options: &CopyOptions) -> Result<MediaPlan, String> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let mut input = match File::open(source) {
        Ok(file) => file,
        Err(_) => {
            if options.max_packets == Some(0) {
                return Err("packet work limit requires at least one packet".into());
            }
            return crate::owned_wave_plan::plan_remux(source, options);
        }
    };
    let mut sig = [0u8; 12];
    let mut read = 0usize;
    while read < sig.len() {
        match input.read(&mut sig[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let is_wave = read == 12
        && matches!(&sig[..4], b"RIFF" | b"RIFX" | b"RF64")
        && &sig[8..12] == b"WAVE";
    if is_wave {
        if options.max_packets == Some(0) {
            return Err("packet work limit requires at least one packet".into());
        }
        return crate::owned_wave_plan::plan_remux(source, options);
    }
    if read >= 8 && crate::owned_mp4::recognizes_prefix(&sig[..8]) {
        let _ = input.seek(SeekFrom::Start(0));
        return crate::owned_mp4_remux::plan_remux_from_file(source, input, options);
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if read >= 7 && crate::owned_aac::adts::header(&sig[..7]).is_some() {
        let _ = input.seek(SeekFrom::Start(0));
        return crate::owned_adts_remux::plan_remux_from_file(source, input, options);
    }
    if read < 4 || sig[..4] != [0x1a, 0x45, 0xdf, 0xa3] {
        return crate::owned_wave_plan::plan_remux(source, options);
    }
    if !crate::owned_matroska_remux::policies(options) {
        return Err("request is not an owned Matroska identity remux".into());
    }
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
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
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let info = crate::owned_webm_probe::probe_webm_from_file_with_options(source, input, options)?;
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
