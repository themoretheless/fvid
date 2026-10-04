//! AAC-LC file export through owned decoding and the shared WAVE DSP pipeline.
use fvid_control::{CopyOptions, ProgressEvent, ProgressHook};
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
type Result<T> = std::result::Result<T, String>;

// The PCM phase cannot apply container tags. Build its policy directly rather
// than cloning potentially unbounded tag strings and discarding them afterward.
fn pcm_decode_options(options: &CopyOptions) -> CopyOptions {
    CopyOptions {
        streams: options.streams.clone(),
        max_packet_bytes: options.max_packet_bytes,
        max_packets: options.max_packets,
        max_controlled_bytes: options.max_controlled_bytes,
        max_rss_bytes: options.max_rss_bytes,
        cancel: options.cancel.clone(),
        progress: options.progress.clone(),
        metadata_set: Vec::new(),
        metadata_delete: Vec::new(),
        stream_metadata_set: Vec::new(),
        stream_metadata_delete: Vec::new(),
    }
}
pub(crate) fn recognizes(source: &Path) -> Result<bool> {
    use std::io::Read;
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut bytes = [0; 7];
    match file.read_exact(&mut bytes) {
        Ok(()) => Ok(crate::owned_aac::adts::header(&bytes).is_some()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}
// Legacy dispatch needs to distinguish owned packet tools from profiles/tools
// still supported by its old backend. Qualification is read-only and emits no
// progress; execution still revalidates the source and never retries failures.
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let qualify = || -> Result<()> {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("cancelled".into());
        }
        crate::owned_budget::check_rss_budget(options)?;
        let reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            options.max_packet_bytes,
        )
        .map_err(|e| e.to_string())?;
        let config = reader.configuration();
        crate::owned_aac::stream::check_decode_admission(config.channels, options)
            .map_err(|e| e.to_string())?;
        let mask = crate::owned_aac::NativeAacDecoder::new(reader.audio_specific_config())
            .map_err(|e| e.to_string())?
            .channel_mask();
        let channels = transform.channels.unwrap_or(i32::from(config.channels));
        if channels != i32::from(config.channels) {
            if !((config.channels <= 8 && matches!(channels, 1 | 2))
                || (config.channels <= 2 && (1..=8).contains(&channels)))
            {
                return Err("unsupported channel conversion".into());
            }
            if !crate::owned_pcm_channels::standard_mask(config.channels)
                .is_some_and(|standard| standard == u64::from(mask))
            {
                return Err("unsupported speaker layout".into());
            }
        }
        drop(reader);
        let mut probe = pcm_decode_options(options);
        probe.progress = None;
        let prefix = decoded_prefix(transform, config.sample_rate);
        crate::owned_aac::decode_adts_pcm(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            &mut std::io::sink(),
            prefix,
            &probe,
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    };
    match qualify() {
        Ok(()) => true,
        // Policy exhaustion is an owned execution error, never a codec fallback.
        Err(error) => error.starts_with("controlled memory budget exceeded:"),
    }
}
fn resample_lookahead(input_rate: u32, output_rate: Option<i32>) -> std::time::Duration {
    let Some(output_rate) = output_rate
        .and_then(|r| u32::try_from(r).ok())
        .filter(|&r| r > 0 && r != input_rate)
    else {
        return std::time::Duration::ZERO;
    };
    let frames = crate::owned_resample::input_radius(input_rate, output_rate);
    let nanos = (u128::from(frames) * 1_000_000_000).div_ceil(u128::from(input_rate));
    std::time::Duration::from_nanos(nanos as u64)
}
/// Retain origin/preroll and the actual sinc radius beyond the selected end.
/// The final WAVE stage still applies the requested output-clock window.
pub(crate) fn decoded_prefix(
    transform: AudioDecodeTransform,
    input_rate: u32,
) -> Option<(std::time::Duration, std::time::Duration)> {
    transform.interval.map(|(_, to)| {
        (
            std::time::Duration::ZERO,
            std::time::Duration::from_micros(to as u64)
                + resample_lookahead(input_rate, transform.sample_rate),
        )
    })
}

