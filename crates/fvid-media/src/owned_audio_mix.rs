//! Owned float WAV mix and channel merge, available without FFmpeg.
#![forbid(unsafe_code)]
use crate::owned_wav_file::write_wav_f32le;
use fvid_media_info::AudioDecodeStats;
pub use fvid_media_info::{MixAudioOptions, MixAudioStats, MixDuration};
use std::path::{Path, PathBuf};
type Result<T> = std::result::Result<T, String>;
type Decoder = fn(&Path) -> Result<(AudioDecodeStats, Vec<u8>)>;

pub fn mix_audio(
    sources: &[PathBuf],
    destination: &Path,
    options: &MixAudioOptions,
) -> Result<MixAudioStats> {
    mix_with_decoder(sources, destination, options, decode_float_wave)
}
pub fn merge_audio(sources: &[PathBuf], destination: &Path) -> Result<MergeAudioStats> {
    merge_with_decoder(sources, destination, decode_float_wave)
}

/// Build a read-only plan using the same PCM geometry required by execution.
pub fn plan_mix_audio(
    sources: &[PathBuf],
    options: &MixAudioOptions,
) -> Result<fvid_media_info::MediaPlan> {
    if !(2..=16).contains(&sources.len()) {
        return Err("mix-audio requires 2..=16 inputs".into());
    }
    let weights = expand_weights(sources.len(), &options.weights)?;
    let sum: f32 = weights.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return Err("mix-audio weight sum must be finite and > 0".into());
    }
    audio_plan(
        sources,
        false,
        format!(
            "weighted float mix, duration={}, normalize={}, weights={weights:?}",
            options.duration.as_str(),
            options.normalize
        ),
    )
}
pub fn plan_merge_audio(sources: &[PathBuf]) -> Result<fvid_media_info::MediaPlan> {
    if sources.len() != 2 {
        return Err("merge-audio v1 requires exactly two inputs".into());
    }
    audio_plan(
        sources,
        true,
        "concatenate channel vectors, shortest input duration".into(),
    )
}
fn audio_plan(
    sources: &[PathBuf],
    merge: bool,
    detail: String,
) -> Result<fvid_media_info::MediaPlan> {
    use fvid_media_info::{MediaPlan, PlanStep};
    let mut geometry = None;
    for source in sources {
        let mut file = std::fs::File::open(source).map_err(|e| e.to_string())?;
        let info =
            crate::owned_wave_inspect::inspect(&mut file, None).map_err(|e| e.to_string())?;
        if !info.float
            || info.bits_per_sample != 32
            || info.sample_frames == 0
            || !(1..=64).contains(&info.channels)
        {
            return Err("audio mix/merge requires nonempty float32 WAVE inputs".into());
        }
        if let Some((rate, channels)) = geometry {
            if rate != info.sample_rate || (!merge && channels != info.channels) {
                return Err("audio inputs must share sample rate and mixing channel count".into());
            }
        } else {
            geometry = Some((info.sample_rate, info.channels));
        }
    }
    Ok(MediaPlan {
        command: if merge { "merge-audio" } else { "mix-audio" }.into(),
        input: sources[0].clone(),
        inputs: sources.to_vec(),
        streams: Vec::new(),
        steps: vec![
            PlanStep {
                action: "decode".into(),
                detail: "owned float32 WAVE reader".into(),
            },
            PlanStep {
                action: if merge { "merge" } else { "mix" }.into(),
                detail,
            },
            PlanStep {
                action: "encode".into(),
                detail: "owned float32 WAVE writer".into(),
            },
        ],
        graph: None,
        notes: vec!["no FFmpeg or libav execution".into()],
    })
}

pub(crate) fn decode_float_wave(source: &Path) -> Result<(AudioDecodeStats, Vec<u8>)> {
    decode_float_wave_controlled(source, None, |_| Ok(()))
}

