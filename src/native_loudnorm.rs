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

/// Route owned WAVE directly, or owned compressed audio through a temporary
/// decoded WAVE. `None` leaves other formats/policies to the caller during
/// migration. Compressed packet/allocation admission policies still need a decoder bridge;
/// they are never silently treated as WAVE packet policies.
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
    if options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_packet_bytes != CopyOptions::default().max_packet_bytes
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
    let selected = match options.streams.first() {
        Some(&index) => index,
        None => {
            crate::native_probe::probe(source)
                .map_err(|e| crate::invalid(&e))?
                .streams
                .iter()
                .find(|s| s.media_type == "audio")
                .ok_or("no audio stream for loudnorm")?
                .index
        }
    };
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
    crate::native_export::export_audio_pcm_selected_with_rss_limit(
        source,
        &wave,
        Some(selected),
        options.max_rss_bytes,
        options.cancel.as_ref(),
        decode_hook.as_ref(),
    )?;
    let mut normalization_options = options.clone();
    normalization_options.streams = vec![0];
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
