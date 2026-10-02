//! Measure loudness via libavfilter `ebur128` and apply `loudnorm`.
use super::lossless::{Codec, Frame};
use super::*;
use serde_json::Value;
use std::path::Path;
use std::ptr;
use std::sync::Mutex;

const AGAIN: i32 = -libc::EAGAIN;

pub use fvid_media_info::{LoudnessStats, LoudnormStats, DEFAULT_LOUDNORM_ARGS, validate_loudnorm_args};
use fvid_media_info::resolve_loudnorm_args;

struct EburGraph {
    graph: *mut AVFilterGraph,
    src: *mut AVFilterContext,
    ebur: *mut AVFilterContext,
    sink: *mut AVFilterContext,
}

impl Drop for EburGraph {
    fn drop(&mut self) {
        unsafe {
            if !self.graph.is_null() {
                avfilter_graph_free(&mut self.graph);
            }
        }
    }
}

fn describe_layout(layout: &AVChannelLayout) -> Result<String> {
    let mut buf = [0i8; 64];
    // SAFETY: layout is a valid AVChannelLayout; buffer is writable.
    let written = unsafe { av_channel_layout_describe(layout, buf.as_mut_ptr(), buf.len()) };
    if written < 0 {
        return Err("channel layout describe failed".into());
    }
    let bytes = buf
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as u8)
        .collect::<Vec<_>>();
    String::from_utf8(bytes).map_err(|_| "channel layout is not UTF-8".into())
}

