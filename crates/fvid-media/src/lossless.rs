//! Decode/crop/FFV1 encode without pixel-format conversion; other streams are copied.
use super::*;
const AGAIN: i32 = -libc::EAGAIN;
pub(super) struct Codec(pub(super) *mut AVCodecContext);
impl Drop for Codec {
    fn drop(&mut self) {
        // SAFETY: Codec exclusively owns the allocated context, including any codec state.
        unsafe {
            avcodec_free_context(&mut self.0);
        }
    }
}
pub(super) struct Frame(pub(super) *mut AVFrame);
impl Frame {
    pub(super) fn new() -> Result<Self> {
        // SAFETY: Fresh allocation; null is checked before any use.
        let p = unsafe { av_frame_alloc() };
        if p.is_null() {
            Err("frame allocation failed".into())
        } else {
            Ok(Self(p))
        }
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: Frame owns its reference and all underlying buffer references.
        unsafe {
            av_frame_free(&mut self.0);
        }
    }
}
pub(super) struct Parameters(pub(super) *mut AVCodecParameters);
impl Drop for Parameters {
    fn drop(&mut self) {
        // SAFETY: Parameters exclusively owns the allocation from avcodec_parameters_alloc.
        unsafe {
            avcodec_parameters_free(&mut self.0);
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct CropRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct LosslessTransform {
    pub crop: Option<CropRect>,
    pub vertical_flip: bool,
    pub horizontal_flip: bool,
    /// Half-open presentation-time interval in microseconds relative to container start.
    pub interval: Option<(i64, i64)>,
    /// Use demuxer seeking before decoding; requires an interval.
    pub seek: bool,
}
#[derive(Serialize, Debug)]
pub struct LosslessStats {
    pub backend: &'static str,
    pub video_frames: u64,
    pub decoded_frames: u64,
    pub seek_used: bool,
    pub video_packets: u64,
    pub copied_packets: u64,
    pub trimmed_audio_sample_frames: u64,
    pub pixel_format: String,
    pub encoder: String,
    pub fvid_crop_payload_copies: u64,
    pub vertical_flip: bool,
    pub horizontal_flip: bool,
}
fn drain_encoder(
    encoder: &mut Codec,
    output: &mut Output,
    packet: &mut Packet,
    index: usize,
    stats: &mut LosslessStats,
) -> Result<()> {
    loop {
        // SAFETY: Valid codec and packet; receive overwrites the empty packet reference.
        let code = unsafe { avcodec_receive_packet(encoder.0, packet.0) };
        if code == AGAIN || code == EOF {
            return Ok(());
        }
        check(code, "receive lossless packet")?;
        // SAFETY: Encoder is initialized and its time base remains fixed.
        let tb = unsafe { (*encoder.0).time_base };
        output.write(packet, index, tb)?;
        stats.video_packets += 1;
    }
}

fn send_encoder_frame(
    encoder: &mut Codec,
    output: &mut Output,
    packet: &mut Packet,
    index: usize,
    frame: *mut AVFrame,
    stats: &mut LosslessStats,
) -> Result<()> {
    loop {
        let code = unsafe { avcodec_send_frame(encoder.0, frame) };
        if code == AGAIN {
            let before = stats.video_packets;
            drain_encoder(encoder, output, packet, index, stats)?;
            if stats.video_packets == before {
                return Err("encoder stalled (EAGAIN with no packets)".into());
            }
            continue;
        }
        check(code, "send frame to encoder")?;
        return Ok(());
    }
}

#[derive(Clone, Copy)]
struct CropStage {
    index: usize,
    crop: CropRect,
    vertical_flip: bool,
    horizontal_flip: bool,
    interval: Option<(i64, i64)>,
}
fn drain_decoder(
    decoder: &mut Codec,
    encoder: &mut Codec,
    output: &mut Output,
    frame: &mut Frame,
    compact: &mut [Frame],
    compact_i: &mut usize,
    packet: &mut Packet,
    stage: CropStage,
    stats: &mut LosslessStats,
) -> Result<()> {
    let CropStage {
        index,
        crop,
        vertical_flip,
        horizontal_flip,
        interval,
    } = stage;
    let full_w = unsafe { (*decoder.0).width };
    let full_h = unsafe { (*decoder.0).height };
    loop {
        // SAFETY: Decoder/frame are live and exclusively borrowed.
        let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
        if code == AGAIN || code == EOF {
            return Ok(());
        }
        check(code, "receive decoded frame")?;
        stats.decoded_frames += 1;
        // SAFETY: Decoded frame owns its planes. crop bounds/subsampling are checked
        // before av_frame_apply_cropping adjusts views. It does not copy pixel payloads.
        unsafe {
            let f = &mut *frame.0;
            if f.width != full_w
                || f.height != full_h
                || f.format != (*encoder.0).pix_fmt
                || (f.width as usize) < crop.x + crop.width
                || (f.height as usize) < crop.y + crop.height
            {
                return Err(
                    "dynamic frame geometry/format is not qualified for lossless export".into(),
                );
            }
            if f.pts == NOPTS {
                f.pts = f.best_effort_timestamp;
            }
            if f.pts == NOPTS {
                return Err("lossless export requires frame timestamps".into());
            }
            if let Some((start, end)) = interval {
                if f.pts < start || f.pts >= end {
                    av_frame_unref(frame.0);
                    continue;
                }
                f.pts = f.pts.checked_sub(start).ok_or("frame timestamp overflow")?;
            }
            // Force CFR PTS 0..N-1 (FFmpeg CLI → libx264) so B-adapt matches.
            f.pts = stats.video_frames as i64;
            f.duration = 1;
            // Decoder pict_type (I/P/B from the source bitstream) must not force
            // libx264 picture types — FFmpeg filters clear this to NONE.
            f.pict_type = 0;
            f.quality = 0;
            if f.flags & AV_FRAME_FLAG_INTERLACED as i32 != 0 {
                return Err("interlaced crop is not qualified".into());
            }
            f.crop_left = crop.x;
            f.crop_top = crop.y;
            f.crop_right = f.width as usize - crop.x - crop.width;
            f.crop_bottom = f.height as usize - crop.y - crop.height;
            check(
                av_frame_apply_cropping(frame.0, AV_FRAME_CROP_UNALIGNED as i32),
                "apply exact crop view",
            )?;
            if vertical_flip {
                flip_view(frame.0)?;
            }
            // Crop/vflip: view (no pack). Hflip: single-pass copy into pool.
            let send = if horizontal_flip {
                let i = *compact_i;
                *compact_i = (i + 1) % compact.len();
                let slot = &mut compact[i];
                horizontal_copy_frame(slot.0, frame.0)?;
                (*slot.0).pict_type = 0;
                (*slot.0).quality = 0;
                (*slot.0).pts = stats.video_frames as i64;
                (*slot.0).duration = 1;
                slot.0
            } else {
                frame.0
            };
            // Drain only on encoder EAGAIN (inside send) so libx264 keeps depth.
            send_encoder_frame(encoder, output, packet, index, send, stats)?;
            av_frame_unref(frame.0);
        }
        stats.video_frames += 1;
    }
}

unsafe fn alloc_like_frame(dst: *mut AVFrame, src: *const AVFrame) -> Result<()> {
    unsafe {
        let s = &*src;
        let d = &mut *dst;
        if d.data[0].is_null()
            || d.width != s.width
            || d.height != s.height
            || d.format != s.format
            || av_frame_is_writable(dst) == 0
        {
            av_frame_unref(dst);
            d.format = s.format;
            d.width = s.width;
            d.height = s.height;
            // Default align (0 → 32) matches FFmpeg filter frames.
            check(av_frame_get_buffer(dst, 0), "alloc compact encode frame")?;
        }
    }
    Ok(())
}

/// Single-pass horizontal flip from `src` into a pooled writable `dst`.
unsafe fn horizontal_copy_frame(dst: *mut AVFrame, src: *mut AVFrame) -> Result<()> {
    unsafe {
        alloc_like_frame(dst, src)?;
        check(av_frame_copy_props(dst, src), "hflip copy props")?;
        let s = &*src;
        let d = &*dst;
        let desc = av_pix_fmt_desc_get(s.format);
        if desc.is_null() || s.width <= 0 || s.height <= 0 {
            return Err("invalid horizontal-filter geometry".into());
        }
        let desc = &*desc;
        let planes = av_pix_fmt_count_planes(s.format);
        if !(1..=4).contains(&planes)
            || desc.flags
                & (AV_PIX_FMT_FLAG_HWACCEL | AV_PIX_FMT_FLAG_PAL | AV_PIX_FMT_FLAG_BITSTREAM) as u64
                != 0
            || (planes == 1 && desc.log2_chroma_w != 0 && desc.flags & AV_PIX_FMT_FLAG_RGB as u64 == 0)
        {
            return Err(
                "horizontal filter requires software planes with uniform pixel groups".into(),
            );
        }
        for plane in 0..planes as usize {
            let mut step = None;
            for component in &desc.comp[..desc.nb_components as usize] {
                if component.plane as usize == plane {
                    if step.is_some_and(|v| v != component.step) {
                        return Err("mixed pixel steps in plane".into());
                    }
                    step = Some(component.step);
                }
            }
            let step = usize::try_from(step.ok_or("plane has no components")?)
                .map_err(|_| "invalid pixel step")?;
            let chroma =
                (plane == 1 || plane == 2) && desc.flags & AV_PIX_FMT_FLAG_RGB as u64 == 0;
            let width =
                (s.width as usize).div_ceil(1usize << if chroma { desc.log2_chroma_w } else { 0 });
            let height =
                (s.height as usize).div_ceil(1usize << if chroma { desc.log2_chroma_h } else { 0 });
            let src_stride = s.linesize[plane].unsigned_abs() as usize;
            let dst_stride = d.linesize[plane].unsigned_abs() as usize;
            if step == 0
                || s.data[plane].is_null()
                || d.data[plane].is_null()
                || width.checked_mul(step).is_none_or(|bytes| bytes > src_stride || bytes > dst_stride)
            {
                return Err("invalid horizontal-filter row extent".into());
            }
            let row_bytes = width * step;
            for row in 0..height {
                let src_off = (row as isize)
                    .checked_mul(s.linesize[plane] as isize)
                    .ok_or("row offset overflow")?;
                let dst_off = (row as isize)
                    .checked_mul(d.linesize[plane] as isize)
                    .ok_or("row offset overflow")?;
                let src_row = std::slice::from_raw_parts(s.data[plane].offset(src_off), row_bytes);
                let dst_row =
                    std::slice::from_raw_parts_mut(d.data[plane].offset(dst_off), row_bytes);
                fvid_cpu::hflip_row_copy(dst_row, src_row, width, step);
            }
        }
    }
    Ok(())
}

/// Reverse row traversal while keeping the decoder-owned AVBuffer references.
/// SAFETY: frame must be a live, writable AVFrame with software video planes.
unsafe fn flip_view(frame: *mut AVFrame) -> Result<()> {
    // SAFETY: Caller owns the decoded frame; validated descriptor/plane bounds below
    // keep offsets within each decoder-provided plane. No buffer ownership changes.
    unsafe {
        let f = &mut *frame;
        let desc = av_pix_fmt_desc_get(f.format);
        if desc.is_null()
            || (*desc).flags
                & (AV_PIX_FMT_FLAG_HWACCEL | AV_PIX_FMT_FLAG_BITSTREAM | AV_PIX_FMT_FLAG_PAL) as u64
                != 0
        {
            return Err("vertical flip requires ordinary software pixel planes".into());
        }
        let planes = av_pix_fmt_count_planes(f.format);
        if !(1..=4).contains(&planes) || f.height <= 0 {
            return Err("invalid video plane geometry".into());
        }
        for plane in 0..planes as usize {
            let chroma =
                (plane == 1 || plane == 2) && (*desc).flags & AV_PIX_FMT_FLAG_RGB as u64 == 0;
            let shift = if chroma { (*desc).log2_chroma_h } else { 0 };
            let height = (f.height as usize).div_ceil(1usize << shift);
            let stride = f.linesize[plane];
            if f.data[plane].is_null() || stride == 0 || stride == i32::MIN {
                return Err("invalid video plane stride".into());
            }
            let offset = (height - 1)
                .checked_mul(stride.unsigned_abs() as usize)
                .and_then(|v| isize::try_from(v).ok())
                .ok_or("plane offset overflow")?;
            f.data[plane] = f.data[plane].offset(if stride < 0 { -offset } else { offset });
            f.linesize[plane] = -stride;
        }
        // Video extended_data normally aliases data; keep a separate table in sync too.
        if f.extended_data != f.data.as_mut_ptr() {
            for plane in 0..planes as usize {
                *f.extended_data.add(plane) = f.data[plane];
            }
        }
        Ok(())
    }
}
/// Lossless FFV1 in Matroska; all selected non-video packets are remuxed.
/// Strictly preserves pixel format/subsampling/bit depth rather than converting silently.
pub fn crop_lossless(
    source: &Path,
    destination: &Path,
    crop: CropRect,
    options: &CopyOptions,
) -> Result<LosslessStats> {
    transcode_lossless(
        source,
        destination,
        LosslessTransform {
            crop: Some(crop),
            vertical_flip: false,
            horizontal_flip: false,
            interval: None,
            seek: false,
        },
        options,
    )
}
/// Clip chapter ranges and rebase them to the selected presentation interval.
/// All arithmetic is validated before mutating the input-owned chapter table.
pub(super) fn retime_chapters(input: &mut Input, from: i64, to: i64) -> Result<()> {
    // SAFETY: Input exclusively owns the context, chapter array and metadata. Removed
    // entries are freed once; kept pointers are compacted and nb_chapters updated.
    unsafe {
        let context = &mut *input.0;
        let count = context.nb_chapters as usize;
        if count > 4096 {
            return Err("too many chapters".into());
        }
        let origin = if context.start_time == NOPTS {
            0
        } else {
            context.start_time
        };
        let start = origin
            .checked_add(from)
            .ok_or("chapter interval overflow")?;
        let end = origin.checked_add(to).ok_or("chapter interval overflow")?;
        let mut ranges = Vec::with_capacity(count);
        for index in 0..count {
            let chapter = &**context.chapters.add(index);
            let micros = |value: i64| -> Result<i64> {
                let tb = chapter.time_base;
                if tb.num <= 0 || tb.den <= 0 {
                    return Err("invalid chapter time base".into());
                }
                let numerator = i128::from(value) * i128::from(tb.num) * 1_000_000;
                if numerator % i128::from(tb.den) != 0 {
                    return Err("chapter time cannot be represented exactly in microseconds".into());
                }
                i64::try_from(numerator / i128::from(tb.den))
                    .map_err(|_| "chapter timestamp overflow".into())
            };
            let left = micros(chapter.start)?;
            let right = micros(chapter.end)?;
            if right < left {
                return Err("reversed chapter range".into());
            }
            let left = left.max(start);
            let right = right.min(end);
            ranges.push(if left < right {
                Some((
                    left.checked_sub(start)
                        .ok_or("chapter timestamp overflow")?,
                    right
                        .checked_sub(start)
                        .ok_or("chapter timestamp overflow")?,
                ))
            } else {
                None
            });
        }
        context.nb_chapters = 0;
        for (index, range) in ranges.into_iter().enumerate() {
            let chapter = *context.chapters.add(index);
            if let Some((start, end)) = range {
                (*chapter).start = start;
                (*chapter).end = end;
                (*chapter).time_base = AVRational {
                    num: 1,
                    den: 1_000_000,
                };
                *context.chapters.add(context.nb_chapters as usize) = chapter;
                context.nb_chapters += 1;
            } else {
                av_dict_free(&mut (*chapter).metadata);
                av_free(chapter.cast());
            }
        }
    }
    Ok(())
}
/// Preserve decoded samples in FFV1; optional crop and vertical flip use plane views.
pub fn transcode_lossless(
    source: &Path,
    destination: &Path,
    transform: LosslessTransform,
    options: &CopyOptions,
) -> Result<LosslessStats> {
    if destination.extension().and_then(|v| v.to_str()) != Some("mkv") {
        return Err("lossless export requires FFV1 in .mkv".into());
    }
    transcode(
        source,
        destination,
        transform,
        options,
        &EncoderSettings {
            name: "ffv1".into(),
            options: Vec::new(),
        },
    )
}
/// Explicit encoder selection; quality is determined by encoder options, not by
/// the LosslessTransform geometry descriptor. No automatic pixel conversion.
#[derive(Clone, Debug)]
pub struct EncoderSettings {
    pub name: String,
    pub options: Vec<(String, String)>,
}
struct CodecOptions(*mut AVDictionary);
impl Drop for CodecOptions {
    fn drop(&mut self) {
        // SAFETY: This guard exclusively owns its dictionary, including leftovers.
        unsafe {
            av_dict_free(&mut self.0);
        }
    }
}
pub fn transcode(
    source: &Path,
    destination: &Path,
    transform: LosslessTransform,
    options: &CopyOptions,
    settings: &EncoderSettings,
) -> Result<LosslessStats> {
    if settings.options.len() > 64 {
        return Err("too many encoder options".into());
    }
    let encoder_name = cstring(&settings.name)?;
    let mut codec_options = CodecOptions(ptr::null_mut());
    for (key, value) in &settings.options {
        if key.is_empty() || key.len() > 256 || value.len() > 8192 {
            return Err("invalid encoder option size".into());
        }
        let key = cstring(key)?;
        let value = cstring(value)?;
        // SAFETY: Dictionary owns copies of the valid NUL-terminated strings.
        check(
            unsafe { av_dict_set(&mut codec_options.0, key.as_ptr(), value.as_ptr(), 0) },
            "set encoder option",
        )?;
    }
    if transform.seek && transform.interval.is_none() {
        return Err("seek requires a lossless interval".into());
    }
    let mut input = Input::open(source)?;
    let selected = selection(&input, options)?;
    // SAFETY: Selection points into Input's live stream table and codec parameters.
    let videos: Vec<_> = selected
        .iter()
        .copied()
        .filter(|&i| unsafe {
            (*(*input.streams()[i]).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
        })
        .collect();
    if videos.len() != 1 {
        return Err("lossless crop requires exactly one selected video stream".into());
    }
    let video = videos[0];
    let interval = if let Some((from, to)) = transform.interval {
        if from < 0 || to <= from {
            return Err("lossless interval requires 0 <= from < to".into());
        }
        for &index in &selected {
            if index != video {
                // SAFETY: Selected codec parameters belong to the live input.
                pcm::validate(unsafe { &*(*input.streams()[index]).codecpar })?;
            }
        }
        if transform.seek && selected.len() != 1 {
            return Err("seek with trimmed audio is not qualified; omit --seek".into());
        }
        // SAFETY: The input context and selected stream remain live.
        unsafe {
            let origin = if (*input.0).start_time == NOPTS {
                0
            } else {
                (*input.0).start_time
            };
            let tb = (*input.streams()[videos[0]]).time_base;
            let ticks = |time: i64| -> Result<i64> {
                let us = origin
                    .checked_add(time)
                    .ok_or("interval timestamp overflow")?;
                let numerator = i128::from(us) * i128::from(tb.den);
                let denominator = 1_000_000i128 * i128::from(tb.num);
                if denominator <= 0 || tb.den <= 0 || numerator % denominator != 0 {
                    return Err("interval boundary is not exact in video time base".into());
                }
                i64::try_from(numerator / denominator)
                    .map_err(|_| "interval timestamp overflow".into())
            };
            Some((ticks(from)?, ticks(to)?))
        }
    } else {
        None
    };

    let crop = transform.crop.unwrap_or_else(|| {
        // SAFETY: Selected stream belongs to the live input.
        let p = unsafe { &*(*input.streams()[video]).codecpar };
        CropRect {
            x: 0,
            y: 0,
            width: p.width.max(0) as usize,
            height: p.height.max(0) as usize,
        }
    });
    let mapped = selected.iter().position(|&i| i == video).unwrap();
    // SAFETY: Contexts allocated below are held in RAII guards before subsequent
    // fallible calls. Codec pointers and source parameters are library-owned/live.
    let (mut decoder, mut encoder, parameters, tb, pixel_format) = unsafe {
        let s = &*input.streams()[video];
        let p = &*s.codecpar;
        if p.nb_coded_side_data != 0 {
            return Err(
                "lossless crop with codec side data requires explicit metadata handling".into(),
            );
        }
        let dec = avcodec_find_decoder(p.codec_id);
        if dec.is_null() {
            return Err("decoder unavailable".into());
        }
        let decoder = Codec(avcodec_alloc_context3(dec));
        if decoder.0.is_null() {
            return Err("decoder allocation failed".into());
        }
        check(
            avcodec_parameters_to_context(decoder.0, s.codecpar),
            "configure decoder",
        )?;
        (*decoder.0).pkt_timebase = s.time_base;
        (*decoder.0).thread_count = 0;
        (*decoder.0).max_pixels = 8192 * 4320;
        check(
            avcodec_open2(decoder.0, dec, ptr::null_mut()),
            "open decoder",
        )?;
        let format = (*decoder.0).pix_fmt;
        let descriptor = av_pix_fmt_desc_get(format);
        if descriptor.is_null() {
            return Err("unknown decoded pixel format".into());
        }
        let sx = 1usize << (*descriptor).log2_chroma_w;
        let sy = 1usize << (*descriptor).log2_chroma_h;
        if crop.width == 0
            || crop.height == 0
            || crop
                .x
                .checked_add(crop.width)
                .is_none_or(|v| v > p.width.max(0) as usize)
            || crop
                .y
                .checked_add(crop.height)
                .is_none_or(|v| v > p.height.max(0) as usize)
            || !crop.x.is_multiple_of(sx)
            || !crop.y.is_multiple_of(sy)
        {
            return Err("crop must be bounded with a chroma-aligned origin".into());
        }
        let enc = avcodec_find_encoder_by_name(encoder_name.as_ptr());
        if enc.is_null() {
            return Err(format!("encoder unavailable: {}", settings.name));
        }
        let encoder = Codec(avcodec_alloc_context3(enc));
        if encoder.0.is_null() {
            return Err("encoder allocation failed".into());
        }
        (*encoder.0).width = crop.width as i32;
        (*encoder.0).height = crop.height as i32;
        (*encoder.0).pix_fmt = format;
        // ffmpeg CLI uses 1/fps as encode time_base; keep stream tb only as fallback.
        let fr = s.avg_frame_rate;
        if fr.num > 0 && fr.den > 0 {
            (*encoder.0).framerate = fr;
            (*encoder.0).time_base = AVRational {
                num: fr.den,
                den: fr.num,
            };
        } else {
            (*encoder.0).time_base = s.time_base;
            (*encoder.0).framerate = fr;
        }
        (*encoder.0).sample_aspect_ratio = p.sample_aspect_ratio;
        (*encoder.0).color_range = p.color_range;
        (*encoder.0).color_primaries = p.color_primaries;
        (*encoder.0).color_trc = p.color_trc;
        (*encoder.0).colorspace = p.color_space;
        (*encoder.0).chroma_sample_location = p.chroma_location;
        (*encoder.0).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        if (*enc).id == AVCodecID_AV_CODEC_ID_FFV1 {
            (*encoder.0).level = 3;
        }
        (*encoder.0).thread_count = 0;
        check(
            avcodec_open2(encoder.0, enc, &mut codec_options.0),
            "open selected encoder without pixel conversion",
        )?;
        if av_dict_count(codec_options.0) != 0 {
            return Err("unrecognized or unused encoder option".into());
        }
        if (*encoder.0).pix_fmt != format {
            return Err("encoder changed pixel format".into());
        }
        let parameters = Parameters(avcodec_parameters_alloc());
        if parameters.0.is_null() {
            return Err("codec parameter allocation failed".into());
        }
        check(
            avcodec_parameters_from_context(parameters.0, encoder.0),
            "export FFV1 parameters",
        )?;
        let enc_tb = (*encoder.0).time_base;
        (
            decoder,
            encoder,
            parameters,
            enc_tb,
            string(av_get_pix_fmt_name(format)),
        )
    };
    if let Some((from, to)) = transform.interval {
        retime_chapters(&mut input, from, to)?;
    }
    if transform.seek {
        let (start, _) = interval.ok_or("seek requires interval")?;
        // SAFETY: Live demuxer, valid stream index/time base. The upper bound forces
        // a seek point at or before the requested start; no decoder packets sent yet.
        check(
            unsafe { avformat_seek_file(input.0, video as i32, i64::MIN, start, start, 0) },
            "seek before lossless interval",
        )?;
    }
    let mut output = Output::with_video(
        destination,
        &input,
        &selected,
        Some((video, parameters.0, tb)),
    )?
    .without_interleave();
    let mut packet = Packet::new()?;
    let mut encoded = Packet::new()?;
    let mut frame = Frame::new()?;
    let mut compact: Vec<Frame> = (0..32).map(|_| Frame::new()).collect::<Result<Vec<_>>>()?;
    let mut compact_i = 0usize;
    let mut stats = LosslessStats {
        backend: "native libavcodec + Fvid crop view",
        video_frames: 0,
        decoded_frames: 0,
        seek_used: transform.seek,
        video_packets: 0,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format,
        encoder: settings.name.clone(),
        fvid_crop_payload_copies: 0,
        vertical_flip: transform.vertical_flip,
        horizontal_flip: transform.horizontal_flip,
    };
    while packet.read(&mut input)? {
        let (index, _) = packet_info(&packet, &input, options)?;
        if index == video {
            // SAFETY: Decoder retains any packet references it needs after the call.
            check(
                unsafe { avcodec_send_packet(decoder.0, packet.0) },
                "send compressed video packet",
            )?;
            drain_decoder(
                &mut decoder,
                &mut encoder,
                &mut output,
                &mut frame,
                &mut compact,
                &mut compact_i,
                &mut encoded,
                CropStage {
                    index: mapped,
                    crop,
                    vertical_flip: transform.vertical_flip,
                    horizontal_flip: transform.horizontal_flip,
                    interval,
                },
                &mut stats,
            )?;
        } else if let Some(mapped) = selected.iter().position(|&i| i == index) {
            // SAFETY: Index checked by packet_info; Input is live.
            let tb = unsafe { (*input.streams()[index]).time_base };
            if let Some((from, to)) = transform.interval {
                let samples = pcm::trim(&mut packet, &input, index, from, to)?;
                if samples == 0 {
                    continue;
                }
                stats.trimmed_audio_sample_frames += samples;
            }
            output.write(&mut packet, mapped, tb)?;
            stats.copied_packets += 1;
        }
    }
    // SAFETY: Null packets/frames are the documented drain signals for open codecs.
    check(
        unsafe { avcodec_send_packet(decoder.0, ptr::null()) },
        "drain decoder",
    )?;
    drain_decoder(
        &mut decoder,
        &mut encoder,
        &mut output,
        &mut frame,
        &mut compact,
        &mut compact_i,
        &mut encoded,
        CropStage {
            index: mapped,
            crop,
            vertical_flip: transform.vertical_flip,
            horizontal_flip: transform.horizontal_flip,
            interval,
        },
        &mut stats,
    )?;
    // SAFETY: Decoder has finished; no new frames follow the encoder drain signal.
    loop {
        let code = unsafe { avcodec_send_frame(encoder.0, ptr::null()) };
        if code == AGAIN {
            let before = stats.video_packets;
            drain_encoder(&mut encoder, &mut output, &mut encoded, mapped, &mut stats)?;
            if stats.video_packets == before {
                return Err("encoder flush stalled".into());
            }
            continue;
        }
        if code == EOF {
            break;
        }
        check(code, "drain encoder")?;
        break;
    }
    drain_encoder(&mut encoder, &mut output, &mut encoded, mapped, &mut stats)?;
    if stats.video_frames == 0 {
        return Err("no video frames decoded".into());
    }
    output.finish()?;
    Ok(stats)
}
