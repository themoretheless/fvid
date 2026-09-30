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
    let scratch = reserve_scratch(destination)?;
    let (mut inputs, descriptions) = decode_sources(sources, &scratch)?;
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

fn reserve_scratch(destination: &Path) -> Result<Scratch> {
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

    Ok(scratch)
}

fn decode_sources(
    sources: &[PathBuf],
    scratch: &Scratch,
) -> Result<(
    Vec<BufReader<File>>,
    Vec<crate::native_media::AudioDecodeStats>,
)> {
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

    Ok((inputs, descriptions))
}

fn merge_geometry(sources: &[PathBuf]) -> Result<(u32, u16)> {
    if sources.len() != 2 {
        return Err(invalid("merge-audio requires exactly two inputs"));
    }
    let mut rate = None;
    let mut channels = 0u16;
    for source in sources {
        crate::native_plan::decode_audio(source, &Default::default()).map_err(|e| invalid(&e))?;
        let (input_rate, input_channels) = if crate::native_pcm::is_wave(source)? {
            let mut input = File::open(source)?;
            let info = crate::native_pcm::inspect(&mut input, None)?;
            (info.sample_rate, info.channels)
        } else {
            let info = crate::native_media::aac_source_info(source)?;
            (info.sample_rate, info.channels)
        };
        if rate.is_some_and(|r| r != input_rate) {
            return Err(invalid("merge-audio inputs must share sample rate"));
        }
        rate = Some(input_rate);
        channels = channels
            .checked_add(input_channels)
            .ok_or_else(|| invalid("merge-audio channel count overflow"))?;
    }
    if !(1..=64).contains(&channels) {
        return Err(invalid("merge-audio output requires 1..=64 channels"));
    }
    Ok((rate.unwrap(), channels))
}

pub fn plan_merge(sources: &[PathBuf]) -> Result<crate::media_info::MediaPlan> {
    let (rate, channels) = merge_geometry(sources)?;
    Ok(crate::media_info::MediaPlan {
        command:"merge-audio".into(),input:sources[0].clone(),inputs:sources.to_vec(),streams:Vec::new(),graph:None,
        steps:vec![
            crate::media_info::PlanStep {action:"decode".into(),detail:"FVid AAC/WAVE decoders to temporary float PCM spools".into()},
            crate::media_info::PlanStep {action:"merge".into(),detail:format!("concatenate input channel vectors at {rate} Hz into {channels} channels, shortest duration")},
            crate::media_info::PlanStep {action:"write".into(),detail:"atomic float32 WAV publication without replacing existing files".into()},
        ],notes:vec!["Channel order is first input then second input, preserving the existing merge-audio API contract".into(),"PCM spools require temporary disk space; mixing memory is bounded independently of duration".into()],
    })
}

/// Concatenate the channel vectors of exactly two inputs at each sample time.
/// No speaker-position remapping or implicit resampling is performed.
pub fn merge_audio(
    sources: &[PathBuf],
    destination: &Path,
) -> Result<crate::media_info::MergeAudioStats> {
    let (rate, channels) = merge_geometry(sources)?;
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err(invalid("merge-audio requires a .wav output"));
    }
    if destination.try_exists()? {
        return Err(invalid("merge-audio destination already exists"));
    }
    let scratch = reserve_scratch(destination)?;
    let (mut inputs, descriptions) = decode_sources(sources, &scratch)?;
    if descriptions.iter().any(|s| s.sample_rate != rate)
        || descriptions
            .iter()
            .map(|s| u32::from(s.channels))
            .sum::<u32>()
            != u32::from(channels)
    {
        return Err(invalid("decoded merge-audio geometry changed"));
    }
    let frames = descriptions.iter().map(|s| s.sample_frames).min().unwrap();
    let header = merged_wav_header(rate, channels, frames)?;
    let output_path = scratch.0.join("merge.wav");
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)?,
    );
    output.write_all(&header)?;
    let mut buffers = descriptions
        .iter()
        .map(|s| vec![0u8; 4096 * usize::from(s.channels) * 4])
        .collect::<Vec<_>>();
    let strides = descriptions
        .iter()
        .map(|s| usize::from(s.channels) * 4)
        .collect::<Vec<_>>();
    let mut remaining = frames;
    while remaining > 0 {
        let count = remaining.min(4096) as usize;
        for ((input, buffer), stride) in inputs.iter_mut().zip(&mut buffers).zip(&strides) {
            input.read_exact(&mut buffer[..count * stride])?;
        }
        for frame in 0..count {
            for (buffer, stride) in buffers.iter().zip(&strides) {
                output.write_all(&buffer[frame * stride..(frame + 1) * stride])?;
            }
        }
        remaining -= count as u64;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    std::fs::hard_link(&output_path, destination)?;
    Ok(crate::media_info::MergeAudioStats {
        backend: "fvid",
        sample_frames: frames,
        sample_rate: rate as i32,
        channels: i32::from(channels),
        sample_format: "flt".into(),
        inputs: 2,
    })
}

// The legacy merge API emits IEEE-float format 3 without a speaker mask. Keep
// that representation: assigning an invented multichannel layout would change
// the meaning of the input-order channel concatenation.
fn merged_wav_header(rate: u32, channels: u16, frames: u64) -> Result<Vec<u8>> {
    let align = channels
        .checked_mul(4)
        .ok_or_else(|| invalid("WAV alignment overflow"))?;
    let bytes = frames
        .checked_mul(u64::from(align))
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= u32::MAX - 36)
        .ok_or_else(|| invalid("WAV exceeds RIFF size limit"))?;
    let byte_rate = rate
        .checked_mul(u32::from(align))
        .ok_or_else(|| invalid("WAV byte rate overflow"))?;
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    Ok(out)
}
