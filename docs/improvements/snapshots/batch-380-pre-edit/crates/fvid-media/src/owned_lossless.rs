//! Owned lossless export admission and public API.
use fvid_control::CopyOptions;
use fvid_media_info::{DecodeTransform, LosslessStats, LosslessTransform};
use std::path::Path;
pub(crate) fn request(t: &LosslessTransform) -> Option<DecodeTransform> {
    let accepted = LosslessTransform {
        crop: t.crop.clone(),
        vertical_flip: t.vertical_flip.clone(),
        horizontal_flip: t.horizontal_flip.clone(),
        scale: t.scale.clone(),
        transpose: t.transpose.clone(),
        rotate: t.rotate,
        pad: t.pad.clone(),
        interval: t.interval,
        framestep: t.framestep.clone(),
        shuffleframes: t.shuffleframes.clone(),
        reverse: t.reverse.clone(),
        overlay: t.overlay.clone(),
        avgblur: t.avgblur.clone(),
        gblur: t.gblur.clone(),
        bilateral: t.bilateral.clone(),
        boxblur: t.boxblur.clone(),
        unsharp: t.unsharp.clone(),
        eq: t.eq.clone(),
        hue: t.hue.clone(),
        colorize: t.colorize.clone(),
        monochrome: t.monochrome.clone(),
        lutyuv: t.lutyuv.clone(),
        colorhold: t.colorhold.clone(),
        colorcontrast: t.colorcontrast.clone(),
        vibrance: t.vibrance.clone(),
        colorlevels: t.colorlevels.clone(),
        colorchannelmixer: t.colorchannelmixer.clone(),
        fade: t.fade.clone(),
        lagfun: t.lagfun.clone(),
        tmix: t.tmix.clone(),
        hqdn3d: t.hqdn3d.clone(),
        exposure: t.exposure.clone(),
        colorbalance: t.colorbalance.clone(),
        curves: t.curves.clone(),
        vignette: t.vignette.clone(),
        smartblur: t.smartblur.clone(),
        sab: t.sab.clone(),
        bitplanenoise: t.bitplanenoise.clone(),
        deband: t.deband.clone(),
        perspective: t.perspective.clone(),
        gradfun: t.gradfun.clone(),
        lenscorrection: t.lenscorrection.clone(),
        drawbox: t.drawbox.clone(),
        drawgrid: t.drawgrid.clone(),
        removegrain: t.removegrain.clone(),
        yaepblur: t.yaepblur.clone(),
        colorcorrect: t.colorcorrect.clone(),
        cas: t.cas.clone(),
        grayworld: t.grayworld.clone(),
        negate: t.negate.clone(),
        sobel: t.sobel.clone(),
        prewitt: t.prewitt.clone(),
        roberts: t.roberts.clone(),
        kirsch: t.kirsch.clone(),
        scharr: t.scharr.clone(),
        pixelize: t.pixelize.clone(),
        chromashift: t.chromashift.clone(),
        dilation: t.dilation.clone(),
        erosion: t.erosion.clone(),
        shuffleplanes: t.shuffleplanes.clone(),
        ..Default::default()
    };
    if *t != accepted {
        return None;
    }
    Some(DecodeTransform {
        crop: t.crop.clone(),
        vertical_flip: t.vertical_flip.clone(),
        horizontal_flip: t.horizontal_flip.clone(),
        scale: t.scale.clone(),
        transpose: t.transpose.clone(),
        rotate: t.rotate,
        pad: t.pad.clone(),
        interval: t.interval,
        framestep: t.framestep.clone(),
        shuffleframes: t.shuffleframes.clone(),
        reverse: t.reverse.clone(),
        overlay: t.overlay.clone(),
        avgblur: t.avgblur.clone(),
        gblur: t.gblur.clone(),
        bilateral: t.bilateral.clone(),
        boxblur: t.boxblur.clone(),
        unsharp: t.unsharp.clone(),
        eq: t.eq.clone(),
        hue: t.hue.clone(),
        colorize: t.colorize.clone(),
        monochrome: t.monochrome.clone(),
        lutyuv: t.lutyuv.clone(),
        colorhold: t.colorhold.clone(),
        colorcontrast: t.colorcontrast.clone(),
        vibrance: t.vibrance.clone(),
        colorlevels: t.colorlevels.clone(),
        colorchannelmixer: t.colorchannelmixer.clone(),
        fade: t.fade.clone(),
        lagfun: t.lagfun.clone(),
        tmix: t.tmix.clone(),
        hqdn3d: t.hqdn3d.clone(),
        exposure: t.exposure.clone(),
        colorbalance: t.colorbalance.clone(),
        curves: t.curves.clone(),
        vignette: t.vignette.clone(),
        smartblur: t.smartblur.clone(),
        sab: t.sab.clone(),
        bitplanenoise: t.bitplanenoise.clone(),
        deband: t.deband.clone(),
        perspective: t.perspective.clone(),
        gradfun: t.gradfun.clone(),
        lenscorrection: t.lenscorrection.clone(),
        drawbox: t.drawbox.clone(),
        drawgrid: t.drawgrid.clone(),
        removegrain: t.removegrain.clone(),
        yaepblur: t.yaepblur.clone(),
        colorcorrect: t.colorcorrect.clone(),
        cas: t.cas.clone(),
        grayworld: t.grayworld.clone(),
        negate: t.negate.clone(),
        sobel: t.sobel.clone(),
        prewitt: t.prewitt.clone(),
        roberts: t.roberts.clone(),
        kirsch: t.kirsch.clone(),
        scharr: t.scharr.clone(),
        pixelize: t.pixelize.clone(),
        chromashift: t.chromashift.clone(),
        dilation: t.dilation.clone(),
        erosion: t.erosion.clone(),
        shuffleplanes: t.shuffleplanes.clone(),
        ..Default::default()
    })
}
pub(crate) fn metadata(o: &CopyOptions) -> Result<crate::owned_matroska::FileMetadata, String> {
    if o.metadata_set.len() + o.metadata_delete.len() > 64 {
        return Err("at most 64 container metadata mutations".into());
    }
    let mut file = crate::owned_matroska::FileMetadata::default();
    for (key, value) in &o.metadata_set {
        if key.contains('\0') || value.contains('\0') {
            return Err("NUL in container metadata".into());
        }
        if !crate::owned_file_tags::FileTags::supports_key(key)
            && (key.is_empty() || key.len() > 128 || value.len() > 1024)
        {
            return Err("invalid container metadata key or value".into());
        }
        // Nonstandard text tags are written alongside these standard fields.
        file.tags.set(key, value);
    }
    for key in &o.metadata_delete {
        if key.contains('\0') { return Err("NUL in container metadata".into()); }
        if !crate::owned_file_tags::FileTags::supports_key(key)
            && (key.is_empty() || key.len() > 128)
        {
            return Err("invalid container metadata key".into());
        }
        file.tags.set(key, "");
    }
    Ok(file)
}
pub(crate) fn estimate_matroska_metadata_working_bytes(
    file: &crate::owned_matroska::FileMetadata,
    text_tags: &std::collections::BTreeMap<String, String>,
    track_tags: &std::collections::BTreeMap<String, String>,
    description: Option<&crate::owned_matroska::VideoTrackDescription>,
) -> usize {
    let mut raw_bytes = description.map_or(0, |d| {
        d.name
            .len()
            .saturating_add(d.language.len())
            .saturating_add(d.legacy_language.len())
    });
    for (_, v) in file.tags.entries() {
        if !v.is_empty() {
            raw_bytes = raw_bytes.saturating_add(v.len()).saturating_add(32);
        }
    }
    for ch in &file.chapters {
        raw_bytes = raw_bytes.saturating_add(ch.title.len()).saturating_add(64);
    }
    for (k, v) in text_tags {
        raw_bytes = raw_bytes
            .saturating_add(k.len())
            .saturating_add(v.len())
            .saturating_add(32);
    }
    for (k, v) in track_tags {
        raw_bytes = raw_bytes
            .saturating_add(k.len())
            .saturating_add(v.len())
            .saturating_add(32);
    }
    raw_bytes.saturating_mul(8)
}
pub(crate) fn check_options_matroska_metadata_budget(o: &CopyOptions) -> Result<(), String> {
    let Some(limit) = o.max_controlled_bytes.filter(|&limit| limit > 0) else {
        return Ok(());
    };
    let file = metadata(o)?;
    let mut text_tags = std::collections::BTreeMap::new();
    for (key, value) in &o.metadata_set {
        text_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
        if !crate::owned_file_tags::FileTags::supports_key(key) && !value.is_empty() {
            text_tags.insert(key.to_ascii_uppercase(), value.clone());
        }
    }
    for key in &o.metadata_delete {
        text_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
    }
    let mut description = crate::owned_matroska::VideoTrackDescription {
        name: String::new(),
        language: "und".into(),
        legacy_language: "und".into(),
        disposition: 1,
    };
    let mut track_tags = std::collections::BTreeMap::new();
    for (index, key, value) in &o.stream_metadata_set {
        let scoped_key = format!("{index}:{key}");
        track_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(&scoped_key));
        if !crate::owned_ffv1_export::edit_track_description(&mut description, key, value)
            && !value.is_empty()
        {
            track_tags.insert(scoped_key.to_ascii_uppercase(), value.clone());
        }
    }
    for (index, key) in &o.stream_metadata_delete {
        let scoped_key = format!("{index}:{key}");
        track_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(&scoped_key));
        crate::owned_ffv1_export::edit_track_description(&mut description, key, "");
    }
    if estimate_matroska_metadata_working_bytes(
        &file,
        &text_tags,
        &track_tags,
        Some(&description),
    ) > limit
    {
        return Err("Matroska metadata working set exceeds controlled memory budget".into());
    }
    Ok(())
}
pub(crate) fn check_source_matroska_metadata_budget(
    source: &Path,
    interval: Option<(i64, i64)>,
    options: &CopyOptions,
) -> Result<(), String> {
    check_options_matroska_metadata_budget(options)?;
    let Some(limit) = options.max_controlled_bytes.filter(|&limit| limit > 0) else {
        return Ok(());
    };
    let mut prefix = [0u8; 4];
    if !std::fs::File::open(source)
        .is_ok_and(|mut f| std::io::Read::read_exact(&mut f, &mut prefix).is_ok() && prefix == [0x1a, 0x45, 0xdf, 0xa3])
    {
        return Ok(());
    }
    let mut limits = crate::owned_webm::Limits::default();
    let memory_packets = limit / std::mem::size_of::<crate::owned_webm::Packet>();
    limits.packets = limits.packets.min(memory_packets);
    let map_packet_limit = |error: crate::owned_webm::Error| {
        if memory_packets < crate::owned_webm::Limits::default().packets
            && error
                .to_string()
                .contains("WebM packet count exceeds limit")
        {
            "controlled memory budget exceeded: Matroska packet index limit".to_owned()
        } else {
            error.to_string()
        }
    };
    let mut reader = crate::owned_webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?),
        limits,
    )
    .map_err(map_packet_limit)?;
    reader.scan_all().map_err(map_packet_limit)?;
    let Some(track) = reader.tracks.iter().find(|t| t.kind == 1) else {
        return Ok(());
    };
    let mut description = crate::owned_matroska::VideoTrackDescription {
        name: track.name.clone(),
        language: reader
            .track_languages
            .get(&track.number)
            .cloned()
            .unwrap_or_else(|| "eng".into()),
        legacy_language: reader
            .track_legacy_languages
            .get(&track.number)
            .cloned()
            .unwrap_or_else(|| "eng".into()),
        disposition: reader
            .track_dispositions
            .get(&track.number)
            .copied()
            .unwrap_or(1),
    };
    let mut track_tags = reader.track_metadata.get(&0).cloned().unwrap_or_default();
    if let Some(uid) = reader.track_uids.get(&track.number) {
        if let Some(scoped) = reader.track_metadata.get(uid) {
            track_tags.extend(scoped.clone());
        }
    }
    for (_, key, value) in &options.stream_metadata_set {
        track_tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
        if !crate::owned_ffv1_export::edit_track_description(&mut description, key, value)
            && !value.is_empty()
        {
            track_tags.insert(key.to_ascii_uppercase(), value.clone());
        }
    }
    for (_, key) in &options.stream_metadata_delete {
        track_tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
        crate::owned_ffv1_export::edit_track_description(&mut description, key, "");
    }
    let origin = reader
        .packets
        .iter()
        .filter(|p| p.track == track.number && !p.invisible)
        .map(|p| p.pts_ns)
        .min()
        .unwrap_or(0);
    let mut file = crate::owned_matroska::FileMetadata {
        tags: reader.tags.clone(),
        chapters: crate::owned_ffv1_export::export_chapters(&reader.chapters, origin, interval)?,
    };
    let mut text_tags = reader.metadata.clone();
    if !reader.writing_app.is_empty()
        && reader.writing_app != "FVid"
        && !reader.writing_app.contains('\0')
        && reader.writing_app.len() <= 1024
        && !text_tags.keys().any(|k| k.eq_ignore_ascii_case("encoder"))
    {
        text_tags.insert("ENCODER".into(), reader.writing_app.clone());
    }
    text_tags.retain(|k, v| {
        !k.is_empty()
            && k.len() <= 128
            && v.len() <= 1024
            && !k.contains('\0')
            && !v.contains('\0')
    });
    track_tags.retain(|k, v| {
        !k.is_empty()
            && k.len() <= 128
            && v.len() <= 1024
            && !k.contains('\0')
            && !v.contains('\0')
    });
    for (key, value) in &options.metadata_set {
        text_tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
        if !file.tags.set(key, value) && !value.is_empty() {
            text_tags.insert(key.to_ascii_uppercase(), value.clone());
        }
    }
    for key in &options.metadata_delete {
        file.tags.set(key, "");
        text_tags.retain(|name, _| !name.eq_ignore_ascii_case(key));
    }
    if estimate_matroska_metadata_working_bytes(
        &file,
        &text_tags,
        &track_tags,
        Some(&description),
    ) > limit
    {
        return Err("Matroska metadata working set exceeds controlled memory budget".into());
    }
    Ok(())
}
pub(crate) fn estimate_transform_working_bytes(
    transform: &DecodeTransform,
    output_frame_len: usize,
    depth: u8,
    overlay_frame_len: usize,
) -> Result<usize, String> {
    let mut working = output_frame_len.saturating_add(overlay_frame_len);
    if let Some(args) = transform.shuffleframes.as_deref() {
        let group = crate::owned_shuffleframes::ShuffleFrames::parse(args)?.group_len();
        working = working.max(output_frame_len.saturating_mul(group));
    }
    if let Some(args) = transform.tmix.as_deref() {
        let frames = crate::owned_tmix::TemporalMix::parse(args)?.frame_count();
        working = working.max(output_frame_len.saturating_mul(frames));
    }
    if transform.lagfun.is_some() {
        let samples = if depth <= 8 {
            output_frame_len
        } else {
            output_frame_len / 2
        };
        let lag_bytes = samples
            .saturating_mul(4)
            .saturating_add(output_frame_len);
        working = working.max(lag_bytes);
    }
    if transform.hqdn3d.is_some() {
        let samples = if depth <= 8 {
            output_frame_len
        } else {
            output_frame_len / 2
        };
        let hq_bytes = samples
            .saturating_mul(2)
            .saturating_add(4 * 5120 * 2)
            .saturating_add(output_frame_len);
        working = working.max(hq_bytes);
    }
    Ok(working)
}
fn stream_tag_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && !key.contains('\0')
        && !matches!(
            key.to_ascii_lowercase().as_str(),
            "rotate" | "stereo_mode" | "alpha_mode"
        )
}
pub(crate) fn policy(o: &CopyOptions, text_tags: bool) -> bool {
    (o.streams.is_empty() || o.streams == [0])
        && o.max_packet_bytes != 0
        && o.max_packets != Some(0)
        && o.max_controlled_bytes != Some(0)
        && o.metadata_set.len() + o.metadata_delete.len() <= 64
        && o.metadata_set.iter().all(|(k, v)| {
            !k.contains('\0')
                && !v.contains('\0')
                && (crate::owned_file_tags::FileTags::supports_key(k)
                    || (text_tags && !k.is_empty() && k.len() <= 128 && v.len() <= 1024))
        })
        && o.metadata_delete.iter().all(|k| {
            !k.contains('\0')
                && (crate::owned_file_tags::FileTags::supports_key(k)
                    || (text_tags && !k.is_empty() && k.len() <= 128))
        })
        && if text_tags {
            o.stream_metadata_set.len() + o.stream_metadata_delete.len() <= 64
                && o.stream_metadata_set.iter().all(|(index, key, value)| {
                    *index == 0
                        && stream_tag_key(key)
                        && !value.contains('\0')
                        && value.len() <= 1024
                        && (!key.eq_ignore_ascii_case("language") || value.len() <= 128)
                })
                && o.stream_metadata_delete
                    .iter()
                    .all(|(index, key)| *index == 0 && stream_tag_key(key))
        } else {
            o.stream_metadata_set.is_empty() && o.stream_metadata_delete.is_empty()
        }
}
/// Whether the owned export pipeline admits this source, transform and policy.
pub fn supports(source: &Path, t: &LosslessTransform, o: &CopyOptions) -> bool {
    if o.cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
        || o.max_packet_bytes == 0
        || o.max_packets == Some(0)
        || o.max_controlled_bytes == Some(0)
        || crate::owned_budget::check_rss_budget(o).is_err()
    {
        return false;
    }
    if crate::owned_mp4_multitrack_export::supports(source,t,o) {return true;}
    if crate::owned_webm_multitrack_export::supports(source,t,o) {return true;}
    if policy(o,true) && request(t).is_some() {
        match crate::owned_mp4_video_bridge::prepare(source,Some(o)) {
            Ok(Some(temporary)) => {
                if request(t).is_some_and(|r|crate::owned_ffv1_export::supports_with_options(&temporary.path,&r,Some(o))){return true;}
            }
            Err(_) if crate::owned_mp4_video_bridge::recognizes(source) => return true,
            _=>{},
        }
    }
    if policy(o, true) && request(t).is_some_and(|r| crate::owned_ffv1_export::supports_with_options(source, &r, Some(o))) {
        return true;
    }
    let mut reader = match std::fs::File::open(source) {
        Ok(file) => std::io::BufReader::new(file),
        Err(_) => return false,
    };
    let mut line = Vec::new();
    if !crate::owned_y4m::line(&mut reader, &mut line).is_ok_and(|present| present) {
        return false;
    }
    let Ok(header) = crate::owned_y4m::Header::parse(&line) else {
        return false;
    };
    // Keep chroma-siting aliases on their previous backend until mapped.
    if header
        .tokens
        .iter()
        .any(|t| matches!(t.as_str(), "C420mpeg2" | "C420paldv"))
    {
        return false;
    }
    policy(o, true) && metadata(o).is_ok()
        && request(t).is_some_and(|r| crate::owned_y4m_decode::supports_transformed(source, &r))
}
pub fn transcode_lossless(
    source: &Path,
    destination: &Path,
    transform: LosslessTransform,
    options: &CopyOptions,
) -> Result<LosslessStats, String> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
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
    crate::owned_budget::check_rss_budget(options)?;
    if destination
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
    {
        return Err("owned FFV1 export requires .mkv".into());
    }
    let request = request(&transform)
        .ok_or("owned Y4M lossless export does not yet implement requested transforms")?;
    if let Some(stats)=crate::owned_mp4_multitrack_export::try_export(source,destination,&transform,options)? {return Ok(stats);}
    if let Some(stats)=crate::owned_webm_multitrack_export::try_export(source,destination,&transform,options)? {return Ok(stats);}

    if policy(options,true) {
        check_options_matroska_metadata_budget(options)?;
        if let Some(temporary)=crate::owned_mp4_video_bridge::prepare(source,Some(options))? {
            check_source_matroska_metadata_budget(&temporary.path, request.interval, options)?;
            return transcode_lossless(&temporary.path,destination,transform,options);
        }
    }
    if policy(options, true) && crate::owned_ffv1_export::supports_with_options(source, &request, Some(options)) {
        check_source_matroska_metadata_budget(source, request.interval, options)?;
        let (stats, event, consumed) =
            crate::owned_ffv1_export::export(source, destination, &request, options)?;
        return Ok(LosslessStats {
            backend: "fvid",
            video_frames: stats.video_frames,
            decoded_frames: consumed,
            seek_used: false,
            video_packets: event.packets,
            copied_packets: 0,
            trimmed_audio_sample_frames: 0,
            pixel_format: stats.pixel_format,
            encoder: "ffv1".into(),
            fvid_crop_payload_copies: 0,
            vertical_flip: transform.vertical_flip,
            horizontal_flip: transform.horizontal_flip,
        });
    }
    let file_metadata = metadata(options)?;
    if !policy(options, true) {
        return Err("owned Y4M lossless export does not yet implement requested policy".into());
    }
    check_options_matroska_metadata_budget(options)?;
    // A recognized compressed container whose owned capability was refused is
    // not Y4M. Keep the refusal explicit instead of parsing binary bytes as text.
    let mut probe = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut prefix = [0u8; 8];
    let count = std::io::Read::read(&mut probe, &mut prefix).map_err(|e| e.to_string())?;
    if count == 8 && crate::owned_mp4::recognizes_prefix(&prefix) {
        return Err("owned MP4 lossless export does not yet support this codec, format or transform".into());
    }
    if count >= 4 && prefix[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        return Err("owned Matroska lossless export does not yet support this codec, format or transform".into());
    }
    if let Some((from, to)) = request.interval {
        if from < 0 || to <= from {
            return Err("lossless interval requires 0 <= from < to".into());
        }
        let mut reader =
            std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
        let mut line = Vec::new();
        crate::owned_y4m::line(&mut reader, &mut line)?;
        let header = crate::owned_y4m::Header::parse(&line)?;
        let [n, d] = header.frame_rate()?;
        let mut a = n as u64;
        let mut b = d as u64 * 1_000_000;
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let n_red = n as u64 / a;
        let den_red = (d as u64 * 1_000_000) / a;
        for time in [from, to] {
            if ((time as u64) % den_red) * (n_red % den_red) % den_red != 0 {
                return Err("interval boundary is not exact in video time base".into());
            }
        }
        ((from as u64) / den_red)
            .checked_mul(n_red)
            .ok_or("interval timestamp overflow")?;
    }
    let mut overlay_frame_len = 0usize;
    if let Some(spec) = &request.overlay {
        if crate::owned_y4m_decode::supports(&spec.path) {
            let mut reader =
                std::io::BufReader::new(std::fs::File::open(&spec.path).map_err(|e| e.to_string())?);
            let mut line = Vec::new();
            if crate::owned_y4m::line(&mut reader, &mut line)? {
                let header = crate::owned_y4m::Header::parse(&line)?;
                let frame_len = header.frame_len()?;
                overlay_frame_len = frame_len;
                if frame_len > options.max_packet_bytes {
                    return Err("Y4M frame byte length exceeds packet byte limit".into());
                }
                if options
                    .max_controlled_bytes
                    .is_some_and(|limit| frame_len > limit)
                {
                    return Err("Y4M frame byte length exceeds controlled memory budget".into());
                }
            }
        }
    }
    if crate::owned_y4m_decode::supports(source) {
        let mut reader =
            std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
        let mut line = Vec::new();
        if crate::owned_y4m::line(&mut reader, &mut line)? {
            let header = crate::owned_y4m::Header::parse(&line)?;
            let frame_len = header.frame_len()?;
            if frame_len > options.max_packet_bytes {
                return Err("Y4M frame byte length exceeds packet byte limit".into());
            }
            if options
                .max_controlled_bytes
                .is_some_and(|limit| frame_len > limit)
            {
                return Err("Y4M frame byte length exceeds controlled memory budget".into());
            }
            if let Some(limit) = options.max_controlled_bytes {
                let presented =
                    crate::owned_y4m_decode::presented_header_after_geometry(&header, &request)?;
                let out_len = presented.frame_len()?;
                let working = estimate_transform_working_bytes(
                    &request,
                    out_len,
                    presented.depth(),
                    overlay_frame_len,
                )?;
                if working > limit {
                    return Err("Y4M frame byte length exceeds controlled memory budget".into());
                }
            }
        }
    }
    let (stats, event, consumed) = crate::owned_matroska::export_y4m_ffv1_policy(
        source,
        destination,
        &request,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        options.max_packet_bytes,
        options.max_packets,
        true,
        &file_metadata,
        Some(options),
    )
    .map_err(|e| e.to_string())?;
    Ok(LosslessStats {
        backend: "fvid",
        video_frames: stats.video_frames,
        decoded_frames: consumed,
        seek_used: false,
        video_packets: event.packets,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format: stats.pixel_format,
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: transform.vertical_flip,
        horizontal_flip: transform.horizontal_flip,
    })
}

