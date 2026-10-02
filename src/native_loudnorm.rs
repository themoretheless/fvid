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
    Ok(crate::native_probe::probe(source)
        .map_err(|e| crate::invalid(&e))?
        .streams
        .iter()
        .find(|s| s.media_type == "audio")
        .ok_or("no audio stream for loudnorm")?
        .index)
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
    if crate::native_export::is_adts_source(source)?
        && fvid_media::owned_adts_loudnorm::supports_plan(source, Some(&resolved), options)
    {
        return Ok(Some(fvid_media::owned_adts_loudnorm::plan_loudnorm(
            source, Some(&resolved), dual_pass, options,
        )?));
    }
    let wave = crate::native_pcm::is_wave(source)?;
    if wave {
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
    } else {
        if options.max_controlled_bytes.is_some()
            || !options.metadata_set.is_empty()
            || !options.metadata_delete.is_empty()
            || !options.stream_metadata_set.is_empty()
            || !options.stream_metadata_delete.is_empty()
        {
            return Ok(None);
        }
        if !crate::native_media::is_owned_audio_trim_source(source)? {
            return Ok(None);
        }
    }
    let index = selected_audio(source, options)?;
    if !wave {
        let info = crate::native_media::audio_source_info_selected(source, Some(index))?;
        if !fvid_media::owned_wave_loudness::qualified_rate(info.sample_rate) {
            return Ok(None);
        }
    }
    let mut plan =
        crate::native_plan::decode_audio_selected(source, &Default::default(), Some(index))
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
    let normalize = |input: &Path, options: &CopyOptions| {
        if dual_pass {
            fvid_media::owned_loudnorm::apply_loudnorm_dual(input, destination, args, options)
        } else {
            fvid_media::owned_loudnorm::apply_loudnorm(input, destination, args, options)
        }
    };
    if crate::native_pcm::is_wave(source)? {
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
    if options.max_controlled_bytes.is_some()
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Ok(None);
    }
    // PCE bootstrap can read the first ADTS payload during metadata inspection.
    // Check its declared size before any such preflight allocation.
    let adts_source = crate::native_export::is_adts_source(source)?;
    if adts_source {
        use std::io::Read;
        let mut prefix = [0; 7];
        std::fs::File::open(source)?.read_exact(&mut prefix)?;
        let header = crate::container::adts::header(&prefix).ok_or("invalid ADTS header")?;
        if header.frame_bytes - header.header_bytes > options.max_packet_bytes {
            return Err("ADTS packet exceeds budget".into());
        }
    }
    if adts_source
        && destination.extension().and_then(|s| s.to_str()) == Some("wav")
        && fvid_media::owned_adts_loudnorm::supports_plan(source, args, options)
    {
        return Ok(Some(fvid_media::owned_adts_loudnorm::apply(
            source, destination, args, dual_pass, options,
        )?));
    }
    if !crate::native_media::is_owned_audio_trim_source(source)? {
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
    crate::native_plan::decode_audio_selected(source, &Default::default(), Some(selected))
        .map_err(|e| crate::invalid(&e))?;
    let info = crate::native_media::audio_source_info_selected(source, Some(selected))?;
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
