//! CUDA decode → fvid-cuda NV12 filter → NVENC encode (device-resident vertical slice).
#![cfg(feature = "cuda-hw")]

use super::*;
use fvid_cuda::{Nv12Processor, Nv12Transform, Nv12View};
use lossless::{Codec, CropRect, Frame, Parameters};
use serde::Serialize;
use std::path::Path;
use std::ptr;

const AGAIN: i32 = -libc::EAGAIN;
/// Filtered-path CUDA output slots (identity/copy uses decoder surfaces).
const OUT_POOL: usize = 8;

#[derive(Clone, Copy, Debug, Default)]
pub struct HwFilterOptions {
    pub crop: Option<CropRect>,
    pub horizontal_flip: bool,
    pub vertical_flip: bool,
    pub device: usize,
    /// Force a host round-trip (hwdownload then hwupload) before the device
    /// filter — same PCIe tax as FFmpeg `hwdownload,hwupload_cuda`. Counts toward
    /// `host_frame_copies`. Default path stays device-resident (`0` copies).
    pub host_bounce: bool,
}

#[derive(Serialize, Debug)]
pub struct HwFilterStats {
    pub backend: &'static str,
    pub device: String,
    pub video_frames: u64,
    pub width: u32,
    pub height: u32,
    /// Full-frame CUDA↔host transfers via `av_hwframe_transfer_data`
    /// (FFmpeg `hwupload_cuda` / `hwdownload` equivalents).
    pub host_frame_copies: u64,
    pub device_filter_passes: u64,
    pub encoder: &'static str,
    pub host_bounce: bool,
}

struct HwDevice(pub *mut AVBufferRef);
impl Drop for HwDevice {
    fn drop(&mut self) {
        // SAFETY: uniquely owned device context reference.
        unsafe {
            av_buffer_unref(&mut self.0);
        }
    }
}

fn make_cuda_device(ordinal: usize) -> Result<HwDevice> {
    let mut device = ptr::null_mut();
    // Share the CUDA primary context with fvid-cuda/cudarc (avoids per-op context switches).
    const AV_CUDA_USE_PRIMARY_CONTEXT: u32 = 1 << 0;
    // SAFETY: Creates a new CUDA hardware device context; null checked below.
    let code = unsafe {
        av_hwdevice_ctx_create(
            &mut device,
            AVHWDeviceType_AV_HWDEVICE_TYPE_CUDA,
            cstring(&ordinal.to_string())?.as_ptr(),
            ptr::null_mut(),
            AV_CUDA_USE_PRIMARY_CONTEXT as i32,
        )
    };
    check(code, "create CUDA hwdevice")?;
    if device.is_null() {
        return Err("CUDA hwdevice allocation returned null".into());
    }
    Ok(HwDevice(device))
}

/// Layout of `AVCUDADeviceContext` (libavutil/hwcontext_cuda.h) without pulling cuda.h into bindgen.
#[repr(C)]
struct AvCudaDeviceContext {
    cuda_ctx: *mut std::ffi::c_void,
    stream: *mut std::ffi::c_void,
    internal: *mut std::ffi::c_void,
}

fn ffmpeg_cuda_stream(device: &HwDevice) -> Result<*mut std::ffi::c_void> {
    // SAFETY: `device` is a live AVHWDeviceContext of type CUDA created above.
    unsafe {
        if device.0.is_null() || (*device.0).data.is_null() {
            return Err("CUDA hwdevice missing data".into());
        }
        let hw = &*((*device.0).data as *const AVHWDeviceContext);
        if hw.hwctx.is_null() {
            return Err("CUDA hwdevice missing hwctx".into());
        }
        let cuda = &*(hw.hwctx as *const AvCudaDeviceContext);
        Ok(cuda.stream)
    }
}

unsafe extern "C" fn get_cuda_format(
    _ctx: *mut AVCodecContext,
    pix_fmts: *const AVPixelFormat,
) -> AVPixelFormat {
    // SAFETY: FFmpeg passes a -1-terminated list of pixel formats.
    unsafe {
        let mut p = pix_fmts;
        while !p.is_null() && *p != -1 {
            if *p == AVPixelFormat_AV_PIX_FMT_CUDA {
                return AVPixelFormat_AV_PIX_FMT_CUDA;
            }
            p = p.add(1);
        }
    }
    AVPixelFormat_AV_PIX_FMT_NONE
}

