//! Decode one audio stream to PCM while retaining its decoded sample precision.
use super::lossless::{Codec, Frame, Parameters};
use super::*;

pub use fvid_media_info::AudioDecodeStats;

pub use fvid_media_info::AudioDecodeTransform;

fn validate_sample_rate(rate: i32) -> Result<()> {
    if !(8_000..=384_000).contains(&rate) {
        return Err("sample rate must be within 8000..=384000".into());
    }
    Ok(())
}

fn validate_channels(channels: i32) -> Result<()> {
    if !(1..=64).contains(&channels) {
        return Err("channels must be within 1..=64".into());
    }
    Ok(())
}

fn validate_volume(gain: f64) -> Result<()> {
    if !gain.is_finite() || !(0.0..=64.0).contains(&gain) {
        return Err("volume must be a finite linear gain within 0..=64".into());
    }
    Ok(())
}

/// Apply linear gain in-place. Matches FFmpeg `volume=` on floating-point samples.
pub(crate) unsafe fn apply_volume(frame: *mut AVFrame, gain: f64) -> Result<()> {
    unsafe {
        if (gain - 1.0).abs() < 1e-15 {
            return Ok(());
        }
        check(
            av_frame_make_writable(frame),
            "make audio writable for volume",
        )?;
        let f = &*frame;
        let channels = f.ch_layout.nb_channels as usize;
        let samples = f.nb_samples as usize;
        if channels == 0 || samples == 0 || f.extended_data.is_null() {
            return Err("invalid audio frame for volume".into());
        }
        let planar = av_sample_fmt_is_planar(f.format) != 0;
        let fmt = f.format;
        if fmt == AVSampleFormat_AV_SAMPLE_FMT_FLT || fmt == AVSampleFormat_AV_SAMPLE_FMT_FLTP {
            let gain = gain as f32;
            if planar {
                for ch in 0..channels {
                    let plane = *f.extended_data.add(ch) as *mut f32;
                    if plane.is_null() {
                        return Err("missing audio plane for volume".into());
                    }
                    for i in 0..samples {
                        *plane.add(i) *= gain;
                    }
                }
            } else {
                let data = *f.extended_data as *mut f32;
                if data.is_null() {
                    return Err("missing audio data for volume".into());
                }
                for i in 0..(samples * channels) {
                    *data.add(i) *= gain;
                }
            }
        } else if fmt == AVSampleFormat_AV_SAMPLE_FMT_DBL
            || fmt == AVSampleFormat_AV_SAMPLE_FMT_DBLP
        {
            if planar {
                for ch in 0..channels {
                    let plane = *f.extended_data.add(ch) as *mut f64;
                    if plane.is_null() {
                        return Err("missing audio plane for volume".into());
                    }
                    for i in 0..samples {
                        *plane.add(i) *= gain;
                    }
                }
            } else {
                let data = *f.extended_data as *mut f64;
                if data.is_null() {
                    return Err("missing audio data for volume".into());
                }
                for i in 0..(samples * channels) {
                    *data.add(i) *= gain;
                }
            }
        } else {
            return Err(
                "volume requires floating-point PCM (flt/fltp/dbl/dblp); integer formats are not qualified"
                    .into(),
            );
        }
        Ok(())
    }
}

struct Resampler {
    owned: Option<crate::owned_resample::Resampler<Vec<u8>>>,
    owned_f64: Option<crate::owned_resample_f64::Resampler<Vec<u8>>>,
    input_rate: i32,
    input_channels: i32,
    swr: *mut SwrContext,
    out_rate: i32,
    out_format: AVSampleFormat,
    ch_layout: AVChannelLayout,
}

impl Drop for Resampler {
    fn drop(&mut self) {
        unsafe {
            av_channel_layout_uninit(&mut self.ch_layout);
            if !self.swr.is_null() {
                swr_free(&mut self.swr);
            }
        }
    }
}

impl Resampler {
    /// Open libswresample for rate and/or channel rematrix. `out_channels == source`
    /// keeps the decoded layout; otherwise uses FFmpeg's default layout for N (`-ac N`).
    unsafe fn open(frame: *const AVFrame, out_rate: i32, out_channels: i32) -> Result<Self> {
        unsafe {
            let f = &*frame;
            let out_format = av_get_packed_sample_fmt(f.format);
            if out_format < 0 {
                return Err("unsupported decoded audio format for resample".into());
            }
            let mut ch_layout = AVChannelLayout {
                order: 0,
                nb_channels: 0,
                u: std::mem::zeroed(),
                opaque: ptr::null_mut(),
            };
            if out_channels == f.ch_layout.nb_channels {
                check(
                    av_channel_layout_copy(&mut ch_layout, &f.ch_layout),
                    "copy resampler channel layout",
                )?;
            } else {
                av_channel_layout_default(&mut ch_layout, out_channels);
                if ch_layout.nb_channels != out_channels {
                    return Err("failed to build default channel layout".into());
                }
            }
            let mut swr = ptr::null_mut();
            let mut built = Self {
                owned: None,
                owned_f64: None,
                input_rate: f.sample_rate,
                input_channels: f.ch_layout.nb_channels,
                swr: ptr::null_mut(),
                out_rate,
                out_format,
                ch_layout,
            };
            let owned_layout = out_channels == f.ch_layout.nb_channels
                || (matches!(out_channels, 1 | 2) && (1..=6).contains(&f.ch_layout.nb_channels)
                    && f.ch_layout.order == AVChannelOrder_AV_CHANNEL_ORDER_NATIVE
                    && crate::owned_wav::default_pcm_mask(f.ch_layout.nb_channels as u16)
                        .is_ok_and(|mask| u64::from(mask) == f.ch_layout.u.mask));
            if owned_layout
                && matches!(f.format, AVSampleFormat_AV_SAMPLE_FMT_FLT | AVSampleFormat_AV_SAMPLE_FMT_FLTP)
            {
                built.owned = Some(crate::owned_resample::Resampler::new(
                    Vec::new(), u32::try_from(f.sample_rate).map_err(|_| "invalid input rate")?,
                    u32::try_from(out_rate).map_err(|_| "invalid output rate")?,
                    u16::try_from(out_channels).map_err(|_| "invalid channel count")?,
                ).map_err(|e| e.to_string())?);
                return Ok(built);
            }
            if out_channels == f.ch_layout.nb_channels
                && matches!(f.format, AVSampleFormat_AV_SAMPLE_FMT_DBL | AVSampleFormat_AV_SAMPLE_FMT_DBLP)
            {
                built.owned_f64 = Some(crate::owned_resample_f64::Resampler::new(
                    Vec::new(), u32::try_from(f.sample_rate).map_err(|_| "invalid input rate")?,
                    u32::try_from(out_rate).map_err(|_| "invalid output rate")?,
                    u16::try_from(out_channels).map_err(|_| "invalid channel count")?,
                ).map_err(|e| e.to_string())?);
                return Ok(built);
            }
            check(
                swr_alloc_set_opts2(
                    &mut swr,
                    &built.ch_layout,
                    out_format,
                    out_rate,
                    &f.ch_layout,
                    f.format,
                    f.sample_rate,
                    0,
                    ptr::null_mut(),
                ),
                "allocate audio resampler",
            )?;
            built.swr = swr;
            check(swr_init(built.swr), "initialize audio resampler")?;
            Ok(built)
        }
    }

