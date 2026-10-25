//! Public media API, available in builds without FFmpeg.
//! Owned codecs and containers handle native routes. The optional `media` feature
//! retains the legacy adapter for operations whose migration is still incomplete.
pub use fvid_media::*;
// Public operation contracts belong to FVid, independently of legacy execution.
pub use crate::media_info::{
    AudioDecodeStats, AudioDecodeTransform, Capabilities, ChapterInfo, CopyStats,
    CropRect, DecodeStats, DecodeTransform, EncoderSettings, LosslessStats,
    LosslessTransform, LoudnessStats, LoudnormStats, MediaInfo, MediaPlan,
    MergeAudioStats, MixAudioOptions, MixAudioStats, MixDuration, OverlaySpec,
    PadRect, PcmTrimStats, PlanStep, PlanStream, RotateAngle, ScaleSize, StreamInfo,
    SubtitleCodec, SubtitleConvertOptions, TransposeMode, XfadeSpec, parse_time,
    DEFAULT_LOUDNORM_ARGS, validate_loudnorm_args,
};
pub use crate::media_info::NativeSubtitleStats as SubtitleConvertStats;
pub use crate::media_control::{CancelFlag, CopyOptions, ProgressEvent, ProgressHook};
pub type Result<T> = std::result::Result<T, String>;
pub use crate::native_export::{export_y4m, export_y4m_interval, export_y4m_transformed};

pub fn plan_overlay(source:&std::path::Path,overlay:&std::path::Path,x:i32,y:i32,options:&CopyOptions)->Result<MediaPlan> {
    if crate::native_export::overlay_eligible(source).map_err(|e|e.to_string())? && validate_native_copy_options(options,false).is_ok() {
        return crate::native_plan::overlay(source,overlay,i64::from(x),i64::from(y));
    }
    fvid_media::plan_overlay(source,overlay,x,y,options)
}

/// Owned opaque overlay for compatible main inputs; retains supported main audio.
pub fn overlay_video(source:&std::path::Path,overlay:&std::path::Path,destination:&std::path::Path,x:i32,y:i32,options:&CopyOptions)->Result<LosslessStats> {
    if crate::native_export::overlay_eligible(source).map_err(|e|e.to_string())? && validate_native_copy_options(options,false).is_ok() {
        return crate::native_export::overlay_video(source,overlay,destination,i64::from(x),i64::from(y),options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string());
    }
    fvid_media::overlay_video(source,overlay,destination,x,y,options)
}

pub fn convert_subtitles(
    source: &std::path::Path,
    destination: &std::path::Path,
    options: &SubtitleConvertOptions,
) -> Result<SubtitleConvertStats> {
    convert_subtitles_with_controls(source, destination, options, &Default::default())
}

pub fn convert_subtitles_with_controls(
    source: &std::path::Path,
    destination: &std::path::Path,
    options: &SubtitleConvertOptions,
    controls: &CopyOptions,
) -> Result<SubtitleConvertStats> {
    if let Some(stats) =
        crate::native_subtitle::try_convert_with_controls(source, destination, options, controls)
            .map_err(|e| e.to_string())?
    {
        return Ok(stats);
    }
    fvid_media::convert_subtitles_with_controls(source, destination, options, controls)
}

pub fn plan_convert_subtitles(
    source: &std::path::Path,
    options: &SubtitleConvertOptions,
) -> Result<MediaPlan> {
    plan_convert_subtitles_with_controls(source, options, &Default::default())
}

pub fn plan_convert_subtitles_with_controls(
    source: &std::path::Path,
    options: &SubtitleConvertOptions,
    controls: &CopyOptions,
) -> Result<MediaPlan> {
    plan_convert_subtitles_for_destination(source, None, options, controls)
}

pub fn plan_convert_subtitles_for_destination(
    source: &std::path::Path,
    destination: Option<&std::path::Path>,
    options: &SubtitleConvertOptions,
    controls: &CopyOptions,
) -> Result<MediaPlan> {
    if let Some(plan) =
        crate::native_subtitle::try_plan_with_controls(source, destination, options, controls)
            .map_err(|e| e.to_string())?
    {
        return Ok(plan);
    }
    if let Some(dest) = destination
        && dest.extension().and_then(|s| s.to_str()) != Some("mkv")
    {
        return Err("convert-subtitles requires Matroska (.mkv) output".into());
    }
    fvid_media::plan_convert_subtitles_with_controls(source, options, controls)
}

pub fn decode_video(source: &std::path::Path) -> Result<DecodeStats> {
    decode_video_interval(source, None)
}

/// Native half-open presentation interval; pre-roll reference frames are not counted.
pub fn decode_video_interval(
    source: &std::path::Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
) -> Result<DecodeStats> {
    crate::native_media::decode_video_interval(source, interval).map_err(|e| e.to_string())
}


/// Geometry-only decoding uses owned codecs and sample planes. Remaining filter
/// operations retain their existing adapter until their native migration.
pub fn decode_video_transformed(source: &std::path::Path, transform: DecodeTransform) -> Result<DecodeStats> {
    if !crate::native_media::supports_deband_source(source,transform.deband.as_deref()).map_err(|e|e.to_string())?
        || !crate::native_media::supports_video_request(&transform)
        || ((transform.sab.is_some() || transform.smartblur.is_some() || transform.bilateral.is_some() || transform.gblur.is_some() || transform.pixelize.is_some() || transform.chromashift.is_some() || transform.avgblur.is_some() || transform.boxblur.is_some()) && !crate::native_media::supports_plane_filter_source(source).map_err(|e|e.to_string())?) {
        return fvid_media::decode_video_transformed(source, transform);
    }
    crate::native_media::decode_video_request(source, &transform).map_err(|e|e.to_string())
}

/// Export AAC or packed WAVE PCM through the owned audio pipeline.
/// Other codecs retain the legacy adapter until their migration is complete.
pub fn decode_audio(
    source: &std::path::Path,
    destination: &std::path::Path,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_audio_transformed(source, destination, AudioDecodeTransform::default(), options)
}