pub(crate) fn decode_float_wave_controlled<F: FnMut(usize) -> Result<()>>(
    source: &Path,
    cancel: Option<&fvid_control::CancelFlag>,
    block: F,
) -> Result<(AudioDecodeStats, Vec<u8>)> {
    read_float_wave_controlled(source, Some(32), cancel, block)
}
pub(crate) fn read_float_wave_controlled<F: FnMut(usize) -> Result<()>>(
    source: &Path,
    bits: Option<u16>,
    cancel: Option<&fvid_control::CancelFlag>,
    block: F,
) -> Result<(AudioDecodeStats, Vec<u8>)> {
    read_float_wave_with_limits(source, bits, cancel, usize::MAX, None, block)
}
pub(crate) fn read_float_wave_with_limits<F: FnMut(usize) -> Result<()>>(
    source: &Path,
    bits: Option<u16>,
    cancel: Option<&fvid_control::CancelFlag>,
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    block: F,
) -> Result<(AudioDecodeStats, Vec<u8>)> {
    read_float_wave_with_admission(
        source,
        bits,
        cancel,
        max_packet_bytes,
        max_packets,
        |_, _, _| Ok(()),
        block,
    )
}
pub(crate) fn read_float_wave_with_admission<F, A>(
    source: &Path,
    bits: Option<u16>,
    cancel: Option<&fvid_control::CancelFlag>,
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    admit: A,
    block: F,
) -> Result<(AudioDecodeStats, Vec<u8>)>
where
    F: FnMut(usize) -> Result<()>,
    A: FnMut(&mut std::fs::File, &crate::owned_wave_inspect::WaveInfo, usize) -> Result<()>,
{
    read_wave_with_admission(
        source,
        bits,
        true,
        cancel,
        max_packet_bytes,
        max_packets,
        admit,
        block,
    )
}
pub(crate) fn read_wave_with_admission<F, A>(
    source: &Path,
    bits: Option<u16>,
    float_only: bool,
    cancel: Option<&fvid_control::CancelFlag>,
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    mut admit: A,
    mut block: F,
) -> Result<(AudioDecodeStats, Vec<u8>)>
where
    F: FnMut(usize) -> Result<()>,
    A: FnMut(&mut std::fs::File, &crate::owned_wave_inspect::WaveInfo, usize) -> Result<()>,
{
    let check = || {
        if cancel.is_some_and(|flag| flag.is_cancelled()) {
            Err("media operation cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let info = crate::owned_wave_inspect::inspect(&mut file, cancel).map_err(|e| e.to_string())?;
    if (float_only && !info.float) || bits.is_some_and(|bits| info.bits_per_sample != bits) {
        return Err(
            "mix-audio requires float PCM inputs (flt); integer formats are not qualified".into(),
        );
    }
    if info.sample_frames == 0 || !(1..=64).contains(&info.channels) {
        return Err("no decoded audio samples or invalid channel count".into());
    }
    let rate = i32::try_from(info.sample_rate).map_err(|_| "invalid decoded audio rate")?;
    let frame_bytes = usize::from(info.block);
    let capacity = (4096 * frame_bytes).min(max_packet_bytes / frame_bytes * frame_bytes);
    if capacity == 0 {
        return Err("PCM packet limit cannot hold one sample frame".into());
    }
    let full_size =
        usize::try_from(info.data_bytes).map_err(|_| "WAVE PCM payload exceeds address space")?;
    let limit = max_packets
        .map(|packets| u128::from(packets) * capacity as u128)
        .unwrap_or(full_size as u128);
    let size = (full_size as u128).min(limit) as usize;
    if size == 0 {
        return Err("packet limit leaves no decoded audio samples".into());
    }
    admit(&mut file, &info, size)?;
    check()?;
    let mut pcm = Vec::new();
    pcm.try_reserve_exact(size)
        .map_err(|_| "cannot allocate WAVE PCM payload")?;
    pcm.resize(size, 0);
    file.seek(SeekFrom::Start(info.data_offset))
        .map_err(|e| e.to_string())?;
    let mut blocks = 0;
    let width = usize::from(info.bits_per_sample / 8);
    for bytes in pcm.chunks_mut(capacity) {
        check()?;
        file.read_exact(bytes).map_err(|e| e.to_string())?;
        if info.float
            && bytes.chunks_exact(width).any(|sample| {
                if width == 4 {
                    !f32::from_le_bytes(sample.try_into().unwrap()).is_finite()
                } else {
                    !f64::from_le_bytes(sample.try_into().unwrap()).is_finite()
                }
            })
        {
            return Err("non-finite PCM sample".into());
        }
        blocks += 1;
        block(bytes.len())?;
        check()?;
    }
    Ok((
        AudioDecodeStats {
            sample_frames: (size / frame_bytes) as u64,
            decoded_frames: blocks,
            sample_rate: rate,
            channels: i32::from(info.channels),
            sample_format: if info.float {
                if width == 4 { "flt" } else { "dbl" }
            } else {
                match width {
                    1 => "u8",
                    2 => "s16",
                    _ => "s32",
                }
            }
            .into(),
            planar_interleave_bytes: 0,
            decode_errors: 0,
        },
        pcm,
    ))
}
/// Mix two or more audio inputs to float PCM WAV.
/// Fair-pairs FFmpeg
/// `amix=inputs=N:duration=shortest:dropout_transition=0:normalize=0|1[:weights=...]`.
pub(crate) fn mix_with_decoder(
    sources: &[PathBuf],
    destination: &Path,
    options: &MixAudioOptions,
    decoder: Decoder,
) -> Result<MixAudioStats> {
    if !cfg!(target_endian = "little") {
        return Err("PCM export requires little-endian host".into());
    }
    if !(2..=16).contains(&sources.len()) {
        return Err("mix-audio requires 2..=16 inputs".into());
    }
    if destination.extension().and_then(|v| v.to_str()) != Some("wav") {
        return Err("mix-audio requires a .wav output".into());
    }
    let _ = options.duration;
    let weights = expand_weights(sources.len(), &options.weights)?;
    let weight_sum: f32 = weights.iter().copied().sum();
    if !(weight_sum.is_finite() && weight_sum > 0.0) {
        return Err("mix-audio weight sum must be finite and > 0".into());
    }

    // Parallel demux/decode: independent contexts per input.
    let decoded = std::thread::scope(|scope| {
        let handles: Vec<_> = sources
            .iter()
            .map(|source| scope.spawn(|| decoder(source)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("mix-audio decode worker"))
            .collect::<Result<Vec<_>>>()
    })?;

    let (stats0, _) = &decoded[0];
    for (stats, _) in &decoded[1..] {
        if stats.sample_rate != stats0.sample_rate || stats.channels != stats0.channels {
            return Err("mix-audio inputs must share sample rate and channel count".into());
        }
    }

    let channels = stats0.channels as usize;
    if channels == 0 {
        return Err("decoded PCM sample count is not channel-aligned".into());
    }
    let frames = decoded
        .iter()
        .map(|(_, pcm)| {
            if pcm.len() % (channels * 4) != 0 {
                return Err("decoded PCM sample count is not channel-aligned".to_string());
            }
            Ok(pcm.len() / (channels * 4))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .min()
        .unwrap_or(0);
    let samples = frames * channels;
    let scales: Vec<f32> = weights
        .iter()
        .map(|&w| if options.normalize { w / weight_sum } else { w })
        .collect();
    let mut out = vec![0f32; samples];
    // Match FFmpeg amix fmac into an f32 buffer: each input is scaled in f64-wide
    // expression then stored back to f32 (N=2 coincides with pure f32; N>2 needs this).
    for (pcm, &scale) in decoded.iter().map(|(_, pcm)| pcm).zip(scales.iter()) {
        let src = pcm
            .chunks_exact(4)
            .take(samples)
            .map(|sample| f32::from_le_bytes(sample.try_into().unwrap()));
        let scale = f64::from(scale);
        for (dst, sample) in out.iter_mut().zip(src) {
            *dst = (f64::from(*dst) + f64::from(sample) * scale) as f32;
        }
    }

    write_wav_f32le(destination, stats0.sample_rate, stats0.channels, &out)?;
    Ok(MixAudioStats {
        backend: "native decode + amix-compatible float mix",
        sample_frames: frames as u64,
        sample_rate: stats0.sample_rate,
        channels: stats0.channels,
        sample_format: "flt".into(),
        inputs: sources.len(),
        duration: options.duration.as_str(),
        normalize: options.normalize,
        weights,
    })
}

fn expand_weights(inputs: usize, weights: &[f32]) -> Result<Vec<f32>> {
    if weights.is_empty() {
        return Ok(vec![1.0; inputs]);
    }
    if weights.len() > inputs {
        return Err("mix-audio weights exceed input count".into());
    }
    for &w in weights {
        if !(w.is_finite() && w >= 0.0) {
            return Err("mix-audio weights must be finite and >= 0".into());
        }
    }
    let mut out = weights.to_vec();
    let last = *out.last().unwrap();
    while out.len() < inputs {
        out.push(last);
    }
    Ok(out)
}

pub use fvid_media_info::MergeAudioStats;

/// Channel-merge exactly two float PCM inputs (FFmpeg `amerge=inputs=2`).
/// Same sample rate required; duration is shortest. Output channel count is the sum.
pub(crate) fn merge_with_decoder(
    sources: &[PathBuf],
    destination: &Path,
    decoder: Decoder,
) -> Result<MergeAudioStats> {
    if !cfg!(target_endian = "little") {
        return Err("PCM export requires little-endian host".into());
    }
    if sources.len() != 2 {
        return Err("merge-audio v1 requires exactly two inputs".into());
    }
    if destination.extension().and_then(|v| v.to_str()) != Some("wav") {
        return Err("merge-audio requires a .wav output".into());
    }
    let (decoded_a, decoded_b) = std::thread::scope(|scope| {
        let left = scope.spawn(|| decoder(&sources[0]));
        let right = scope.spawn(|| decoder(&sources[1]));
        (
            left.join().expect("merge-audio decode worker"),
            right.join().expect("merge-audio decode worker"),
        )
    });
    let (stats_a, pcm_a) = decoded_a?;
    let (stats_b, pcm_b) = decoded_b?;
    if stats_a.sample_rate != stats_b.sample_rate {
        return Err("merge-audio inputs must share sample rate".into());
    }
    let ch_a = stats_a.channels as usize;
    let ch_b = stats_b.channels as usize;
    if ch_a == 0 || ch_b == 0 {
        return Err("merge-audio inputs must have at least one channel".into());
    }
    if pcm_a.len() % (ch_a * 4) != 0 || pcm_b.len() % (ch_b * 4) != 0 {
        return Err("decoded PCM sample count is not channel-aligned".into());
    }
    let frames = (pcm_a.len() / (ch_a * 4)).min(pcm_b.len() / (ch_b * 4));
    let out_ch = ch_a
        .checked_add(ch_b)
        .ok_or("merge-audio channel count overflow")?;
    if out_ch > 64 {
        return Err("merge-audio output exceeds 64 channels".into());
    }
    let a: Vec<_> = pcm_a
        .chunks_exact(4)
        .take(frames * ch_a)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let b: Vec<_> = pcm_b
        .chunks_exact(4)
        .take(frames * ch_b)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let mut out = vec![0f32; frames * out_ch];
    for frame in 0..frames {
        let dst = &mut out[frame * out_ch..(frame + 1) * out_ch];
        dst[..ch_a].copy_from_slice(&a[frame * ch_a..(frame + 1) * ch_a]);
        dst[ch_a..].copy_from_slice(&b[frame * ch_b..(frame + 1) * ch_b]);
    }
    write_wav_f32le(destination, stats_a.sample_rate, out_ch as i32, &out)?;
    Ok(MergeAudioStats {
        backend: "native decode + amerge-compatible channel merge",
        sample_frames: frames as u64,
        sample_rate: stats_a.sample_rate,
        channels: out_ch as i32,
        sample_format: "flt".into(),
        inputs: 2,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Files(PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn wav_mix_weights_shortest_merge_and_public_entrypoints_are_owned() {
        let files =
            Files(std::env::temp_dir().join(format!("fvid-owned-mix-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&files.0);
        std::fs::create_dir(&files.0).unwrap();
        let first = files.0.join("a.wav");
        let second = files.0.join("b.wav");
        write_wav_f32le(&first, 48000, 2, &[0.25, -0.5, 0.5, 0.25, 1., 1.]).unwrap();
        write_wav_f32le(&second, 48000, 2, &[0.75, 0.5, -0.25, 0.75]).unwrap();
        let sources = [first, second];
        let plan = plan_mix_audio(&sources, &MixAudioOptions::default()).unwrap();
        assert_eq!(plan.command, "mix-audio");
        assert!(plan.graph.is_none());
        assert_eq!(plan_merge_audio(&sources).unwrap().command, "merge-audio");
        assert!(
            plan_mix_audio(
                &sources,
                &MixAudioOptions {
                    weights: vec![-1.0],
                    ..Default::default()
                }
            )
            .is_err()
        );

        let mixed = files.0.join("mix.wav");
        let stats = crate::mix_audio(&sources, &mixed, &MixAudioOptions::default()).unwrap();
        assert_eq!(stats.sample_frames, 2);
        assert_eq!(stats.channels, 2);
        assert_eq!(stats.weights, [1., 1.]);
        let (_, pcm) = decode_float_wave(&mixed).unwrap();
        let floats: Vec<_> = pcm
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(floats, [0.5, 0., 0.125, 0.5]);
        let weighted = files.0.join("weighted.wav");
        let options = MixAudioOptions {
            normalize: false,
            weights: vec![2., 1.],
            ..Default::default()
        };
        mix_audio(&sources, &weighted, &options).unwrap();
        let (_, pcm) = decode_float_wave(&weighted).unwrap();
        let floats: Vec<_> = pcm
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(floats, [1.25, -0.5, 0.75, 1.25]);
        let merged = files.0.join("merge.wav");
        let stats = crate::merge_audio(&sources, &merged).unwrap();
        assert_eq!(stats.sample_frames, 2);
        assert_eq!(stats.channels, 4);
        let (_, pcm) = decode_float_wave(&merged).unwrap();
        let floats: Vec<_> = pcm
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(floats, [0.25, -0.5, 0.75, 0.5, 0.5, 0.25, -0.25, 0.75]);
        let original = std::fs::read(&mixed).unwrap();
        assert!(mix_audio(&sources, &mixed, &options).is_err());
        assert_eq!(std::fs::read(&mixed).unwrap(), original);
        let invalid = files.0.join("invalid.wav");
        let options = MixAudioOptions {
            weights: vec![-1.],
            ..Default::default()
        };
        assert!(mix_audio(&sources, &invalid, &options).is_err());
        assert!(!invalid.exists());
    }
}
