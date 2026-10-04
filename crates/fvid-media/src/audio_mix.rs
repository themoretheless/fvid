//! Multi-input audio mix fair-paired with FFmpeg `amix`.
use super::audio::AudioDecodeStats;
use super::lossless::{Codec, Frame};
use super::*;
use std::path::{Path, PathBuf};

pub use fvid_media_info::{MixDuration, MixAudioOptions, MixAudioStats, MergeAudioStats};
pub fn mix_audio(sources: &[PathBuf], destination: &Path, options: &MixAudioOptions) -> Result<MixAudioStats> {
    crate::owned_audio_mix::mix_with_decoder(sources, destination, options, decode_to_packed_f32)
}
pub fn merge_audio(sources: &[PathBuf], destination: &Path) -> Result<MergeAudioStats> {
    crate::owned_audio_mix::merge_with_decoder(sources, destination, decode_to_packed_f32)
}

fn decode_to_packed_f32(source: &Path) -> Result<(AudioDecodeStats, Vec<u8>)> {
    let options = CopyOptions::default();
    if crate::owned_audio_export::supports(source, Path::new("owned-mix.wav"), Default::default(), &options) {
        return crate::owned_audio_mix::decode_owned_audio(source);
    }
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