pub fn decode_audio_interval(
    source: &std::path::Path,
    destination: &std::path::Path,
    interval: Option<(i64, i64)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_audio_transformed(source, destination, AudioDecodeTransform {
        interval, ..Default::default()
    }, options)
}

fn has_metadata_mutations(options: &CopyOptions) -> bool {
    !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
}

fn has_only_wave_info_metadata(options: &CopyOptions) -> bool {
    options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
        && options
            .metadata_set
            .iter()
            .all(|(k, _)| fvid_media::owned_wave_metadata::tag_for_key(k).is_ok())
        && options
            .metadata_delete
            .iter()
            .all(|k| fvid_media::owned_wave_metadata::tag_for_key(k).is_ok())
}

fn uses_native_audio_route(source: &std::path::Path, options: &CopyOptions, transform: &AudioDecodeTransform) -> Result<bool> {
    if options.max_controlled_bytes.is_some() || has_metadata_mutations(options) {
        return Ok(false);
    }
    if crate::native_pcm::is_wave(source).map_err(|e| e.to_string())? {
        return Ok(true);
    }
    // The library spool owns configuration changes, normalization and PCM
    // precision. Keep the public API and CLI on that same implementation.
    {
        use std::io::Read;
        let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
        let mut prefix = [0u8; 8];
        match input.read_exact(&mut prefix) {
            Ok(()) if crate::container::mp4::recognizes_prefix(&prefix) => {
                use std::io::{Seek, SeekFrom};
                input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
                let reader = crate::container::mp4::Mp4Reader::open_indexed(
                    std::io::BufReader::new(input),
                    crate::container::mp4::Limits::default(),
                ).map_err(|e| e.to_string())?;
                if reader.tracks().iter().any(|t| t.ensure_constant_description().is_err()) {
                    return Ok(false);
                }
                if transform.channels.is_some_and(|c|reader.tracks().iter().enumerate().any(|(i,t)|t.handler==*b"soun"&&(options.streams.is_empty()||options.streams.contains(&i))&&t.description_for_sample(0).is_some_and(|d|d.channels>2&&c!=i32::from(d.channels)))) {
                    return Ok(false);
                }
            }
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let interval = transform.interval.and_then(|(from, to)| {
        (from >= 0 && to > from).then(|| {
            (
                std::time::Duration::from_micros(from as u64),
                std::time::Duration::from_micros(to as u64),
            )
        })
    });
    if !crate::native_media::is_owned_audio_trim_source_with_interval_and_options(source, interval, options).map_err(|e| e.to_string())? {
        return Ok(false);
    }
    if options.streams.len() == 1 {
        return Ok(
            crate::native_media::audio_source_info_selected_with_interval_and_options(source, Some(options.streams[0]), interval, options)
                .is_ok(),
        );
    }
    Ok(true)
}

pub(crate) struct SpoolWave(pub(crate) std::path::PathBuf);
impl SpoolWave {
    pub(crate) fn create() -> Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "fvid-owned-audio-spool-{}-{}.wav",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Ok(Self(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("cannot reserve private WAVE spool".into())
    }
}
impl Drop for SpoolWave {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(crate) fn decode_fvid_media_audio_to_destination(
    source: &std::path::Path,
    destination: &std::path::Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    let ext = destination.extension().and_then(|s| s.to_str());
    if !matches!(ext, Some("f32le" | "mka" | "mkv")) {
        return fvid_media::decode_audio_transformed(source, destination, transform, options);
    }
    let matroska_out = matches!(ext, Some("mka" | "mkv"));
    if has_metadata_mutations(options) {
        if !matroska_out {
            return Err("native audio does not yet support requested allocation budget or metadata/stream mutations".into());
        }
        let mut meta_options = options.clone();
        meta_options.max_controlled_bytes = None;
        validate_owned_audio_options_for_destination(&meta_options, Some(destination))?;
    }
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    let spool = SpoolWave::create()?;
    let mut spool_options = options.clone();
    spool_options.metadata_set.clear();
    spool_options.metadata_delete.clear();
    spool_options.stream_metadata_set.clear();
    spool_options.stream_metadata_delete.clear();
    let original_progress = spool_options.progress.take();
    let decoded = fvid_media::decode_audio_transformed(source, &spool.0, transform, &spool_options)?;
    let mut publish_options = CopyOptions::default();
    publish_options.cancel = options.cancel.clone();
    publish_options.max_rss_bytes = options.max_rss_bytes;
    publish_options.progress = original_progress;
    if matroska_out {
        publish_options.metadata_set = options.metadata_set.clone();
        publish_options.metadata_delete = options.metadata_delete.clone();
        publish_options.stream_metadata_set = options.stream_metadata_set.clone();
        publish_options.stream_metadata_delete = options.stream_metadata_delete.clone();
    }
    let source_matroska_metadata = if matroska_out {
        let interval = transform
            .interval
            .filter(|&(from, to)| from >= 0 && to > from)
            .map(|(from, to)| {
                (
                    std::time::Duration::from_micros(from as u64),
                    std::time::Duration::from_micros(to as u64),
                )
            });
        Some(
            crate::native_export::pcm_matroska::extract_source_pcm_metadata_with_interval_and_packet_limit(
                source,
                options.streams.first().copied(),
                true,
                interval,
                Some(options.max_packet_bytes),
                options.max_packets,
                options.cancel.as_ref(),
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let published = crate::native_export::export_audio_pcm_transformed_with_source_metadata(
        &spool.0,
        destination,
        None,
        1.0,
        None,
        None,
        &publish_options,
        source_matroska_metadata,
    )
    .map_err(|e| e.to_string())?;
    Ok(AudioDecodeStats {
        sample_frames: published.sample_frames,
        decoded_frames: decoded.decoded_frames,
        sample_rate: published.sample_rate as i32,
        channels: i32::from(published.channels),
        sample_format: "flt".into(),
        planar_interleave_bytes: 0,
        decode_errors: decoded.decode_errors,
    })
}

/// AAC, MP4 ALAC and WAVE exports use native float PCM, including source edit-list trimming.
/// Explicit options without a native implementation are rejected, never ignored.
pub fn decode_audio_transformed(
    source: &std::path::Path,
    destination: &std::path::Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if !uses_native_audio_route(source, options, &transform)? {
        return decode_fvid_media_audio_to_destination(source, destination, transform, options);
    }
    validate_owned_audio_options_for_destination(options, Some(destination))?;
    let interval = transform.interval.map(|(from, to)| {
        if from < 0 || to <= from {
            return Err("decode-audio interval requires 0 <= from < to".to_owned());
        }
        Ok((std::time::Duration::from_micros(from as u64),
            std::time::Duration::from_micros(to as u64)))
    }).transpose()?;
    let channels = transform.channels.map(|n| u16::try_from(n)
        .map_err(|_| "invalid channel count".to_owned())).transpose()?;
    let sample_rate = transform.sample_rate.map(|n| u32::try_from(n)
        .map_err(|_| "invalid sample rate".to_owned())).transpose()?;
    let stats = crate::native_export::export_audio_pcm_transformed_with_controls(
        source, destination, interval, transform.volume.unwrap_or(1.0),
        channels, sample_rate, options,
    ).map_err(|e| e.to_string())?;
    Ok(AudioDecodeStats {
        sample_frames: stats.sample_frames,
        decoded_frames: stats.decoded_frames,
        sample_rate: stats.sample_rate as i32,
        channels: i32::from(stats.channels),
        sample_format: "flt".into(),
        planar_interleave_bytes: 0,
        decode_errors: 0,
    })
}

pub(crate) fn validate_owned_audio_options(options: &CopyOptions) -> Result<()> {
    validate_owned_audio_options_for_destination(options, None)
}

pub(crate) fn validate_owned_audio_options_for_destination(
    options: &CopyOptions,
    destination: Option<&std::path::Path>,
) -> Result<()> {
    let matroska_out = destination.is_some_and(|d| {
        matches!(d.extension().and_then(|s| s.to_str()), Some("mka" | "mkv"))
    });
    if options.streams.len() > 1
        || options.max_controlled_bytes.is_some()
        || (!matroska_out && has_metadata_mutations(options))
    {
        return Err("native audio does not yet support requested allocation budget or metadata/stream mutations".into());
    }
    if matroska_out && has_metadata_mutations(options) {
        crate::native_export::pcm_matroska::validate_matroska_pcm_metadata(Some(options))
            .map_err(|e| e.to_string())?;
    }
    if options.max_packet_bytes == 0 { return Err("packet byte limit must be positive".into()); }
    if options.max_packets == Some(0) { return Err("packet work limit requires at least one packet".into()); }
    fvid_media::owned_budget::check_rss_budget(options)
}

fn validate_native_copy_options(options: &CopyOptions, audio_selection: bool) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(crate::media_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    if (if audio_selection { options.streams.len() > 1 } else { !options.streams.is_empty() })
        || options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err("native media operation does not yet support stream selection, custom budgets or metadata mutations".into());
    }
    Ok(())
}

fn validate_matroska_copy_options(options: &CopyOptions) -> Result<()> {
    if options.max_packet_bytes == 0 { return Err("packet byte limit must be positive".into()); }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    let mut remaining = options.clone();
    remaining.max_packet_bytes = CopyOptions::default().max_packet_bytes;
    remaining.max_controlled_bytes = None;
    remaining.max_rss_bytes = None;
    validate_native_copy_options(&remaining, false)
}

fn validate_mp4_relocate_options(options: &CopyOptions) -> Result<()> {
    if options.max_packet_bytes == 0 { return Err("packet byte limit must be positive".into()); }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)?;
    let mut remaining = options.clone();
    remaining.max_packet_bytes = CopyOptions::default().max_packet_bytes;
    remaining.max_controlled_bytes = None;
    remaining.max_rss_bytes = None;
    validate_native_copy_options(&remaining, false)
}

fn validate_mp4_packet_budget(source: &std::path::Path, options: &CopyOptions) -> Result<()> {
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
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
    Ok(())
}

/// Plan AAC or WAVE decoding using owned metadata and decoder configuration.
/// Packet contents and timeline consistency are checked during execution.
pub fn plan_decode_audio(
    source: &std::path::Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if !uses_native_audio_route(source, options, transform)? {
        if has_metadata_mutations(options) && !has_only_wave_info_metadata(options) {
            let mut meta_options = options.clone();
            meta_options.max_controlled_bytes = None;
            validate_owned_audio_options_for_destination(
                &meta_options,
                Some(std::path::Path::new("planned.mka")),
            )?;
            let mut stripped = options.clone();
            stripped.metadata_set.clear();
            stripped.metadata_delete.clear();
            stripped.stream_metadata_set.clear();
            stripped.stream_metadata_delete.clear();
            let mut plan = fvid_media::plan_decode_audio(source, transform, &stripped)?;
            plan.steps.push(PlanStep {
                action: "metadata".into(),
                detail: "apply Matroska container and stream 0 metadata mutations".into(),
            });
            return Ok(plan);
        }
        return fvid_media::plan_decode_audio(source, transform, options);
    }
    validate_owned_audio_options(options)?;
    crate::native_plan::decode_audio_with_options(source, transform, options)
}

fn validate_adts_copy_options_with_metadata(
    options: &CopyOptions,
    audio_selection: bool,
    allow_matroska_metadata: bool,
) -> Result<()> {
    if (if audio_selection {
        options.streams.len() > 1
    } else {
        !options.streams.is_empty()
    }) || (!allow_matroska_metadata && has_metadata_mutations(options))
    {
        return Err("native media operation does not yet support stream selection, custom budgets or metadata mutations".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    if allow_matroska_metadata && has_metadata_mutations(options) {
        crate::container::matroska_write::extract_adts_matroska_metadata(Some(options))
            .map_err(|e| e.to_string())?;
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_rss_bytes.is_some()
        && fvid_media::owned_budget::check_rss_budget(options).is_err()
    {
        return Err("native media operation does not yet support stream selection, custom budgets or metadata mutations".into());
    }
    Ok(())
}

fn validate_concat_matroska_options(options: &CopyOptions) -> Result<()> {
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    fvid_media::owned_budget::check_rss_budget(options)
}

/// Native ADTS-to-MP4/Matroska muxing and MP4 fast-start relocation. Other container
/// conversions retain the legacy adapter until their owned muxers are available.
/// For opaque MP4 relocation, packets=0 means uncounted; payload_bytes counts
/// mdat bytes. fvid_payload_copies counts additional per-packet payload clones,
/// not file I/O buffering (MP4 relocation does not create packet objects).
pub fn remux(
    source: &std::path::Path,
    destination: &std::path::Path,
    options: &CopyOptions,
) -> Result<CopyStats> {
    use std::io::Read;
    if !matches!(destination.extension().and_then(|s| s.to_str()), Some("mp4" | "m4a" | "mka" | "mkv")) {
        return fvid_media::remux(source, destination, options);
    }
    let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut prefix = [0; 8];
    input.read_exact(&mut prefix).map_err(|e| e.to_string())?;
    let adts = crate::container::adts::header(&prefix).is_some();
    let mp4 = crate::container::mp4::recognizes_prefix(&prefix);
    let matroska = matches!(destination.extension().and_then(|s|s.to_str()),Some("mka"|"mkv"));
    let matroska_copy = matroska && prefix[..4] == [0x1a,0x45,0xdf,0xa3];
    if matroska_copy && validate_matroska_copy_options(options).is_err() {return fvid_media::remux(source,destination,options);}
    let mp4_matroska = mp4 && matroska && crate::native_export::is_native_mp4_matroska(source).map_err(|e| e.to_string())?;
    if !adts && !mp4_matroska && !matroska_copy && (matroska || !mp4) { return fvid_media::remux(source, destination, options); }
    // Preserve unmigrated metadata mutations/stream options on the legacy path.
    if mp4_matroska && validate_native_copy_options(options, false).is_err() {
        let mut stats = fvid_media::remux(source, destination, options)?;
        if stats.backend == "owned Matroska" {
            stats.backend = "fvid";
            stats.fvid_payload_copies = 0;
        }
        return Ok(stats);
    }
    if matroska_copy {
        validate_matroska_copy_options(options)?;
    } else if adts {
        if options.streams.iter().any(|&s| s != 0) {
            return Err("ADTS has only stream 0".into());
        }
        validate_adts_copy_options_with_metadata(options, true, matroska)?;
    } else if mp4_matroska {
        validate_native_copy_options(options, false)?;
    } else {
        validate_mp4_relocate_options(options)?;
        validate_mp4_packet_budget(source, options)?;
    }
    let stats = if matroska_copy {
        crate::native_export::remux_matroska_with_packet_and_memory_limits(
            source,
            destination,
            options.cancel.as_ref(),
            options.progress.as_ref(),
            options.max_packet_bytes,
            options.max_controlled_bytes,
        )
    } else if mp4_matroska {
        crate::native_export::remux_mp4_matroska(source, destination,
            options.cancel.as_ref(), options.progress.as_ref())
    } else if adts {
        crate::native_export::remux_adts_aac_with_options(source, destination, options)
    } else {
        crate::native_export::remux_mp4_stats_with_memory_limit(
            source,
            destination,
            options.cancel.as_ref(),
            options.progress.as_ref(),
            options.max_controlled_bytes,
        )
    }.map_err(|e| e.to_string())?;
    Ok(CopyStats { packets: stats.packets, payload_bytes: stats.payload_bytes,
        segments: 1, backend: "fvid", fvid_payload_copies: 0 })
}

/// Own parsers describe supported containers; unmigrated formats retain the
/// legacy adapter. A native parsing error never falls back to another parser.
pub fn probe(source: &std::path::Path) -> Result<MediaInfo> { probe_as(source, None) }
pub fn probe_as(source: &std::path::Path, format: Option<&str>) -> Result<MediaInfo> {
    match crate::native_probe::try_probe_as(source, format)? {
        Some(info) => Ok(info),
        None => fvid_media::probe_as(source, format),
    }
}

/// Sample-exact RIFF PCM uses the owned streaming slicer and publisher.
pub fn trim_pcm(source:&std::path::Path,destination:&std::path::Path,from:i64,to:i64,options:&CopyOptions)->Result<PcmTrimStats> {
    if !crate::native_pcm::is_wave(source).map_err(|e|e.to_string())? {
        return fvid_media::trim_pcm(source,destination,from,to,options);
    }
    if options.streams.iter().any(|&n|n!=0) {return Err("WAVE has only stream 0".into());}
    if validate_native_copy_options(options,true).is_err() {
        return fvid_media::trim_pcm(source,destination,from,to,options);
    }
    crate::native_pcm::trim_wave(source,destination,from,to,options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string())
}

/// Plan owned RIFF PCM slicing; other containers retain the legacy adapter.
pub fn plan_trim_pcm(
    source: &std::path::Path,
    from: i64,
    to: i64,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if !crate::native_pcm::is_wave(source).map_err(|e| e.to_string())? {
        return fvid_media::plan_trim_pcm(source, from, to, options);
    }
    if options.streams.iter().any(|&n| n != 0) {
        return Err("WAVE has only stream 0".into());
    }
    if validate_native_copy_options(options, true).is_err() {
        return fvid_media::plan_trim_pcm(source, from, to, options);
    }
    crate::native_plan::trim_pcm(source, from, to, options.streams.first().copied())
}

/// General trim of a PCM WAVE file shares the exact native sample-slicing path.
pub fn trim(
    source: &std::path::Path,
    destination: &std::path::Path,
    from: i64,
    to: i64,
    options: &CopyOptions,
) -> Result<CopyStats> {
    if destination.extension().and_then(|s|s.to_str())==Some("y4m") {
        let stats=crate::native_export::trim_y4m_with_options(source,destination,from,to,options).map_err(|e|e.to_string())?;
        return Ok(CopyStats {packets:stats.packets,payload_bytes:stats.payload_bytes,segments:1,backend:"fvid",fvid_payload_copies:0});
    }
    let trim_interval = (from >= 0 && to > from).then(|| {
        (
            std::time::Duration::from_micros(from as u64),
            std::time::Duration::from_micros(to as u64),
        )
    });
    if options.max_controlled_bytes.is_none()
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
        && crate::native_media::is_owned_audio_trim_source_with_interval_and_options(source, trim_interval, options).map_err(|e|e.to_string())?
        && matches!(destination.extension().and_then(|s|s.to_str()),Some("wav" | "f32le" | "mka" | "mkv"))
        && (options.streams.len() != 1
            || crate::native_media::audio_source_info_selected_with_interval_and_options(source, Some(options.streams[0]), trim_interval, options)
                .is_ok()) {
        validate_owned_audio_options(options)?;
        let stats=crate::native_export::trim_audio_pcm_with_controls(source,destination,from,to,options).map_err(|e|e.to_string())?;
        return Ok(CopyStats {packets:stats.decoded_frames,payload_bytes:stats.sample_frames*u64::from(stats.channels)*4,segments:1,backend:"fvid",fvid_payload_copies:0});
    }
    if !crate::native_pcm::is_wave(source).map_err(|e| e.to_string())? {
        if matches!(destination.extension().and_then(|s| s.to_str()), Some("wav" | "f32le" | "mka" | "mkv")) {
            let stats = decode_fvid_media_audio_to_destination(
                source,
                destination,
                AudioDecodeTransform { interval: Some((from, to)), ..Default::default() },
                options,
            )?;
            return Ok(CopyStats {
                packets: stats.decoded_frames,
                payload_bytes: stats
                    .sample_frames
                    .saturating_mul(u64::try_from(stats.channels).unwrap_or(0))
                    .saturating_mul(4),
                segments: 1,
                backend: "fvid",
                fvid_payload_copies: 0,
            });
        }
        return fvid_media::trim(source, destination, from, to, options);
    }
    if matches!(destination.extension().and_then(|s| s.to_str()), Some("f32le" | "mka" | "mkv")) {
        let stats = decode_fvid_media_audio_to_destination(
            source,
            destination,
            AudioDecodeTransform { interval: Some((from, to)), ..Default::default() },
            options,
        )?;
        return Ok(CopyStats {
            packets: stats.decoded_frames,
            payload_bytes: stats
                .sample_frames
                .saturating_mul(u64::try_from(stats.channels).unwrap_or(0))
                .saturating_mul(4),
            segments: 1,
            backend: "fvid",
            fvid_payload_copies: 0,
        });
    }
    let stats = trim_pcm(source, destination, from, to, options)?;
    Ok(CopyStats {
        packets: stats.packets,
        payload_bytes: stats.payload_bytes,
        segments: 1,
        backend: "fvid",
        fvid_payload_copies: stats.fvid_payload_copies,
    })
}
pub fn plan_trim(
    source: &std::path::Path,
    from: i64,
    to: i64,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    let trim_interval = (from >= 0 && to > from).then(|| {
        (
            std::time::Duration::from_micros(from as u64),
            std::time::Duration::from_micros(to as u64),
        )
    });
    if options.max_controlled_bytes.is_none()
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
        && crate::native_media::is_owned_audio_trim_source_with_interval_and_options(source, trim_interval, options).map_err(|e|e.to_string())?
        && (options.streams.len() != 1
            || crate::native_media::audio_source_info_selected_with_interval_and_options(source, Some(options.streams[0]), trim_interval, options)
                .is_ok()) {
        validate_owned_audio_options(options)?;
        return crate::native_plan::trim_audio_with_options(source,from,to,options);
    }
    if has_metadata_mutations(options) && !has_only_wave_info_metadata(options) {
        let mut planned = plan_decode_audio(
            source,
            &AudioDecodeTransform { interval: Some((from, to)), ..Default::default() },
            options,
        )?;
        planned.command = "trim".into();
        return Ok(planned);
    }
    if !crate::native_pcm::is_wave(source).map_err(|e| e.to_string())? {
        return fvid_media::plan_trim(source, from, to, options);
    }
    let mut plan = plan_trim_pcm(source, from, to, options)?;
    plan.command = "trim".into();
    Ok(plan)
}

/// Plan owned ADTS/MP4 remuxing with the same option restrictions as execution.
pub fn plan_remux(source: &std::path::Path, options: &CopyOptions) -> Result<MediaPlan> {
    if crate::native_export::is_matroska_source(source).map_err(|e|e.to_string())? {
        if validate_matroska_copy_options(options).is_err() {return fvid_media::plan_remux(source,options);}
        return crate::native_plan::remux_with_options(source, options)?
            .ok_or_else(|| "missing Matroska remux plan".into());
    }
    if crate::native_export::is_adts_source(source).map_err(|e| e.to_string())? {
        if validate_native_copy_options(options, false).is_ok() {
            return crate::native_plan::remux_with_options(source, options)?
                .ok_or_else(|| "missing ADTS remux plan".into());
        }
        if options.streams.iter().any(|&s| s != 0) {
            return Err("ADTS has only stream 0".into());
        }
        validate_adts_copy_options_with_metadata(options, false, true)?;
        return crate::native_plan::remux_adts_with_options(source, options);
    }
    if crate::native_export::is_native_mp4_matroska(source).unwrap_or(false)
        && validate_native_copy_options(options, false).is_err()
    {
        return fvid_media::plan_remux(source, options);
    }
    match crate::native_plan::remux_with_options(source, options)? {
        Some(plan) => { validate_mp4_relocate_options(options)?; Ok(plan) }
        None => fvid_media::plan_remux(source, options),
    }
}

/// Concatenate compatible packed WAVE PCM through the owned streaming writer.
pub fn concat(sources: &[std::path::PathBuf], destination: &std::path::Path, options: &CopyOptions) -> Result<CopyStats> {
    if destination.extension().and_then(|s|s.to_str())==Some("y4m") {
        let stats=crate::native_export::concat_y4m_with_options(sources,destination,options).map_err(|e|e.to_string())?;
        return Ok(CopyStats {packets:stats.packets,payload_bytes:stats.payload_bytes,segments:sources.len(),backend:"fvid",fvid_payload_copies:0});
    }
    if validate_concat_matroska_options(options).is_ok()
        && let Some(stats)=crate::native_export::try_concat_mp4_matroska_with_options(sources,destination,options).map_err(|e|e.to_string())? {
            return Ok(CopyStats {packets:stats.packets,payload_bytes:stats.payload_bytes,segments:sources.len(),backend:"fvid",fvid_payload_copies:0});
        }
    if sources.first().map(|p|crate::native_export::is_adts_source(p)).transpose().map_err(|e|e.to_string())?.unwrap_or(false) {
        if options.streams.iter().any(|&s|s!=0) {return Err("ADTS has only stream 0".into());}
        let matroska_out = matches!(destination.extension().and_then(|s| s.to_str()), Some("mka" | "mkv"));
        validate_adts_copy_options_with_metadata(options, true, matroska_out)?;
        let stats=crate::native_export::concat_adts_aac_with_options(sources,destination,options).map_err(|e|e.to_string())?;
        return Ok(CopyStats {packets:stats.packets,payload_bytes:stats.payload_bytes,segments:sources.len(),backend:"fvid",fvid_payload_copies:0});
    }
    if !sources.first().map(|p|crate::native_pcm::is_wave(p)).transpose().map_err(|e|e.to_string())?.unwrap_or(false) {
        return fvid_media::concat(sources,destination,options);
    }
    if options.streams.iter().any(|&s|s!=0) {return Err("WAVE has only stream 0".into());}
    if validate_native_copy_options(options, true).is_err() {
        let mut stats = fvid_media::concat(sources,destination,options)?;
        if stats.backend == "owned streaming WAVE concat" {
            stats.backend = "fvid";
        }
        return Ok(stats);
    }
    let stats=crate::native_pcm::concat_wave(sources,destination,options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string())?;
    Ok(CopyStats {packets:stats.packets,payload_bytes:stats.payload_bytes,segments:sources.len(),backend:"fvid",fvid_payload_copies:0})
}

pub fn plan_concat(sources: &[std::path::PathBuf], options: &CopyOptions) -> Result<MediaPlan> {
    if sources.first().map(|p|crate::native_export::is_adts_source(p)).transpose().map_err(|e|e.to_string())?.unwrap_or(false) {
        if options.streams.iter().any(|&s|s!=0) {return Err("ADTS has only stream 0".into());}
        validate_adts_copy_options_with_metadata(options, true, true)?;
        return crate::native_plan::concat_adts_with_options(sources, options);
    }
    let all_wave = sources.iter().try_fold(!sources.is_empty(), |all, source| {
        crate::native_pcm::is_wave(source).map(|wave| all && wave).map_err(|e| e.to_string())
    })?;
    if all_wave && crate::native_pcm::concat_info(sources).is_ok() {
        if options.streams.iter().any(|&s|s!=0) {return Err("WAVE has only stream 0".into());}
        if validate_native_copy_options(options, true).is_err() {
            if !(has_metadata_mutations(options) && !has_only_wave_info_metadata(options)) {
                return fvid_media::plan_concat(sources, options);
            }
            if let Ok(plan) = fvid_media::plan_concat(sources, options) {
                return Ok(plan);
            }
        } else {
            return crate::native_plan::concat_wave(sources);
        }
    }
    if validate_concat_matroska_options(options).is_ok()
        && let Some(plan)=crate::native_plan::concat_mp4_matroska_with_options(sources, options)? { return Ok(plan); }
    if !sources.first().map(|p|crate::native_pcm::is_wave(p)).transpose().map_err(|e|e.to_string())?.unwrap_or(false) {
        return fvid_media::plan_concat(sources,options);
    }
    if options.streams.iter().any(|&s|s!=0) {return Err("WAVE has only stream 0".into());}
    if validate_native_copy_options(options, true).is_err() {
        return fvid_media::plan_concat(sources,options);
    }
    crate::native_plan::concat_wave(sources)
}

/// Plan decoded video concatenation with an explicit Y4M output contract.
pub fn plan_concat_y4m(sources: &[std::path::PathBuf], options: &CopyOptions) -> Result<MediaPlan> {
    crate::native_plan::concat_y4m_with_options(sources,options)
}

/// Plan an explicitly requested decoded Y4M video trim.
pub fn plan_trim_y4m(source: &std::path::Path, from: i64, to: i64, options: &CopyOptions) -> Result<MediaPlan> {
    crate::native_plan::trim_y4m_with_options(source,from,to,options)
}

/// Crop through the same owned FFV1 path as general lossless transforms.
pub fn crop_lossless(source: &std::path::Path, destination: &std::path::Path,
    crop: CropRect, options: &CopyOptions) -> Result<LosslessStats> {
    transcode_lossless(source,destination,LosslessTransform {crop:Some(crop),..Default::default()},options)
}

/// Owned spatial FFV1 export for Y4M and MP4 video, retaining all supported AAC tracks.
pub fn transcode_lossless(source:&std::path::Path,destination:&std::path::Path,transform:LosslessTransform,options:&CopyOptions)->Result<LosslessStats> {
    if !crate::native_media::supports_deband_source(source,transform.deband.as_deref()).map_err(|e|e.to_string())? {return fvid_media::transcode_lossless(source,destination,transform,options);}
    if crate::native_lossless::supports_framestep(&transform)
        && validate_native_copy_options(options,false).is_ok()
        && crate::native_lossless::eligible(source).map_err(|e|e.to_string())? {
        let step=fvid_media::owned_framestep::FrameStep::parse(transform.framestep.as_deref().unwrap())?;
        let mut spatial=transform.clone(); spatial.framestep=None;
        let (geometry,filters)=crate::native_lossless::configuration(&spatial).map_err(|e|e.to_string())?;
        return crate::native_export::transcode_ffv1_selected(source,destination,&geometry,&filters,options.cancel.as_ref(),options.progress.as_ref(),Some(step)).map_err(|e|e.to_string());
    }
    if crate::native_lossless::supports_overlay(&transform) {
        let spec=transform.overlay.as_ref().unwrap();
        if validate_native_copy_options(options,false).is_ok() && crate::native_export::overlay_eligible(source).map_err(|e|e.to_string())? {
            let mut remaining=transform.clone();remaining.overlay=None;
            let (geometry,filters)=crate::native_lossless::configuration(&remaining).map_err(|e|e.to_string())?;
            return crate::native_export::overlay_video_transformed(source,&spec.path,destination,i64::from(spec.x),i64::from(spec.y),options.cancel.as_ref(),options.progress.as_ref(),&geometry,&filters).map_err(|e|e.to_string());
        }
    }
    if crate::native_lossless::supports(&transform) && validate_native_copy_options(options,false).is_ok() && crate::native_lossless::eligible(source).map_err(|e|e.to_string())? {
        let (geometry,filters)=crate::native_lossless::configuration(&transform).map_err(|e|e.to_string())?;
        return crate::native_export::transcode_ffv1_transformed(source,destination,&geometry,&filters,options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string());
    }
    fvid_media::transcode_lossless(source,destination,transform,options)
}

/// Explicit settings supported by the owned FFV1 encoder share its exporter.
pub fn transcode(source: &std::path::Path, destination: &std::path::Path,
    transform: LosslessTransform, options: &CopyOptions, settings: &EncoderSettings) -> Result<LosslessStats> {
    settings.validate()?;
    if !crate::native_media::supports_deband_source(source,transform.deband.as_deref()).map_err(|e|e.to_string())? {return fvid_media::transcode(source,destination,transform,options,settings);}
    if fvid_media::owned_lossless::supports_encoder(settings)
        && (crate::native_lossless::supports(&transform) || crate::native_lossless::supports_overlay(&transform) || crate::native_lossless::supports_framestep(&transform))
        && validate_native_copy_options(options,false).is_ok()
        && crate::native_lossless::eligible(source).map_err(|e|e.to_string())? {
        return transcode_lossless(source,destination,transform,options);
    }
    fvid_media::transcode(source,destination,transform,options,settings)
}

/// Plan eligible FFV1 exports without opening the legacy demuxer.
pub fn plan_transcode_lossless(source: &std::path::Path, transform: &LosslessTransform,
    options: &CopyOptions, encoder: Option<&str>) -> Result<MediaPlan> {
    if !crate::native_media::supports_deband_source(source,transform.deband.as_deref()).map_err(|e|e.to_string())? {return fvid_media::plan_transcode_lossless(source,transform,options,encoder);}
    if matches!(encoder,None|Some("ffv1")) && crate::native_lossless::supports_framestep(transform)
        && validate_native_copy_options(options,false).is_ok()
        && crate::native_lossless::eligible(source).map_err(|e|e.to_string())? {
        let mut spatial=transform.clone(); spatial.framestep=None;
        let mut plan=crate::native_plan::transcode_lossless(source,&spatial)?;
        let position=plan.steps.iter().position(|s|s.action=="encode").unwrap_or(plan.steps.len());
        plan.steps.insert(position,crate::native_plan::PlanStep { action:"filter".into(), detail:format!("owned framestep {} after spatial filters",transform.framestep.as_deref().unwrap()) });
        plan.notes.push(format!("owned framestep {}; retain selected frame PTS/duration and every AAC packet",transform.framestep.as_deref().unwrap()));
        return Ok(plan);
    }
    if matches!(encoder,None|Some("ffv1")) && validate_native_copy_options(options,false).is_ok()
        && crate::native_lossless::supports_overlay(transform)
            && crate::native_export::overlay_eligible(source).map_err(|e|e.to_string())? {
                return crate::native_plan::transcode_lossless(source,transform);
            }
    if matches!(encoder,None|Some("ffv1")) && crate::native_lossless::supports(transform)
        && validate_native_copy_options(options,false).is_ok()
        && crate::native_lossless::eligible(source).map_err(|e|e.to_string())? {
        return crate::native_plan::transcode_lossless(source,transform);
    }
    fvid_media::plan_transcode_lossless(source,transform,options,encoder)
}

/// Mix AAC/WAVE sources through owned decoders and a bounded-memory PCM mixer.
pub fn mix_audio(
    sources: &[std::path::PathBuf],
    destination: &std::path::Path,
    options: &MixAudioOptions,
) -> Result<MixAudioStats> {
    mix_audio_with_controls(sources, destination, options, &Default::default())
}
pub fn mix_audio_with_controls(
    sources: &[std::path::PathBuf],
    destination: &std::path::Path,
    options: &MixAudioOptions,
    controls: &CopyOptions,
) -> Result<MixAudioStats> {
    let mut default_controls = CopyOptions::default();
    default_controls.cancel = controls.cancel.clone();
    if crate::native_audio_mix::eligible_with_options(sources, controls).map_err(|e| e.to_string())?
        || crate::native_audio_mix::eligible_with_options(sources, &default_controls)
            .map_err(|e| e.to_string())?
    {
        crate::native_audio_mix::mix_audio_with_controls(sources, destination, options, controls)
            .map_err(|e| e.to_string())
    } else {
        validate_owned_audio_options(controls)?;
        fvid_media::mix_audio(sources, destination, options)
    }
}
pub fn plan_mix_audio(
    sources: &[std::path::PathBuf],
    options: &MixAudioOptions,
) -> Result<MediaPlan> {
    plan_mix_audio_with_controls(sources, options, &Default::default())
}
pub fn plan_mix_audio_with_controls(
    sources: &[std::path::PathBuf],
    options: &MixAudioOptions,
    controls: &CopyOptions,
) -> Result<MediaPlan> {
    let mut default_controls = CopyOptions::default();
    default_controls.cancel = controls.cancel.clone();
    if crate::native_audio_mix::eligible_with_options(sources, controls).map_err(|e| e.to_string())?
        || crate::native_audio_mix::eligible_with_options(sources, &default_controls)
            .map_err(|e| e.to_string())?
    {
        crate::native_audio_mix::plan_with_controls(sources, options, controls)
            .map_err(|e| e.to_string())
    } else {
        validate_owned_audio_options(controls)?;
        fvid_media::plan_mix_audio(sources, options)
    }
}

/// Merge channel vectors through owned AAC/WAVE decoders.
pub fn merge_audio(
    sources: &[std::path::PathBuf],
    destination: &std::path::Path,
) -> Result<MergeAudioStats> {
    merge_audio_with_controls(sources, destination, &Default::default())
}
pub fn merge_audio_with_controls(
    sources: &[std::path::PathBuf],
    destination: &std::path::Path,
    controls: &CopyOptions,
) -> Result<MergeAudioStats> {
    let mut default_controls = CopyOptions::default();
    default_controls.cancel = controls.cancel.clone();
    if crate::native_audio_mix::eligible_with_options(sources, controls).map_err(|e| e.to_string())?
        || crate::native_audio_mix::eligible_with_options(sources, &default_controls)
            .map_err(|e| e.to_string())?
    {
        crate::native_audio_mix::merge_audio_with_controls(sources, destination, controls)
            .map_err(|e| e.to_string())
    } else {
        validate_owned_audio_options(controls)?;
        fvid_media::merge_audio(sources, destination)
    }
}
pub fn plan_merge_audio(sources: &[std::path::PathBuf]) -> Result<MediaPlan> {
    plan_merge_audio_with_controls(sources, &Default::default())
}
pub fn plan_merge_audio_with_controls(
    sources: &[std::path::PathBuf],
    controls: &CopyOptions,
) -> Result<MediaPlan> {
    let mut default_controls = CopyOptions::default();
    default_controls.cancel = controls.cancel.clone();
    if crate::native_audio_mix::eligible_with_options(sources, controls).map_err(|e| e.to_string())?
        || crate::native_audio_mix::eligible_with_options(sources, &default_controls)
            .map_err(|e| e.to_string())?
    {
        crate::native_audio_mix::plan_merge_with_controls(sources, controls)
            .map_err(|e| e.to_string())
    } else {
        validate_owned_audio_options(controls)?;
        fvid_media::plan_merge_audio(sources)
    }
}

/// Owned integrated loudness, LRA and sample peak. True peak is not inferred.
pub fn measure_loudness(source: &std::path::Path, options: &CopyOptions) -> Result<crate::native_pcm::IntegratedLoudness> {
    if options.streams.len() > 1
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err("owned loudness requires at most one audio stream and no metadata edits".into());
    }
    let weights = crate::native_pcm::loudness_channel_weights_with_options(source, options)
        .map_err(|e| e.to_string())?;
    measure_loudness_with_weights(source, options, &weights)
}
/// Explicit energy weights for multichannel layouts; no implicit channel remapping.
pub fn measure_loudness_with_weights(source: &std::path::Path, options: &CopyOptions, weights: &[f64]) -> Result<crate::native_pcm::IntegratedLoudness> {
    crate::native_pcm::measure_loudness_file_with_options(source,weights,options).map_err(|e|e.to_string())
}

/// Owned metadata-only loudness plan; no libavfilter graph is created.
pub fn plan_loudness(source: &std::path::Path, options: &CopyOptions) -> Result<MediaPlan> {
    crate::native_plan::loudness_with_options(source,options,None)
}

/// Plan constant-gain normalization with an explicit sample-peak ceiling.
pub fn plan_normalize_loudness(source: &std::path::Path, options: &CopyOptions,
    weights: Option<&[f64]>, target: crate::native_pcm::NormalizeTarget) -> Result<MediaPlan> {
    crate::native_plan::normalize_loudness_with_options(source,options,weights,target)
}

/// Constant-gain normalization with typed FVid targets, owned codecs and WAVE output.
pub fn normalize_loudness(source: &std::path::Path, destination: &std::path::Path,
    options: &CopyOptions, weights: Option<&[f64]>, target: crate::media_info::NormalizeTarget) -> Result<crate::media_info::NormalizeReport> {
    normalize_loudness_controlled(source,destination,options,weights,target,None)
}

/// Phase-aware progress for owned two-pass normalization.
/// `done` is emitted only after successful atomic output publication.
pub fn normalize_loudness_controlled(source: &std::path::Path, destination: &std::path::Path,
    options: &CopyOptions, weights: Option<&[f64]>, target: crate::media_info::NormalizeTarget,
    progress: Option<&crate::native_pcm::NormalizationProgressHook>) -> Result<crate::media_info::NormalizeReport> {
    target.validate()?;
    if options.cancel.as_ref().is_some_and(|flag|flag.is_cancelled()) {return Err("operation cancelled".into());}
    if options.progress.is_some() {return Err("two-pass normalization requires phase-aware progress, not a single-pass hook".into());}
    let defaults;
    let weights=match weights {
        Some(weights)=>weights,
        None=>{
            let mut weight_options = options.clone();
            weight_options.metadata_set.clear();
            weight_options.metadata_delete.clear();
            weight_options.stream_metadata_set.clear();
            weight_options.stream_metadata_delete.clear();
            defaults=crate::native_pcm::loudness_channel_weights_with_options(source,&weight_options).map_err(|e|e.to_string())?;
            &defaults
        },
    };
    crate::native_pcm::normalize_loudness_file_with_options(source,destination,weights,target,options,progress).map_err(|e|e.to_string())
}

/// Owned component inventory; consult operation plans for profile/workflow support.
pub fn capabilities() -> Capabilities {
    crate::native_capabilities::inventory()
}

pub fn plan_loudnorm(
    source: &std::path::Path,
    normalization: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if let Some(plan) = crate::native_loudnorm::try_plan(source, normalization, dual_pass, options)
        .map_err(|e| e.to_string())?
    {
        return Ok(plan);
    }
    fvid_media::plan_loudnorm(source, normalization, dual_pass, options)
}

pub fn apply_loudnorm(
    source: &std::path::Path,
    destination: &std::path::Path,
    normalization: Option<&str>,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    if let Some(stats) = crate::native_loudnorm::try_apply(source, destination, normalization, false, options)
        .map_err(|e| e.to_string())?
    {
        return Ok(stats);
    }
    fvid_media::apply_loudnorm(source, destination, normalization, options)
}

pub fn apply_loudnorm_dual(
    source: &std::path::Path,
    destination: &std::path::Path,
    normalization: Option<&str>,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    if let Some(stats) = crate::native_loudnorm::try_apply(source, destination, normalization, true, options)
        .map_err(|e| e.to_string())?
    {
        return Ok(stats);
    }
    fvid_media::apply_loudnorm_dual(source, destination, normalization, options)
}
