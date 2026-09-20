//! CUDA decode → fvid-cuda NV12 filter → NVENC encode (device-resident vertical slice).
#![cfg(feature = "cuda-hw")]

use super::*;
use fvid_cuda::{Nv12Processor, Nv12Transform, Nv12View, copy_crop_on_stream};
use lossless::{Codec, CropRect, Frame, Parameters};
use serde::Serialize;
use std::path::Path;
use std::ptr;
use std::thread;

const AGAIN: i32 = -libc::EAGAIN;
/// Filtered-path CUDA output slots (identity/copy uses decoder surfaces).
const OUT_POOL: usize = 8;
/// Extra NVDEC surfaces retained while NVENC owns passthrough inputs.
const EXTRA_HW_FRAMES: i32 = 16;
/// Wall-time gate targets multi-NVENC GPUs. Below this duration the extra
/// session startup and concat exceed the encode savings.
const PARALLEL_MIN_DURATION_US: i64 = 20_000_000;
/// RTX 5090 exposes three NVENC engines; identity encode can use all three.
/// Filter sessions also need NVDEC, so hflip caps at two to avoid decode stalls.
const PARALLEL_SESSIONS_IDENTITY: usize = 3;
const PARALLEL_SESSIONS_HFLIP: usize = 3;

fn gcd_i128(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.abs().max(1)
}

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
    /// Half-open presentation interval in microseconds from container start.
    /// Frames outside `[from, to)` are dropped; kept frames get CFR PTS 0..N-1.
    pub interval: Option<(i64, i64)>,
    /// Parallel workers share the CUDA primary context so concurrent
    /// `av_hwdevice_ctx_create` calls do not fight over incompatible flags.
    pub share_primary_context: bool,
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

