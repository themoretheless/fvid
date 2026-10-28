//! Owned compressed-audio bridge to the fvid-media normalizer.
//! Decoded WAVE is spooled on disk, never retained as a second full PCM buffer.
use fvid_media::{CopyOptions, LoudnormStats, ProgressHook};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Spool(PathBuf);
impl Drop for Spool {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Spool {
    fn create() -> std::io::Result<Self> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "fvid-loudnorm-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

fn selected_audio(source: &Path, options: &CopyOptions) -> Result<usize> {
    if options.streams.len() > 1 {
        return Err("loudnorm requires one selected audio stream".into());
    }
    if let Some(&index) = options.streams.first() {
        return Ok(index);
    }
    if crate::native_export::is_adts_source(source)? {
        return Ok(0);
    }
    Ok(crate::native_probe::probe_with_options(source, options)
        .map_err(|e| crate::invalid(&e))?
        .streams
        .iter()
        .find(|s| s.media_type == "audio")
        .ok_or("no audio stream for loudnorm")?
        .index)
}

fn has_metadata_mutations(options: &CopyOptions) -> bool {
    !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
}

fn valid_wave_metadata_options(options: &CopyOptions) -> bool {
    options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
        && options.metadata_set.len() + options.metadata_delete.len() <= 64
        && options
            .metadata_set
            .iter()
            .all(|(k, v)| !v.contains('\0') && fvid_media::owned_wave_metadata::tag_for_key(k).is_ok())
        && options
            .metadata_delete
            .iter()
            .all(|k| fvid_media::owned_wave_metadata::tag_for_key(k).is_ok())
}

fn strip_metadata_options(options: &CopyOptions) -> CopyOptions {
    let mut clean = options.clone();
    clean.metadata_set.clear();
    clean.metadata_delete.clear();
    clean.stream_metadata_set.clear();
    clean.stream_metadata_delete.clear();
    clean
}

pub fn validate_matroska_destination_metadata(options: &CopyOptions) -> Result<()> {
    crate::native_export::pcm_matroska::validate_matroska_pcm_metadata(Some(options))
        .map_err(Into::into)
}

/// Describe the owned route from container/configuration metadata. No loudness
/// measurement, PCM decoding, output publication or codec reference process.
pub fn try_plan(
    source: &Path,
    args: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> Result<Option<fvid_media_info::MediaPlan>> {
    use fvid_media_info::PlanStep;
    let resolved = fvid_media::owned_loudnorm::validate_request(args)?;
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("loudnorm requires at least one packet".into());
    }
    if options.streams.len() > 1 {
        return Err("loudnorm requires one selected audio stream".into());
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| crate::invalid(&e))?;
    if !valid_wave_metadata_options(options) {
        crate::native_export::pcm_matroska::validate_matroska_pcm_metadata(Some(options))?;
        let clean = strip_metadata_options(options);
        let Some(mut plan) = try_plan(source, args, dual_pass, &clean)? else {
            return Ok(None);
        };
        if options.max_controlled_bytes.is_some() {
            if let Ok((_, channels)) =
                fvid_media::owned_audio_plan::source_geometry(source, &clean)
            {
                crate::native_export::pcm_matroska::check_output_memory_budget(
                    source,
                    channels,
                    65536,
                    options,
                    Some(true),
                )?;
            }
        }
        let insert_idx = plan
            .steps
            .iter()
            .position(|s| s.action == "write")
            .unwrap_or(plan.steps.len());
        plan.steps.insert(
            insert_idx,
            PlanStep {
                action: "metadata".into(),
                detail: "apply Matroska container and stream 0 metadata mutations".into(),
            },
        );
        return Ok(Some(plan));
    }
    if crate::native_export::is_adts_source(source)? {
        if options.streams.first().is_some_and(|&s| s != 0) {
            return Err("owned ADTS loudnorm requires stream 0 and valid WAVE INFO metadata edits".into());
        }
        use std::io::Read;
        let mut prefix = [0; 7];
        std::fs::File::open(source)?.read_exact(&mut prefix)?;
        let header = crate::container::adts::header(&prefix).ok_or("invalid ADTS header")?;
        if header.frame_bytes - header.header_bytes > options.max_packet_bytes {
            return Err("ADTS packet exceeds budget".into());
        }
        return Ok(Some(
            fvid_media::owned_adts_loudnorm::plan_loudnorm(
                source,
                Some(&resolved),
                dual_pass,
                options,
            )
            .map_err(|e| crate::invalid(&e))?,
        ));
    }
    if fvid_media::owned_container_loudness::recognizes_standalone_audio(source)
        .map_err(|e| crate::invalid(&e))?
    {
        return Ok(Some(
            fvid_media::owned_container_loudnorm::plan_loudnorm(
                source,
                Some(&resolved),
                dual_pass,
                options,
            )
            .map_err(|e| crate::invalid(&e))?,
        ));
    }
    let wave = crate::native_pcm::is_wave(source)?;
    if wave {
        if options.streams.len() > 1 {
            return Err("loudnorm requires one selected audio stream".into());
        }
        if options.streams.first().is_some_and(|&s| s != 0) {
            return Err("WAVE has only stream 0".into());
        }
        if let Ok(info) = crate::native_pcm::inspect(
            &mut std::fs::File::open(source)?,
            options.cancel.as_ref(),
        ) {
            if info.sample_frames == 0 {
                return Err("no decoded audio samples or invalid channel count".into());
            }
            let frame_bytes = usize::from(info.frame_bytes());
            if frame_bytes == 0
                || (4096 * frame_bytes).min(options.max_packet_bytes / frame_bytes * frame_bytes)
                    == 0
            {
                return Err("PCM packet limit cannot hold one sample frame".into());
            }
        }
        if !fvid_media::owned_loudnorm::supports_request(
            source,
            Path::new("output.wav"),
            Some(&resolved),
            dual_pass,
            options,
        ) {
            return Ok(None);
        }
        return Ok(Some(
            fvid_media::owned_loudnorm::plan_loudnorm(
                source, Some(&resolved), dual_pass, options,
            )
            .map_err(|e| crate::invalid(&e))?,
        ));
    }
    let index = if wave {
        selected_audio(source, options)?
    } else if options.max_controlled_bytes.is_some()
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
    {
        let mut default_opts = CopyOptions::default();
        default_opts.streams = options.streams.clone();
        default_opts.max_controlled_bytes = options.max_controlled_bytes;
        if fvid_media::owned_container_loudnorm::supports_plan(
            source,
            Some(&resolved),
            &default_opts,
        ) {
            return Ok(Some(
                fvid_media::owned_container_loudnorm::plan_loudnorm(
                    source,
                    Some(&resolved),
                    dual_pass,
                    options,
                )
                .map_err(|e| crate::invalid(&e))?,
            ));
        }
        return Ok(None);
    } else {
        let mut default_opts = CopyOptions::default();
        default_opts.streams = options.streams.clone();
        default_opts.cancel = options.cancel.clone();
        let native_stream = if crate::native_media::is_owned_audio_trim_source_with_options(source, &default_opts)? {
            let idx = selected_audio(source, options)?;
            crate::native_media::audio_source_info_selected_with_options(source, Some(idx), &default_opts)
                .ok()
                .map(|info| (idx, info))
        } else {
            None
        };
        if let Some((idx, info)) = native_stream {
            if !fvid_media::owned_wave_loudness::qualified_rate(info.sample_rate) {
                return Ok(None);
            }
            if options.max_packet_bytes != CopyOptions::default().max_packet_bytes {
                use std::io::{Read, Seek, SeekFrom};
                let mut input = std::io::BufReader::new(std::fs::File::open(source)?);
                let mut prefix = [0u8; 8];
                input.read_exact(&mut prefix)?;
                input.seek(SeekFrom::Start(0))?;
                if crate::container::mp4::recognizes_prefix(&prefix) {
                    let reader = crate::container::mp4::Mp4Reader::open(
                        input,
                        crate::container::mp4::Limits::default(),
                    )?;
                    if let Some(track) = reader.tracks().get(idx) {
                        let max_samples = crate::native_media::mp4_active_samples(
                            track,
                            None,
                            options.max_packets,
                        );
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
                } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
                    let mut reader = crate::container::webm::WebmReader::open(
                        input,
                        crate::container::webm::Limits::default(),
                    )?;
                    reader.scan_all()?;
                    if let Some(track) = reader.tracks.get(idx) {
                        let track_number = track.number;
                        let mut track_packets = 0u64;
                        for (packet_index, packet) in reader.packets.iter().enumerate() {
                            if packet.track == track_number {
                                if options.max_packets.is_some_and(|limit| track_packets >= limit) {
                                    break;
                                }
                                if reader.packet_payload_size(packet_index)?
                                    > options.max_packet_bytes
                                {
                                    return Err("WebM packet exceeds budget".into());
                                }
                                track_packets += 1;
                            }
                        }
                    }
                }
            }
            idx
        } else {
            let mut default_opts = CopyOptions::default();
            default_opts.streams = options.streams.clone();
            if fvid_media::owned_container_loudnorm::supports_plan(
                source,
                Some(&resolved),
                &default_opts,
            ) {
                return Ok(Some(
                    fvid_media::owned_container_loudnorm::plan_loudnorm(
                        source,
                        Some(&resolved),
                        dual_pass,
                        options,
                    )
                    .map_err(|e| crate::invalid(&e))?,
                ));
            } else {
                return Ok(None);
            }
        }
    };
    let mut plan_options = options.clone();
    plan_options.streams = vec![index];
    let mut plan =
        crate::native_plan::decode_audio_with_options(source, &Default::default(), &plan_options)
            .map_err(|e| crate::invalid(&e))?;
    plan.command = "loudnorm".into();
    plan.steps.truncate(1);
    if wave {
        plan.steps[0].detail =
            "owned WAVE reader retaining PCM precision and speaker layout for f64 processing"
                .into();
    } else {
        plan.steps.push(PlanStep { action: "spool".into(), detail: "owned float32 WAVE in a private temporary directory; preserve presentation edits/priming; cleanup on success/error".into() });
    }
    if dual_pass || fvid_media::owned_loudnorm::report_requested(Some(&resolved))? {
        plan.steps.push(PlanStep { action: "analyze".into(), detail: if dual_pass { "owned K-weighting, loudness gating, LRA and true peak; select measured linear gain when range/peak margins permit" } else { "owned input loudness/true-peak measurement for the requested report; retain the configured normalization mode" }.into() });
    }
    plan.steps.push(PlanStep { action: "filter".into(), detail: format!("owned loudnorm targets {resolved}; eligible measured linear gain, otherwise owned dynamic controller and linked true-peak limiter") });
    if fvid_media::owned_loudnorm::report_requested(Some(&resolved))? {
        plan.steps.push(PlanStep { action: "analyze-output".into(), detail: "owned output loudness/true-peak measurement before publication; print JSON/summary after successful publication".into() });
    }
    plan.steps.push(PlanStep { action: "write".into(), detail: "float32 WAVE; linear retains source clock, dynamic uses 192 kHz; atomic publication without overwrite; report/completion only after publication".into() });
    plan.graph = None;
    plan.notes.push(
        "normalization backend: fvid; dynamic PCM is not claimed equivalent to libavfilter".into(),
    );
    plan.notes.push("measurement values, mode selection, packet contents, timeline consistency and memory admission are execution checks".into());
    if let Some(maximum) = options.max_packets {
        plan.notes.push(if wave {
            format!("read at most {maximum} original WAVE PCM blocks per pass")
        } else {
            format!("decode at most {maximum} selected compressed packets including preroll/repeated decoding; normalize that presentation prefix without counting PCM blocks again")
        });
    }
    if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        plan.notes.push(format!(
            "encoded packet limit: {} bytes; controlled memory limit: {:?}; RSS limit: {:?}",
            options.max_packet_bytes, options.max_controlled_bytes, options.max_rss_bytes
        ));
    }
    Ok(Some(plan))
}

/// Route owned WAVE directly, or owned compressed audio through a temporary
/// decoded WAVE. `None` leaves other formats/policies to the caller during
/// migration. Compressed allocation admission still needs a decoder bridge;
/// Remaining policies are never silently treated as WAVE packet policies.
pub fn try_apply(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> Result<Option<LoudnormStats>> {
    let ext = destination.extension().and_then(|s| s.to_str());
    let matroska_out = matches!(ext, Some("mka" | "mkv"));
    if matroska_out {
        crate::native_export::pcm_matroska::validate_matroska_pcm_metadata(Some(options))?;
        if destination.symlink_metadata().is_ok() {
            return Err("loudnorm destination already exists".into());
        }
        let clean = strip_metadata_options(options);
        if options.max_controlled_bytes.is_some() {
            if let Ok((_, channels)) =
                fvid_media::owned_audio_plan::source_geometry(source, &clean)
            {
                crate::native_export::pcm_matroska::check_output_memory_budget(
                    source,
                    channels,
                    65536,
                    options,
                    Some(true),
                )?;
            }
        }
        let spool = Spool::create()?;
        let spool_wav = spool.0.join("normalized.wav");
        let mut norm_options = clean;
        norm_options.progress = options.progress.as_ref().map(|hook| {
            let hook = hook.clone();
            ProgressHook::new(move |mut event| {
                event.done = false;
                hook.emit(event);
            })
        });
        let Some(stats) = try_apply(source, &spool_wav, args, dual_pass, &norm_options)? else {
            return Ok(None);
        };
        if options.max_controlled_bytes.is_some() {
            crate::native_export::pcm_matroska::check_output_memory_budget(
                source,
                u16::try_from(stats.channels).unwrap_or(2),
                65536,
                options,
                Some(true),
            )?;
        }
        let mut publish_options = CopyOptions::default();
        publish_options.cancel = options.cancel.clone();
        publish_options.max_rss_bytes = options.max_rss_bytes;
        publish_options.progress = options.progress.clone();
        publish_options.metadata_set = options.metadata_set.clone();
        publish_options.metadata_delete = options.metadata_delete.clone();
        publish_options.stream_metadata_set = options.stream_metadata_set.clone();
        publish_options.stream_metadata_delete = options.stream_metadata_delete.clone();
        let source_matroska_metadata = Some(
            crate::native_export::pcm_matroska::extract_source_pcm_metadata_with_interval_packet_and_memory_limit(
                source,
                options.streams.first().copied(),
                true,
                None,
                Some(options.max_packet_bytes),
                options.max_packets,
                options.max_controlled_bytes,
                options.cancel.as_ref(),
            )?,
        );
        crate::native_export::export_audio_pcm_transformed_with_source_metadata(
            &spool_wav,
            destination,
            None,
            1.0,
            None,
            None,
            &publish_options,
            source_matroska_metadata,
        )?;
        return Ok(Some(stats));
    }
    if ext == Some("wav") && has_metadata_mutations(options) && !valid_wave_metadata_options(options) {
        if !options.stream_metadata_set.is_empty() || !options.stream_metadata_delete.is_empty() {
            return Err("loudnorm to .wav does not support stream metadata mutations".into());
        }
        fvid_media::owned_wave_metadata::edit_info_chunks(
            &[],
            &options.metadata_delete,
            &options.metadata_set,
        )?;
        return Err("invalid WAVE INFO metadata mutation".into());
    }
    let normalize = |input: &Path, options: &CopyOptions| {
        if dual_pass {
            fvid_media::owned_loudnorm::apply_loudnorm_dual(input, destination, args, options)
        } else {
            fvid_media::owned_loudnorm::apply_loudnorm(input, destination, args, options)
        }
    };
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("loudnorm requires at least one packet".into());
    }
    if options.streams.len() > 1 {
        return Err("loudnorm requires one selected audio stream".into());
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| crate::invalid(&e))?;
    if !valid_wave_metadata_options(options) {
        return Ok(None);
    }
    let mut default_opts = CopyOptions::default();
    default_opts.streams = options.streams.clone();
    if fvid_media::owned_container_loudness::recognizes_standalone_audio(source)
        .map_err(|e| crate::invalid(&e))?
    {
        if destination.extension().and_then(|s| s.to_str()) == Some("wav") {
            return Ok(Some(
                fvid_media::owned_container_loudnorm::apply(
                    source,
                    destination,
                    args,
                    dual_pass,
                    options,
                )
                .map_err(|e| crate::invalid(&e))?,
            ));
        }
        return Ok(None);
    }
    if crate::native_pcm::is_wave(source)? {
        if options.streams.len() > 1 {
            return Err("loudnorm requires one selected audio stream".into());
        }
        if options.streams.first().is_some_and(|&s| s != 0) {
            return Err("WAVE has only stream 0".into());
        }
        if fvid_media::owned_loudnorm::supports_request(
            source,
            destination,
            args,
            dual_pass,
            options,
        ) {
            return Ok(Some(normalize(source, options)?));
        }
        return Ok(None);
    }
    // PCE bootstrap can read the first ADTS payload during metadata inspection.
    // Check its declared size before any such preflight allocation.
    let adts_source = crate::native_export::is_adts_source(source)?;
    if adts_source {
        if options.streams.first().is_some_and(|&s| s != 0) {
            return Err("owned ADTS loudnorm requires stream 0 and valid WAVE INFO metadata edits".into());
        }
        use std::io::Read;
        let mut prefix = [0; 7];
        std::fs::File::open(source)?.read_exact(&mut prefix)?;
        let header = crate::container::adts::header(&prefix).ok_or("invalid ADTS header")?;
        if header.frame_bytes - header.header_bytes > options.max_packet_bytes {
            return Err("ADTS packet exceeds budget".into());
        }
    }
    if adts_source && destination.extension().and_then(|s| s.to_str()) == Some("wav") {
        return Ok(Some(
            fvid_media::owned_adts_loudnorm::apply(source, destination, args, dual_pass, options)
                .map_err(|e| crate::invalid(&e))?,
        ));
    }
    default_opts.cancel = options.cancel.clone();
    let native_supported = options.max_controlled_bytes.is_none()
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && crate::native_media::is_owned_audio_trim_source_with_options(source, &default_opts)?
        && selected_audio(source, options).is_ok_and(|idx| {
            crate::native_media::audio_source_info_selected_with_options(
                source,
                Some(idx),
                &default_opts,
            )
            .is_ok()
        });
    if !native_supported {
        let mut plan_opts = default_opts;
        plan_opts.max_controlled_bytes = options.max_controlled_bytes;
        if destination.extension().and_then(|s| s.to_str()) == Some("wav")
            && fvid_media::owned_container_loudnorm::supports_plan(source, args, &plan_opts)
        {
            return Ok(Some(
                fvid_media::owned_container_loudnorm::apply(
                    source,
                    destination,
                    args,
                    dual_pass,
                    options,
                )
                .map_err(|e| crate::invalid(&e))?,
            ));
        }
        return Ok(None);
    }
    if options.streams.len() > 1 {
        return Err("loudnorm requires one selected audio stream".into());
    }
    if destination.exists() {
        return Err("loudnorm destination already exists".into());
    }
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Ok(None);
    }
    // Preserve the established loudnorm default: first audio, rather than
    // choosing the first container stream when video precedes audio.
    let selected = selected_audio(source, options)?;
    let mut plan_options = options.clone();
    plan_options.streams = vec![selected];
    crate::native_plan::decode_audio_with_options(source, &Default::default(), &plan_options)
        .map_err(|e| crate::invalid(&e))?;
    let info = crate::native_media::audio_source_info_selected_with_options(source, Some(selected), options)?;
    if !fvid_media::owned_wave_loudness::qualified_rate(info.sample_rate) {
        return Ok(None);
    }
    let spool = Spool::create()?;
    let wave = spool.0.join("decoded.wav");
    let packets = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let decode_hook = options.progress.as_ref().map(|hook| {
        let hook = hook.clone();
        let packets = packets.clone();
        let bytes = bytes.clone();
        ProgressHook::new(move |mut event| {
            packets.store(event.packets, Ordering::Relaxed);
            bytes.store(event.payload_bytes, Ordering::Relaxed);
            event.done = false;
            hook.emit(event);
        })
    });
    let mut decode_options = options.clone();
    decode_options.progress = decode_hook;
    crate::native_export::export_audio_pcm_selected_with_controls(
        source,
        &wave,
        Some(selected),
        &decode_options,
    )?;
    let mut normalization_options = options.clone();
    normalization_options.streams = vec![0];
    normalization_options.max_packets = None;
    normalization_options.max_packet_bytes = CopyOptions::default().max_packet_bytes;
    normalization_options.progress = options.progress.as_ref().map(|hook| {
        let hook = hook.clone();
        let prefix_packets = packets.load(Ordering::Relaxed);
        let prefix_bytes = bytes.load(Ordering::Relaxed);
        ProgressHook::new(move |mut event| {
            event.packets = event.packets.saturating_add(prefix_packets);
            event.payload_bytes = event.payload_bytes.saturating_add(prefix_bytes);
            hook.emit(event);
        })
    });
    Ok(Some(normalize(&wave, &normalization_options)?))
}