fn open_cuda_decoder(
    input: &Input,
    stream_index: usize,
    device: &HwDevice,
) -> Result<(Codec, *mut AVBufferRef)> {
    let stream = input.streams()[stream_index];
    // SAFETY: Stream pointer from live input; codecpar is owned by the stream.
    let codec_id = unsafe { (*(*stream).codecpar).codec_id };
    let decoder = unsafe { avcodec_find_decoder(codec_id) };
    if decoder.is_null() {
        return Err("no decoder for input video stream".into());
    }
    let mut codec = Codec(unsafe { avcodec_alloc_context3(decoder) });
    if codec.0.is_null() {
        return Err("decoder context allocation failed".into());
    }
    check(
        unsafe { avcodec_parameters_to_context(codec.0, (*stream).codecpar) },
        "copy decoder parameters",
    )?;
    unsafe {
        (*codec.0).get_format = Some(get_cuda_format);
        (*codec.0).hw_device_ctx = av_buffer_ref(device.0);
        if (*codec.0).hw_device_ctx.is_null() {
            return Err("failed to ref CUDA hwdevice for decoder".into());
        }
        (*codec.0).pkt_timebase = (*stream).time_base;
    }
    check(
        unsafe { avcodec_open2(codec.0, decoder, ptr::null_mut()) },
        "open CUDA decoder",
    )?;
    let frames = unsafe { (*codec.0).hw_frames_ctx };
    Ok((codec, frames))
}

fn open_nvenc(
    width: i32,
    height: i32,
    time_base: AVRational,
    framerate: AVRational,
    frames_ctx: *mut AVBufferRef,
) -> Result<Codec> {
    let encoder = unsafe { avcodec_find_encoder_by_name(c"h264_nvenc".as_ptr()) };
    if encoder.is_null() {
        return Err("h264_nvenc encoder is unavailable in this FFmpeg build".into());
    }
    let mut codec = Codec(unsafe { avcodec_alloc_context3(encoder) });
    if codec.0.is_null() {
        return Err("NVENC context allocation failed".into());
    }
    unsafe {
        (*codec.0).width = width;
        (*codec.0).height = height;
        (*codec.0).time_base = time_base;
        (*codec.0).framerate = if framerate.num > 0 && framerate.den > 0 {
            framerate
        } else {
            AVRational { num: 30, den: 1 }
        };
        (*codec.0).pix_fmt = AVPixelFormat_AV_PIX_FMT_CUDA;
        (*codec.0).hw_frames_ctx = av_buffer_ref(frames_ctx);
        if (*codec.0).hw_frames_ctx.is_null() {
            return Err("failed to ref CUDA frames for NVENC".into());
        }
    }
    let mut opts = ptr::null_mut();
    // Match bench FFmpeg GPU (`-preset p1 -bf 0`): throughput, not lookahead.
    unsafe {
        av_dict_set(&mut opts, c"preset".as_ptr(), c"p1".as_ptr(), 0);
        av_dict_set(&mut opts, c"bf".as_ptr(), c"0".as_ptr(), 0);
    }
    let open = unsafe { avcodec_open2(codec.0, encoder, &mut opts) };
    unsafe {
        av_dict_free(&mut opts);
    }
    check(open, "open h264_nvenc")?;
    Ok(codec)
}

fn nv12_view(frame: *mut AVFrame) -> Result<Nv12View> {
    // SAFETY: Caller holds a CUDA AVFrame; data[] hold CUdeviceptr values for NV12.
    unsafe {
        if (*frame).format != AVPixelFormat_AV_PIX_FMT_CUDA {
            return Err("decoded frame is not AV_PIX_FMT_CUDA".into());
        }
        let y = (*frame).data[0] as usize as u64;
        let uv = (*frame).data[1] as usize as u64;
        if y == 0 || uv == 0 {
            return Err("CUDA frame missing NV12 device pointers".into());
        }
        Ok(Nv12View {
            y,
            uv,
            pitch_y: (*frame).linesize[0] as u32,
            pitch_uv: (*frame).linesize[1] as u32,
            width: (*frame).width as u32,
            height: (*frame).height as u32,
        })
    }
}

fn alloc_cuda_frame(frames_ctx: *mut AVBufferRef) -> Result<Frame> {
    let frame = Frame::new()?;
    unsafe {
        if av_hwframe_get_buffer(frames_ctx, frame.0, 0) < 0 {
            return Err("alloc CUDA output frame failed".into());
        }
    }
    Ok(frame)
}

fn alloc_sw_nv12(width: i32, height: i32) -> Result<Frame> {
    let frame = Frame::new()?;
    unsafe {
        (*frame.0).format = AVPixelFormat_AV_PIX_FMT_NV12;
        (*frame.0).width = width;
        (*frame.0).height = height;
        check(av_frame_get_buffer(frame.0, 32), "alloc host NV12 frame")?;
    }
    Ok(frame)
}

