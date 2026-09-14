//! Optional native FFmpeg-library adapter. No subprocess execution in production.
#[allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    dead_code,
    unnecessary_transmutes,
    clippy::all
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/av.rs"));
}
mod edit;
mod input_policy;
pub use edit::{concat, parse_time, trim};
pub use input_policy::with_standalone_inputs;
mod audio;
mod audio_layout;
mod lossless;
mod pcm;
pub use audio::{AudioDecodeStats, decode_audio};
use ffi::*;
pub use lossless::{
    CropRect, EncoderSettings, LosslessStats, LosslessTransform, crop_lossless, transcode,
    transcode_lossless,
};
pub use pcm::{PcmTrimStats, trim_pcm};
use serde::Serialize;
use std::{
    ffi::{CStr, CString},
    path::{Path, PathBuf},
    ptr, slice,
};
pub type Result<T> = std::result::Result<T, String>;
const NOPTS: i64 = i64::MIN;
const EOF: i32 = -541478725;
const MAX_STREAMS: usize = 64;
fn cstring(text: &str) -> Result<CString> {
    CString::new(text).map_err(|_| "embedded NUL".into())
}
fn path_string(path: &Path) -> Result<CString> {
    cstring(path.to_str().ok_or("path must be UTF-8")?)
}
fn check(code: i32, operation: &str) -> Result<()> {
    if code >= 0 {
        return Ok(());
    }
    let mut buffer = [0i8; 256];
    // SAFETY: The error formatter writes at most the supplied buffer length.
    unsafe {
        av_strerror(code, buffer.as_mut_ptr(), buffer.len());
    }
    // SAFETY: buffer starts zeroed; av_strerror always terminates within its bound.
    let detail = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
    Err(format!("{operation}: {detail} ({code})"))
}
fn string(pointer: *const std::ffi::c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: Internal callers pass live FFmpeg-owned, NUL-terminated strings.
    unsafe { CStr::from_ptr(pointer).to_string_lossy().into_owned() }
}
struct Input(*mut AVFormatContext);
impl Input {
    fn open(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Err("media input must be an existing local file".into());
        }
        let path = path_string(path)?;
        // SAFETY: Fresh context, immediately guarded. FFmpeg owns/frees its fields.
        let mut input = Self(unsafe { avformat_alloc_context() });
        if input.0.is_null() {
            return Err("input context allocation failed".into());
        }
        // SAFETY: Context and option dictionary are uniquely owned; strings live
        // for the calls. The dictionary is released on success and failure.
        unsafe {
            (*input.0).max_streams = MAX_STREAMS as i32;
            (*input.0).probesize = 5 * 1024 * 1024;
            (*input.0).max_analyze_duration = 5_000_000;
            let mut options = ptr::null_mut();
            check(
                av_dict_set(
                    &mut options,
                    c"protocol_whitelist".as_ptr(),
                    c"file".as_ptr(),
                    0,
                ),
                "set local input policy",
            )?;
            if input_policy::active() {
                let code = av_dict_set(&mut options, c"format_whitelist".as_ptr(),
                    c"mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,wav,mp3,flac,ogg,aac,avi,mpeg,mpegts,asf,flv,yuv4mpegpipe,png_pipe,jpeg_pipe".as_ptr(), 0);
                if code < 0 {
                    av_dict_free(&mut options);
                    check(code, "set standalone input policy")?;
                }
            }
            let code = avformat_open_input(&mut input.0, path.as_ptr(), ptr::null(), &mut options);
            av_dict_free(&mut options);
            check(code, "open input")?;
        }
        // SAFETY: Successful open creates a live input context, owned by this guard.
        check(
            unsafe { avformat_find_stream_info(input.0, ptr::null_mut()) },
            "read stream information",
        )?;
        // SAFETY: The live context owns its stream table until Input is dropped.
        if unsafe { (*input.0).nb_streams as usize } > MAX_STREAMS {
            return Err("too many streams; maximum 64".into());
        }
        Ok(input)
    }
    fn streams(&self) -> &[*mut AVStream] {
        // SAFETY: Input owns the stream pointer array for the duration of this borrow.
        unsafe {
            if (*self.0).nb_streams == 0 {
                &[]
            } else {
                slice::from_raw_parts((*self.0).streams, (*self.0).nb_streams as usize)
            }
        }
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        // SAFETY: This guard uniquely owns the context returned by open_input.
        unsafe {
            avformat_close_input(&mut self.0);
        }
    }
}
struct Packet(*mut AVPacket);
impl Packet {
    fn new() -> Result<Self> {
        // SAFETY: FFmpeg allocates a fresh empty packet; null is checked.
        let p = unsafe { av_packet_alloc() };
        if p.is_null() {
            Err("packet allocation failed".into())
        } else {
            Ok(Self(p))
        }
    }
    fn read(&mut self, input: &mut Input) -> Result<bool> {
        // SAFETY: Both resources are exclusively borrowed and valid. Unref permits reuse.
        unsafe {
            av_packet_unref(self.0);
            let code = av_read_frame(input.0, self.0);
            if code == EOF {
                Ok(false)
            } else {
                check(code, "read packet")?;
                Ok(true)
            }
        }
    }
}
impl Drop for Packet {
    fn drop(&mut self) {
        // SAFETY: Unique ownership; av_packet_free releases payload references as well.
        unsafe {
            av_packet_free(&mut self.0);
        }
    }
}
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub index: usize,
    pub media_type: String,
    pub codec: String,
    pub time_base: [i32; 2],
    pub start: Option<i64>,
    pub duration: Option<i64>,
    pub width: i32,
    pub height: i32,
    pub pixel_format: i32,
    pub sample_rate: i32,
    pub channels: i32,
    pub video_delay: i32,
    pub extradata_bytes: usize,
}
#[derive(Serialize, Debug)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub format: String,
    pub duration_us: Option<i64>,
    pub streams: Vec<StreamInfo>,
}
pub fn probe(path: &Path) -> Result<MediaInfo> {
    let input = Input::open(path)?;
    // SAFETY: Input owns all contexts, stream parameters and strings for this block.
    unsafe {
        let mut streams = Vec::new();
        for (index, stream) in input.streams().iter().enumerate() {
            let s = &**stream;
            let p = &*s.codecpar;
            streams.push(StreamInfo {
                index,
                media_type: string(av_get_media_type_string(p.codec_type)),
                codec: string(avcodec_get_name(p.codec_id)),
                time_base: [s.time_base.num, s.time_base.den],
                start: (s.start_time != NOPTS).then_some(s.start_time),
                duration: (s.duration != NOPTS).then_some(s.duration),
                width: p.width,
                height: p.height,
                pixel_format: p.format,
                sample_rate: p.sample_rate,
                channels: p.ch_layout.nb_channels,
                video_delay: p.video_delay,
                extradata_bytes: p.extradata_size.max(0) as usize,
            });
        }
        Ok(MediaInfo {
            path: path.into(),
            format: string((*(*input.0).iformat).name),
            duration_us: ((*input.0).duration != NOPTS).then_some((*input.0).duration),
            streams,
        })
    }
}
#[derive(Serialize)]
pub struct Capabilities {
    pub library_version: String,
    pub demuxers: Vec<String>,
    pub muxers: Vec<String>,
    pub decoders: Vec<String>,
    pub encoders: Vec<String>,
    pub filters: Vec<String>,
}
pub fn capabilities() -> Capabilities {
    // SAFETY: Iterators use library-owned static descriptors, each with its own opaque cursor.
    unsafe {
        let mut result = Capabilities {
            library_version: string(av_version_info()),
            demuxers: vec![],
            muxers: vec![],
            decoders: vec![],
            encoders: vec![],
            filters: vec![],
        };
        let mut opaque = ptr::null_mut();
        loop {
            let p = av_demuxer_iterate(&mut opaque);
            if p.is_null() {
                break;
            }
            result.demuxers.push(string((*p).name));
        }
        opaque = ptr::null_mut();
        loop {
            let p = av_muxer_iterate(&mut opaque);
            if p.is_null() {
                break;
            }
            result.muxers.push(string((*p).name));
        }
        opaque = ptr::null_mut();
        loop {
            let p = av_codec_iterate(&mut opaque);
            if p.is_null() {
                break;
            }
            if av_codec_is_decoder(p) != 0 {
                result.decoders.push(string((*p).name));
            }
            if av_codec_is_encoder(p) != 0 {
                result.encoders.push(string((*p).name));
            }
        }
        opaque = ptr::null_mut();
        loop {
            let p = av_filter_iterate(&mut opaque);
            if p.is_null() {
                break;
            }
            result.filters.push(string((*p).name));
        }
        result
    }
}