/// Preserve the library crop API without a libav decoder or encoder.
pub fn crop_lossless(
    source: &Path,
    destination: &Path,
    crop: fvid_media_info::CropRect,
    options: &CopyOptions,
) -> Result<LosslessStats, String> {
    transcode_lossless(
        source,
        destination,
        LosslessTransform {
            crop: Some(crop),
            ..Default::default()
        },
        options,
    )
}

/// Settings implemented by the owned version-1 FFV1 encoder.
/// Unknown options never become silently ignored codec requests.
pub fn supports_encoder(settings: &fvid_media_info::EncoderSettings) -> bool {
    settings.validate().is_ok() && settings.name == "ffv1"
        && settings.options.iter().all(|(key, value)| key == "level" && value == "1")
}

/// Explicit FFV1 encoding through the owned lossless pipeline.
pub fn transcode(
    source: &Path,
    destination: &Path,
    transform: LosslessTransform,
    options: &CopyOptions,
    settings: &fvid_media_info::EncoderSettings,
) -> Result<LosslessStats, String> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
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
    crate::owned_budget::check_rss_budget(options)?;
    settings.validate()?;
    if !supports_encoder(settings) {
        return Err("owned transcode does not yet implement requested encoder/settings".into());
    }
    transcode_lossless(source, destination, transform, options)
}

