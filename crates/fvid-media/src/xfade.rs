//! Dual-input `xfade` (fair-pairs FFmpeg `-filter_complex [0:v][1:v]xfade=...`).
use super::lossless::{Codec, Frame, LosslessStats, Parameters};
use super::*;
use filter::{us_to_filter_secs, validate_xfade_transition};
use std::path::Path;
use std::ptr;

const AGAIN: i32 = -libc::EAGAIN;

struct DualXfade {
    graph: *mut AVFilterGraph,
    main: *mut AVFilterContext,
    other: *mut AVFilterContext,
    sink: *mut AVFilterContext,
    time_base: AVRational,
}

impl Drop for DualXfade {
    fn drop(&mut self) {
        unsafe {
            if !self.graph.is_null() {
                avfilter_graph_free(&mut self.graph);
            }
        }
    }
}

impl DualXfade {
    unsafe fn open(
        width: i32,
        height: i32,
        format: i32,
        sar: AVRational,
        stream_tb: AVRational,
        fps_num: i32,
        fps_den: i32,
        color_range: i32,
        transition: &str,
        duration: &str,
        offset: &str,
    ) -> Result<Self> {
        unsafe {
            let fps_num = if fps_num > 0 { fps_num } else { 25 };
            let fps_den = if fps_den > 0 { fps_den } else { 1 };
            // Match FFmpeg `-i` buffer defaults: stream time base + declared frame_rate.
            let time_base = if stream_tb.num > 0 && stream_tb.den > 0 {
                stream_tb
            } else {
                AVRational {
                    num: fps_den,
                    den: fps_num,
                }
            };
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let xfade = avfilter_get_by_name(c"xfade".as_ptr());
            let format_f = avfilter_get_by_name(c"format".as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null()
                || xfade.is_null()
                || format_f.is_null()
                || buffersink.is_null()
            {
                return Err("xfade filters unavailable in linked libavfilter".into());
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("xfade graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                main: ptr::null_mut(),
                other: ptr::null_mut(),
                sink: ptr::null_mut(),
                time_base,
            };
            let range = if color_range != AVColorRange_AVCOL_RANGE_UNSPECIFIED {
                let name = av_color_range_name(color_range);
                if name.is_null() {
                    None
                } else {
                    Some(string(name))
                }
            } else {
                None
            };
            let mut buf_args = format!(
                "video_size={width}x{height}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}:frame_rate={fps_num}/{fps_den}",
                time_base.num,
                time_base.den,
                sar.num.max(1),
                sar.den.max(1)
            );
            if let Some(range) = range.as_deref() {
                buf_args.push_str(&format!(":range={range}"));
            }
            let buf_args = cstring(&buf_args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.main,
                    buffersrc,
                    c"main".as_ptr(),
                    buf_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create xfade main buffer",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.other,
                    buffersrc,
                    c"other".as_ptr(),
                    buf_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create xfade other buffer",
            )?;
            let mut xfade_ctx = ptr::null_mut();
            let xfade_args =
                format!("transition={transition}:duration={duration}:offset={offset}");
            let xfade_args = cstring(&xfade_args)?;
            check(
                avfilter_graph_create_filter(
                    &mut xfade_ctx,
                    xfade,
                    c"xfade".as_ptr(),
                    xfade_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create xfade filter",
            )?;
            // Lock output to the source pixel format (xfade otherwise prefers yuv444).
            let mut format_ctx = ptr::null_mut();
            let pix_name = string(av_get_pix_fmt_name(format));
            if pix_name.is_empty() {
                return Err("xfade pixel format name unavailable".into());
            }
            let format_args = cstring(&pix_name)?;
            check(
                avfilter_graph_create_filter(
                    &mut format_ctx,
                    format_f,
                    c"format".as_ptr(),
                    format_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create xfade format lock",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create xfade sink",
            )?;
            check(avfilter_link(built.main, 0, xfade_ctx, 0), "link main to xfade")?;
            check(avfilter_link(built.other, 0, xfade_ctx, 1), "link other to xfade")?;
            check(
                avfilter_link(xfade_ctx, 0, format_ctx, 0),
                "link xfade to format",
            )?;
            check(
                avfilter_link(format_ctx, 0, built.sink, 0),
                "link format to sink",
            )?;
            check(
                avfilter_graph_config(built.graph, ptr::null_mut()),
                "configure xfade graph",
            )?;
            Ok(built)
        }
    }
}

struct VideoPump {
    input: Input,
    index: usize,
    decoder: Codec,
    packet: Packet,
    frame: Frame,
    stream_tb: AVRational,
    eof_demux: bool,
    eof_decode: bool,
    pending: bool,
    closed: bool,
}

impl VideoPump {
    fn open(path: &Path) -> Result<Self> {
        let input = Input::open(path)?;
        let index = input
            .streams()
            .iter()
            .position(|&s| unsafe {
                (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
            })
            .ok_or("xfade requires a video stream")?;
        unsafe {
            let stream = &*input.streams()[index];
            let par = &*stream.codecpar;
            let codec = avcodec_find_decoder(par.codec_id);
            if codec.is_null() {
                return Err("video decoder unavailable".into());
            }
            let decoder = Codec(avcodec_alloc_context3(codec));
            if decoder.0.is_null() {
                return Err("video decoder allocation failed".into());
            }
            check(
                avcodec_parameters_to_context(decoder.0, stream.codecpar),
                "configure video decoder",
            )?;
            (*decoder.0).pkt_timebase = stream.time_base;
            check(
                avcodec_open2(decoder.0, codec, ptr::null_mut()),
                "open video decoder",
            )?;
            Ok(Self {
                input,
                index,
                decoder,
                packet: Packet::new()?,
                frame: Frame::new()?,
                stream_tb: stream.time_base,
                eof_demux: false,
                eof_decode: false,
                pending: false,
                closed: false,
            })
        }
    }

    fn geometry(&self) -> (i32, i32, i32, AVRational, AVRational, AVRational, i32) {
        unsafe {
            let d = &*self.decoder.0;
            let stream = &*self.input.streams()[self.index];
            let fps = if stream.avg_frame_rate.num > 0 && stream.avg_frame_rate.den > 0 {
                stream.avg_frame_rate
            } else if stream.r_frame_rate.num > 0 && stream.r_frame_rate.den > 0 {
                stream.r_frame_rate
            } else {
                AVRational { num: 25, den: 1 }
            };
            let sar = if d.sample_aspect_ratio.num > 0 {
                d.sample_aspect_ratio
            } else if (*stream.codecpar).sample_aspect_ratio.num > 0 {
                (*stream.codecpar).sample_aspect_ratio
            } else {
                AVRational { num: 1, den: 1 }
            };
            (
                d.width,
                d.height,
                d.pix_fmt,
                fps,
                sar,
                stream.time_base,
                d.color_range,
            )
        }
    }

    /// Decode until a frame is pending or the stream is exhausted.
    fn ensure_frame(&mut self) -> Result<bool> {
        if self.pending {
            return Ok(true);
        }
        if self.eof_decode {
            return Ok(false);
        }
        loop {
            let code = unsafe { avcodec_receive_frame(self.decoder.0, self.frame.0) };
            if code >= 0 {
                unsafe {
                    let f = &mut *self.frame.0;
                    if f.time_base.num <= 0 || f.time_base.den <= 0 {
                        f.time_base = self.stream_tb;
                    }
                }
                self.pending = true;
                return Ok(true);
            }
            if code == EOF {
                self.eof_decode = true;
                return Ok(false);
            }
            if code != AGAIN {
                check(code, "receive xfade decode frame")?;
            }
            if self.eof_demux {
                check(
                    unsafe { avcodec_send_packet(self.decoder.0, ptr::null()) },
                    "flush xfade decoder",
                )?;
                continue;
            }
            loop {
                if !self.packet.read(&mut self.input)? {
                    self.eof_demux = true;
                    break;
                }
                let si = unsafe { (*self.packet.0).stream_index };
                if si == self.index as i32 {
                    check(
                        unsafe { avcodec_send_packet(self.decoder.0, self.packet.0) },
                        "send xfade packet",
                    )?;
                    break;
                }
            }
        }
    }

    fn push_pending(&mut self, src: *mut AVFilterContext, filter_tb: AVRational) -> Result<()> {
        if !self.pending {
            return Err("xfade push without pending frame".into());
        }
        unsafe {
            let f = &mut *self.frame.0;
            let src_tb = if f.time_base.num > 0 && f.time_base.den > 0 {
                f.time_base
            } else {
                filter_tb
            };
            if f.pts != NOPTS && (src_tb.num != filter_tb.num || src_tb.den != filter_tb.den) {
                f.pts = av_rescale_q(f.pts, src_tb, filter_tb);
            }
            f.time_base = filter_tb;
            check(av_buffersrc_write_frame(src, self.frame.0), "feed xfade buffer")?;
            av_frame_unref(self.frame.0);
        }
        self.pending = false;
        Ok(())
    }

    fn close_src(&mut self, src: *mut AVFilterContext) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        check(
            unsafe { av_buffersrc_write_frame(src, ptr::null()) },
            "close xfade buffer",
        )?;
        self.closed = true;
        Ok(())
    }
}

fn drain_encoder(
    encoder: &mut Codec,
    output: &mut Output,
    packet: &mut Packet,
    stats: &mut LosslessStats,
) -> Result<()> {
    loop {
        let code = unsafe { avcodec_receive_packet(encoder.0, packet.0) };
        if code == AGAIN || code == EOF {
            return Ok(());
        }
        check(code, "receive xfade packet")?;
        let tb = unsafe { (*encoder.0).time_base };
        output.write(packet, 0, tb)?;
        stats.video_packets += 1;
    }
}

fn send_encoder_frame(
    encoder: &mut Codec,
    output: &mut Output,
    packet: &mut Packet,
    frame: *mut AVFrame,
    stats: &mut LosslessStats,
) -> Result<()> {
    loop {
        let code = unsafe { avcodec_send_frame(encoder.0, frame) };
        if code == AGAIN {
            let before = stats.video_packets;
            drain_encoder(encoder, output, packet, stats)?;
            if stats.video_packets == before {
                return Err("xfade encoder stalled (EAGAIN with no packets)".into());
            }
            continue;
        }
        check(code, "send xfade frame to encoder")?;
        return Ok(());
    }
}

/// Cross-fade two videos into FFV1/Matroska (video-only).
/// Fair-pairs FFmpeg `-filter_complex [0:v][1:v]xfade=transition=...:duration=...:offset=...`.
pub fn xfade_video(
    source: &Path,
    other: &Path,
    destination: &Path,
    transition: &str,
    duration_us: i64,
    offset_us: i64,
    options: &CopyOptions,
) -> Result<LosslessStats> {
    validate_xfade_transition(transition)?;
    if duration_us <= 0 {
        return Err("xfade --duration must be > 0".into());
    }
    if offset_us < 0 {
        return Err("xfade --offset must be >= 0".into());
    }
    if destination.extension().and_then(|v| v.to_str()) != Some("mkv") {
        return Err("xfade requires FFV1 in .mkv".into());
    }
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if !other.is_file() {
        return Err("xfade second video file not found".into());
    }
    if !options.streams.is_empty() {
        return Err("xfade v1 does not take --streams".into());
    }

    let mut main = VideoPump::open(source)?;
    let mut side = VideoPump::open(other)?;
    crate::budget::admit_input_controlled_budget(&main.input, options, 4, true)?;
    crate::budget::check_rss_budget(options)?;

    let (w, h, pix_fmt, fps, sar, stream_tb, color_range) = main.geometry();
    let (w2, h2, pix_fmt2, fps2, _, _, _) = side.geometry();
    if w != w2 || h != h2 {
        return Err("xfade inputs must share width and height".into());
    }
    if pix_fmt != pix_fmt2 {
        return Err("xfade inputs must share pixel format".into());
    }
    if fps.num != fps2.num || fps.den != fps2.den {
        return Err("xfade inputs must share frame rate".into());
    }
    if w <= 0 || h <= 0 {
        return Err("xfade requires valid frame geometry".into());
    }

    let duration = us_to_filter_secs(duration_us)?;
    let offset = us_to_filter_secs(offset_us)?;
    let graph = unsafe {
        DualXfade::open(
            w,
            h,
            pix_fmt,
            sar,
            stream_tb,
            fps.num,
            fps.den,
            color_range,
            transition,
            &duration,
            &offset,
        )?
    };

    let (mut encoder, parameters, pixel_format) = unsafe {
        let codec = avcodec_find_encoder(AVCodecID_AV_CODEC_ID_FFV1);
        if codec.is_null() {
            return Err("FFV1 encoder unavailable".into());
        }
        let encoder = Codec(avcodec_alloc_context3(codec));
        if encoder.0.is_null() {
            return Err("FFV1 encoder allocation failed".into());
        }
        let d = &*main.decoder.0;
        (*encoder.0).width = w;
        (*encoder.0).height = h;
        (*encoder.0).pix_fmt = pix_fmt;
        (*encoder.0).time_base = graph.time_base;
        (*encoder.0).framerate = fps;
        (*encoder.0).sample_aspect_ratio = sar;
        (*encoder.0).color_range = d.color_range;
        (*encoder.0).color_primaries = d.color_primaries;
        (*encoder.0).color_trc = d.color_trc;
        (*encoder.0).colorspace = d.colorspace;
        (*encoder.0).chroma_sample_location = d.chroma_sample_location;
        (*encoder.0).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        (*encoder.0).level = 3;
        (*encoder.0).thread_count = 0;
        check(
            avcodec_open2(encoder.0, codec, ptr::null_mut()),
            "open FFV1 encoder",
        )?;
        let parameters = Parameters(avcodec_parameters_alloc());
        if parameters.0.is_null() {
            return Err("codec parameter allocation failed".into());
        }
        check(
            avcodec_parameters_from_context(parameters.0, encoder.0),
            "export FFV1 parameters",
        )?;
        (
            encoder,
            parameters,
            string(av_get_pix_fmt_name(pix_fmt)),
        )
    };

    let main_idx = main.index;
    let mut output = Output::with_video(
        destination,
        &main.input,
        &[main_idx],
        Some((
            main_idx,
            parameters.0 as *const _,
            unsafe { (*encoder.0).time_base },
        )),
    )?
    .without_interleave();

    let mut encoded = Packet::new()?;
    let out_frame = Frame::new()?;
    let mut stats = LosslessStats {
        backend: "native libavfilter xfade",
        video_frames: 0,
        decoded_frames: 0,
        seek_used: false,
        video_packets: 0,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format,
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: false,
        horizontal_flip: false,
    };

    let filter_tb = graph.time_base;
    let mut stalled = 0u32;
    loop {
        unsafe {
            av_frame_unref(out_frame.0);
        }
        let code = unsafe { av_buffersink_get_frame(graph.sink, out_frame.0) };
        if code >= 0 {
            stats.video_frames += 1;
            send_encoder_frame(
                &mut encoder,
                &mut output,
                &mut encoded,
                out_frame.0,
                &mut stats,
            )?;
            drain_encoder(&mut encoder, &mut output, &mut encoded, &mut stats)?;
            stalled = 0;
            continue;
        }
        if code == EOF {
            break;
        }
        if code != AGAIN {
            check(code, "receive xfade frame")?;
        }

        let main_req = unsafe { av_buffersrc_get_nb_failed_requests(graph.main) };
        let other_req = unsafe { av_buffersrc_get_nb_failed_requests(graph.other) };
        let mut fed = false;

        if (main_req > 0 || other_req == 0) && !main.closed {
            if main.ensure_frame()? {
                main.push_pending(graph.main, filter_tb)?;
                stats.decoded_frames += 1;
                fed = true;
            } else {
                main.close_src(graph.main)?;
                fed = true;
            }
        }
        if (!fed || other_req > 0) && !side.closed {
            if side.ensure_frame()? {
                side.push_pending(graph.other, filter_tb)?;
                stats.decoded_frames += 1;
                fed = true;
            } else {
                side.close_src(graph.other)?;
                fed = true;
            }
        }
        if fed {
            stalled = 0;
            continue;
        }
        stalled += 1;
        if main.closed && side.closed {
            if stalled > 4 {
                break;
            }
            continue;
        }
        if stalled > 8 {
            return Err("xfade filter stalled waiting for frames".into());
        }
    }

    send_encoder_frame(
        &mut encoder,
        &mut output,
        &mut encoded,
        ptr::null_mut(),
        &mut stats,
    )?;
    drain_encoder(&mut encoder, &mut output, &mut encoded, &mut stats)?;
    if stats.video_frames == 0 {
        return Err("xfade produced no frames".into());
    }
    output.finish()?;
    Ok(stats)
}

/// FFmpeg `-filter_complex` fair-pair expression for dual-input xfade.
pub fn xfade_filter_complex(
    transition: &str,
    duration_us: i64,
    offset_us: i64,
    pix_fmt: &str,
) -> Result<String> {
    validate_xfade_transition(transition)?;
    if pix_fmt.is_empty() || pix_fmt.len() > 32 || !pix_fmt.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("xfade pix_fmt for fair-pair must be alphanumeric".into());
    }
    let duration = us_to_filter_secs(duration_us)?;
    let offset = us_to_filter_secs(offset_us)?;
    Ok(format!(
        "[0:v][1:v]xfade=transition={transition}:duration={duration}:offset={offset},format={pix_fmt}"
    ))
}