/// FFmpeg `hwupload_cuda`: copy a software frame into a CUDA `AVFrame`.
pub fn hw_upload(dst_cuda: *mut AVFrame, src_host: *const AVFrame) -> Result<()> {
    if dst_cuda.is_null() || src_host.is_null() {
        return Err("hw_upload requires non-null frames".into());
    }
    check(
        unsafe { av_hwframe_transfer_data(dst_cuda, src_host, 0) },
        "hwupload_cuda (av_hwframe_transfer_data)",
    )
}

/// FFmpeg `hwdownload`: copy a CUDA `AVFrame` into a software frame.
pub fn hw_download(dst_host: *mut AVFrame, src_cuda: *const AVFrame) -> Result<()> {
    if dst_host.is_null() || src_cuda.is_null() {
        return Err("hw_download requires non-null frames".into());
    }
    check(
        unsafe { av_hwframe_transfer_data(dst_host, src_cuda, 0) },
        "hwdownload (av_hwframe_transfer_data)",
    )
}

/// Optional download→upload bounce (PCIe like FFmpeg `hwdownload,hwupload_cuda`).
/// Returns a fresh CUDA frame when bounce is enabled; otherwise `None` (use `cuda_src`).
fn host_bounce_cuda_frame(
    cuda_src: *mut AVFrame,
    frames_ctx: *mut AVBufferRef,
    enabled: bool,
    copies: &mut u64,
) -> Result<Option<Frame>> {
    if !enabled {
        return Ok(None);
    }
    let w = unsafe { (*cuda_src).width };
    let h = unsafe { (*cuda_src).height };
    let mut host = alloc_sw_nv12(w, h)?;
    hw_download(host.0, cuda_src)?;
    *copies += 1;
    let mut back = alloc_cuda_frame(frames_ctx)?;
    hw_upload(back.0, host.0)?;
    *copies += 1;
    unsafe {
        (*back.0).pts = (*cuda_src).pts;
    }
    Ok(Some(back))
}