pub(crate) struct Spool(pub(crate) PathBuf);
impl Spool {
    pub(crate) fn create() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "fvid-adts-export-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
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
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}
impl Drop for Spool {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub(crate) struct PcmSpool {
    output: BufWriter<File>,
    bytes: u64,
}
impl Write for PcmSpool {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .checked_add(bytes.len() as u64)
            .is_none_or(|n| n > u64::from(u32::MAX - 72))
        {
            return Err(std::io::Error::other("WAV exceeds RIFF size limit"));
        }
        let written = self.output.write(bytes)?;
        self.bytes += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let prefix = transform.interval.map(|(_, to)| {
        (
            std::time::Duration::ZERO,
            std::time::Duration::from_micros(to as u64),
        )
    });
    let spool = decode_to_wave_with_rate(source, prefix, transform.sample_rate, options)?;
    export_spool(spool, destination, transform, options)
}

pub(crate) fn export_spool(
    spool: DecodedSpool,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    let base = spool.progress;
    let original = options.progress.clone();
    let mut pcm_options = options.clone();
    pcm_options.streams = vec![0];
    pcm_options.max_packets = None;
    pcm_options.max_packet_bytes = CopyOptions::default().max_packet_bytes;
    pcm_options.progress = original.map(|hook| {
        ProgressHook::new(move |mut event| {
            event.packets += base.packets;
            event.payload_bytes += base.payload_bytes;
            hook.emit(event);
        })
    });
    let mut result = crate::owned_audio_export::decode_audio_transformed(
        &spool.wave,
        destination,
        transform,
        &pcm_options,
    )?;
    result.decoded_frames = spool.stats.decoded_frames;
    Ok(result)
}

pub(crate) struct DecodedSpool {
    _storage: Spool,
    pub wave: PathBuf,
    pub stats: crate::owned_aac::AdtsPcmStats,
    pub progress: ProgressEvent,
}
pub(crate) fn decode_to_wave(
    source: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    options: &CopyOptions,
) -> Result<DecodedSpool> {
    decode_to_wave_with_rate(source, interval, None, options)
}
fn decode_to_wave_with_rate(
    source: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    output_rate: Option<i32>,
    options: &CopyOptions,
) -> Result<DecodedSpool> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let config = reader.configuration();
    crate::owned_aac::stream::check_decode_admission(config.channels, options)
        .map_err(|e| e.to_string())?;
    let interval = interval.map(|(from, to)| {
        (
            from,
            to + resample_lookahead(config.sample_rate, output_rate),
        )
    });
    let mask = crate::owned_aac::NativeAacDecoder::new(reader.audio_specific_config())
        .map_err(|e| e.to_string())?
        .channel_mask();
    drop(reader);
    spool_decoded(
        config.sample_rate,
        config.channels,
        mask,
        options,
        |writer, options| {
            crate::owned_aac::decode_adts_pcm(
                BufReader::new(File::open(source).map_err(|e| e.to_string())?),
                writer,
                interval,
                options,
            )
            .map_err(|e| e.to_string())
        },
    )
}

pub(crate) fn spool_decoded(
    rate: u32,
    channels: u16,
    mask: u32,
    options: &CopyOptions,
    decode: impl FnOnce(&mut PcmSpool, &CopyOptions) -> Result<crate::owned_aac::AdtsPcmStats>,
) -> Result<DecodedSpool> {
    spool_decoded_with_precision(rate, channels, mask, 32, options, decode)
}
pub(crate) fn spool_decoded_with_precision(
    rate: u32,
    channels: u16,
    mask: u32,
    bits: u16,
    options: &CopyOptions,
    decode: impl FnOnce(&mut PcmSpool, &CopyOptions) -> Result<crate::owned_aac::AdtsPcmStats>,
) -> Result<DecodedSpool> {
    let spool = Spool::create()?;
    let wave = spool.0.join("decoded.wav");
    let mut file = File::create(&wave).map_err(|e| e.to_string())?;
    file.write_all(&crate::owned_wav::float_wav_header_with_precision(
        rate, channels, 0, mask, bits,
    )?)
    .map_err(|e| e.to_string())?;
    let mut writer = PcmSpool {
        output: BufWriter::new(file),
        bytes: 0,
    };
    let totals = Arc::new(Mutex::new(ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    }));
    let captured = totals.clone();
    let original = options.progress.clone();
    let mut decode_options = pcm_decode_options(options);
    decode_options.progress = Some(ProgressHook::new(move |event| {
        *captured.lock().unwrap() = event;
        if let Some(hook) = &original {
            hook.emit(event);
        }
    }));
    let decoded = decode(&mut writer, &decode_options)?;
    writer.flush().map_err(|e| e.to_string())?;
    let header = crate::owned_wav::float_wav_header_with_precision(
        rate,
        channels,
        decoded.sample_frames,
        mask,
        bits,
    )?;
    writer
        .output
        .seek(SeekFrom::Start(0))
        .map_err(|e| e.to_string())?;
    writer
        .output
        .write_all(&header)
        .map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    drop(writer);
    let base = *totals.lock().unwrap();
    Ok(DecodedSpool {
        _storage: spool,
        wave,
        stats: decoded,
        progress: base,
    })
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn adts_export_admits_large_policy_and_rejects_small_without_publication() {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/audio/aac-stereo.aac");
        let output =
            std::env::temp_dir().join(format!("fvid-adts-admission-{}.wav", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let transform = AudioDecodeTransform::default();
        let too_small = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        assert!(supports(&source, transform, &too_small));
        let error =
            crate::decode_audio_transformed(&source, &output, transform, &too_small).unwrap_err();
        assert!(
            error.contains("controlled memory budget exceeded"),
            "{error}"
        );
        assert!(!output.exists());
        let options = CopyOptions {
            max_controlled_bytes: Some(16 * 1024 * 1024),
            ..Default::default()
        };
        assert!(supports(&source, transform, &options));
        crate::plan_decode_audio(&source, &transform, &options).unwrap();
        let result =
            crate::decode_audio_transformed(&source, &output, transform, &options).unwrap();
        assert!(result.decoded_frames > 0);
        let admitted = std::fs::read(&output).unwrap();
        std::fs::remove_file(&output).unwrap();
        crate::decode_audio_transformed(&source, &output, transform, &CopyOptions::default())
            .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), admitted);
        std::fs::remove_file(output).unwrap();
        let mut bytes = Vec::new();
        let error = crate::owned_aac::decode_adts_pcm(
            BufReader::new(File::open(source).unwrap()),
            &mut bytes,
            None,
            &too_small,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("controlled memory budget exceeded")
        );
        assert!(bytes.is_empty());
    }
}