    /// Convert one input frame (or flush with null). Returns output sample count.
    unsafe fn convert(&mut self, dst: *mut AVFrame, src: *const AVFrame) -> Result<i32> {
        unsafe {
            av_frame_unref(dst);
            if let Some(owned) = &mut self.owned {
                use std::io::Write;
                let channels = self.ch_layout.nb_channels as usize;
                if src.is_null() {
                    owned.finish().map_err(|e| e.to_string())?;
                } else {
                    let input = &*src;
                    if input.sample_rate != self.input_rate || input.ch_layout.nb_channels != self.input_channels
                        || input.nb_samples < 0 || input.extended_data.is_null()
                        || !matches!(input.format, AVSampleFormat_AV_SAMPLE_FMT_FLT | AVSampleFormat_AV_SAMPLE_FMT_FLTP)
                    { return Err("owned resampler input format changed".into()); }
                    let planar = input.format == AVSampleFormat_AV_SAMPLE_FMT_FLTP;
                    let mut bytes = Vec::with_capacity(input.nb_samples as usize * self.input_channels as usize * 4);
                    for sample in 0..input.nb_samples as usize {
                        for channel in 0..self.input_channels as usize {
                            let plane = *input.extended_data.add(if planar { channel } else { 0 });
                            if plane.is_null() { return Err("missing resampler PCM plane".into()); }
                            let index = if planar { sample } else { sample * self.input_channels as usize + channel };
                            let value = ptr::read_unaligned(plane.cast::<f32>().add(index));
                            bytes.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    let mut rematrix = crate::owned_pcm_gain::PcmGain::new(
                        &mut *owned, 1.0, self.input_channels as u16, channels as u16)?;
                    rematrix.write_all(&bytes).map_err(|e| e.to_string())?;
                    if !rematrix.frame_complete() { return Err("incomplete resampler input frame".into()); }
                }
                let bytes = owned.take_output();
                let count = i32::try_from(bytes.len() / (channels * 4)).map_err(|_| "resampled audio size overflow")?;
                if count == 0 { return Ok(0); }
                (*dst).format = AVSampleFormat_AV_SAMPLE_FMT_FLT;
                (*dst).sample_rate = self.out_rate;
                (*dst).nb_samples = count;
                check(av_channel_layout_copy(&mut (*dst).ch_layout, &self.ch_layout), "copy owned resample layout")?;
                check(av_frame_get_buffer(dst, 0), "allocate owned resample output")?;
                // Store native-endian AVFrame floats from the owned little-endian PCM stream.
                for (index, sample) in bytes.chunks_exact(4).enumerate() {
                    ptr::write_unaligned((*dst).data[0].cast::<f32>().add(index), f32::from_le_bytes(sample.try_into().unwrap()));
                }
                return Ok(count);
            }
            if let Some(owned) = &mut self.owned_f64 {
                use std::io::Write;
                let channels = self.ch_layout.nb_channels as usize;
                if src.is_null() {
                    owned.finish().map_err(|e| e.to_string())?;
                } else {
                    let input = &*src;
                    if input.sample_rate != self.input_rate || input.ch_layout.nb_channels != self.input_channels
                        || input.nb_samples < 0 || input.extended_data.is_null()
                        || !matches!(input.format, AVSampleFormat_AV_SAMPLE_FMT_DBL | AVSampleFormat_AV_SAMPLE_FMT_DBLP)
                    { return Err("owned resampler input format changed".into()); }
                    let planar = input.format == AVSampleFormat_AV_SAMPLE_FMT_DBLP;
                    let mut bytes = Vec::with_capacity(input.nb_samples as usize * self.input_channels as usize * 8);
                    for sample in 0..input.nb_samples as usize {
                        for channel in 0..self.input_channels as usize {
                            let plane = *input.extended_data.add(if planar { channel } else { 0 });
                            if plane.is_null() { return Err("missing resampler PCM plane".into()); }
                            let index = if planar { sample } else { sample * self.input_channels as usize + channel };
                            let value = ptr::read_unaligned(plane.cast::<f64>().add(index));
                            bytes.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    owned.write_all(&bytes).map_err(|e| e.to_string())?;
                }
                let bytes = owned.take_output();
                let count = i32::try_from(bytes.len() / (channels * 8)).map_err(|_| "resampled audio size overflow")?;
                if count == 0 { return Ok(0); }
                (*dst).format = AVSampleFormat_AV_SAMPLE_FMT_DBL;
                (*dst).sample_rate = self.out_rate;
                (*dst).nb_samples = count;
                check(av_channel_layout_copy(&mut (*dst).ch_layout, &self.ch_layout), "copy owned resample layout")?;
                check(av_frame_get_buffer(dst, 0), "allocate owned resample output")?;
                // Store native-endian AVFrame floats from the owned little-endian PCM stream.
                for (index, sample) in bytes.chunks_exact(8).enumerate() {
                    ptr::write_unaligned((*dst).data[0].cast::<f64>().add(index), f64::from_le_bytes(sample.try_into().unwrap()));
                }
                return Ok(count);
            }
            let out_samples = if src.is_null() {
                let delay = swr_get_delay(self.swr, i64::from(self.out_rate));
                if delay <= 0 {
                    return Ok(0);
                }
                delay
            } else {
                let s = &*src;
                let delay = swr_get_delay(self.swr, i64::from(self.out_rate));
                av_rescale_rnd(
                    delay + i64::from(s.nb_samples),
                    i64::from(self.out_rate),
                    i64::from(s.sample_rate),
                    AVRounding_AV_ROUND_UP,
                )
            };
            if out_samples < 0 || out_samples > i64::from(i32::MAX) {
                return Err("resampled audio size overflow".into());
            }
            (*dst).format = self.out_format;
            (*dst).sample_rate = self.out_rate;
            (*dst).nb_samples = out_samples as i32;
            check(
                av_channel_layout_copy(&mut (*dst).ch_layout, &self.ch_layout),
                "copy resample layout",
            )?;
            check(
                av_frame_get_buffer(dst, 0),
                "allocate resampled audio buffer",
            )?;
            let code = swr_convert_frame(self.swr, dst, src);
            if code < 0 {
                return Err(check(code, "resample audio frame").unwrap_err());
            }
            Ok((*dst).nb_samples)
        }
    }
}
#[derive(Default)]
pub(super) struct PacketPool {
    raw: *mut AVBufferPool,
    capacity: usize,
}
impl Drop for PacketPool {
    fn drop(&mut self) {
        // SAFETY: Pool is exclusively owned; outstanding refs keep their storage alive.
        unsafe {
            av_buffer_pool_uninit(&mut self.raw);
        }
    }
}
impl PacketPool {
    pub(super) fn get(&mut self, size: usize) -> Result<*mut AVBufferRef> {
        let needed = size
            .checked_add(AV_INPUT_BUFFER_PADDING_SIZE as usize)
            .ok_or("audio buffer size overflow")?;
        // SAFETY: Replacing a pool does not invalidate outstanding buffer references.
        unsafe {
            if needed > self.capacity {
                let replacement = av_buffer_pool_init(needed, None);
                if replacement.is_null() {
                    return Err("audio buffer pool allocation failed".into());
                }
                av_buffer_pool_uninit(&mut self.raw);
                self.raw = replacement;
                self.capacity = needed;
            }
            let buffer = av_buffer_pool_get(self.raw);
            if buffer.is_null() {
                return Err("audio buffer allocation failed".into());
            }
            ptr::write_bytes(
                (*buffer).data.add(size),
                0,
                AV_INPUT_BUFFER_PADDING_SIZE as usize,
            );
            Ok(buffer)
        }
    }
}
struct AudioSink {
    output: Option<Output>,
    wav_pcm: Option<Vec<u8>>,
    destination: PathBuf,
    format: AVSampleFormat,
    parameters: Parameters,
    pool: PacketPool,
    stats: AudioDecodeStats,
}
impl AudioSink {
    fn new(destination: &Path, input: &Input, index: usize, frame: &Frame) -> Result<Self> {
        // SAFETY: The decoder supplied a live audio frame; fresh parameters are
        // guarded before fallible calls and copied by Output before their release.
        unsafe {
            let f = &*frame.0;
            let format = av_get_packed_sample_fmt(f.format);
            let ids = [
                AVCodecID_AV_CODEC_ID_PCM_U8,
                AVCodecID_AV_CODEC_ID_PCM_S16LE,
                AVCodecID_AV_CODEC_ID_PCM_S32LE,
                AVCodecID_AV_CODEC_ID_PCM_F32LE,
                AVCodecID_AV_CODEC_ID_PCM_F64LE,
            ];
            let codec = if format == AVSampleFormat_AV_SAMPLE_FMT_S64 {
                AVCodecID_AV_CODEC_ID_PCM_S64LE
            } else {
                *ids.get(format as usize)
                    .ok_or("unsupported decoded audio format")?
            };
            if f.sample_rate <= 0 || !(1..=64).contains(&f.ch_layout.nb_channels) {
                return Err("invalid decoded audio rate/channel count".into());
            }
            let parameters = Parameters(avcodec_parameters_alloc());
            if parameters.0.is_null() {
                return Err("audio parameter allocation failed".into());
            }
            let p = &mut *parameters.0;
            p.codec_type = AVMediaType_AVMEDIA_TYPE_AUDIO;
            p.codec_id = codec;
            p.format = format;
            p.sample_rate = f.sample_rate;
            p.bits_per_coded_sample = av_get_bytes_per_sample(format) * 8;
            p.bits_per_raw_sample = p.bits_per_coded_sample;
            p.block_align = p.bits_per_coded_sample / 8 * f.ch_layout.nb_channels;
            p.bit_rate = i64::from(p.sample_rate) * i64::from(p.block_align) * 8;
            check(
                av_channel_layout_copy(&mut p.ch_layout, &f.ch_layout),
                "copy audio layout",
            )?;
            let stats = AudioDecodeStats {
                sample_frames: 0,
                decoded_frames: 0,
                sample_rate: f.sample_rate,
                channels: f.ch_layout.nb_channels,
                sample_format: string(av_get_sample_fmt_name(format)),
                planar_interleave_bytes: 0,
                decode_errors: 0,
            };
            let direct_float_wav = destination.extension().and_then(|v| v.to_str()) == Some("wav")
                && format == AVSampleFormat_AV_SAMPLE_FMT_FLT;
            if direct_float_wav {
                return Ok(Self {
                    output: None,
                    wav_pcm: Some(Vec::new()),
                    destination: destination.to_path_buf(),
                    format,
                    parameters,
                    pool: PacketPool::default(),
                    stats,
                });
            }
            let tb = AVRational {
                num: 1,
                den: f.sample_rate,
            };
            let output = Output::with_video(
                destination,
                input,
                &[index],
                Some((index, parameters.0, tb)),
            )?;
            Ok(Self {
                output: Some(output),
                wav_pcm: None,
                destination: destination.to_path_buf(),
                format,
                parameters,
                pool: PacketPool::default(),
                stats,
            })
        }
    }
    fn write(
        &mut self,
        frame: &Frame,
        packet: &mut Packet,
        limit: usize,
        offset: usize,
        count: usize,
    ) -> Result<()> {
        // SAFETY: Frame is decoder-owned and live. Packed payloads retain AVBuffer
        // ownership; planar interleave writes only the checked newly allocated packet.
        unsafe {
            let f = &*frame.0;
            let channels = self.stats.channels as usize;
            if av_get_packed_sample_fmt(f.format) != self.format
                || f.sample_rate != self.stats.sample_rate
                || av_channel_layout_compare(&f.ch_layout, &(*self.parameters.0).ch_layout) != 0
                || f.nb_samples <= 0
            {
                return Err("dynamic audio format/rate/layout is not supported".into());
            }
            let bytes = av_get_bytes_per_sample(self.format) as usize;
            let frame_count = f.nb_samples as usize;
            if count == 0
                || offset
                    .checked_add(count)
                    .is_none_or(|end| end > frame_count)
            {
                return Err("invalid decoded audio sample range".into());
            }
            let size = count
                .checked_mul(channels)
                .and_then(|n| n.checked_mul(bytes))
                .filter(|&n| n <= limit && n <= i32::MAX as usize)
                .ok_or("decoded audio block exceeds packet budget")?;
            if f.extended_data.is_null() {
                return Err("missing audio data".into());
            }
            if let Some(pcm) = self.wav_pcm.as_mut() {
                let start = pcm.len();
                pcm.resize(start + size, 0);
                if av_sample_fmt_is_planar(f.format) != 0 && channels > 1 {
                    if (f.linesize[0].max(0) as usize) < frame_count * bytes {
                        return Err("short planar audio buffer".into());
                    }
                    let mut planes = [ptr::null(); 64];
                    for (channel, plane) in planes[..channels].iter_mut().enumerate() {
                        let base = *f.extended_data.add(channel);
                        if base.is_null() {
                            return Err("missing audio plane".into());
                        }
                        *plane = base.add(offset * bytes);
                    }
                    super::audio_layout::interleave(
                        &planes[..channels],
                        pcm[start..].as_mut_ptr(),
                        count,
                        bytes,
                    );
                    self.stats.planar_interleave_bytes += size as u64;
                } else {
                    let data = *f.extended_data;
                    if data.is_null() {
                        return Err("missing audio data".into());
                    }
                    ptr::copy_nonoverlapping(
                        data.add(offset * channels * bytes),
                        pcm[start..].as_mut_ptr(),
                        size,
                    );
                }
                self.stats.sample_frames += count as u64;
                self.stats.decoded_frames += 1;
                return Ok(());
            }
            av_packet_unref(packet.0);
            if av_sample_fmt_is_planar(f.format) != 0 && channels > 1 {
                if (f.linesize[0].max(0) as usize) < frame_count * bytes {
                    return Err("short planar audio buffer".into());
                }
                let mut planes = [ptr::null(); 64];
                for (channel, plane) in planes[..channels].iter_mut().enumerate() {
                    let base = *f.extended_data.add(channel);
                    if base.is_null() {
                        return Err("missing audio plane".into());
                    }
                    *plane = base.add(offset * bytes);
                }
                (*packet.0).buf = self.pool.get(size)?;
                (*packet.0).data = (*(*packet.0).buf).data;
                (*packet.0).size = size as i32;
                super::audio_layout::interleave(
                    &planes[..channels],
                    (*packet.0).data,
                    count,
                    bytes,
                );
                self.stats.planar_interleave_bytes += size as u64;
            } else {
                if (*f.extended_data).is_null()
                    || f.buf[0].is_null()
                    || (f.linesize[0].max(0) as usize)
                        < frame_count
                            .checked_mul(channels)
                            .and_then(|n| n.checked_mul(bytes))
                            .ok_or("decoded audio frame size overflow")?
                {
                    return Err("invalid packed audio buffer".into());
                }
                (*packet.0).buf = av_buffer_ref(f.buf[0]);
                if (*packet.0).buf.is_null() {
                    return Err("audio buffer reference failed".into());
                }
                (*packet.0).data = (*f.extended_data).add(offset * channels * bytes);
                (*packet.0).size = size as i32;
            }
            (*packet.0).pts =
                i64::try_from(self.stats.sample_frames).map_err(|_| "audio timeline overflow")?;
            (*packet.0).dts = (*packet.0).pts;
            (*packet.0).duration = i64::try_from(count).map_err(|_| "audio duration overflow")?;
            self.output.as_mut().unwrap().write(
                packet,
                0,
                AVRational {
                    num: 1,
                    den: f.sample_rate,
                },
            )?;
            self.stats.sample_frames += count as u64;
            self.stats.decoded_frames += 1;
        }
        Ok(())
    }
    fn finish(mut self) -> Result<AudioDecodeStats> {
        if let Some(pcm) = self.wav_pcm.take() {
            let samples =
                unsafe { slice::from_raw_parts(pcm.as_ptr() as *const f32, pcm.len() / 4) };
            super::wav::write_wav_f32le(
                &self.destination,
                self.stats.sample_rate,
                self.stats.channels,
                samples,
            )?;
            return Ok(self.stats);
        }
        self.output.take().unwrap().finish()?;
        Ok(self.stats)
    }
}
fn emit_decoded_audio(
    destination: &Path,
    input: &Input,
    index: usize,
    frame: &Frame,
    sink: &mut Option<AudioSink>,
    pcm: &mut Packet,
    sample_bounds: &mut Option<(u64, u64)>,
    output_sample_frames: &mut u64,
    interval: Option<(i64, i64)>,
    volume: Option<f64>,
    max_packet_bytes: usize,
    decoded_bump: bool,
) -> Result<bool> {
    if let Some(gain) = volume {
        unsafe {
            apply_volume(frame.0, gain)?;
        }
    }
    let (frame_count, sample_rate) = unsafe {
        let f = &*frame.0;
        if f.nb_samples <= 0 || f.sample_rate <= 0 {
            return Err("invalid decoded audio frame geometry".into());
        }
        (f.nb_samples as u64, f.sample_rate)
    };
    if sample_bounds.is_none() {
        let sample_at = |time_us: i64| -> Result<u64> {
            let scaled = i128::from(time_us) * i128::from(sample_rate);
            let samples = (scaled + 999_999) / 1_000_000;
            u64::try_from(samples).map_err(|_| "audio interval overflow".into())
        };
        *sample_bounds = Some(match interval {
            Some((from, to)) => (sample_at(from)?, sample_at(to)?),
            None => (0, u64::MAX),
        });
    }
    let (wanted_start, wanted_end) = sample_bounds.expect("sample bounds initialized");
    let frame_start = *output_sample_frames;
    let frame_end = frame_start
        .checked_add(frame_count)
        .ok_or("decoded audio timeline overflow")?;
    let overlap_start = frame_start.max(wanted_start);
    let overlap_end = frame_end.min(wanted_end);
    if overlap_start < overlap_end {
        if sink.is_none() {
            *sink = Some(AudioSink::new(destination, input, index, frame)?);
        }
        let active = sink.as_mut().unwrap();
        if decoded_bump {
            active.stats.decoded_frames += 1;
        }
        active.write(
            frame,
            pcm,
            max_packet_bytes,
            usize::try_from(overlap_start - frame_start)
                .map_err(|_| "audio sample offset overflow")?,
            usize::try_from(overlap_end - overlap_start)
                .map_err(|_| "audio sample count overflow")?,
        )?;
    } else if decoded_bump {
        if let Some(active) = sink.as_mut() {
            active.stats.decoded_frames += 1;
        }
    }
    *output_sample_frames = frame_end;
    Ok(*output_sample_frames >= wanted_end)
}

/// Export the contiguous decoded sample sequence, preserving decoded precision.
/// Source timestamp gaps are not synthesized into silence in this extraction API.
pub fn decode_audio(
    source: &Path,
    destination: &Path,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_audio_transformed(
        source,
        destination,
        AudioDecodeTransform::default(),
        options,
    )
}

/// Export a half-open interval from the contiguous decoded sample sequence.
/// Boundaries are microseconds from the first decoded sample and round up to
/// the first sample whose presentation time is not before the boundary.
pub fn decode_audio_interval(
    source: &Path,
    destination: &Path,
    interval: Option<(i64, i64)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_audio_transformed(
        source,
        destination,
        AudioDecodeTransform {
            interval,
            sample_rate: None,
            channels: None,
            volume: None,
        },
        options,
    )
}

/// Decode audio with optional interval, sample-rate, and channel rematrix (`libswresample`).
pub fn decode_audio_transformed(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if !cfg!(target_endian = "little") {
        return Err("PCM export requires little-endian host".into());
    }
    if transform
        .interval
        .is_some_and(|(from, to)| from < 0 || to <= from)
    {
        return Err("decode-audio interval requires 0 <= from < to".into());
    }
    if let Some(rate) = transform.sample_rate {
        validate_sample_rate(rate)?;
    }
    if let Some(channels) = transform.channels {
        validate_channels(channels)?;
    }
    if let Some(gain) = transform.volume {
        validate_volume(gain)?;
    }
    let mut input = Input::open_fast(source)?;
    // SAFETY: Input owns a live format context. A contiguous sample extraction
    // cannot carry arbitrary source chapter times without a separate clock mapping.
    if unsafe { (*input.0).nb_chapters } != 0 {
        return Err("decode-audio with chapters requires explicit timeline mapping".into());
    }
    let selected = selection(&input, options)?;
    if selected.len() != 1 {
        return Err(
            "decode-audio requires exactly one selected audio stream; use --streams".into(),
        );
    }
    let index = selected[0];
    // SAFETY: Input and codec parameters remain live; RAII owns allocated decoder.
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
    let mut pcm = Packet::new()?;
    let frame = Frame::new()?;
    let resampled = Frame::new()?;
    let mut resampler: Option<Resampler> = None;
    let mut sink = None;
    let mut sample_bounds = None;
    let mut output_sample_frames = 0u64;
    let mut finished = false;
    let mut decode_errors = 0u64;
    let mut demux_errors = 0u64;
    let mut final_drain = false;

    'packets: loop {
        let available = match if final_drain {
            Ok(false)
        } else {
            packet.try_read(&mut input)
        } {
            Ok(available) => available,
            Err(INVALID_DATA) => {
                demux_errors += 1;
                // A demuxer that keeps reporting damage without advancing has nothing left to
                // salvage. Stop where the CLI stops: keep the audio already decoded, drain what
                // the decoder still holds and report the error count. Only a stream that never
                // produced anything is an outright failure.
                if demux_errors > 64 {
                    if sink.is_none() {
                        return Err(check(INVALID_DATA, "read packet").unwrap_err());
                    }
                    final_drain = true;
                }
                continue;
            }
            Err(code) => return Err(check(code, "read packet").unwrap_err()),
        };
        if available {
            let (stream, _) = packet_info_for_decode(&packet, &input, options)?;
            if stream != index {
                continue;
            }
        }
        // SAFETY: Packet remains live for send; null signals final decoder drain.
        let send = unsafe {
            avcodec_send_packet(decoder.0, if available { packet.0 } else { ptr::null() })
        };
        if send == INVALID_DATA {
            decode_errors += 1;
            if !available {
                break;
            }
            continue 'packets;
        }
        check(send, "send audio packet")?;
        loop {
            // SAFETY: Decoder and reusable frame are live; receive owns returned buffers.
            let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
            if code == -libc::EAGAIN || code == EOF {
                break;
            }
            if code == INVALID_DATA {
                decode_errors += 1;
                unsafe { av_frame_unref(frame.0) };
                break;
            }
            check(code, "receive audio frame")?;
            let needs_swr = unsafe {
                let f = &*frame.0;
                transform
                    .sample_rate
                    .is_some_and(|rate| rate != f.sample_rate)
                    || transform
                        .channels
                        .is_some_and(|channels| channels != f.ch_layout.nb_channels)
            };
            if needs_swr {
                unsafe {
                    if resampler.is_none() {
                        let f = &*frame.0;
                        let out_rate = transform.sample_rate.unwrap_or(f.sample_rate);
                        let out_channels = transform.channels.unwrap_or(f.ch_layout.nb_channels);
                        resampler = Some(Resampler::open(frame.0, out_rate, out_channels)?);
                    }
                    let swr = resampler.as_mut().unwrap();
                    let produced = swr.convert(resampled.0, frame.0)?;
                    if produced > 0 {
                        if emit_decoded_audio(
                            destination,
                            &input,
                            index,
                            &resampled,
                            &mut sink,
                            &mut pcm,
                            &mut sample_bounds,
                            &mut output_sample_frames,
                            transform.interval,
                            transform.volume,
                            options.max_packet_bytes,
                            true,
                        )? {
                            finished = true;
                        }
                    } else if let Some(active) = sink.as_mut() {
                        active.stats.decoded_frames += 1;
                    }
                    av_frame_unref(frame.0);
                }
            } else if emit_decoded_audio(
                destination,
                &input,
                index,
                &frame,
                &mut sink,
                &mut pcm,
                &mut sample_bounds,
                &mut output_sample_frames,
                transform.interval,
                transform.volume,
                options.max_packet_bytes,
                true,
            )? {
                finished = true;
                unsafe {
                    av_frame_unref(frame.0);
                }
                break;
            } else {
                unsafe {
                    av_frame_unref(frame.0);
                }
            }
            if finished {
                break;
            }
        }
        if finished {
            break 'packets;
        }
        if !available {
            break;
        }
    }
    if let Some(swr) = resampler.as_mut() {
        loop {
            let produced = unsafe { swr.convert(resampled.0, ptr::null())? };
            if produced <= 0 {
                break;
            }
            if emit_decoded_audio(
                destination,
                &input,
                index,
                &resampled,
                &mut sink,
                &mut pcm,
                &mut sample_bounds,
                &mut output_sample_frames,
                transform.interval,
                transform.volume,
                options.max_packet_bytes,
                false,
            )? {
                break;
            }
        }
    }
    let mut sink = sink.ok_or("no decoded audio samples")?;
    sink.stats.decode_errors = decode_errors + demux_errors;
    sink.finish()
}

/// PCM codec parameters for muxing decoded AAC/MP3/FLAC during lossless intervals.
pub(super) fn pcm_parameters_for_interval_decode(
    codecpar: &AVCodecParameters,
) -> Result<(Parameters, AVRational, AVSampleFormat)> {
    unsafe {
        let format = match codecpar.codec_id {
            AVCodecID_AV_CODEC_ID_AAC | AVCodecID_AV_CODEC_ID_MP3 => {
                AVSampleFormat_AV_SAMPLE_FMT_FLT
            }
            AVCodecID_AV_CODEC_ID_FLAC => AVSampleFormat_AV_SAMPLE_FMT_S16,
            _ => return Err("lossless interval decode supports AAC, MP3, and FLAC only".into()),
        };
        if codecpar.sample_rate <= 0 || !(1..=64).contains(&codecpar.ch_layout.nb_channels) {
            return Err("invalid compressed audio rate/channel count".into());
        }
        let codec = match format {
            AVSampleFormat_AV_SAMPLE_FMT_FLT => AVCodecID_AV_CODEC_ID_PCM_F32LE,
            AVSampleFormat_AV_SAMPLE_FMT_S16 => AVCodecID_AV_CODEC_ID_PCM_S16LE,
            _ => return Err("unsupported interval PCM format".into()),
        };
        let parameters = Parameters(avcodec_parameters_alloc());
        if parameters.0.is_null() {
            return Err("audio parameter allocation failed".into());
        }
        let p = &mut *parameters.0;
        p.codec_type = AVMediaType_AVMEDIA_TYPE_AUDIO;
        p.codec_id = codec;
        p.format = format;
        p.sample_rate = codecpar.sample_rate;
        p.bits_per_coded_sample = av_get_bytes_per_sample(format) * 8;
        p.bits_per_raw_sample = p.bits_per_coded_sample;
        p.block_align = p.bits_per_coded_sample / 8 * codecpar.ch_layout.nb_channels;
        p.bit_rate = i64::from(p.sample_rate) * i64::from(p.block_align) * 8;
        check(
            av_channel_layout_copy(&mut p.ch_layout, &codecpar.ch_layout),
            "copy audio layout",
        )?;
        let tb = AVRational {
            num: 1,
            den: codecpar.sample_rate,
        };
        Ok((parameters, tb, format))
    }
}

/// Write a decoded audio frame slice into an existing muxer as packed PCM.
/// Returns true once the contiguous decoded timeline reaches the interval end.
pub(super) fn write_interval_pcm_frame(
    output: &mut Output,
    mapped: usize,
    frame: &Frame,
    packet: &mut Packet,
    pool: &mut PacketPool,
    expected_format: AVSampleFormat,
    decoded_sample_frames: &mut u64,
    written_sample_frames: &mut u64,
    sample_bounds: &mut Option<(u64, u64)>,
    interval_us: (i64, i64),
    max_packet_bytes: usize,
) -> Result<bool> {
    let (frame_count, sample_rate, channels, packed) = unsafe {
        let f = &*frame.0;
        if f.nb_samples <= 0 || f.sample_rate <= 0 {
            return Err("invalid decoded audio frame geometry".into());
        }
        let packed = av_get_packed_sample_fmt(f.format);
        if packed != expected_format {
            return Err("decoded audio format does not match interval PCM override".into());
        }
        (
            f.nb_samples as u64,
            f.sample_rate,
            f.ch_layout.nb_channels as usize,
            packed,
        )
    };
    if sample_bounds.is_none() {
        let sample_at = |time_us: i64| -> Result<u64> {
            let scaled = i128::from(time_us) * i128::from(sample_rate);
            let samples = (scaled + 999_999) / 1_000_000;
            u64::try_from(samples).map_err(|_| "audio interval overflow".into())
        };
        *sample_bounds = Some((sample_at(interval_us.0)?, sample_at(interval_us.1)?));
    }
    let (wanted_start, wanted_end) = sample_bounds.expect("sample bounds initialized");
    let frame_start = *decoded_sample_frames;
    let frame_end = frame_start
        .checked_add(frame_count)
        .ok_or("decoded audio timeline overflow")?;
    let overlap_start = frame_start.max(wanted_start);
    let overlap_end = frame_end.min(wanted_end);
    if overlap_start < overlap_end {
        let offset = usize::try_from(overlap_start - frame_start)
            .map_err(|_| "audio sample offset overflow")?;
        let count = usize::try_from(overlap_end - overlap_start)
            .map_err(|_| "audio sample count overflow")?;
        unsafe {
            let f = &*frame.0;
            let bytes = av_get_bytes_per_sample(packed) as usize;
            let size = count
                .checked_mul(channels)
                .and_then(|n| n.checked_mul(bytes))
                .filter(|&n| n <= max_packet_bytes && n <= i32::MAX as usize)
                .ok_or("decoded audio block exceeds packet budget")?;
            av_packet_unref(packet.0);
            if av_sample_fmt_is_planar(f.format) != 0 && channels > 1 {
                if (f.linesize[0].max(0) as usize) < frame_count as usize * bytes {
                    return Err("short planar audio buffer".into());
                }
                let mut planes = [ptr::null(); 64];
                for (channel, plane) in planes[..channels].iter_mut().enumerate() {
                    let base = *f.extended_data.add(channel);
                    if base.is_null() {
                        return Err("missing audio plane".into());
                    }
                    *plane = base.add(offset * bytes);
                }
                (*packet.0).buf = pool.get(size)?;
                (*packet.0).data = (*(*packet.0).buf).data;
                (*packet.0).size = size as i32;
                super::audio_layout::interleave(
                    &planes[..channels],
                    (*packet.0).data,
                    count,
                    bytes,
                );
            } else {
                if (*f.extended_data).is_null()
                    || f.buf[0].is_null()
                    || (f.linesize[0].max(0) as usize)
                        < (frame_count as usize)
                            .checked_mul(channels)
                            .and_then(|n| n.checked_mul(bytes))
                            .ok_or("decoded audio frame size overflow")?
                {
                    return Err("invalid packed audio buffer".into());
                }
                (*packet.0).buf = av_buffer_ref(f.buf[0]);
                if (*packet.0).buf.is_null() {
                    return Err("audio buffer reference failed".into());
                }
                (*packet.0).data = (*f.extended_data).add(offset * channels * bytes);
                (*packet.0).size = size as i32;
            }
            (*packet.0).pts =
                i64::try_from(*written_sample_frames).map_err(|_| "audio timeline overflow")?;
            (*packet.0).dts = (*packet.0).pts;
            (*packet.0).duration = i64::try_from(count).map_err(|_| "audio duration overflow")?;
            output.write(
                packet,
                mapped,
                AVRational {
                    num: 1,
                    den: sample_rate,
                },
            )?;
            *written_sample_frames = written_sample_frames
                .checked_add(count as u64)
                .ok_or("audio timeline overflow")?;
        }
    }
    *decoded_sample_frames = frame_end;
    Ok(*decoded_sample_frames >= wanted_end)
}

/// Decode one compressed audio stream from a fresh demuxer (no seek) into `output`.
/// Used when the video path seeks: mid-stream AAC/MP3 state is not sample-identical to a
/// from-start decode, so audio keeps the contiguous sample-window path on its own Input.
pub(super) fn mux_interval_pcm_from_path(
    source: &Path,
    output: &mut Output,
    stream_index: usize,
    mapped: usize,
    interval_us: (i64, i64),
    max_packet_bytes: usize,
) -> Result<u64> {
    let mut input = Input::open_fast(source)?;
    if stream_index >= input.streams().len() {
        return Err("audio stream index out of range".into());
    }
    // SAFETY: Index checked; codecpar owned by Input.
    let codecpar = unsafe { &*(*input.streams()[stream_index]).codecpar };
    let (pcm_params, _tb, format) = pcm_parameters_for_interval_decode(codecpar)?;
    drop(pcm_params);
    let decoder = unsafe {
        let codec = avcodec_find_decoder(codecpar.codec_id);
        if codec.is_null() {
            return Err("audio decoder unavailable".into());
        }
        let decoder = Codec(avcodec_alloc_context3(codec));
        if decoder.0.is_null() {
            return Err("audio decoder allocation failed".into());
        }
        check(
            avcodec_parameters_to_context(decoder.0, codecpar),
            "configure audio decoder",
        )?;
        (*decoder.0).pkt_timebase = (*input.streams()[stream_index]).time_base;
        check(
            avcodec_open2(decoder.0, codec, ptr::null_mut()),
            "open audio decoder",
        )?;
        decoder
    };
    let mut packet = Packet::new()?;
    let mut encoded = Packet::new()?;
    let frame = Frame::new()?;
    let mut pool = PacketPool::default();
    let mut decoded_sample_frames = 0u64;
    let mut written_sample_frames = 0u64;
    let mut sample_bounds = None;
    let options = CopyOptions {
        max_packet_bytes,
        ..CopyOptions::default()
    };
    let mut done = false;
    loop {
        let available = packet.read(&mut input)?;
        if available {
            let (index, _) = packet_info_for_decode(&packet, &input, &options)?;
            if index != stream_index {
                continue;
            }
        }
        check(
            unsafe {
                avcodec_send_packet(decoder.0, if available { packet.0 } else { ptr::null() })
            },
            "send compressed audio packet",
        )?;
        loop {
            let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
            if code == -libc::EAGAIN || code == EOF {
                break;
            }
            check(code, "receive decoded audio frame")?;
            done = write_interval_pcm_frame(
                output,
                mapped,
                &frame,
                &mut encoded,
                &mut pool,
                format,
                &mut decoded_sample_frames,
                &mut written_sample_frames,
                &mut sample_bounds,
                interval_us,
                max_packet_bytes,
            )?;
            unsafe {
                av_frame_unref(frame.0);
            }
            if done {
                return Ok(written_sample_frames);
            }
        }
        if !available {
            break;
        }
    }
    if !done {
        return Err("compressed audio interval ended before sample window completed".into());
    }
    Ok(written_sample_frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packet_pool_preserves_retained_refs_reuses_free_storage_and_pads() {
        let mut pool = PacketPool::default();
        let mut first = pool.get(32).unwrap();
        let mut second = pool.get(32).unwrap();
        // SAFETY: Test owns all returned AVBuffer references; each is released once.
        unsafe {
            let original = (*first).data;
            assert_ne!(original, (*second).data);
            ptr::write_bytes(original, 0x5a, 32);
            assert_eq!(
                slice::from_raw_parts(original.add(32), AV_INPUT_BUFFER_PADDING_SIZE as usize),
                &[0; AV_INPUT_BUFFER_PADDING_SIZE as usize]
            );
            av_buffer_unref(&mut first);
            let mut reused = pool.get(16).unwrap();
            assert_eq!((*reused).data, original);
            assert_eq!(*(*reused).data, 0x5a);
            assert_eq!(
                slice::from_raw_parts(
                    (*reused).data.add(16),
                    AV_INPUT_BUFFER_PADDING_SIZE as usize
                ),
                &[0; AV_INPUT_BUFFER_PADDING_SIZE as usize]
            );
            let mut grown = pool.get(4096).unwrap();
            drop(pool);
            // Replacing and then dropping the pool must not invalidate live refs.
            assert_eq!(*(*reused).data, 0x5a);
            ptr::write_bytes((*grown).data, 0x33, 4096);
            assert_eq!(*(*grown).data.add(4095), 0x33);
            av_buffer_unref(&mut reused);
            av_buffer_unref(&mut second);
            av_buffer_unref(&mut grown);
        }
    }
}

#[cfg(test)]
mod owned_rate_tests {
    use super::*;
    #[test]
    fn float_rate_adapter_uses_owned_filter_for_packed_and_planar_pcm() {
        use std::io::Write;
        for (planar, input_channels, output_channels) in [(false, 2, 2), (true, 2, 2), (false, 2, 1), (true, 2, 1), (false, 6, 2), (true, 6, 2)] {
            let pcm: Vec<f32> = (0..997).flat_map(|i| [(i as f32 * 0.07).sin(), -0.25, 0.5, 1.0, 0.1, 0.2].into_iter().take(input_channels as usize)).collect();
            let mut reference = crate::owned_resample::Resampler::new(Vec::new(), 48000, 16000, output_channels).unwrap();
            crate::owned_pcm_gain::PcmGain::new(&mut reference, 1.0, input_channels, output_channels).unwrap()
                .write_all(&pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>()).unwrap();
            reference.finish().unwrap();
            let expected = reference.take_output();
            let input = Frame::new().unwrap();
            let output = Frame::new().unwrap();
            // SAFETY: RAII owns both frames; libav allocates the checked 997x2 float buffers.
            unsafe {
                (*input.0).format = if planar { AVSampleFormat_AV_SAMPLE_FMT_FLTP } else { AVSampleFormat_AV_SAMPLE_FMT_FLT };
                (*input.0).sample_rate = 48000;
                (*input.0).nb_samples = 997;
                av_channel_layout_default(&mut (*input.0).ch_layout, i32::from(input_channels));
                check(av_frame_get_buffer(input.0, 0), "test input").unwrap();
                for sample in 0..997 {
                    for channel in 0..input_channels as usize {
                        let plane = *(*input.0).extended_data.add(if planar { channel } else { 0 });
                        ptr::write_unaligned(plane.cast::<f32>().add(if planar {sample} else {sample * input_channels as usize + channel}), pcm[sample * input_channels as usize + channel]);
                    }
                }
                let mut adapter = Resampler::open(input.0, 16000, i32::from(output_channels)).unwrap();
                assert!(adapter.owned.is_some());
                assert!(adapter.swr.is_null());
                let mut actual = Vec::new();
                for source in [input.0 as *const AVFrame, ptr::null()] {
                    let count = adapter.convert(output.0, source).unwrap();
                    for i in 0..count as usize * output_channels as usize {
                        actual.extend_from_slice(&ptr::read_unaligned((*output.0).data[0].cast::<f32>().add(i)).to_le_bytes());
                    }
                }
                assert_eq!(actual, expected);
                assert_eq!(actual.len(), 333 * output_channels as usize * 4);
                assert_eq!(adapter.convert(output.0, ptr::null()).unwrap(), 0);
            }
        }
    }
}

#[cfg(test)]
mod owned_double_rate_tests {
    use super::*;
    #[test]
    fn double_rate_adapter_uses_owned_filter_for_packed_and_planar_pcm() {
        use std::io::Write;
        for (planar, input_channels, output_channels) in [(false, 2, 2), (true, 2, 2)] {
            let pcm: Vec<f64> = (0..997).flat_map(|i| [(i as f64 * 0.07).sin(), -0.25, 0.5, 1.0, 0.1, 0.2].into_iter().take(input_channels as usize)).collect();
            let mut reference = crate::owned_resample_f64::Resampler::new(Vec::new(), 48000, 16000, output_channels).unwrap();
            reference.write_all(&pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>()).unwrap();
            reference.finish().unwrap();
            let expected = reference.take_output();
            let input = Frame::new().unwrap();
            let output = Frame::new().unwrap();
            // SAFETY: RAII owns both frames; libav allocates the checked 997x2 float buffers.
            unsafe {
                (*input.0).format = if planar { AVSampleFormat_AV_SAMPLE_FMT_DBLP } else { AVSampleFormat_AV_SAMPLE_FMT_DBL };
                (*input.0).sample_rate = 48000;
                (*input.0).nb_samples = 997;
                av_channel_layout_default(&mut (*input.0).ch_layout, i32::from(input_channels));
                check(av_frame_get_buffer(input.0, 0), "test input").unwrap();
                for sample in 0..997 {
                    for channel in 0..input_channels as usize {
                        let plane = *(*input.0).extended_data.add(if planar { channel } else { 0 });
                        ptr::write_unaligned(plane.cast::<f64>().add(if planar {sample} else {sample * input_channels as usize + channel}), pcm[sample * input_channels as usize + channel]);
                    }
                }
                let mut adapter = Resampler::open(input.0, 16000, i32::from(output_channels)).unwrap();
                assert!(adapter.owned_f64.is_some());
                assert!(adapter.swr.is_null());
                let mut actual = Vec::new();
                for source in [input.0 as *const AVFrame, ptr::null()] {
                    let count = adapter.convert(output.0, source).unwrap();
                    for i in 0..count as usize * output_channels as usize {
                        actual.extend_from_slice(&ptr::read_unaligned((*output.0).data[0].cast::<f64>().add(i)).to_le_bytes());
                    }
                }
                assert_eq!(actual, expected);
                assert_eq!(actual.len(), 333 * output_channels as usize * 8);
                assert_eq!(adapter.convert(output.0, ptr::null()).unwrap(), 0);
            }
        }
    }
}