/// H.264 CUDA decode → optional host bounce → fvid-cuda NV12 crop/flip → h264_nvenc.
///
/// Default: device-resident (`host_frame_copies=0`), FFmpeg NVDEC + NVENC.
/// `--host-bounce`: insert `hwdownload`+`hwupload_cuda` before the filter.
pub fn hw_filter(
    source: &Path,
    destination: &Path,
    options: &HwFilterOptions,
) -> Result<HwFilterStats> {
    if destination.exists() {
        return Err("output already exists".into());
    }
    let device = make_cuda_device(options.device)?;
    let mut input = Input::open(source)?;
    let video = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO })
        .ok_or("input has no video stream")?;
    let (mut decoder, mut frames_ctx) = open_cuda_decoder(&input, video, &device)?;
    if frames_ctx.is_null() {
        // Some decoders populate hw_frames_ctx after the first frame; allocate a pool.
        frames_ctx = unsafe { av_hwframe_ctx_alloc(device.0) };
        if frames_ctx.is_null() {
            return Err("CUDA frames context allocation failed".into());
        }
        unsafe {
            let ctx = (*frames_ctx).data as *mut AVHWFramesContext;
            (*ctx).format = AVPixelFormat_AV_PIX_FMT_CUDA;
            (*ctx).sw_format = AVPixelFormat_AV_PIX_FMT_NV12;
            (*ctx).width = (*decoder.0).width;
            (*ctx).height = (*decoder.0).height;
            (*ctx).initial_pool_size = OUT_POOL as i32;
            check(av_hwframe_ctx_init(frames_ctx), "init CUDA frames")?;
            (*decoder.0).hw_frames_ctx = av_buffer_ref(frames_ctx);
        }
    }
    let (out_w, out_h, crop) = match options.crop {
        Some(c) => {
            if c.x % 2 != 0 || c.y % 2 != 0 || c.width % 2 != 0 || c.height % 2 != 0 {
                return Err("CUDA NV12 crop requires even coordinates and size".into());
            }
            (c.width as i32, c.height as i32, c)
        }
        None => {
            let w = unsafe { (*decoder.0).width as usize };
            let h = unsafe { (*decoder.0).height as usize };
            (
                w as i32,
                h as i32,
                CropRect {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                },
            )
        }
    };
    let identity = !options.horizontal_flip
        && !options.vertical_flip
        && crop.x == 0
        && crop.y == 0
        && crop.width == out_w as usize
        && crop.height == out_h as usize
        && !options.host_bounce;

    // Identity/copy: NVENC on decoder surfaces — no filter, no second frame pool, no PTX.
    // Filtered: separate encoder pool + Nv12Processor (DtoD / kernel on FFmpeg stream).
    let enc_frames_owned: Option<HwDevice>;
    let enc_frames_ptr: *mut AVBufferRef;
    if identity {
        if frames_ctx.is_null() {
            return Err("CUDA decoder did not provide hw_frames_ctx for passthrough".into());
        }
        enc_frames_owned = None;
        enc_frames_ptr = frames_ctx;
    } else {
        let enc_frames = unsafe { av_hwframe_ctx_alloc(device.0) };
        if enc_frames.is_null() {
            return Err("encoder CUDA frames allocation failed".into());
        }
        unsafe {
            let ctx = (*enc_frames).data as *mut AVHWFramesContext;
            (*ctx).format = AVPixelFormat_AV_PIX_FMT_CUDA;
            (*ctx).sw_format = AVPixelFormat_AV_PIX_FMT_NV12;
            (*ctx).width = out_w;
            (*ctx).height = out_h;
            (*ctx).initial_pool_size = OUT_POOL as i32;
            check(av_hwframe_ctx_init(enc_frames), "init encoder CUDA frames")?;
        }
        enc_frames_ptr = enc_frames;
        enc_frames_owned = Some(HwDevice(enc_frames));
    }
    let tb = unsafe { (*input.streams()[video]).time_base };
    let framerate = unsafe { (*input.streams()[video]).avg_frame_rate };
    let encoder = open_nvenc(out_w, out_h, tb, framerate, enc_frames_ptr)?;
    let mut parameters = Parameters(unsafe { avcodec_parameters_alloc() });
    if parameters.0.is_null() {
        return Err("NVENC parameter allocation failed".into());
    }
    check(
        unsafe { avcodec_parameters_from_context(parameters.0, encoder.0) },
        "export NVENC parameters",
    )?;
    let mut output = Output::with_video(
        destination,
        &input,
        &[video],
        Some((video, parameters.0, unsafe { (*encoder.0).time_base })),
    )?
    .without_interleave();

    let mut filter = if identity {
        None
    } else {
        let mut proc = Nv12Processor::new(options.device)
            .map_err(|e| format!("CUDA NV12 processor: {e}"))?;
        let stream = ffmpeg_cuda_stream(&device)?;
        proc.follow_stream(stream as u64);
        Some(proc)
    };
    let transform = Nv12Transform {
        crop_x: crop.x as u32,
        crop_y: crop.y as u32,
        out_width: crop.width as u32,
        out_height: crop.height as u32,
        hflip: options.horizontal_flip,
        vflip: options.vertical_flip,
    };
    let mut packet = Packet::new()?;
    let mut dec_frame = Frame::new()?;
    let mut enc_packet = Packet::new()?;
    let mut out_pool: Option<[Frame; OUT_POOL]> = if identity {
        None
    } else {
        let ctx = enc_frames_owned.as_ref().map(|h| h.0).unwrap_or(enc_frames_ptr);
        Some([
            alloc_cuda_frame(ctx)?,
            alloc_cuda_frame(ctx)?,
            alloc_cuda_frame(ctx)?,
            alloc_cuda_frame(ctx)?,
            alloc_cuda_frame(ctx)?,
            alloc_cuda_frame(ctx)?,
            alloc_cuda_frame(ctx)?,
            alloc_cuda_frame(ctx)?,
        ])
    };
    let mut out_pool_i = 0usize;
    let mut in_flight: usize = 0;
    let device_name = filter
        .as_ref()
        .map(|f| f.device_name().to_owned())
        .unwrap_or_else(|| "CUDA NVENC passthrough".into());
    let mut stats = HwFilterStats {
        backend: if identity {
            "cuda-nvdec-nvenc-passthrough"
        } else {
            "cuda-nvdec-nvenc"
        },
        device: device_name,
        video_frames: 0,
        width: out_w as u32,
        height: out_h as u32,
        host_frame_copies: 0,
        device_filter_passes: 0,
        encoder: "h264_nvenc",
        host_bounce: options.host_bounce,
    };

    let drain_available = |encoder: &Codec,
                           output: &mut Output,
                           enc_packet: &mut Packet,
                           stats: &mut HwFilterStats,
                           in_flight: &mut usize|
     -> Result<()> {
        loop {
            let code = unsafe { avcodec_receive_packet(encoder.0, enc_packet.0) };
            if code == AGAIN || code == EOF {
                break;
            }
            check(code, "receive NVENC packet")?;
            let tb = unsafe { (*encoder.0).time_base };
            output.write(enc_packet, 0, tb)?;
            stats.video_frames += 1;
            *in_flight = in_flight.saturating_sub(1);
        }
        Ok(())
    };

    // Only drain when NVENC is full (EAGAIN) — keep the encode queue deep.
    let send_frame = |encoder: &Codec,
                      frame: *mut AVFrame,
                      output: &mut Output,
                      enc_packet: &mut Packet,
                      stats: &mut HwFilterStats,
                      in_flight: &mut usize|
     -> Result<()> {
        loop {
            let code = unsafe { avcodec_send_frame(encoder.0, frame) };
            if code == AGAIN {
                let before = *in_flight;
                drain_available(encoder, output, enc_packet, stats, in_flight)?;
                if *in_flight == before {
                    return Err("NVENC stalled (EAGAIN with no packets)".into());
                }
                continue;
            }
            check(code, "send NVENC frame")?;
            *in_flight += 1;
            break;
        }
        Ok(())
    };

    let mut handle_decoded = |dec: *mut AVFrame| -> Result<()> {
        if identity {
            return send_frame(
                &encoder,
                dec,
                &mut output,
                &mut enc_packet,
                &mut stats,
                &mut in_flight,
            );
        }
        let src = nv12_view(dec)?;
        if crop.x + crop.width > src.width as usize
            || crop.y + crop.height > src.height as usize
        {
            return Err("crop exceeds decoded CUDA frame".into());
        }
        let pool = out_pool.as_mut().expect("filter path has out_pool");
        let filt = filter.as_mut().expect("filter path has Nv12Processor");
        let enc_ctx = enc_frames_owned.as_ref().map(|h| h.0).unwrap_or(enc_frames_ptr);
        let mut tries = 0usize;
        while tries < OUT_POOL {
            let slot = &mut pool[out_pool_i];
            out_pool_i = (out_pool_i + 1) % OUT_POOL;
            unsafe {
                if (*slot.0).data[0].is_null() || av_frame_is_writable(slot.0) == 0 {
                    drain_available(
                        &encoder,
                        &mut output,
                        &mut enc_packet,
                        &mut stats,
                        &mut in_flight,
                    )?;
                    if (*slot.0).data[0].is_null() || av_frame_is_writable(slot.0) == 0 {
                        av_frame_unref(slot.0);
                        if av_hwframe_get_buffer(enc_ctx, slot.0, 0) < 0 {
                            tries += 1;
                            continue;
                        }
                    }
                }
                (*slot.0).pts = (*dec).pts;
            }
            let dst = nv12_view(slot.0)?;
            filt.apply(src, dst, transform)
                .map_err(|e| format!("NV12 filter: {e}"))?;
            stats.device_filter_passes += 1;
            return send_frame(
                &encoder,
                slot.0,
                &mut output,
                &mut enc_packet,
                &mut stats,
                &mut in_flight,
            );
        }
        Err("CUDA output frame pool exhausted".into())
    };

    while packet.read(&mut input)? {
        if unsafe { (*packet.0).stream_index } != video as i32 {
            continue;
        }
        check(
            unsafe { avcodec_send_packet(decoder.0, packet.0) },
            "send packet to CUDA decoder",
        )?;
        loop {
            let code = unsafe { avcodec_receive_frame(decoder.0, dec_frame.0) };
            if code == AGAIN || code == EOF {
                break;
            }
            check(code, "receive CUDA frame")?;
            handle_decoded(dec_frame.0)?;
        }
    }
    check(
        unsafe { avcodec_send_packet(decoder.0, ptr::null_mut()) },
        "flush CUDA decoder",
    )?;
    loop {
        let code = unsafe { avcodec_receive_frame(decoder.0, dec_frame.0) };
        if code == AGAIN || code == EOF {
            break;
        }
        check(code, "flush receive CUDA frame")?;
        handle_decoded(dec_frame.0)?;
    }
    loop {
        let code = unsafe { avcodec_send_frame(encoder.0, ptr::null_mut()) };
        if code == AGAIN {
            let before = in_flight;
            drain_available(
                &encoder,
                &mut output,
                &mut enc_packet,
                &mut stats,
                &mut in_flight,
            )?;
            if in_flight == before {
                return Err("NVENC flush stalled (EAGAIN with no packets)".into());
            }
            continue;
        }
        if code == EOF {
            break;
        }
        check(code, "flush NVENC")?;
        break;
    }
    drain_available(
        &encoder,
        &mut output,
        &mut enc_packet,
        &mut stats,
        &mut in_flight,
    )?;
    output.finish()?;
    Ok(stats)
}