pub(crate) fn check_encoded_ffv1(
    view: crate::owned_video_decode::FrameView<'_>,
    max_packet_bytes: usize,
) -> Result<(), String> {
    check_encoded_ffv1_with_controlled(view, max_packet_bytes, None)
}

pub(crate) fn check_encoded_ffv1_with_controlled(
    view: crate::owned_video_decode::FrameView<'_>,
    max_packet_bytes: usize,
    max_controlled_bytes: Option<usize>,
) -> Result<(), String> {
    let packet = if view.monochrome {
        let samples = (view.width as usize)
            .checked_mul(view.height as usize)
            .and_then(|v| v.checked_mul(if view.depth == 8 { 1 } else { 2 }))
            .ok_or("FFV1 gray size overflow")?;
        match view.alpha {
            Some(alpha) => crate::owned_ffv1_encoder::encode_gray_with_alpha(
                view.width as usize,
                view.height as usize,
                &view.pixels[..samples],
                alpha,
                view.depth,
            )?,
            None => crate::owned_ffv1_encoder::encode_gray(
                view.width as usize,
                view.height as usize,
                &view.pixels[..samples],
                view.depth,
            )?,
        }
    } else {
        let mut data = crate::owned_frame::buffer(view.pixels.len())?;
        data.copy_from_slice(view.pixels);
        let frame = crate::owned_frame::GeometryFrame {
            width: view.width as usize,
            height: view.height as usize,
            subsampling: view.subsampling,
            data,
        };
        match view.alpha {
            Some(alpha) => {
                crate::owned_ffv1_encoder::encode_with_alpha(&frame, view.depth, alpha)?
            }
            None => crate::owned_ffv1_encoder::encode(&frame, view.depth)?,
        }
    };
    if packet.len() > max_packet_bytes {
        return Err("encoded FFV1 packet exceeds byte limit".into());
    }
    if max_controlled_bytes.is_some_and(|limit| packet.len() > limit) {
        return Err("encoded FFV1 packet exceeds controlled memory budget".into());
    }
    Ok(())
}