#[derive(Clone, Debug)]
pub struct CopyOptions {
    /// Empty selects every stream; otherwise indices are preserved in this order.
    pub streams: Vec<usize>,
    pub max_packet_bytes: usize,
}
impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            streams: vec![],
            max_packet_bytes: 64 * 1024 * 1024,
        }
    }
}
#[derive(Serialize, Default, Debug)]
pub struct CopyStats {
    pub packets: u64,
    pub payload_bytes: u64,
    pub segments: usize,
    pub backend: &'static str,
    pub fvid_payload_copies: u64,
}
struct Output {
    context: *mut AVFormatContext,
    temporary: PathBuf,
    destination: PathBuf,
    strict_timing: bool,
}
impl Output {
    fn new(destination: &Path, source: &Input, selected: &[usize]) -> Result<Self> {
        Self::with_video(destination, source, selected, None)
    }
    fn with_video(
        destination: &Path,
        source: &Input,
        selected: &[usize],
        video: Option<(usize, *const AVCodecParameters, AVRational)>,
    ) -> Result<Self> {
        if destination.symlink_metadata().is_ok() {
            return Err("output already exists".into());
        }
        let directory = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = None;
        for attempt in 0..100 {
            let path = directory.join(format!(".fvid-media-{}-{attempt}.tmp", std::process::id()));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => {
                    temporary = Some(path);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        let temporary = temporary.ok_or("cannot reserve temporary output")?;
        let mut out = Self {
            context: ptr::null_mut(),
            temporary,
            destination: destination.into(),
            strict_timing: false,
        };
        let target = path_string(destination)?;
        let temp = path_string(&out.temporary)?;
        // SAFETY: All descriptors and owned input parameters remain live; Output
        // takes ownership of the newly allocated format context and its AVIO.
        unsafe {
            check(
                avformat_alloc_output_context2(
                    &mut out.context,
                    ptr::null(),
                    ptr::null(),
                    target.as_ptr(),
                ),
                "select output container",
            )?;
            if out.context.is_null() {
                return Err("output context allocation failed".into());
            }
            if (*(*out.context).oformat).flags & AVFMT_NOFILE as i32 != 0 {
                return Err("output must be a file muxer".into());
            }
            (*out.context).max_interleave_delta = 1_000_000;
            (*out.context).avoid_negative_ts = AVFMT_AVOID_NEG_TS_DISABLED as i32;
            check(
                av_dict_copy(&mut (*out.context).metadata, (*source.0).metadata, 0),
                "copy container metadata",
            )?;
            let count = (*source.0).nb_chapters as usize;
            if count > 4096 {
                return Err("too many chapters".into());
            }
            if count > 0 {
                (*out.context).chapters =
                    av_calloc(count, std::mem::size_of::<*mut AVChapter>()).cast();
                if (*out.context).chapters.is_null() {
                    return Err("chapter table allocation failed".into());
                }
                for index in 0..count {
                    let src = *(*source.0).chapters.add(index);
                    let dst: *mut AVChapter = av_mallocz(std::mem::size_of::<AVChapter>()).cast();
                    if dst.is_null() {
                        return Err("chapter allocation failed".into());
                    }
                    *(*out.context).chapters.add(index) = dst;
                    (*out.context).nb_chapters += 1;
                    (*dst).id = (*src).id;
                    (*dst).time_base = (*src).time_base;
                    (*dst).start = (*src).start;
                    (*dst).end = (*src).end;
                    check(
                        av_dict_copy(&mut (*dst).metadata, (*src).metadata, 0),
                        "copy chapter metadata",
                    )?;
                }
            }
            for &index in selected {
                let src = &*source.streams()[index];
                let dst = avformat_new_stream(out.context, ptr::null());
                if dst.is_null() {
                    return Err("output stream allocation failed".into());
                }
                let override_video = video.filter(|(i, _, _)| *i == index);
                let parameters = override_video.map_or(src.codecpar.cast_const(), |(_, p, _)| p);
                check(
                    avcodec_parameters_copy((*dst).codecpar, parameters),
                    "copy codec parameters",
                )?;
                (*(*dst).codecpar).codec_tag = 0;
                (*dst).time_base = override_video.map_or(src.time_base, |(_, _, tb)| tb);
                (*dst).avg_frame_rate = src.avg_frame_rate;
                (*dst).sample_aspect_ratio = src.sample_aspect_ratio;
                (*dst).disposition = src.disposition;
                check(
                    av_dict_copy(&mut (*dst).metadata, src.metadata, 0),
                    "copy stream metadata",
                )?;
            }
            check(
                avio_open(
                    &mut (*out.context).pb,
                    temp.as_ptr(),
                    AVIO_FLAG_WRITE as i32,
                ),
                "open staged output",
            )?;
            check(
                avformat_write_header(out.context, ptr::null_mut()),
                "write container header",
            )?;
        }
        Ok(out)
    }
    fn write(&mut self, packet: &mut Packet, index: usize, time_base: AVRational) -> Result<()> {
        // SAFETY: Stream index is from the checked selection map. Muxing consumes
        // the packet reference, not a Rust-owned payload copy; Packet remains valid/empty.
        unsafe {
            let stream = *(*self.context).streams.add(index);
            if self.strict_timing {
                let target = (*stream).time_base;
                if time_base.num <= 0 || time_base.den <= 0 || target.num <= 0 || target.den <= 0 {
                    return Err("invalid mux time base".into());
                }
                let denominator = i128::from(time_base.den) * i128::from(target.num);
                for value in [(*packet.0).pts, (*packet.0).dts, (*packet.0).duration] {
                    let numerator =
                        i128::from(value) * i128::from(time_base.num) * i128::from(target.den);
                    if numerator % denominator != 0 {
                        return Err("output container cannot represent exact packet timing".into());
                    }
                    i64::try_from(numerator / denominator)
                        .map_err(|_| "rescaled timestamp overflow")?;
                }
            }
            (*packet.0).stream_index = index as i32;
            av_packet_rescale_ts(packet.0, time_base, (*stream).time_base);
            (*packet.0).pos = -1;
            check(
                av_interleaved_write_frame(self.context, packet.0),
                "mux packet",
            )
        }
    }
    fn finish(self) -> Result<()> {
        // SAFETY: The live output is uniquely owned; close before atomic publication.
        unsafe {
            check(av_write_trailer(self.context), "write trailer")?;
            check(avio_closep(&mut (*self.context).pb), "close output")?;
        }
        std::fs::hard_link(&self.temporary, &self.destination)
            .map_err(|e| format!("publish output: {e}"))?;
        // Drop removes only the temporary hard link and frees the context.
        Ok(())
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        // SAFETY: Output uniquely owns its format context and any remaining AVIO.
        unsafe {
            if !self.context.is_null() {
                if !(*self.context).pb.is_null() {
                    avio_closep(&mut (*self.context).pb);
                }
                avformat_free_context(self.context);
            }
        }
        let _ = std::fs::remove_file(&self.temporary);
    }
}
fn selection(input: &Input, options: &CopyOptions) -> Result<Vec<usize>> {
    let selected = if options.streams.is_empty() {
        (0..input.streams().len()).collect()
    } else {
        options.streams.clone()
    };
    if selected.is_empty() {
        return Err("input has no streams".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for &index in &selected {
        if index >= input.streams().len() || !seen.insert(index) {
            return Err("stream index missing or duplicated".into());
        }
    }
    Ok(selected)
}
fn packet_info(packet: &Packet, input: &Input, options: &CopyOptions) -> Result<(usize, usize)> {
    // SAFETY: Live packet created by av_read_frame; numeric fields checked before use.
    let (index, size, flags) = unsafe {
        (
            (*packet.0).stream_index,
            (*packet.0).size,
            (*packet.0).flags,
        )
    };
    if index < 0 || index as usize >= input.streams().len() {
        return Err("packet has invalid stream index".into());
    }
    if size < 0 || size as usize > options.max_packet_bytes {
        return Err("packet exceeds payload budget".into());
    }
    if flags & AV_PKT_FLAG_CORRUPT as i32 != 0 {
        return Err("corrupt packet rejected".into());
    }
    Ok((index as usize, size as usize))
}
pub fn remux(source: &Path, destination: &Path, options: &CopyOptions) -> Result<CopyStats> {
    let mut input = Input::open(source)?;
    let selected = selection(&input, options)?;
    let mut output = Output::new(destination, &input, &selected)?;
    let mut packet = Packet::new()?;
    let mut stats = CopyStats {
        backend: "libavformat (native)",
        segments: 1,
        ..Default::default()
    };
    while packet.read(&mut input)? {
        let (index, size) = packet_info(&packet, &input, options)?;
        if let Some(mapped) = selected.iter().position(|&i| i == index) {
            // SAFETY: Packet index and selected stream are checked and input is live.
            let tb = unsafe { (*input.streams()[index]).time_base };
            output.write(&mut packet, mapped, tb)?;
            stats.packets += 1;
            stats.payload_bytes += size as u64;
        }
    }
    output.finish()?;
    Ok(stats)
}
