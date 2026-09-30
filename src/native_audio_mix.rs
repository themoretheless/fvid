//! Owned AAC/WAVE decoding and blockwise weighted float PCM mixing.
pub use crate::media_info::{MixAudioOptions, MixAudioStats, MixDuration};
use crate::{Result, invalid};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn eligible(sources: &[PathBuf]) -> Result<bool> {
    for source in sources {
        if crate::native_pcm::is_wave(source)? {
            let mut input = File::open(source)?;
            if !crate::native_pcm::inspect(&mut input, None)
                .is_ok_and(|info| info.validate_decode().is_ok())
            {
                return Ok(false);
            }
        } else if crate::native_media::is_aac_source(source)? {
            // Retain the adapter for unsupported profiles and ambiguous audio
            // selection until the owned mix API can represent those requests.
            if crate::native_media::aac_source_info(source).is_err() {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
    }
    Ok(true)
}

fn weights(inputs: usize, options: &MixAudioOptions) -> Result<Vec<f32>> {
    if !(2..=16).contains(&inputs) {
        return Err(invalid("mix-audio requires 2..=16 inputs"));
    }
    if options.weights.len() > inputs || options.weights.iter().any(|w| !w.is_finite() || *w < 0.0)
    {
        return Err(invalid(
            "mix-audio requires finite nonnegative weights, at most one per input",
        ));
    }
    let mut weights = options.weights.clone();
    weights.resize(inputs, *weights.last().unwrap_or(&1.0));
    let sum: f32 = weights.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return Err(invalid("mix-audio weight sum must be finite and > 0"));
    }
    Ok(weights)
}

/// Plan and validate all source geometries without decoding packet contents.
pub fn plan(
    sources: &[PathBuf],
    options: &MixAudioOptions,
) -> Result<crate::media_info::MediaPlan> {
    weights(sources.len(), options)?;
    let mut geometry = None;
    for source in sources {
        crate::native_plan::decode_audio(source, &Default::default()).map_err(|e| invalid(&e))?;
        let shape = if crate::native_pcm::is_wave(source)? {
            let mut input = File::open(source)?;
            let info = crate::native_pcm::inspect(&mut input, None)?;
            (info.sample_rate, info.channels)
        } else {
            let info = crate::native_media::aac_source_info(source)?;
            (info.sample_rate, info.channels)
        };
        if geometry.is_some_and(|g| g != shape) {
            return Err(invalid(
                "mix-audio inputs must share sample rate and channel count",
            ));
        }
        geometry = Some(shape);
    }
    Ok(crate::media_info::MediaPlan {
        command:"mix-audio".into(),input:sources[0].clone(),inputs:sources.to_vec(),streams:Vec::new(),graph:None,
        steps:vec![
            crate::media_info::PlanStep {action:"decode".into(),detail:"FVid AAC/WAVE decoders to temporary float PCM spools".into()},
            crate::media_info::PlanStep {action:"mix".into(),detail:format!("FVid weighted float mixing; shortest duration; normalize={}",options.normalize)},
            crate::media_info::PlanStep {action:"write".into(),detail:"atomic float32 WAV publication without overwriting an existing destination".into()},
        ],notes:vec!["PCM spools bound mixing memory independently of input duration; temporary disk space is required".into()],
    })
}

pub fn mix_audio(
    sources: &[PathBuf],
    destination: &Path,
    options: &MixAudioOptions,
) -> Result<MixAudioStats> {
    let weights = weights(sources.len(), options)?;
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err(invalid("mix-audio requires a .wav output"));
    }
    if destination.try_exists()? {
        return Err(invalid("mix-audio destination already exists"));
    }
    plan(sources, options)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let scratch = (0..100)
        .find_map(|_| {
            let path = parent.join(format!(
                ".fvid-mix-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => Some(Ok(Scratch(path))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .ok_or_else(|| invalid("cannot reserve mix-audio scratch directory"))??;
    let mut inputs = Vec::new();
    let mut descriptions = Vec::new();
    for (i, source) in sources.iter().enumerate() {
        let path = scratch.0.join(format!("{i}.f32le"));
        let stats = crate::native_export::export_audio_pcm_selected(
            source, &path, None, 1.0, None, None, None, None, None,
        )?;
        inputs.push(BufReader::new(File::open(path)?));
        descriptions.push(stats);
    }
    let channels = descriptions[0].channels;
    let rate = descriptions[0].sample_rate;
    if descriptions
        .iter()
        .any(|s| s.channels != channels || s.sample_rate != rate)
    {
        return Err(invalid("decoded mix-audio geometry changed"));
    }
    let frames = descriptions.iter().map(|s| s.sample_frames).min().unwrap();
    let stats = crate::native_media::AudioDecodeStats {
        sample_frames: frames,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let header = crate::native_export::float_wav_header(&stats)?;
    let output_path = scratch.0.join("mix.wav");
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)?,
    );
    output.write_all(&header)?;
    let total: f32 = weights.iter().sum();
    let scales: Vec<f32> = weights
        .iter()
        .map(|&w| if options.normalize { w / total } else { w })
        .collect();
    let block_samples = 4096 * usize::from(channels);
    let mut encoded = vec![0u8; block_samples * 4];
    let mut mixed = vec![0f32; block_samples];
    let mut remaining = frames;
    while remaining > 0 {
        let count = remaining.min(4096) as usize * usize::from(channels);
        mixed[..count].fill(0.0);
        for (input, scale) in inputs.iter_mut().zip(&scales) {
            input.read_exact(&mut encoded[..count * 4])?;
            for (dst, src) in mixed[..count]
                .iter_mut()
                .zip(encoded[..count * 4].chunks_exact(4))
            {
                let sample = f32::from_le_bytes(src.try_into().unwrap());
                *dst = (f64::from(*dst) + f64::from(sample) * f64::from(*scale)) as f32;
            }
        }
        for (sample, bytes) in mixed[..count]
            .iter()
            .zip(encoded[..count * 4].chunks_exact_mut(4))
        {
            bytes.copy_from_slice(&sample.to_le_bytes());
        }
        output.write_all(&encoded[..count * 4])?;
        remaining -= count as u64 / u64::from(channels);
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    std::fs::hard_link(&output_path, destination)?;
    Ok(MixAudioStats {
        backend: "fvid",
        sample_frames: frames,
        sample_rate: rate as i32,
        channels: i32::from(channels),
        sample_format: "flt".into(),
        inputs: sources.len(),
        duration: options.duration.as_str(),
        normalize: options.normalize,
        weights,
    })
}