/// Plan the same owned Y4M-to-FFV1 execution admitted by the export API.
/// Unsupported requests are refused rather than advertised as executable.
pub fn plan_transcode_lossless(
    source: &Path,
    transform: &LosslessTransform,
    options: &CopyOptions,
    encoder: Option<&str>,
) -> Result<fvid_media_info::MediaPlan, String> {
    use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
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
    crate::owned_budget::check_rss_budget(options)?;
    let mut default_options = CopyOptions::default();
    default_options.max_packet_bytes = options.max_packet_bytes;
    default_options.max_packets = options.max_packets;
    default_options.max_controlled_bytes = options.max_controlled_bytes;
    default_options.max_rss_bytes = options.max_rss_bytes;
    default_options.cancel = options.cancel.clone();
    if encoder.is_some_and(|name| name != "ffv1")
        || (!supports(source, transform, options) && !supports(source, transform, &default_options))
    {
        return Err("request has no owned lossless export plan".into());
    }
    let request = request(transform)
        .ok_or("owned Y4M lossless export does not yet implement requested transforms")?;
    let multitrack = if let Some(selection) =
        crate::owned_mp4_multitrack_export::plan_selection(source, transform, options)?
    {
        Some(selection)
    } else {
        crate::owned_webm_multitrack_export::plan_selection(source, transform, options)?
    };
    metadata(options)?;
    if let Some((_, ref selected)) = multitrack {
        let mut selected_options = options.clone();
        selected_options
            .stream_metadata_set
            .retain(|(i, _, _)| selected.contains(i));
        selected_options
            .stream_metadata_delete
            .retain(|(i, _)| selected.contains(i));
        check_options_matroska_metadata_budget(&selected_options)?;
    } else {
        check_options_matroska_metadata_budget(options)?;
    }
    if multitrack.is_none() {
        if !policy(options, true) {
            return Err("owned Y4M lossless export does not yet implement requested policy".into());
        }
        let mut check_encoded =
            |view: crate::owned_video_decode::FrameView<'_>| -> Result<(), String> {
                check_encoded_ffv1_with_controlled(
                    view,
                    options.max_packet_bytes,
                    options.max_controlled_bytes,
                )
            };
        let enforce_encoded_budget =
            options.max_packet_bytes != CopyOptions::default().max_packet_bytes
                || options.max_controlled_bytes.is_some();
        if crate::owned_mp4_video_bridge::recognizes(source) {
            if let Some(temporary) =
                crate::owned_mp4_video_bridge::prepare(source, Some(options))?
            {
                check_source_matroska_metadata_budget(&temporary.path, request.interval, options)?;
                let mut stage = options.clone();
                stage.streams.clear();
                stage.stream_metadata_set.clear();
                stage.stream_metadata_delete.clear();
                let Some((stats, _)) = crate::owned_video_decode::decode_webm(
                    &temporary.path,
                    &request,
                    if enforce_encoded_budget {
                        Some(&mut check_encoded)
                    } else {
                        None
                    },
                    Some(&stage),
                )? else {
                    return Err(
                        "owned MP4 lossless export does not yet support this codec, format or transform"
                            .into(),
                    );
                };
                if stats.video_frames == 0 {
                    return Err("Matroska has no selected frames".into());
                }
            } else {
                return Err(
                    "owned MP4 lossless export does not yet support this codec, format or transform"
                        .into(),
                );
            }
        } else if crate::owned_ffv1_export::supports_with_options(source, &request, Some(options)) {
            check_source_matroska_metadata_budget(source, request.interval, options)?;
            let mut stage = options.clone();
            stage.streams.clear();
            stage.stream_metadata_set.clear();
            stage.stream_metadata_delete.clear();
            let Some((stats, _)) = crate::owned_video_decode::decode_webm(
                source,
                &request,
                if enforce_encoded_budget {
                    Some(&mut check_encoded)
                } else {
                    None
                },
                Some(&stage),
            )? else {
                return Err(
                    "owned Matroska lossless export does not yet support this codec, format or transform"
                        .into(),
                );
            };
            if stats.video_frames == 0 {
                return Err("Matroska has no selected frames".into());
            }
        }
    }
    let info = crate::owned_probe::probe_with_options(source, options)?;
    let mut steps = vec![PlanStep {
        action: "decode".into(),
        detail: "read source video frames with the owned container parser and decoder".into(),
    }];
    if let Some((from, to)) = transform.interval {
        if from < 0 || to <= from {
            return Err("lossless interval requires 0 <= from < to".into());
        }
        if multitrack.is_none() && crate::owned_y4m_decode::supports(source) {
            let mut input =
                std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
            let mut bytes = Vec::new();
            crate::owned_y4m::line(&mut input, &mut bytes)?;
            let header = crate::owned_y4m::Header::parse(&bytes)?;
            let [n, d] = header.frame_rate()?;
            if [from, to]
                .into_iter()
                .any(|time| time as u128 * n as u128 % (d as u128 * 1_000_000) != 0)
            {
                return Err("interval boundary is not exact in video time base".into());
            }
        }
        steps.push(PlanStep {
            action: "interval".into(),
            detail: format!("presentation window [{from},{to}) µs"),
        });
    }
    if multitrack.is_none() && crate::owned_y4m_decode::supports(source) {
        if options.max_packets.is_some()
            || options.max_packet_bytes != CopyOptions::default().max_packet_bytes
            || options.max_controlled_bytes.is_some()
            || request.interval.is_some()
            || request.framestep.is_some()
            || request.shuffleframes.is_some()
            || request.reverse.is_some()
        {
            let mut overlay_frame_len = 0usize;
            if let Some(spec) = &request.overlay {
                if crate::owned_y4m_decode::supports(&spec.path) {
                    let mut ov_reader = std::io::BufReader::new(
                        std::fs::File::open(&spec.path).map_err(|e| e.to_string())?,
                    );
                    let mut ov_line = Vec::new();
                    if crate::owned_y4m::line(&mut ov_reader, &mut ov_line)? {
                        let ov_header = crate::owned_y4m::Header::parse(&ov_line)?;
                        let ov_len = ov_header.frame_len()?;
                        overlay_frame_len = ov_len;
                        if ov_len > options.max_packet_bytes {
                            return Err("Y4M frame byte length exceeds packet byte limit".into());
                        }
                        if options
                            .max_controlled_bytes
                            .is_some_and(|limit| ov_len > limit)
                        {
                            return Err(
                                "Y4M frame byte length exceeds controlled memory budget".into(),
                            );
                        }
                    }
                }
            }
            let mut reader =
                std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
            let mut line = Vec::new();
            if crate::owned_y4m::line(&mut reader, &mut line)? {
                let header = crate::owned_y4m::Header::parse(&line)?;
                let frame_len = header.frame_len()?;
                if frame_len > options.max_packet_bytes {
                    return Err("Y4M frame byte length exceeds packet byte limit".into());
                }
                if options
                    .max_controlled_bytes
                    .is_some_and(|limit| frame_len > limit)
                {
                    return Err("Y4M frame byte length exceeds controlled memory budget".into());
                }
                if let Some(limit) = options.max_controlled_bytes {
                    let presented =
                        crate::owned_y4m_decode::presented_header_after_geometry(&header, &request)?;
                    let out_len = presented.frame_len()?;
                    let working = estimate_transform_working_bytes(
                        &request,
                        out_len,
                        presented.depth(),
                        overlay_frame_len,
                    )?;
                    if working > limit {
                        return Err(
                            "Y4M frame byte length exceeds controlled memory budget".into(),
                        );
                    }
                }
            }
            let mut reader =
                std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
            let mut visit = |header: &crate::owned_y4m::Header, data: &[u8], _: u64, _: u64| {
                if options
                    .cancel
                    .as_ref()
                    .is_some_and(fvid_control::CancelFlag::is_cancelled)
                {
                    return Err("media operation cancelled".into());
                }
                crate::owned_budget::check_rss_budget(options)?;
                if options
                    .max_controlled_bytes
                    .is_some_and(|limit| data.len() > limit)
                {
                    return Err("Y4M frame byte length exceeds controlled memory budget".into());
                }
                if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
                    || options.max_controlled_bytes.is_some()
                {
                    let (sx, sy) = header.format.subsampling();
                    let frame = crate::owned_frame::GeometryFrame {
                        data: data.to_vec(),
                        width: header.width,
                        height: header.height,
                        subsampling: Some([sx, sy]),
                    };
                    let packet = crate::owned_ffv1_encoder::encode(&frame, header.depth())?;
                    if packet.len() > options.max_packet_bytes {
                        return Err("encoded FFV1 packet exceeds byte limit".into());
                    }
                    if options
                        .max_controlled_bytes
                        .is_some_and(|limit| packet.len() > limit)
                    {
                        return Err("encoded FFV1 packet exceeds controlled memory budget".into());
                    }
                }
                Ok(())
            };
            let (stats, _) = crate::owned_y4m_decode::visit_reader_transformed_counted(
                &mut reader,
                &request,
                options.max_packets,
                &mut visit,
            )?;
            if stats.video_frames == 0 {
                return Err(
                    if request.framestep.is_some()
                        || request.shuffleframes.is_some()
                        || request.reverse.is_some()
                    {
                        "Matroska has no selected frames".into()
                    } else {
                        "Y4M has no selected frames".into()
                    },
                );
            }
        } else {
            let mut reader =
                std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
            let mut line = Vec::new();
            if !crate::owned_y4m::line(&mut reader, &mut line)?
                || !crate::owned_y4m::line(&mut reader, &mut line)?
            {
                return Err("Matroska has no selected frames".into());
            }
            let mut reader =
                std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
            let mut first = false;
            let mut visit = |_: &crate::owned_y4m::Header, _: &[u8], _: u64, _: u64| {
                first = true;
                Err::<(), String>("__fvid_plan_first_frame_ok__".into())
            };
            match crate::owned_y4m_decode::visit_reader_transformed_counted(
                &mut reader,
                &request,
                None,
                &mut visit,
            ) {
                Ok((stats, _)) => {
                    if stats.video_frames == 0 {
                        return Err("Matroska has no selected frames".into());
                    }
                }
                Err(err) if first && err == "__fvid_plan_first_frame_ok__" => {}
                Err(err) => return Err(err),
            }
        }
    }
    if *transform != LosslessTransform::default() {
        steps.push(PlanStep {
            action: "filter".into(),
            detail: "apply requested geometry and pixel transforms in the owned frame pipeline"
                .into(),
        });
    }
    steps.push(PlanStep {
        action: "encode".into(),
        detail: "encode FFV1 with the owned encoder".into(),
    });
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        steps.push(PlanStep {
            action: "metadata".into(),
            detail: format!(
                "container set={}, delete={}; stream set={}, delete={}",
                options.metadata_set.len(),
                options.metadata_delete.len(),
                options.stream_metadata_set.len(),
                options.stream_metadata_delete.len()
            ),
        });
    }
    steps.push(PlanStep {
        action: "mux".into(),
        detail: "write Matroska with the owned muxer".into(),
    });
    let streams = if let Some((video, selected)) = multitrack {
        selected
            .into_iter()
            .filter_map(|index| {
                let stream = info.streams.iter().find(|s| s.index == index)?;
                Some(PlanStream {
                    index: stream.index,
                    media_type: stream.media_type.clone(),
                    codec: stream.codec.clone(),
                    disposition: if index == video {
                        "primary_video"
                    } else {
                        "copy"
                    }
                    .into(),
                })
            })
            .collect()
    } else {
        info.streams
            .into_iter()
            .map(|stream| PlanStream {
                index: stream.index,
                media_type: stream.media_type,
                codec: stream.codec,
                disposition: "primary_video".into(),
            })
            .collect()
    };
    let mut notes = vec![
        "backend: owned video/FFV1/Matroska; no external demuxer, filter graph or encoder".into(),
        "destination must support the owned Matroska export; packet, cancellation and publication checks also run during execution".into(),
    ];
    if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?} bytes; RSS limit: {:?} bytes",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    Ok(MediaPlan {
        command: "transcode-lossless".into(),
        input: source.into(),
        inputs: std::iter::once(source.to_path_buf())
            .chain(transform.overlay.iter().map(|spec| spec.path.clone()))
            .collect(),
        streams,
        steps,
        graph: None,
        notes,
    })
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    fn source() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/shuffleplanes-444-8.y4m")
    }
    #[test]
    fn owned_plan_matches_executed_filter_export() {
        let source = source();
        let transform = LosslessTransform {
            negate: Some("".into()),
            ..Default::default()
        };
        let options = CopyOptions::default();
        let plan = plan_transcode_lossless(&source, &transform, &options, Some("ffv1")).unwrap();
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(plan.streams[0].disposition, "primary_video");
        assert_eq!(
            plan.steps
                .iter()
                .map(|step| step.action.as_str())
                .collect::<Vec<_>>(),
            ["decode", "filter", "encode", "mux"]
        );
        assert!(plan.notes[0].starts_with("backend: owned"));
        let directory =
            std::env::temp_dir().join(format!("fvid-owned-lossless-plan-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("filtered.mkv");
        let stats = transcode_lossless(&source, &output, transform.clone(), &options).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.encoder, "ffv1");
        assert!(stats.video_frames > 0);
        let info = crate::owned_webm_probe::probe_webm(&output).unwrap();
        assert_eq!(info.streams[0].codec, "ffv1");
        {
            let public =
                crate::plan_transcode_lossless(&source, &transform, &options, Some("ffv1"))
                    .unwrap();
            assert_eq!(public.steps, plan.steps);
            assert_eq!(public.notes, plan.notes);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn rotation_library_export_and_reexport_preserve_pixels_and_timing() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/rotate-grid.y4m");
        let directory =
            std::env::temp_dir().join(format!("fvid-library-rotate-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let options = CopyOptions::default();
        let turn = LosslessTransform {
            rotate: Some(fvid_media_info::RotateAngle::parse("90").unwrap()),
            ..Default::default()
        };
        assert!(supports(&source, &turn, &options));
        plan_transcode_lossless(&source, &turn, &options, None).unwrap();
        let first = directory.join("first.mkv");
        let second = directory.join("second.mkv");
        assert_eq!(
            transcode_lossless(&source, &first, turn.clone(), &options)
                .unwrap()
                .video_frames,
            3
        );
        assert!(supports(&first, &turn, &options));
        transcode_lossless(&first, &second, turn, &options).unwrap();
        let mut frames = 0;
        crate::owned_video_decode::decode_ffv1(
            &second,
            &Default::default(),
            Some(&mut |frame| {
                assert_eq!((frame.width, frame.height), (3, 2));
                assert_eq!(&frame.pixels[..6], &[6, 5, 4, 3, 2, 1]);
                assert_eq!(&frame.pixels[6..], &[128; 12]);
                assert_eq!(frame.pts_ns, frames * 500_000_000);
                assert_eq!(frame.duration_ns, Some(500_000_000));
                frames += 1;
                Ok(())
            }),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(frames, 3);
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/rotate-white420.y4m");
        let turn = LosslessTransform {
            rotate: Some(fvid_media_info::RotateAngle::parse("45").unwrap()),
            ..Default::default()
        };
        std::fs::remove_file(&first).unwrap();
        transcode_lossless(&source, &first, turn, &options).unwrap();
        let mut frames = 0;
        crate::owned_video_decode::decode_ffv1(
            &first,
            &Default::default(),
            Some(&mut |frame| {
                assert_eq!((frame.width, frame.height), (11, 11));
                assert_eq!(frame.pixels.len(), 193);
                assert_eq!(frame.pixels[0], 16);
                assert_eq!(frame.pixels[60], 235);
                assert!(frame.pixels[121..].iter().all(|&sample| sample == 128));
                frames += 1;
                Ok(())
            }),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(frames, 3);
        std::fs::remove_file(&first).unwrap();
        let transform = LosslessTransform {
            rotate: Some(fvid_media_info::RotateAngle::parse("45").unwrap()),
            pad: Some(fvid_media_info::PadRect {
                width: 12,
                height: 12,
                x: 0,
                y: 0,
            }),
            scale: Some(fvid_media_info::ScaleSize {
                width: 6,
                height: 6,
            }),
            ..Default::default()
        };
        transcode_lossless(&source, &first, transform, &options).unwrap();
        let mut frames = 0;
        crate::owned_video_decode::decode_ffv1(
            &first,
            &Default::default(),
            Some(&mut |frame| {
                assert_eq!((frame.width, frame.height), (6, 6));
                assert_eq!(frame.pixels.len(), 54);
                assert!(frame.pixels[36..].iter().all(|&sample| sample == 128));
                frames += 1;
                Ok(())
            }),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(frames, 3);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn plan_refuses_unsupported_execution_and_inexact_interval() {
        let source = source();
        let options = CopyOptions::default();
        assert!(
            plan_transcode_lossless(&source, &Default::default(), &options, Some("h264")).is_err()
        );
        let inexact = LosslessTransform {
            interval: Some((1, 40_000)),
            ..Default::default()
        };
        assert!(
            plan_transcode_lossless(&source, &inexact, &options, None)
                .unwrap_err()
                .contains("not exact")
        );
        let unsupported = LosslessTransform {
            gblur: Some("sigma=NaN".into()),
            ..Default::default()
        };
        assert!(plan_transcode_lossless(&source, &unsupported, &options, None).is_err());
    }
}

/// Composite a scheduled Y4M foreground, then export with owned FFV1/Matroska.
pub fn overlay_video(
    source: &Path,
    overlay: &Path,
    destination: &Path,
    x: i32,
    y: i32,
    options: &CopyOptions,
) -> Result<LosslessStats, String> {
    transcode_lossless(
        source,
        destination,
        LosslessTransform {
            overlay: Some(fvid_media_info::OverlaySpec {
                path: overlay.into(),
                x,
                y,
            }),
            ..Default::default()
        },
        options,
    )
}
pub fn plan_overlay(
    source: &Path,
    overlay: &Path,
    x: i32,
    y: i32,
    options: &CopyOptions,
) -> Result<fvid_media_info::MediaPlan, String> {
    let transform = LosslessTransform {
        overlay: Some(fvid_media_info::OverlaySpec {
            path: overlay.into(),
            x,
            y,
        }),
        ..Default::default()
    };
    let mut plan = plan_transcode_lossless(source, &transform, options, None)?;
    plan.command = "overlay".into();
    Ok(plan)
}

#[cfg(test)]
mod framestep_tests {
    use super::*;
    #[test]
    fn stepped_file_export_counts_consumed_frames_and_preserves_packet_timing() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = root.join("framestep-six-frames.y4m");
        for (case, interval, indices, consumed) in [
            ("full", None, vec![0u8, 2, 4], 6),
            ("range", Some((250_000, 1_000_000)), vec![1u8, 3], 4),
        ] {
            let transform = LosslessTransform {
                framestep: Some("step=2".into()),
                interval,
                ..Default::default()
            };
            let options = CopyOptions::default();
            assert!(supports(&source, &transform, &options));
            let plan = crate::plan_transcode_lossless(&source, &transform, &options, None).unwrap();
            assert!(plan.notes[0].starts_with("backend: owned"));
            let output = std::env::temp_dir().join(format!(
                "fvid-framestep-export-{}-{case}.mkv",
                std::process::id()
            ));
            let stats = crate::transcode_lossless(&source, &output, transform, &options).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.decoded_frames, consumed);
            assert_eq!(stats.video_frames, indices.len() as u64);
            assert_eq!(stats.video_packets, indices.len() as u64);
            let mut reader = crate::owned_webm::WebmReader::open(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            reader.scan_all().unwrap();
            assert_eq!(reader.packets.len(), indices.len());
            let origin = interval.map_or(0, |(from, _)| from as i64 * 1000);
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(4, 4, 1 << 20).unwrap();
            for (slot, index) in indices.into_iter().enumerate() {
                let packet = &reader.packets[slot];
                assert_eq!(packet.pts_ns, i64::from(index) * 250_000_000 - origin);
                assert_eq!(packet.duration_ns, Some(250_000_000));
                let mut expected = vec![10 + index; 16];
                expected.extend([128; 8]);
                assert_eq!(
                    decoder
                        .decode(&reader.read_packet(slot).unwrap())
                        .unwrap()
                        .frame
                        .data,
                    expected
                );
            }
            drop(reader);
            std::fs::remove_file(output).unwrap();
        }
    }
    #[test]
    fn stepped_export_checks_discarded_payloads_and_input_packet_limit() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        for (case, name, max_packets, message) in [
            (
                "truncated",
                "framestep-discarded-truncated.y4m",
                None,
                "truncated Y4M frame payload",
            ),
            (
                "limit",
                "framestep-six-frames.y4m",
                Some(5),
                "Y4M input packet count exceeds limit",
            ),
        ] {
            let source = root.join(name);
            let output = std::env::temp_dir().join(format!(
                "fvid-framestep-refusal-{}-{case}.mkv",
                std::process::id()
            ));
            let transform = LosslessTransform {
                framestep: Some("2".into()),
                ..Default::default()
            };
            let options = CopyOptions {
                max_packets,
                ..Default::default()
            };
            assert!(supports(&source, &transform, &options));
            let error =
                crate::transcode_lossless(&source, &output, transform, &options).unwrap_err();
            assert!(error.contains(message), "{error}");
            assert!(!output.exists());
        }
    }
}

#[cfg(test)]
mod shuffleframes_tests {
    use super::*;
    #[test]
    fn shuffled_files_preserve_samples_timing_and_consumed_counts() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = root.join("shuffleframes-seven-frames.y4m");
        for (case, mapping, interval, step, expected, consumed) in [
            (
                "inverse",
                "2 1 0",
                None,
                None,
                vec![(2u8, 0u64), (1, 1), (0, 2), (5, 3), (4, 4), (3, 5)],
                7,
            ),
            (
                "drops",
                "mapping=2|-1|2",
                None,
                None,
                vec![(2, 0), (2, 2), (5, 3), (5, 5)],
                7,
            ),
            (
                "range",
                "2 1 0",
                Some((250_000, 1_500_000)),
                None,
                vec![(3, 1), (2, 2), (1, 3)],
                6,
            ),
            (
                "step",
                "2 1 0",
                None,
                Some("2"),
                vec![(4, 0), (2, 2), (0, 4)],
                7,
            ),
        ] {
            let transform = LosslessTransform {
                shuffleframes: Some(mapping.into()),
                interval,
                framestep: step.map(String::from),
                ..Default::default()
            };
            let options = CopyOptions::default();
            assert!(supports(&source, &transform, &options));
            let plan = crate::plan_transcode_lossless(&source, &transform, &options, None).unwrap();
            assert!(plan.notes[0].starts_with("backend: owned"));
            let output = std::env::temp_dir().join(format!(
                "fvid-shuffle-export-{}-{case}.mkv",
                std::process::id()
            ));
            let stats = crate::transcode_lossless(&source, &output, transform, &options).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.decoded_frames, consumed);
            assert_eq!(stats.video_frames, expected.len() as u64);
            assert_eq!(stats.video_packets, expected.len() as u64);
            let mut reader = crate::owned_webm::WebmReader::open(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            reader.scan_all().unwrap();
            assert_eq!(reader.packets.len(), expected.len());
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(4, 4, 1 << 20).unwrap();
            let origin = interval.map_or(0, |(from, _)| from as i64 * 1000);
            for (slot, (value, position)) in expected.into_iter().enumerate() {
                assert_eq!(
                    reader.packets[slot].pts_ns,
                    position as i64 * 250_000_000 - origin
                );
                assert_eq!(reader.packets[slot].duration_ns, Some(250_000_000));
                let mut pixels = vec![10 + value; 16];
                pixels.extend([128; 8]);
                assert_eq!(
                    decoder
                        .decode(&reader.read_packet(slot).unwrap())
                        .unwrap()
                        .frame
                        .data,
                    pixels
                );
            }
            drop(reader);
            std::fs::remove_file(output).unwrap();
        }
    }
    #[test]
    fn all_dropped_or_incomplete_groups_do_not_publish_empty_video() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = root.join("shuffleframes-seven-frames.y4m");
        for (case, mapping) in [("all-drop", "-1"), ("partial", "0 1 2 3 4 5 6 7")] {
            let transform = LosslessTransform {
                shuffleframes: Some(mapping.into()),
                ..Default::default()
            };
            let output = std::env::temp_dir().join(format!(
                "fvid-shuffle-empty-{}-{case}.mkv",
                std::process::id()
            ));
            let error =
                crate::transcode_lossless(&source, &output, transform, &CopyOptions::default())
                    .unwrap_err();
            assert!(error.contains("Matroska has no selected frames"), "{error}");
            assert!(!output.exists());
        }
    }
    #[test]
    fn discarded_shuffle_tail_still_validates_payload_before_publication() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/shuffleframes-discarded-truncated.y4m");
        let output =
            std::env::temp_dir().join(format!("fvid-shuffle-truncated-{}.mkv", std::process::id()));
        let transform = LosslessTransform {
            shuffleframes: Some("2 1 0".into()),
            ..Default::default()
        };
        assert!(supports(&source, &transform, &CopyOptions::default()));
        let error = crate::transcode_lossless(&source, &output, transform, &CopyOptions::default())
            .unwrap_err();
        assert!(error.contains("truncated Y4M frame payload"), "{error}");
        assert!(!output.exists());
    }
}

#[cfg(test)]
mod reverse_tests {
    use super::*;
    #[test]
    fn reversed_files_preserve_forward_packet_timing_and_source_counts() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/reverse-six-frames.y4m");
        for (case, interval, step, shuffle, values, positions, consumed) in [
            (
                "full",
                None,
                None,
                None,
                vec![5u8, 4, 3, 2, 1, 0],
                vec![0u64, 1, 2, 3, 4, 5],
                6,
            ),
            (
                "range",
                Some((250_000, 1_000_000)),
                None,
                None,
                vec![3, 2, 1],
                vec![1, 2, 3],
                4,
            ),
            (
                "step",
                None,
                Some("2"),
                None,
                vec![4, 2, 0],
                vec![0, 2, 4],
                6,
            ),
            (
                "shuffle",
                None,
                None,
                Some("2 1 0"),
                vec![3, 4, 5, 0, 1, 2],
                vec![0, 1, 2, 3, 4, 5],
                6,
            ),
        ] {
            let transform = LosslessTransform {
                reverse: Some(String::new()),
                interval,
                framestep: step.map(String::from),
                shuffleframes: shuffle.map(String::from),
                ..Default::default()
            };
            let options = CopyOptions::default();
            assert!(supports(&source, &transform, &options));
            let plan = crate::plan_transcode_lossless(&source, &transform, &options, None).unwrap();
            assert!(plan.notes[0].starts_with("backend: owned"));
            let output = std::env::temp_dir().join(format!(
                "fvid-reverse-export-{}-{case}.mkv",
                std::process::id()
            ));
            let stats = crate::transcode_lossless(&source, &output, transform, &options).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.decoded_frames, consumed);
            assert_eq!(stats.video_frames, values.len() as u64);
            let mut reader = crate::owned_webm::WebmReader::open(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            reader.scan_all().unwrap();
            assert_eq!(reader.packets.len(), values.len());
            let origin = interval.map_or(0, |(from, _)| from as i64 * 1000);
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(4, 4, 1 << 20).unwrap();
            for (slot, (value, position)) in values.into_iter().zip(positions).enumerate() {
                assert_eq!(
                    reader.packets[slot].pts_ns,
                    position as i64 * 250_000_000 - origin
                );
                assert_eq!(reader.packets[slot].duration_ns, Some(250_000_000));
                let mut expected = vec![10 + value; 16];
                expected.extend([128; 8]);
                assert_eq!(
                    decoder
                        .decode(&reader.read_packet(slot).unwrap())
                        .unwrap()
                        .frame
                        .data,
                    expected
                );
            }
            drop(reader);
            std::fs::remove_file(output).unwrap();
        }
    }
    #[test]
    fn reverse_rejects_damaged_input_before_any_final_file_is_published() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/reverse-truncated-last-frame.y4m");
        let output =
            std::env::temp_dir().join(format!("fvid-reverse-damaged-{}.mkv", std::process::id()));
        let transform = LosslessTransform {
            reverse: Some(String::new()),
            ..Default::default()
        };
        assert!(supports(&source, &transform, &CopyOptions::default()));
        let error = crate::transcode_lossless(&source, &output, transform, &CopyOptions::default())
            .unwrap_err();
        assert!(error.contains("truncated Y4M frame payload"), "{error}");
        assert!(!output.exists());
    }
}
