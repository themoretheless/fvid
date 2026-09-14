//! Decode one audio stream to PCM while retaining its decoded sample precision.
use super::lossless::{Codec, Frame, Parameters};
use super::*;

#[derive(Serialize, Debug)]
pub struct AudioDecodeStats {
    pub sample_frames: u64,
    pub decoded_frames: u64,
    pub sample_rate: i32,
    pub channels: i32,
    pub sample_format: String,
    pub planar_interleave_bytes: u64,
}
#[derive(Default)]
struct PacketPool {
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
    fn get(&mut self, size: usize) -> Result<*mut AVBufferRef> {
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
    output: Output,
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
                output,
                format,
                parameters,
                pool: PacketPool::default(),
                stats: AudioDecodeStats {
                    sample_frames: 0,
                    decoded_frames: 0,
                    sample_rate: f.sample_rate,
                    channels: f.ch_layout.nb_channels,
                    sample_format: string(av_get_sample_fmt_name(format)),
                    planar_interleave_bytes: 0,
                },
            })
        }
    }
    fn write(&mut self, frame: &Frame, packet: &mut Packet, limit: usize) -> Result<()> {
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
            let count = f.nb_samples as usize;
            let size = count
                .checked_mul(channels)
                .and_then(|n| n.checked_mul(bytes))
                .filter(|&n| n <= limit && n <= i32::MAX as usize)
                .ok_or("decoded audio block exceeds packet budget")?;
            if f.extended_data.is_null() {
                return Err("missing audio data".into());
            }
            av_packet_unref(packet.0);
            if av_sample_fmt_is_planar(f.format) != 0 && channels > 1 {
                if (f.linesize[0].max(0) as usize) < count * bytes {
                    return Err("short planar audio buffer".into());
                }
                let mut planes = [ptr::null(); 64];
                for (channel, plane) in planes[..channels].iter_mut().enumerate() {
                    *plane = *f.extended_data.add(channel);
                    if plane.is_null() {
                        return Err("missing audio plane".into());
                    }
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
                    || (f.linesize[0].max(0) as usize) < size
                {
                    return Err("invalid packed audio buffer".into());
                }
                (*packet.0).buf = av_buffer_ref(f.buf[0]);
                if (*packet.0).buf.is_null() {
                    return Err("audio buffer reference failed".into());
                }
                (*packet.0).data = *f.extended_data;
                (*packet.0).size = size as i32;
            }
            (*packet.0).pts =
                i64::try_from(self.stats.sample_frames).map_err(|_| "audio timeline overflow")?;
            (*packet.0).dts = (*packet.0).pts;
            (*packet.0).duration = f.nb_samples as i64;
            self.output.write(
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
}
/// Export the contiguous decoded sample sequence, preserving decoded precision.
/// Source timestamp gaps are not synthesized into silence in this extraction API.
pub fn decode_audio(
    source: &Path,
    destination: &Path,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if !cfg!(target_endian = "little") {
        return Err("PCM export requires little-endian host".into());
    }
    let mut input = Input::open(source)?;
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
    let mut sink = None;
    loop {
        let available = packet.read(&mut input)?;
        if available {
            let (stream, _) = packet_info(&packet, &input, options)?;
            if stream != index {
                continue;
            }
        }
        // SAFETY: Packet remains live for send; null signals final decoder drain.
        check(
            unsafe {
                avcodec_send_packet(decoder.0, if available { packet.0 } else { ptr::null() })
            },
            "send audio packet",
        )?;
        loop {
            // SAFETY: Decoder and reusable frame are live; receive owns returned buffers.
            let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
            if code == -libc::EAGAIN || code == EOF {
                break;
            }
            check(code, "receive audio frame")?;
            if sink.is_none() {
                sink = Some(AudioSink::new(destination, &input, index, &frame)?);
            }
            sink.as_mut()
                .unwrap()
                .write(&frame, &mut pcm, options.max_packet_bytes)?;
            // SAFETY: PCM packet retained its own ref or copied planar samples.
            unsafe {
                av_frame_unref(frame.0);
            }
        }
        if !available {
            break;
        }
    }
    let sink = sink.ok_or("no decoded audio samples")?;
    sink.output.finish()?;
    Ok(sink.stats)
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