fn make_cuda_device(ordinal: usize, use_primary_context: bool) -> Result<HwDevice> {
    let mut device = ptr::null_mut();
    let ordinal_name = (ordinal != 0)
        .then(|| cstring(&ordinal.to_string()))
        .transpose()?;
    let device_name = ordinal_name
        .as_ref()
        .map_or(ptr::null(), |name| name.as_ptr());
    // Share the primary context only when fvid-cuda launches kernels. FFmpeg-only
    // decode/copy paths avoid retaining global primary-context state.
    const AV_CUDA_USE_PRIMARY_CONTEXT: u32 = 1 << 0;
    // SAFETY: Creates a new CUDA hardware device context; null checked below.
    let code = unsafe {
        av_hwdevice_ctx_create(
            &mut device,
            AVHWDeviceType_AV_HWDEVICE_TYPE_CUDA,
            device_name,
            ptr::null_mut(),
            if use_primary_context {
                AV_CUDA_USE_PRIMARY_CONTEXT as i32
            } else {
                0
            },
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

fn ffmpeg_cuda_context(device: &HwDevice) -> Result<*mut std::ffi::c_void> {
    // SAFETY: Same checked AVCUDADeviceContext layout as `ffmpeg_cuda_stream`.
    unsafe {
        if device.0.is_null() || (*device.0).data.is_null() {
            return Err("CUDA hwdevice missing data".into());
        }
        let hw = &*((*device.0).data as *const AVHWDeviceContext);
        if hw.hwctx.is_null() {
            return Err("CUDA hwdevice missing hwctx".into());
        }
        Ok((*(hw.hwctx as *const AvCudaDeviceContext)).cuda_ctx)
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
    extra_hw_frames: i32,
) -> Result<(Codec, *mut AVBufferRef)> {
    let stream = input.streams()[stream_index];
    // SAFETY: Stream pointer from live input; codecpar is owned by the stream.
    let codec_id = unsafe { (*(*stream).codecpar).codec_id };
    let decoder = unsafe { avcodec_find_decoder(codec_id) };
    if decoder.is_null() {
        return Err("no decoder for input video stream".into());
    }
    let codec = Codec(unsafe { avcodec_alloc_context3(decoder) });
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
        // Keep NVDEC ahead of NVENC when surfaces are shared. Decode-only does
        // not retain surfaces and passes zero to avoid an oversized pool.
        (*codec.0).extra_hw_frames = extra_hw_frames;
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
    let codec = Codec(unsafe { avcodec_alloc_context3(encoder) });
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

/// H.264 CUDA decode → optional host bounce → fvid-cuda NV12 crop/flip → h264_nvenc.
///
/// Default: device-resident (`host_frame_copies=0`), FFmpeg NVDEC + NVENC.
/// `--host-bounce`: insert `hwdownload`+`hwupload_cuda` before the filter.
///
/// Encode-heavy timelines (≥20s of work) fan out across multiple NVENC
/// sessions and concat the closed segments. Soft full-frame vflip stays
/// single-session (host view path already beats FFmpeg). Short pixel-oracle
/// clips remain single-session.
pub fn hw_filter(
    source: &Path,
    destination: &Path,
    options: &HwFilterOptions,
) -> Result<HwFilterStats> {
    if destination.exists() {
        return Err("output already exists".into());
    }
    // Soft full-frame vflip is already faster than FFmpeg's host bounce as a
    // single session; parallelizing it only adds startup/concat tax.
    // Fused crop+hflip+vflip regresses under multi-session NVDEC contention;
    // keep the single device-resident pass and rely on longer fair-pair work.
    let soft_vflip_only = options.vertical_flip
        && !options.horizontal_flip
        && options.crop.is_none()
        && !options.host_bounce;
    let fused = options.crop.is_some() && options.horizontal_flip && options.vertical_flip;
    if !soft_vflip_only
        && !fused
        && !options.host_bounce
        && let Some(stats) = hw_filter_parallel(source, destination, options)?
    {
        return Ok(stats);
    }
    hw_filter_session(source, destination, options)
}

fn hw_filter_parallel(
    source: &Path,
    destination: &Path,
    options: &HwFilterOptions,
) -> Result<Option<HwFilterStats>> {
    let input = Input::open(source)?;
    let video = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO })
        .ok_or("input has no video stream")?;
    let tb = unsafe { (*input.streams()[video]).time_base };
    // Filter paths share NVDEC with NVENC. Fused crop+flip work saturates
    // decode harder, so cap it at two sessions; plain hflip can use three.
    let sessions = if options.horizontal_flip && options.vertical_flip {
        2
    } else if options.horizontal_flip || options.vertical_flip {
        PARALLEL_SESSIONS_HFLIP
    } else {
        PARALLEL_SESSIONS_IDENTITY
    };
    if sessions < 2 {
        return Ok(None);
    }
    let (work_us, ranges) = unsafe {
        let container = (*input.0).duration;
        if container == NOPTS {
            return Ok(None);
        }
        let stream_duration = (*input.streams()[video]).duration;
        let total_ticks = if stream_duration != NOPTS && stream_duration > 0 {
            stream_duration
        } else {
            let numerator = i128::from(container) * i128::from(tb.den);
            let denominator = 1_000_000i128 * i128::from(tb.num);
            if denominator <= 0 || numerator % denominator != 0 {
                return Ok(None);
            }
            i64::try_from(numerator / denominator).map_err(|_| "duration overflow")?
        };
        let ticks_to_us = |ticks: i64| -> Result<i64> {
            let numerator = i128::from(ticks) * i128::from(tb.num) * 1_000_000;
            let denominator = i128::from(tb.den);
            if denominator <= 0 {
                return Err("invalid video time base".into());
            }
            if numerator % denominator != 0 {
                return Err("parallel boundary is not exact in microseconds".into());
            }
            i64::try_from(numerator / denominator).map_err(|_| "timestamp overflow".into())
        };
        let us_to_ticks = |us: i64| -> Result<i64> {
            let numerator = i128::from(us) * i128::from(tb.den);
            let denominator = 1_000_000i128 * i128::from(tb.num);
            if denominator <= 0 || numerator % denominator != 0 {
                return Err("parallel boundary is not exact in video time base".into());
            }
            i64::try_from(numerator / denominator).map_err(|_| "timestamp overflow".into())
        };
        let tick_step = {
            let a = i128::from(tb.num) * 1_000_000;
            let den = i128::from(tb.den);
            let g = gcd_i128(a.abs(), den.abs());
            let step = den / g;
            i64::try_from(step).map_err(|_| "time base step overflow")?
        };
        if tick_step <= 0 {
            return Ok(None);
        }
        let align = |ticks: i64| ticks - ticks.rem_euclid(tick_step);
        let (window_start, window_end) = match options.interval {
            Some((from, to)) => (us_to_ticks(from)?, us_to_ticks(to)?),
            None => (0, total_ticks),
        };
        if window_end <= window_start {
            return Ok(None);
        }
        let window_ticks = window_end - window_start;
        let work_us = ticks_to_us(align(window_ticks))?;
        if work_us < PARALLEL_MIN_DURATION_US || window_ticks < sessions as i64 {
            return Ok(None);
        }
        let mut ranges = Vec::with_capacity(sessions);
        for index in 0..sessions {
            let raw_start = window_start + index as i64 * (window_ticks / sessions as i64);
            let raw_end = if index + 1 == sessions {
                window_end
            } else {
                window_start + (index as i64 + 1) * (window_ticks / sessions as i64)
            };
            let start_ticks = align(raw_start);
            let end_ticks = align(raw_end);
            if end_ticks <= start_ticks {
                return Ok(None);
            }
            ranges.push((ticks_to_us(start_ticks)?, ticks_to_us(end_ticks)?));
        }
        (work_us, ranges)
    };
    drop(input);
    if work_us < PARALLEL_MIN_DURATION_US {
        return Ok(None);
    }
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut parts = Vec::with_capacity(sessions);
    for index in 0..sessions {
        parts.push(directory.join(format!(
            ".fvid-hw-{}-{}-{index}.mp4",
            std::process::id(),
            destination
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("out")
        )));
    }
    let source = source.to_path_buf();
    let options = *options;
    let result = thread::scope(|scope| -> Result<HwFilterStats> {
        let mut handles = Vec::with_capacity(sessions);
        for (part, (from, to)) in parts.iter().zip(ranges.iter().copied()) {
            let source = source.clone();
            let part = part.clone();
            handles.push(scope.spawn(move || {
                let mut session = options;
                session.interval = Some((from, to));
                session.share_primary_context = true;
                hw_filter_session(&source, &part, &session)
            }));
        }
        let mut stats = handles
            .into_iter()
            .map(|handle| match handle.join() {
                Ok(Ok(stats)) => Ok(stats),
                Ok(Err(err)) => Err(err),
                Err(_) => Err("CUDA multi-session worker panicked".into()),
            })
            .collect::<Result<Vec<_>>>()?;
        let mut aggregate = stats.remove(0);
        for next in stats {
            aggregate.video_frames += next.video_frames;
            aggregate.host_frame_copies += next.host_frame_copies;
            aggregate.device_filter_passes += next.device_filter_passes;
        }
        aggregate.backend = "cuda-nvdec-nvenc-multisession";
        crate::concat(&parts, destination, &CopyOptions::default())?;
        Ok(aggregate)
    });
    for part in &parts {
        let _ = std::fs::remove_file(part);
    }
    result.map(Some)
}

fn hw_filter_session(
    source: &Path,
    destination: &Path,
    options: &HwFilterOptions,
) -> Result<HwFilterStats> {
    if destination.exists() {
        return Err("output already exists".into());
    }
    let device = make_cuda_device(
        options.device,
        options.horizontal_flip || options.share_primary_context,
    )?;
    let mut input = Input::open_fast(source)?;
    let video = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO })
        .ok_or("input has no video stream")?;
    let tb = unsafe { (*input.streams()[video]).time_base };
    let framerate = unsafe { (*input.streams()[video]).avg_frame_rate };
    let interval = if let Some((from, to)) = options.interval {
        if from < 0 || to <= from {
            return Err("hw-filter interval requires 0 <= from < to".into());
        }
        let origin = unsafe { (*input.0).start_time };
        let origin = if origin == NOPTS { 0 } else { origin };
        let ticks = |time: i64| -> Result<i64> {
            let us = origin
                .checked_add(time)
                .ok_or("interval timestamp overflow")?;
            let numerator = i128::from(us) * i128::from(tb.den);
            let denominator = 1_000_000i128 * i128::from(tb.num);
            if denominator <= 0 || tb.den <= 0 || numerator % denominator != 0 {
                return Err("interval boundary is not exact in video time base".into());
            }
            i64::try_from(numerator / denominator).map_err(|_| "interval timestamp overflow".into())
        };
        Some((ticks(from)?, ticks(to)?))
    } else {
        None
    };
    if let Some((start, _)) = interval {
        check(
            unsafe { avformat_seek_file(input.0, video as i32, i64::MIN, start, start, 0) },
            "seek before CUDA interval",
        )?;
    }
    let (decoder, mut frames_ctx) = open_cuda_decoder(&input, video, &device, EXTRA_HW_FRAMES)?;
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
    // Full-frame vflip only: FFmpeg does hwdownload+vflip(view)+hwupload — match that
    // (kernel path loses because software vflip is free after download; host-hflip
    // is not free, so hflip stays on the device kernel).
    let soft_vflip = options.vertical_flip
        && !options.horizontal_flip
        && crop.x == 0
        && crop.y == 0
        && crop.width == out_w as usize
        && crop.height == out_h as usize
        && !options.host_bounce;
    let direct_crop =
        !identity && !options.horizontal_flip && !options.vertical_flip && !options.host_bounce;
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
    let encoder = open_nvenc(out_w, out_h, tb, framerate, enc_frames_ptr)?;
    let parameters = Parameters(unsafe { avcodec_parameters_alloc() });
    if parameters.0.is_null() {
        return Err("NVENC parameter allocation failed".into());
    }
    check(
        unsafe { avcodec_parameters_from_context(parameters.0, encoder.0) },
        "export NVENC parameters",
    )?;
    let mut output = Output::with_video_direct(
        destination,
        &input,
        &[video],
        Some((video, parameters.0, unsafe { (*encoder.0).time_base })),
    )?
    .without_interleave();

    let cuda_stream = ffmpeg_cuda_stream(&device)? as u64;
    let cuda_context = ffmpeg_cuda_context(&device)? as u64;
    let mut filter = if identity || soft_vflip || direct_crop {
        None
    } else {
        let mut proc =
            Nv12Processor::new(options.device).map_err(|e| format!("CUDA NV12 processor: {e}"))?;
        proc.follow_stream(cuda_stream);
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
    let dec_frame = Frame::new()?;
    let mut enc_packet = Packet::new()?;
    let mut out_pool: Option<Vec<Frame>> = if identity {
        None
    } else {
        let ctx = enc_frames_owned
            .as_ref()
            .map(|h| h.0)
            .unwrap_or(enc_frames_ptr);
        let mut frames = Vec::with_capacity(OUT_POOL);
        for _ in 0..OUT_POOL {
            frames.push(alloc_cuda_frame(ctx)?);
        }
        Some(frames)
    };
    let mut out_pool_i = 0usize;
    let mut in_flight: usize = 0;
    let mut soft_host: Option<Frame> = if soft_vflip {
        Some(alloc_sw_nv12(out_w, out_h)?)
    } else {
        None
    };
    let device_name = if let Some(filter) = filter.as_ref() {
        filter.device_name().to_owned()
    } else if soft_vflip {
        "CUDA NVENC host-vflip".into()
    } else if direct_crop {
        "CUDA NVENC direct-crop".into()
    } else {
        "CUDA NVENC passthrough".into()
    };
    let mut stats = HwFilterStats {
        backend: if identity {
            "cuda-nvdec-nvenc-passthrough"
        } else if soft_vflip {
            "cuda-nvdec-host-vflip-nvenc"
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
        host_bounce: options.host_bounce || soft_vflip,
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

    let mut handle_decoded = |dec: *mut AVFrame| -> Result<bool> {
        let pts_out = unsafe {
            if (*dec).pts == NOPTS {
                (*dec).pts = (*dec).best_effort_timestamp;
            }
            if let Some((start, end)) = interval {
                let pts = (*dec).pts;
                if pts != NOPTS && pts >= end {
                    return Ok(true);
                }
                if pts == NOPTS || pts < start {
                    return Ok(false);
                }
                let p = pts
                    .checked_sub(start)
                    .ok_or("CUDA interval timestamp overflow")?;
                (*dec).pts = p;
                Some(p)
            } else {
                None
            }
        };
        if identity {
            send_frame(
                &encoder,
                dec,
                &mut output,
                &mut enc_packet,
                &mut stats,
                &mut in_flight,
            )?;
            return Ok(false);
        }
        if soft_vflip {
            let host = soft_host.as_mut().expect("soft_vflip host frame");
            unsafe {
                av_frame_unref(host.0);
            }
            hw_download(host.0, dec)?;
            stats.host_frame_copies += 1;
            unsafe {
                (*host.0).width = out_w;
                (*host.0).height = out_h;
            }
            unsafe { crate::lossless::flip_view(host.0)? };
            let pool = out_pool.as_mut().expect("soft_vflip has out_pool");
            let enc_ctx = enc_frames_owned
                .as_ref()
                .map(|h| h.0)
                .unwrap_or(enc_frames_ptr);
            let mut tries = 0usize;
            while tries < OUT_POOL {
                let slot = &mut pool[out_pool_i];
                out_pool_i = (out_pool_i + 1) % OUT_POOL;
                unsafe {
                    if (*slot.0).data[0].is_null() {
                        if av_hwframe_get_buffer(enc_ctx, slot.0, 0) < 0 {
                            tries += 1;
                            continue;
                        }
                    } else if av_frame_is_writable(slot.0) == 0 {
                        // NVENC commonly releases this surface while packets are
                        // drained. Recheck and retain its CUDA allocation rather
                        // than unref/get_buffer on every pool rotation.
                        drain_available(
                            &encoder,
                            &mut output,
                            &mut enc_packet,
                            &mut stats,
                            &mut in_flight,
                        )?;
                        if av_frame_is_writable(slot.0) == 0 {
                            tries += 1;
                            continue;
                        }
                    }
                    (*slot.0).pts = pts_out.unwrap_or((*dec).pts);
                    (*slot.0).duration = (*dec).duration;
                }
                hw_upload(slot.0, host.0)?;
                stats.host_frame_copies += 1;
                send_frame(
                    &encoder,
                    slot.0,
                    &mut output,
                    &mut enc_packet,
                    &mut stats,
                    &mut in_flight,
                )?;
                return Ok(false);
            }
            return Err("CUDA output frame pool exhausted".into());
        }
        let src = nv12_view(dec)?;
        if crop.x + crop.width > src.width as usize || crop.y + crop.height > src.height as usize {
            return Err("crop exceeds decoded CUDA frame".into());
        }
        let pool = out_pool.as_mut().expect("filter path has out_pool");
        let enc_ctx = enc_frames_owned
            .as_ref()
            .map(|h| h.0)
            .unwrap_or(enc_frames_ptr);
        let mut tries = 0usize;
        while tries < OUT_POOL {
            let slot = &mut pool[out_pool_i];
            out_pool_i = (out_pool_i + 1) % OUT_POOL;
            unsafe {
                if (*slot.0).data[0].is_null() {
                    if av_hwframe_get_buffer(enc_ctx, slot.0, 0) < 0 {
                        tries += 1;
                        continue;
                    }
                } else if av_frame_is_writable(slot.0) == 0 {
                    // Drain can make the existing CUDA surface reusable.
                    drain_available(
                        &encoder,
                        &mut output,
                        &mut enc_packet,
                        &mut stats,
                        &mut in_flight,
                    )?;
                    if av_frame_is_writable(slot.0) == 0 {
                        tries += 1;
                        continue;
                    }
                }
                (*slot.0).pts = pts_out.unwrap_or((*dec).pts);
                (*slot.0).duration = (*dec).duration;
            }
            let dst = nv12_view(slot.0)?;
            if direct_crop {
                copy_crop_on_stream(src, dst, transform, cuda_context, cuda_stream)
                    .map_err(|e| format!("NV12 direct crop: {e}"))?;
            } else {
                filter
                    .as_mut()
                    .expect("flip path has Nv12Processor")
                    .apply(src, dst, transform)
                    .map_err(|e| format!("NV12 filter: {e}"))?;
            }
            stats.device_filter_passes += 1;
            send_frame(
                &encoder,
                slot.0,
                &mut output,
                &mut enc_packet,
                &mut stats,
                &mut in_flight,
            )?;
            return Ok(false);
        }
        Err("CUDA output frame pool exhausted".into())
    };

    let mut finished = false;
    'packets: while packet.read(&mut input)? {
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
            if handle_decoded(dec_frame.0)? {
                finished = true;
                break 'packets;
            }
        }
    }
    if !finished {
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
            if handle_decoded(dec_frame.0)? {
                break;
            }
        }
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

/// NVDEC-only throughput: decode CUDA frames and discard (no filter/NVENC/mux).
pub fn hw_decode_only(source: &Path, device_ordinal: usize) -> Result<(u64, u32, u32)> {
    let device = make_cuda_device(device_ordinal, false)?;
    let mut input = Input::open_fast(source)?;
    let video = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO })
        .ok_or("input has no video stream")?;
    let (decoder, _frames_ctx) = open_cuda_decoder(&input, video, &device, 0)?;
    let (width, height) = unsafe {
        (
            (*decoder.0).width.max(0) as u32,
            (*decoder.0).height.max(0) as u32,
        )
    };
    let mut packet = Packet::new()?;
    let dec_frame = Frame::new()?;
    let mut video_frames = 0u64;
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
            video_frames += 1;
            unsafe { av_frame_unref(dec_frame.0) };
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
        video_frames += 1;
        unsafe { av_frame_unref(dec_frame.0) };
    }
    Ok((video_frames, width, height))
}
