//! Multi-input audio mix fair-paired with FFmpeg `amix`.
use super::audio::AudioDecodeStats;
use super::lossless::{Codec, Frame};
use super::wav::write_wav_f32le;
use super::*;
use std::path::{Path, PathBuf};

pub use fvid_media_info::{MixDuration, MixAudioOptions, MixAudioStats};

/// Mix two or more audio inputs to float PCM WAV.
/// Fair-pairs FFmpeg
/// `amix=inputs=N:duration=shortest:dropout_transition=0:normalize=0|1[:weights=...]`.
pub fn mix_audio(
    sources: &[PathBuf],
    destination: &Path,
    options: &MixAudioOptions,
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
            .map(|source| scope.spawn(|| decode_to_packed_f32(source)))
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
        let src = unsafe { slice::from_raw_parts(pcm.as_ptr() as *const f32, samples) };
        let scale = f64::from(scale);
        for (dst, &sample) in out.iter_mut().zip(src.iter()) {
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
pub fn merge_audio(sources: &[PathBuf], destination: &Path) -> Result<MergeAudioStats> {
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
        let left = scope.spawn(|| decode_to_packed_f32(&sources[0]));
        let right = scope.spawn(|| decode_to_packed_f32(&sources[1]));
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
    let a = unsafe { slice::from_raw_parts(pcm_a.as_ptr() as *const f32, frames * ch_a) };
    let b = unsafe { slice::from_raw_parts(pcm_b.as_ptr() as *const f32, frames * ch_b) };
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

fn decode_to_packed_f32(source: &Path) -> Result<(AudioDecodeStats, Vec<u8>)> {
    let mut input = Input::open_fast(source)?;
    if unsafe { (*input.0).nb_chapters } != 0 {
        return Err("mix-audio with chapters requires explicit timeline mapping".into());
    }
    let selected = selection(&input, &CopyOptions::default())?;
    if selected.len() != 1 {
        return Err("mix-audio requires exactly one audio stream per input".into());
    }
    let index = selected[0];
    let decoder = unsafe {
        let stream = &*input.streams()[index];
        if (*stream.codecpar).codec_type != AVMediaType_AVMEDIA_TYPE_AUDIO {
            return Err("selected stream is not audio".into());
        }
        let codec = avcodec_find_decoder((*stream.codecpar).codec_id);
        if codec.is_null() {
            return Err("audio decoder unavailable".into());
        }
        let decoder = Codec(avcodec_alloc_context3(codec));
        if decoder.0.is_null() {
            return Err("audio decoder allocation failed".into());
        }
        check(
            avcodec_parameters_to_context(decoder.0, stream.codecpar),
            "configure audio decoder",
        )?;
        (*decoder.0).pkt_timebase = stream.time_base;
        check(
            avcodec_open2(decoder.0, codec, ptr::null_mut()),
            "open audio decoder",
        )?;
        decoder
    };

    let mut packet = Packet::new()?;
    let frame = Frame::new()?;
    let mut pcm = Vec::new();
    let mut stats: Option<AudioDecodeStats> = None;

    'packets: loop {
        let available = packet.read(&mut input)?;
        if available {
            let (stream, _) = packet_info(&packet, &input, &CopyOptions::default())?;
            if stream != index {
                continue;
            }
        }
        check(
            unsafe {
                avcodec_send_packet(decoder.0, if available { packet.0 } else { ptr::null() })
            },
            "send audio packet",
        )?;
        loop {
            let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
            if code == -libc::EAGAIN || code == EOF {
                break;
            }
            check(code, "receive audio frame")?;
            unsafe {
                let f = &*frame.0;
                let packed = super::pcm_format_adapter::packed(f.format);
                if packed != AVSampleFormat_AV_SAMPLE_FMT_FLT {
                    av_frame_unref(frame.0);
                    return Err(
                        "mix-audio requires float PCM inputs (flt); integer formats are not qualified"
                            .into(),
                    );
                }
                if f.sample_rate <= 0 || !(1..=64).contains(&f.ch_layout.nb_channels) {
                    av_frame_unref(frame.0);
                    return Err("invalid decoded audio rate/channel count".into());
                }
                let channels = f.ch_layout.nb_channels as usize;
                let samples = f.nb_samples as usize;
                let bytes = 4usize;
                if stats.is_none() {
                    stats = Some(AudioDecodeStats {
                        sample_frames: 0,
                        decoded_frames: 0,
                        sample_rate: f.sample_rate,
                        channels: f.ch_layout.nb_channels,
                        sample_format: "flt".into(),
                        planar_interleave_bytes: 0,
                        decode_errors: 0,
                    });
                } else {
                    let active = stats.as_ref().unwrap();
                    if active.sample_rate != f.sample_rate
                        || active.channels != f.ch_layout.nb_channels
                    {
                        av_frame_unref(frame.0);
                        return Err("dynamic audio format/rate/layout is not supported".into());
                    }
                }
                let active = stats.as_mut().unwrap();
                if super::pcm_format_adapter::planar(f.format) && channels > 1 {
                    let mut planes = [ptr::null(); 64];
                    for (channel, plane) in planes[..channels].iter_mut().enumerate() {
                        let base = *f.extended_data.add(channel);
                        if base.is_null() {
                            av_frame_unref(frame.0);
                            return Err("missing audio plane".into());
                        }
                        *plane = base;
                    }
                    let start = pcm.len();
                    let size = samples * channels * bytes;
                    pcm.resize(start + size, 0);
                    super::audio_layout::interleave(
                        &planes[..channels],
                        pcm[start..].as_mut_ptr(),
                        samples,
                        bytes,
                    );
                    active.planar_interleave_bytes += size as u64;
                } else {
                    let data = *f.extended_data;
                    if data.is_null() {
                        av_frame_unref(frame.0);
                        return Err("missing audio data".into());
                    }
                    pcm.extend_from_slice(slice::from_raw_parts(data, samples * channels * bytes));
                }
                active.sample_frames += samples as u64;
                active.decoded_frames += 1;
                av_frame_unref(frame.0);
            }
        }
        if !available {
            break 'packets;
        }
    }
    let stats = stats.ok_or("no decoded audio samples")?;
    Ok((stats, pcm))
}