impl EburGraph {
    unsafe fn open(frame: &AVFrame) -> Result<Self> {
        unsafe {
            let abuffer = avfilter_get_by_name(c"abuffer".as_ptr());
            let ebur128 = avfilter_get_by_name(c"ebur128".as_ptr());
            let abuffersink = avfilter_get_by_name(c"abuffersink".as_ptr());
            if abuffer.is_null() || ebur128.is_null() || abuffersink.is_null() {
                return Err("ebur128 filter unavailable in linked libavfilter".into());
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("ebur128 graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                ebur: ptr::null_mut(),
                sink: ptr::null_mut(),
            };
            let layout = describe_layout(&frame.ch_layout)?;
            let sample_fmt = super::pcm_format_adapter::name(frame.format);
            let mut time_base = frame.time_base;
            if time_base.num <= 0 || time_base.den <= 0 {
                time_base = AVRational {
                    num: 1,
                    den: frame.sample_rate.max(1),
                };
            }
            let args = format!(
                "time_base={}/{}:sample_rate={}:sample_fmt={sample_fmt}:channel_layout={layout}",
                time_base.num, time_base.den, frame.sample_rate
            );
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    abuffer,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create abuffer for ebur128",
            )?;
            let ebur_args = cstring("peak=true+sample:framelog=quiet")?;
            check(
                avfilter_graph_create_filter(
                    &mut built.ebur,
                    ebur128,
                    c"ebur128".as_ptr(),
                    ebur_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create ebur128 filter",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    abuffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create abuffersink for ebur128",
            )?;
            check(
                avfilter_link(built.src, 0, built.ebur, 0),
                "link abuffer to ebur128",
            )?;
            check(
                avfilter_link(built.ebur, 0, built.sink, 0),
                "link ebur128 to sink",
            )?;
            check(
                configure_filter_graph(built.graph),
                "configure ebur128 graph",
            )?;
            Ok(built)
        }
    }

    unsafe fn read_double(&self, name: &str) -> Result<f64> {
        unsafe {
            let mut value = 0.0f64;
            let cname = cstring(name)?;
            check(
                av_opt_get_double(
                    self.ebur as *mut _,
                    cname.as_ptr(),
                    AV_OPT_SEARCH_CHILDREN as i32,
                    &mut value,
                ),
                &format!("read ebur128 {name}"),
            )?;
            Ok(value)
        }
    }
}

struct LoudnormGraph {
    graph: *mut AVFilterGraph,
    src: *mut AVFilterContext,
    sink: *mut AVFilterContext,
}

impl Drop for LoudnormGraph {
    fn drop(&mut self) {
        unsafe {
            if !self.graph.is_null() {
                avfilter_graph_free(&mut self.graph);
            }
        }
    }
}

impl LoudnormGraph {
    unsafe fn open(frame: &AVFrame, loudnorm_args: &str) -> Result<Self> {
        unsafe {
            let abuffer = avfilter_get_by_name(c"abuffer".as_ptr());
            let loudnorm = avfilter_get_by_name(c"loudnorm".as_ptr());
            let aformat = avfilter_get_by_name(c"aformat".as_ptr());
            let abuffersink = avfilter_get_by_name(c"abuffersink".as_ptr());
            if abuffer.is_null() || loudnorm.is_null() || aformat.is_null() || abuffersink.is_null()
            {
                return Err("loudnorm/aformat filters unavailable in linked libavfilter".into());
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("loudnorm graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                sink: ptr::null_mut(),
            };
            let layout = describe_layout(&frame.ch_layout)?;
            let sample_fmt = super::pcm_format_adapter::name(frame.format);
            let mut time_base = frame.time_base;
            if time_base.num <= 0 || time_base.den <= 0 {
                time_base = AVRational {
                    num: 1,
                    den: frame.sample_rate.max(1),
                };
            }
            let src_args = format!(
                "time_base={}/{}:sample_rate={}:sample_fmt={sample_fmt}:channel_layout={layout}",
                time_base.num, time_base.den, frame.sample_rate
            );
            let src_args = cstring(&src_args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    abuffer,
                    c"in".as_ptr(),
                    src_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create abuffer for loudnorm",
            )?;
            let mut loud_ctx = ptr::null_mut();
            let loud_args = cstring(loudnorm_args)?;
            check(
                avfilter_graph_create_filter(
                    &mut loud_ctx,
                    loudnorm,
                    c"loudnorm".as_ptr(),
                    loud_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create loudnorm filter",
            )?;
            let mut fmt_ctx = ptr::null_mut();
            // Force packed float so WAV write matches FFmpeg `-c:a pcm_f32le`.
            let fmt_args = cstring(&format!("sample_fmts=flt:channel_layouts={layout}"))?;
            check(
                avfilter_graph_create_filter(
                    &mut fmt_ctx,
                    aformat,
                    c"aformat".as_ptr(),
                    fmt_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create aformat after loudnorm",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    abuffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create abuffersink for loudnorm",
            )?;
            check(
                avfilter_link(built.src, 0, loud_ctx, 0),
                "link abuffer to loudnorm",
            )?;
            check(
                avfilter_link(loud_ctx, 0, fmt_ctx, 0),
                "link loudnorm to aformat",
            )?;
            check(
                avfilter_link(fmt_ctx, 0, built.sink, 0),
                "link aformat to sink",
            )?;
            check(
                configure_filter_graph(built.graph),
                "configure loudnorm graph",
            )?;
            Ok(built)
        }
    }
}

unsafe fn append_flt_frame(frame: *const AVFrame, pcm: &mut Vec<f32>) -> Result<()> {
    unsafe {
        let f = &*frame;
        if f.format != AVSampleFormat_AV_SAMPLE_FMT_FLT {
            return Err("loudnorm sink must produce packed float PCM".into());
        }
        if f.nb_samples <= 0 || f.ch_layout.nb_channels <= 0 {
            return Err("invalid loudnorm output frame".into());
        }
        let channels = f.ch_layout.nb_channels as usize;
        let samples = f.nb_samples as usize;
        let data = *f.extended_data;
        if data.is_null() {
            return Err("missing loudnorm audio data".into());
        }
        let count = samples
            .checked_mul(channels)
            .ok_or("loudnorm sample count overflow")?;
        pcm.extend_from_slice(slice::from_raw_parts(data as *const f32, count));
        Ok(())
    }
}

fn select_audio_index(input: &Input, options: &CopyOptions, command: &str) -> Result<usize> {
    let selected = selection(input, options)?;
    if selected.is_empty() {
        input
            .streams()
            .iter()
            .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_AUDIO })
            .ok_or_else(|| format!("{command} requires an audio stream"))
    } else if selected.len() == 1 {
        let index = selected[0];
        // SAFETY: selection checked against live input.
        if unsafe { (*(*input.streams()[index]).codecpar).codec_type }
            != AVMediaType_AVMEDIA_TYPE_AUDIO
        {
            return Err(format!("{command} --streams must select an audio stream"));
        }
        Ok(index)
    } else {
        Err(format!("{command} requires exactly one audio stream"))
    }
}

fn open_audio_decoder(input: &Input, index: usize) -> Result<Codec> {
    unsafe {
        let stream = &*input.streams()[index];
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
        Ok(decoder)
    }
}

/// Measure EBU R128 loudness of the selected audio stream (default: first audio).
/// Supported PCM WAVE rates uses the owned streaming meter; other inputs
/// temporarily retain the legacy backend during migration.
pub fn measure_loudness(source: &Path, options: &CopyOptions) -> Result<LoudnessStats> {
    if crate::owned_adts_loudness::supports(source, options) {
        return crate::owned_adts_loudness::measure_loudness(source, options);
    }
    if crate::owned_wave_loudness::supports(source, options) {
        return crate::owned_wave_loudness::measure_loudness(source, options);
    }
    let mut input = Input::open_fast(source)?;
    let index = select_audio_index(&input, options, "loudness")?;
    let decoder = open_audio_decoder(&input, index)?;

    let mut packet = Packet::new()?;
    let mut frame = Frame::new()?;
    let mut filtered = Frame::new()?;
    let mut graph: Option<EburGraph> = None;
    let mut sample_frames = 0u64;
    let mut sample_rate = 0i32;
    let mut channels = 0i32;

    let mut feed = |frame: &mut Frame| -> Result<()> {
        unsafe {
            let f = &*frame.0;
            if f.nb_samples <= 0 || f.sample_rate <= 0 || f.ch_layout.nb_channels <= 0 {
                return Err("invalid decoded audio frame for loudness".into());
            }
            if f.time_base.num <= 0 || f.time_base.den <= 0 {
                (*frame.0).time_base = (*decoder.0).pkt_timebase;
            }
            if graph.is_none() {
                sample_rate = f.sample_rate;
                channels = f.ch_layout.nb_channels;
                graph = Some(EburGraph::open(f)?);
            }
            let active = graph.as_mut().ok_or("ebur128 graph missing")?;
            check(
                av_buffersrc_write_frame(active.src, frame.0),
                "feed ebur128",
            )?;
            loop {
                let code = av_buffersink_get_frame(active.sink, filtered.0);
                if code == AGAIN || code == EOF {
                    break;
                }
                check(code, "drain ebur128 sink")?;
                av_frame_unref(filtered.0);
            }
            sample_frames += f.nb_samples as u64;
        }
        Ok(())
    };

    while packet.read(&mut input)? {
        if unsafe { (*packet.0).stream_index } != index as i32 {
            continue;
        }
        check(
            unsafe { avcodec_send_packet(decoder.0, packet.0) },
            "send audio packet",
        )?;
        loop {
            let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
            if code == AGAIN || code == EOF {
                break;
            }
            check(code, "receive audio frame")?;
            feed(&mut frame)?;
            unsafe { av_frame_unref(frame.0) };
        }
    }
    check(
        unsafe { avcodec_send_packet(decoder.0, ptr::null()) },
        "flush audio decoder",
    )?;
    loop {
        let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
        if code == AGAIN || code == EOF {
            break;
        }
        check(code, "receive flushed audio frame")?;
        feed(&mut frame)?;
        unsafe { av_frame_unref(frame.0) };
    }

    let active = graph
        .as_mut()
        .ok_or("loudness requires at least one audio frame")?;
    // Flush the filter graph so integrated metrics finalize.
    check(
        unsafe { av_buffersrc_write_frame(active.src, ptr::null()) },
        "flush ebur128 source",
    )?;
    loop {
        let code = unsafe { av_buffersink_get_frame(active.sink, filtered.0) };
        if code == AGAIN || code == EOF {
            break;
        }
        check(code, "drain flushed ebur128")?;
        unsafe { av_frame_unref(filtered.0) };
    }

    let integrated = unsafe { active.read_double("integrated") }?;
    let range = unsafe { active.read_double("range") }?;
    let lra_low = unsafe { active.read_double("lra_low") }?;
    let lra_high = unsafe { active.read_double("lra_high") }?;
    let true_peak = unsafe { active.read_double("true_peak") }?;
    let sample_peak = unsafe { active.read_double("sample_peak") }?;

    Ok(LoudnessStats {
        backend: "native libavfilter ebur128",
        sample_frames,
        sample_rate,
        channels,
        integrated_lufs: integrated,
        range_lu: range,
        lra_low_lufs: lra_low,
        lra_high_lufs: lra_high,
        true_peak_dbfs: true_peak,
        sample_peak_dbfs: sample_peak,
    })
}

/// Apply loudnorm targets and write IEEE float WAV. Qualified WAVE requests
/// use the owned FVid normalizer; its dynamic controller is not bit-equivalent
/// to libavfilter. Other formats/policies retain the temporary legacy backend.
pub fn apply_loudnorm(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    let loudnorm_args = resolve_loudnorm_args(args)?;
    if crate::owned_loudnorm::supports(source, destination, &loudnorm_args, options) {
        return crate::owned_loudnorm::apply_loudnorm(source, destination, Some(&loudnorm_args), options);
    }
    let mut input = Input::open_fast(source)?;
    let index = select_audio_index(&input, options, "loudnorm")?;
    let decoder = open_audio_decoder(&input, index)?;

    let mut packet = Packet::new()?;
    let mut frame = Frame::new()?;
    let mut filtered = Frame::new()?;
    let mut graph: Option<LoudnormGraph> = None;
    let mut pcm: Vec<f32> = Vec::new();
    let mut sample_rate = 0i32;
    let mut channels = 0i32;

    let mut feed = |frame: &mut Frame| -> Result<()> {
        unsafe {
            let f = &*frame.0;
            if f.nb_samples <= 0 || f.sample_rate <= 0 || f.ch_layout.nb_channels <= 0 {
                return Err("invalid decoded audio frame for loudnorm".into());
            }
            if f.time_base.num <= 0 || f.time_base.den <= 0 {
                (*frame.0).time_base = (*decoder.0).pkt_timebase;
            }
            if graph.is_none() {
                sample_rate = f.sample_rate;
                channels = f.ch_layout.nb_channels;
                graph = Some(LoudnormGraph::open(f, &loudnorm_args)?);
            } else if sample_rate != f.sample_rate || channels != f.ch_layout.nb_channels {
                return Err(
                    "dynamic audio format/rate/layout is not supported for loudnorm".into(),
                );
            }
            let active = graph.as_mut().ok_or("loudnorm graph missing")?;
            check(
                av_buffersrc_write_frame(active.src, frame.0),
                "feed loudnorm",
            )?;
            loop {
                let code = av_buffersink_get_frame(active.sink, filtered.0);
                if code == AGAIN || code == EOF {
                    break;
                }
                check(code, "drain loudnorm sink")?;
                append_flt_frame(filtered.0, &mut pcm)?;
                av_frame_unref(filtered.0);
            }
        }
        Ok(())
    };

    while packet.read(&mut input)? {
        if unsafe { (*packet.0).stream_index } != index as i32 {
            continue;
        }
        check(
            unsafe { avcodec_send_packet(decoder.0, packet.0) },
            "send audio packet",
        )?;
        loop {
            let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
            if code == AGAIN || code == EOF {
                break;
            }
            check(code, "receive audio frame")?;
            feed(&mut frame)?;
            unsafe { av_frame_unref(frame.0) };
        }
    }
    check(
        unsafe { avcodec_send_packet(decoder.0, ptr::null()) },
        "flush audio decoder",
    )?;
    loop {
        let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
        if code == AGAIN || code == EOF {
            break;
        }
        check(code, "receive flushed audio frame")?;
        feed(&mut frame)?;
        unsafe { av_frame_unref(frame.0) };
    }

    let active = graph
        .as_mut()
        .ok_or("loudnorm requires at least one audio frame")?;
    check(
        unsafe { av_buffersrc_write_frame(active.src, ptr::null()) },
        "flush loudnorm source",
    )?;
    loop {
        let code = unsafe { av_buffersink_get_frame(active.sink, filtered.0) };
        if code == AGAIN || code == EOF {
            break;
        }
        check(code, "drain flushed loudnorm")?;
        unsafe {
            append_flt_frame(filtered.0, &mut pcm)?;
            av_frame_unref(filtered.0);
        }
    }

    if pcm.is_empty() || sample_rate <= 0 || channels <= 0 {
        return Err("loudnorm produced no audio samples".into());
    }
    let sample_frames = (pcm.len() as u64) / channels as u64;
    wav::write_wav_f32le(destination, sample_rate, channels, &pcm)?;
    Ok(LoudnormStats {
        backend: "native libavfilter loudnorm",
        sample_frames,
        sample_rate,
        channels,
        args: loudnorm_args,
        dual_pass: false,
    })
}

#[derive(Debug, Clone)]
struct LoudnormMeasured {
    input_i: String,
    input_tp: String,
    input_lra: String,
    input_thresh: String,
    target_offset: String,
}

static LOG_CAPTURE: Mutex<String> = Mutex::new(String::new());

unsafe extern "C" fn capture_av_log(
    ptr: *mut std::ffi::c_void,
    level: i32,
    fmt: *const std::ffi::c_char,
    vl: va_list,
) {
    let mut line = [0i8; 2048];
    let mut print_prefix = 1i32;
    // SAFETY: FFmpeg formats into the provided buffer; callback is process-global.
    unsafe {
        av_log_format_line(
            ptr,
            level,
            fmt,
            vl,
            line.as_mut_ptr(),
            line.len() as i32,
            &mut print_prefix,
        );
        let text = CStr::from_ptr(line.as_ptr()).to_string_lossy();
        if let Ok(mut buf) = LOG_CAPTURE.lock() {
            buf.push_str(&text);
        }
    }
}

fn with_info_log_capture<T>(body: impl FnOnce() -> Result<T>) -> Result<(T, String)> {
    if let Ok(mut buf) = LOG_CAPTURE.lock() {
        buf.clear();
    }
    let previous = unsafe { av_log_get_level() };
    unsafe {
        av_log_set_level(AV_LOG_INFO as i32);
        av_log_set_callback(Some(capture_av_log));
    }
    let outcome = body();
    unsafe {
        av_log_set_callback(Some(av_log_default_callback));
        av_log_set_level(previous);
    }
    let captured = LOG_CAPTURE.lock().map(|b| b.clone()).unwrap_or_default();
    Ok((outcome?, captured))
}

fn parse_loudnorm_json(log: &str) -> Result<LoudnormMeasured> {
    let start = log
        .rfind('{')
        .ok_or("loudnorm measure pass did not print JSON stats")?;
    let json_text = &log[start..];
    let end = json_text
        .find('}')
        .ok_or("loudnorm measure JSON was truncated")?;
    let value: Value = serde_json::from_str(&json_text[..=end])
        .map_err(|e| format!("parse loudnorm measure JSON: {e}"))?;
    let get = |key: &str| -> Result<String> {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
            .ok_or_else(|| format!("loudnorm measure JSON missing {key}"))
    };
    Ok(LoudnormMeasured {
        input_i: get("input_i")?,
        input_tp: get("input_tp")?,
        input_lra: get("input_lra")?,
        input_thresh: get("input_thresh")?,
        target_offset: get("target_offset")?,
    })
}

fn dual_pass_args(base: &str, measured: &LoudnormMeasured) -> String {
    format!(
        "{base}:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
        measured.input_i,
        measured.input_tp,
        measured.input_lra,
        measured.input_thresh,
        measured.target_offset
    )
}

/// First-pass `loudnorm=...:print_format=json` (discard samples; capture measured_*).
fn measure_loudnorm_values(
    source: &Path,
    loudnorm_args: &str,
    options: &CopyOptions,
) -> Result<LoudnormMeasured> {
    let measure_args = format!("{loudnorm_args}:print_format=json");
    let ((), log) = with_info_log_capture(|| {
        let mut input = Input::open_fast(source)?;
        let index = select_audio_index(&input, options, "loudnorm")?;
        let decoder = open_audio_decoder(&input, index)?;
        let mut packet = Packet::new()?;
        let mut frame = Frame::new()?;
        let mut filtered = Frame::new()?;
        let mut graph: Option<LoudnormGraph> = None;

        let mut feed = |frame: &mut Frame| -> Result<()> {
            unsafe {
                let f = &*frame.0;
                if f.nb_samples <= 0 || f.sample_rate <= 0 || f.ch_layout.nb_channels <= 0 {
                    return Err("invalid decoded audio frame for loudnorm measure".into());
                }
                if f.time_base.num <= 0 || f.time_base.den <= 0 {
                    (*frame.0).time_base = (*decoder.0).pkt_timebase;
                }
                if graph.is_none() {
                    graph = Some(LoudnormGraph::open(f, &measure_args)?);
                }
                let active = graph.as_mut().ok_or("loudnorm measure graph missing")?;
                check(
                    av_buffersrc_write_frame(active.src, frame.0),
                    "feed loudnorm measure",
                )?;
                loop {
                    let code = av_buffersink_get_frame(active.sink, filtered.0);
                    if code == AGAIN || code == EOF {
                        break;
                    }
                    check(code, "drain loudnorm measure")?;
                    av_frame_unref(filtered.0);
                }
            }
            Ok(())
        };

        while packet.read(&mut input)? {
            if unsafe { (*packet.0).stream_index } != index as i32 {
                continue;
            }
            check(
                unsafe { avcodec_send_packet(decoder.0, packet.0) },
                "send audio packet",
            )?;
            loop {
                let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
                if code == AGAIN || code == EOF {
                    break;
                }
                check(code, "receive audio frame")?;
                feed(&mut frame)?;
                unsafe { av_frame_unref(frame.0) };
            }
        }
        check(
            unsafe { avcodec_send_packet(decoder.0, ptr::null()) },
            "flush audio decoder",
        )?;
        loop {
            let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
            if code == AGAIN || code == EOF {
                break;
            }
            check(code, "receive flushed audio frame")?;
            feed(&mut frame)?;
            unsafe { av_frame_unref(frame.0) };
        }
        let active = graph
            .as_mut()
            .ok_or("loudnorm measure requires at least one audio frame")?;
        check(
            unsafe { av_buffersrc_write_frame(active.src, ptr::null()) },
            "flush loudnorm measure source",
        )?;
        loop {
            let code = unsafe { av_buffersink_get_frame(active.sink, filtered.0) };
            if code == AGAIN || code == EOF {
                break;
            }
            check(code, "drain flushed loudnorm measure")?;
            unsafe { av_frame_unref(filtered.0) };
        }
        // Drop graph under the log callback so uninit prints JSON into LOG_CAPTURE.
        drop(graph);
        Ok(())
    })?;
    parse_loudnorm_json(&log)
}

/// Dual-pass `loudnorm`: measure (`print_format=json`) then apply with `measured_*` + `linear=true`.
/// Fair-pairs FFmpeg two-pass loudnorm → float WAV.
pub fn apply_loudnorm_dual(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    let base = resolve_loudnorm_args(args)?;
    if crate::owned_loudnorm::supports_dual(source, destination, &base, options) {
        return crate::owned_loudnorm::apply_loudnorm_dual(source, destination, Some(&base), options);
    }
    let measured = measure_loudnorm_values(source, &base, options)?;
    let pass2 = dual_pass_args(&base, &measured);
    let mut stats = apply_loudnorm(source, destination, Some(&pass2), options)?;
    stats.backend = "native libavfilter loudnorm dual-pass";
    stats.dual_pass = true;
    stats.args = pass2;
    Ok(stats)
}
