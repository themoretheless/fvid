//! Media operations implemented by FVid's own container and codec pipeline.
//! This module is available without the legacy `media` feature or FFmpeg.
use crate::playback_native::{NativeReader, RawFrame};
use crate::{Result, invalid};
use std::{fs::File, io::BufReader, path::Path, time::Duration};

pub use fvid_media_info::{DecodeStats, DecodeTransform};

/// Decode and discard video frames without converting them to RGB.
/// Errors are propagated, never retried through a foreign decoder. Platform
/// decoder features do not override the owned software codec selection.
pub fn decode_video(source: &Path) -> Result<DecodeStats> {
    decode_video_interval(source, None)
}

/// Decode frames whose presentation start belongs to the half-open interval.
/// Reference frames before the interval are decoded but not counted. Timing
/// comparisons retain source-clock precision; no rounded frame rate is used.
pub fn decode_video_interval(
    source: &Path,
    interval: Option<(Duration, Duration)>,
) -> Result<DecodeStats> {
    decode_video_transformed(source, interval, &crate::native_geometry::VideoGeometry::default())
}

/// Owned decoding followed by sample-preserving crop, flips and nearest resize.
pub fn decode_video_transformed(
    source: &Path,
    interval: Option<(Duration, Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
) -> Result<DecodeStats> {
    decode_video_filtered(source, interval, geometry, None)
}

/// Owned geometry followed by optional sample inversion.
pub fn decode_video_filtered(
    source: &Path,
    interval: Option<(Duration, Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
    negate: Option<crate::native_pixels::Negate>,
) -> Result<DecodeStats> {
    let filters = crate::native_pixels::PixelFilters { negate, ..Default::default() };
    decode_video_pipeline(source, interval, geometry, &filters)
}

/// Owned geometry followed by the ordered spatial pixel filters.
pub fn decode_video_pipeline(
    source: &Path,
    interval: Option<(Duration, Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
) -> Result<DecodeStats> {
    decode_video_pipeline_overlay(source, interval, geometry, filters, None)
}
/// Decode geometry, timed foreground compositing, then ordered pixel filters.
/// The interval selects main presentation starts without resetting either file origin.
pub fn decode_video_pipeline_overlay(
    source: &Path,
    interval: Option<(Duration, Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    overlay: Option<&crate::media_info::OverlaySpec>,
) -> Result<DecodeStats> {
    decode_video_pipeline_overlay_step(source, interval, geometry, filters, overlay,
        fvid_media::owned_framestep::FrameStep::parse("").map_err(|e| invalid(&e))?)
}

/// Decode every reference and filter input, then select one output per step.
/// The step counter begins after presentation interval selection.
pub fn decode_video_pipeline_overlay_step(
    source: &Path, interval: Option<(Duration, Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    overlay: Option<&crate::media_info::OverlaySpec>,
    step: fvid_media::owned_framestep::FrameStep,
) -> Result<DecodeStats> {
    let mut compositor = overlay.map(|spec| crate::native_export::TimedOverlay::new(&spec.path, i64::from(spec.x), i64::from(spec.y))).transpose()?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("decode interval requires from < to"));
    }
    let mut reader = NativeReader::software(BufReader::new(File::open(source)?), usize::MAX)?;
    let mut stats = DecodeStats {
        backend: if reader.hardware_accelerated() {
            "videotoolbox"
        } else {
            "fvid"
        },
        video_frames: 0,
        width: 0,
        height: 0,
        pixel_format: String::new(),
        decode_errors: 0,
    };
    let mut input_frames = 0u64;
    while let Some(frame) = reader.read_frame_raw()? {
        let overlay_pts = if let Some(compositor) = compositor.as_mut() {
            let (start, _, scale) = reader.frame_interval().ok_or_else(|| invalid("overlay frame has no timing"))?;
            if scale == 0 {return Err(invalid("overlay clock is zero"));}
            let pts = u64::try_from(start.checked_mul(1_000_000_000).ok_or_else(|| invalid("overlay timestamp overflow"))? / u128::from(scale)).map_err(|_| invalid("overlay timestamp overflow"))?;
            compositor.validate_main(&reader, pts)?;
            Some(pts)
        } else { None };
        if let Some((from, to)) = interval {
            let (start, _, scale) = reader
                .frame_interval()
                .ok_or_else(|| invalid("missing frame timestamp"))?;
            let stamp = start
                .checked_mul(1_000_000_000)
                .ok_or_else(|| invalid("frame timestamp overflow"))?;
            let from = from
                .as_nanos()
                .checked_mul(u128::from(scale))
                .ok_or_else(|| invalid("interval overflow"))?;
            let to = to
                .as_nanos()
                .checked_mul(u128::from(scale))
                .ok_or_else(|| invalid("interval overflow"))?;
            if stamp >= to {
                break;
            }
            if stamp < from {
                continue;
            }
        }
        let [width, height] = reader.dimensions();
        let mut output_width = u32::try_from(width).map_err(|_| invalid("video width overflow"))?;
        let mut output_height = u32::try_from(height).map_err(|_| invalid("video height overflow"))?;
        let mut pixel_format = match &frame {
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            RawFrame::Surface { surface, .. } => match surface.depth() {
                8 => "nv12".into(), _ => "p010le".into(),
            },
            RawFrame::Rgb(_) => "rgb24".into(),
            RawFrame::Planar(p) => p.pixel_format()?,
            RawFrame::Avc { picture, .. } => match picture.bit_depth {
                8 => "yuv420p".into(),
                depth => format!("yuv420p{depth}le"),
            },
            RawFrame::Planar8(p) => planar_format(p.width, p.height, p.chroma_width, p.chroma_height)?,
            RawFrame::Yuv { sx, sy, .. } => planar_format(width, height, width / sx, height / sy)?,
        };
        if !geometry.is_identity() || !filters.is_empty() || compositor.is_some() {
            let mut output = geometry.apply_cropped_display(&frame, width, height, reader.rotation(), reader.insets())?;
            output_width = u32::try_from(output.width).map_err(|_| invalid("video width overflow"))?;
            output_height = u32::try_from(output.height).map_err(|_| invalid("video height overflow"))?;
            if geometry.transpose.is_some() {
                pixel_format = match pixel_format.as_str() {
                    "yuv422p" => "yuv440p".into(),
                    "yuv440p" => "yuv422p".into(),
                    _ => pixel_format,
                };
            }
            let depth = match &frame {
                RawFrame::Avc { picture, .. } => picture.bit_depth,
                RawFrame::Planar(p) => p.depth,
                _ => 8,
            };
            if let Some(compositor) = compositor.as_mut() { compositor.apply(&mut output, depth, overlay_pts.unwrap(), None)?; }
            filters.configure_vignette_source(&reader,geometry)?;
            filters.apply_colour_clock(&mut output, depth, reader.colour().full_range, reader.colour().matrix, input_frames, reader.frame_interval().map(|(start,_,scale)|start as f64/scale as f64), crate::native_pixels::frame_clock(&reader)?)?;
            std::hint::black_box(output);
        }
        let emit = step.emits(input_frames);
        input_frames = input_frames.checked_add(1).ok_or_else(|| invalid("input frame count overflow"))?;
        if !emit { continue; }
        stats.width = output_width;
        stats.height = output_height;
        stats.pixel_format = pixel_format;
        stats.video_frames = stats
            .video_frames
            .checked_add(1)
            .ok_or_else(|| invalid("frame count overflow"))?;
    }
    if stats.video_frames == 0 {
        return Err(invalid("input has no decoded video frames"));
    }
    Ok(stats)
}
fn planar_format(w: usize, h: usize, cw: usize, ch: usize) -> Result<String> {
    Ok(if cw == w && ch == h {
        "yuv444p"
    } else if cw == w && ch == h.div_ceil(2) {
        "yuv440p"
    } else if cw == w.div_ceil(2) && ch == h {
        "yuv422p"
    } else if cw == w.div_ceil(2) && ch == h.div_ceil(2) {
        "yuv420p"
    } else {
        return Err(invalid("unrecognized decoded plane geometry"));
    }
    .into())
}

/// Shared block/packet-work reporting for native audio container paths.
pub(crate) struct DecodeProgress<'a> {
    cancel: Option<&'a crate::media_control::CancelFlag>,
    hook: Option<&'a crate::media_control::ProgressHook>,
    max_rss_bytes: Option<u64>,
    max_packets: Option<u64>,
    event: crate::media_control::ProgressEvent,
}
impl<'a> DecodeProgress<'a> {
    pub(crate) fn new(cancel: Option<&'a crate::media_control::CancelFlag>, hook: Option<&'a crate::media_control::ProgressHook>) -> Result<Self> {
        Self::new_with_rss_limit(cancel, hook, None)
    }
    pub(crate) fn new_with_rss_limit(cancel: Option<&'a crate::media_control::CancelFlag>, hook: Option<&'a crate::media_control::ProgressHook>, max_rss_bytes: Option<u64>) -> Result<Self> {
        Self::new_with_limits(cancel, hook, max_rss_bytes, None)
    }
    pub(crate) fn new_with_limits(cancel: Option<&'a crate::media_control::CancelFlag>, hook: Option<&'a crate::media_control::ProgressHook>, max_rss_bytes: Option<u64>, max_packets: Option<u64>) -> Result<Self> {
        let state = Self { cancel, hook, max_rss_bytes, max_packets, event: crate::media_control::ProgressEvent { packets: 0, payload_bytes: 0, done: false } };
        state.check()?;
        state.emit(false);
        state.check()?;
        Ok(state)
    }
    // This core streaming API exposes RSS/packet controls, not a controlled
    // allocation limit. The media-file adapter supplies admission separately.
    pub(crate) fn check_admission(&self, _channels: u16) -> Result<()> {
        Ok(())
    }
    pub(crate) fn check(&self) -> Result<()> {
        if self.cancel.is_some_and(|flag| flag.is_cancelled()) { return Err(invalid("media operation cancelled")); }
        if self.max_rss_bytes.is_some() {
            let options = fvid_media::CopyOptions { max_rss_bytes: self.max_rss_bytes, ..Default::default() };
            fvid_media::owned_budget::check_rss_budget(&options).map_err(|e| invalid(&e))?;
        }
        Ok(())
    }
    pub(crate) fn packet_limit_reached(&self) -> bool {
        self.max_packets.is_some_and(|maximum| self.event.packets >= maximum)
    }
    pub(crate) fn emit(&self, done: bool) {
        if let Some(hook) = self.hook { hook.emit(crate::media_control::ProgressEvent { done, ..self.event }); }
    }
    pub(crate) fn packet(&mut self, bytes: usize) -> Result<()> {
        self.event.packets = self.event.packets.checked_add(1).ok_or_else(|| invalid("audio packet count overflow"))?;
        self.event.payload_bytes = self.event.payload_bytes.checked_add(bytes as u64).ok_or_else(|| invalid("audio byte count overflow"))?;
        if self.event.packets % 256 == 0 { self.emit(false); }
        self.check()
    }
}

/// Result of owned audio decoding to interleaved little-endian float PCM.
#[derive(Debug, PartialEq, Eq)]
pub struct AudioDecodeStats {
    pub sample_frames: u64,
    pub decoded_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Decode ADTS packets using the owned AAC codec, without playback dependencies.
/// Encoder priming/padding is retained because ADTS carries no trim metadata.
/// The caller owns the destination; an error can leave partial PCM in it.
pub fn decode_aac_pcm(
    data: &[u8],
    output: &mut impl std::io::Write,
    limits: &crate::container::adts::Limits,
) -> Result<AudioDecodeStats> {
    decode_aac_pcm_interval(data, output, limits, None)
}

/// Select sample starts in [from, to), rounding each boundary up to a sample.
/// Decode preceding packets to preserve overlap, noise and prediction state.
pub fn decode_aac_pcm_interval(
    data: &[u8],
    output: &mut impl std::io::Write,
    limits: &crate::container::adts::Limits,
    interval: Option<(Duration, Duration)>,
) -> Result<AudioDecodeStats> {
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    let stream = crate::container::adts::Aac::parse(data, limits)?;
    let boundary = |time: Duration| -> Result<u64> {
        let ticks = time
            .as_nanos()
            .checked_mul(u128::from(stream.sample_rate))
            .ok_or_else(|| invalid("audio interval overflow"))?;
        u64::try_from(ticks.div_ceil(1_000_000_000)).map_err(|_| invalid("audio interval overflow"))
    };
    let (from, to) = match interval {
        Some((from, to)) => (boundary(from)?, boundary(to)?),
        None => (0, u64::MAX),
    };
    let mut decoder = crate::codec::aac_native::NativeAacDecoder::new(&stream.configuration)?;
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: stream.sample_rate,
        channels: stream.channels,
    };
    let mut position = 0u64;
    for index in 0..stream.packets() {
        if position >= to {
            break;
        }
        let samples = decoder.decode(stream.packet(index))?;
        let channels = usize::from(stream.channels);
        let frames = (samples.len() / channels) as u64;
        let end = position
            .checked_add(frames)
            .ok_or_else(|| invalid("audio position overflow"))?;
        let first = from.saturating_sub(position).min(frames) as usize;
        let last = to.saturating_sub(position).min(frames) as usize;
        let selected = &samples[first * channels..last.max(first) * channels];
        for sample in selected {
            output.write_all(&sample.to_le_bytes())?;
        }
        stats.sample_frames += (selected.len() / channels) as u64;
        stats.decoded_frames += 1;
        position = end;
    }
    if stats.sample_frames == 0 {
        return Err(invalid("audio interval contains no samples"));
    }
    Ok(stats)
}

/// Decode a sequential ADTS source, retaining decoder pre-roll but no file index.
/// Stops reading once the requested interval ends. A full export validates every
/// frame boundary and rejects truncated tails; ADTS has no priming metadata.
pub fn decode_adts_aac_reader<R: std::io::Read>(
    reader: crate::container::adts::StreamReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
) -> Result<AudioDecodeStats> {
    let mut control = DecodeProgress::new(None, None)?;
    decode_adts_aac_reader_controlled(reader, output, interval, &mut control)
}

use crate::container::adts::StreamReader as AdtsStreamReader;
use crate::codec::aac_native::NativeAacDecoder as AdtsPacketDecoder;
include!("../crates/fvid-media/src/owned_aac/stream_impl.rs");

/// Decode a single AAC MP4 track with its priming/tail edit applied.
/// Accepts sample-aligned track clocks, repeated media edits and empty edits.
/// Replays decoder pre-roll for each selected media segment without retaining PCM.
pub fn decode_mp4_aac_pcm(
    data: &[u8],
    output: &mut impl std::io::Write,
) -> Result<AudioDecodeStats> {
    decode_mp4_aac_pcm_interval(data, output, None)
}

/// Select an interval relative to the edited presentation timeline.
pub fn decode_mp4_aac_pcm_interval(
    data: &[u8],
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
) -> Result<AudioDecodeStats> {
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    let reader = crate::container::mp4::Mp4Reader::open(
        std::io::Cursor::new(data), Default::default(),
    )?;
    decode_mp4_aac_reader(reader, output, interval)
}

/// Decode indexed AAC packets without loading the MP4 media payload into memory.
/// The reader retains container tables; only the current encoded packet and decoder
/// state are needed for media. Edits and pre-roll use the same sample timeline as
/// the byte-slice API. The caller must discard partial output if decoding fails.
pub fn decode_mp4_aac_reader<R: std::io::Read + std::io::Seek>(
    reader: crate::container::mp4::Mp4Reader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
) -> Result<AudioDecodeStats> {
    let mut control = DecodeProgress::new(None, None)?;
    decode_mp4_aac_reader_controlled(reader, output, interval, None, &mut control)
}

pub(crate) fn decode_mp4_aac_reader_controlled<R: std::io::Read + std::io::Seek>(
    reader: crate::container::mp4::Mp4Reader<R>, output:&mut impl std::io::Write,
    interval:Option<(Duration,Duration)>, selected:Option<usize>, control:&mut DecodeProgress<'_>,
) -> Result<AudioDecodeStats> {
    mp4_aac_index(&reader,selected)?;
    decode_mp4_audio_reader_controlled(reader,output,interval,selected,control)
}

use crate::container::mp4::Mp4Reader as Mp4TimelineReader;
use crate::native_audio_decoder::PacketPcmDecoder as Mp4TimelineDecoder;
use crate::container::audio_timeline::AudioTimeline as Mp4AudioSchedule;
use crate::codec::aac_native::AacCheckpoint as Mp4AacCheckpoint;
include!("../crates/fvid-media/src/owned_mp4_audio_timeline_impl.rs");

/// Decode a contiguous Matroska AAC stream, discarding codec delay and per-block
/// padding. Sub-tick timestamp quantization is tolerated without drifting PCM.
/// Intervals address the trimmed sample sequence starting at its first sample.
pub fn decode_matroska_aac_pcm_interval(
    data: &[u8],
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
) -> Result<AudioDecodeStats> {
    let reader = crate::container::webm::WebmReader::open(
        std::io::Cursor::new(data), Default::default(),
    )?;
    decode_matroska_aac_reader(reader, output, interval)
}

/// Decode indexed AAC from a seekable Matroska source without retaining the file.
/// Container delay, signed discard padding and interval selection are preserved.
/// The container index is retained; encoded payloads are read one packet at a time.
pub fn decode_matroska_aac_reader<R: std::io::Read + std::io::Seek>(
    reader: crate::container::webm::WebmReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
) -> Result<AudioDecodeStats> {
    let mut control = DecodeProgress::new(None, None)?;
    decode_matroska_aac_reader_controlled(reader, output, interval, None, &mut control)
}

pub(crate) fn decode_matroska_aac_reader_controlled<R:std::io::Read+std::io::Seek>(
    reader:crate::container::webm::WebmReader<R>,output:&mut impl std::io::Write,
    interval:Option<(Duration,Duration)>,selected:Option<usize>,control:&mut DecodeProgress<'_>,
)->Result<AudioDecodeStats> {
    matroska_aac_index(&reader,selected)?;
    decode_matroska_audio_reader_controlled(reader,output,interval,selected,control)
}

use crate::container::webm::WebmReader as MatroskaTimelineReader;
use crate::native_audio_decoder::PacketPcmDecoder as MatroskaTimelineDecoder;
include!("../crates/fvid-media/src/owned_matroska_audio_timeline_impl.rs");

/// Identify AAC in supported containers by their headers and track configuration.
/// This only detects the codec; decoding still validates the complete bitstream.
pub fn is_aac_source(path: &std::path::Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut prefix = [0; 8];
    match file.read_exact(&mut prefix) {
        Ok(()) if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) => {
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start(0))?;
            let reader = crate::container::webm::WebmReader::open(file, Default::default())
                .map_err(std::io::Error::other)?;
            Ok(reader.tracks.iter().any(|track| track.kind == 2 && track.codec == "A_AAC"))
        }
        Ok(()) if crate::container::mp4::recognizes_prefix(&prefix) => {
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start(0))?;
            let reader = crate::container::mp4::Mp4Reader::open(file, Default::default())
                .map_err(std::io::Error::other)?;
            Ok(reader.tracks().iter().any(|track| track.handler == *b"soun" && track.codec == *b"mp4a"))
        }
        Ok(()) => Ok(crate::container::adts::header(&prefix).is_some()),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error),
    }
}

/// AAC stream geometry from owned container/configuration parsers. This does not
/// decode packets and therefore does not certify the rest of the bitstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AacSourceInfo {
    /// Canonical WAVE speaker bits from AAC configuration.
    pub channel_mask: u32,
    /// Zero-based container stream order, not MP4 track ID / Matroska track number.
    pub stream_index: usize,
    pub sample_rate: u32,
    pub channels: u16,
}

pub fn aac_source_info(source: &Path) -> Result<AacSourceInfo> {
    aac_source_info_selected(source, None)
}

/// Select a zero-based container stream index; None requires one audio track.
pub fn aac_source_info_selected(source: &Path, selected: Option<usize>) -> Result<AacSourceInfo> {
    use std::io::{BufReader, Read, Seek, SeekFrom};
    let mut input = BufReader::new(std::fs::File::open(source)?);
    let mut prefix = [0; 8];
    input.read_exact(&mut prefix)?;
    input.seek(SeekFrom::Start(0))?;
    let (stream_index, asc, declared) = if crate::container::mp4::recognizes_prefix(&prefix) {
        let reader = crate::container::mp4::Mp4Reader::open(input, Default::default())?;
        let index = mp4_aac_index(&reader, selected)?;
        let track = &reader.tracks()[index];
        (index, crate::codec::config::aac_specific_config(&track.configuration)?.to_vec(),
            (u64::from(track.sample_rate), u64::from(track.channels)))
    } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let reader = crate::container::webm::WebmReader::open(input, Default::default())?;
        let index = matroska_aac_index(&reader, selected)?;
        let track = &reader.tracks[index];
        (index, track.codec_private.clone(), (track.sample_rate, track.channels))
    } else {
        if selected.is_some_and(|index| index != 0) { return Err(invalid("ADTS has only stream 0")); }
        let reader = crate::container::adts::StreamReader::open(input)?;
        let header = reader.configuration();
        (0, reader.audio_specific_config().to_vec(), (u64::from(header.sample_rate), u64::from(header.channels)))
    };
    let decoder = crate::codec::aac_native::NativeAacDecoder::new(&asc)?;
    let sample_rate = decoder.sample_rate();
    let channels = u16::from(decoder.channels());
    if declared != (u64::from(sample_rate), u64::from(channels)) {
        return Err(crate::invalid("AAC container geometry disagrees with AudioSpecificConfig"));
    }
    Ok(AacSourceInfo { stream_index, sample_rate, channels, channel_mask: decoder.channel_mask() })
}

/// Exact ADTS packet/sample counts from a sequential owned parser. Encoder
/// priming is retained because ADTS provides no trimming metadata. Raw AAC
/// packet contents are not decoded by this inspection operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdtsInfo {
    pub packets: u64,
    pub payload_bytes: u64,
    pub sample_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
}
pub fn inspect_adts<R: std::io::Read>(source: R) -> Result<AdtsInfo> {
    let mut reader = crate::container::adts::StreamReader::open(source)?;
    let config = crate::codec::config::AacConfig::parse(reader.audio_specific_config())?;
    let mut info = AdtsInfo { packets: 0, payload_bytes: 0, sample_frames: 0,
        sample_rate: config.sample_rate, channels: u16::from(config.channels) };
    while let Some(packet) = reader.next_packet()? {
        info.packets = info.packets.checked_add(1).ok_or_else(|| crate::invalid("AAC packet count overflow"))?;
        info.payload_bytes = info.payload_bytes.checked_add(packet.len() as u64).ok_or_else(|| crate::invalid("AAC payload size overflow"))?;
        info.sample_frames = info.sample_frames.checked_add(u64::from(config.frame_samples)).ok_or_else(|| crate::invalid("AAC duration overflow"))?;
    }
    Ok(info)
}

fn audio_index(kinds: impl Iterator<Item=bool>, selected: Option<usize>) -> Result<usize> {
    let indices: Vec<_> = kinds.enumerate().filter_map(|(index,audio)| audio.then_some(index)).collect();
    if let Some(index) = selected {
        if indices.contains(&index) { return Ok(index); }
        return Err(invalid("selected stream is absent or is not audio"));
    }
    if indices.len() != 1 { return Err(invalid("native audio export requires exactly one audio track or an explicit stream index")); }
    Ok(indices[0])
}
pub(crate) fn mp4_aac_index<R: std::io::Read + std::io::Seek>(reader: &crate::container::mp4::Mp4Reader<R>, selected: Option<usize>) -> Result<usize> {
    if selected.is_some() && !reader.refused().is_empty() { return Err(invalid("cannot select an MP4 stream while some sample entries are unindexed")); }
    let index = audio_index(reader.tracks().iter().map(|t| t.handler == *b"soun"),selected)?;
    if reader.tracks()[index].codec != *b"mp4a" { return Err(invalid("selected MP4 audio stream is not AAC")); }
    Ok(index)
}
pub(crate) fn matroska_aac_index<R: std::io::Read + std::io::Seek>(reader: &crate::container::webm::WebmReader<R>, selected: Option<usize>) -> Result<usize> {
    let index = audio_index(reader.tracks.iter().map(|t| t.kind == 2),selected)?;
    if reader.tracks[index].codec != "A_AAC" { return Err(invalid("selected Matroska audio stream is not AAC")); }
    Ok(index)
}

/// Whether all requested operations have an owned implementation.
/// Exhaustive matching forces new request fields to receive an explicit policy.
/// Admit plane filters only when the native reader actually supplies YUV.
/// The first frame is inspected here; execution still validates the whole stream.
/// Unsupported native formats retain the adapter before execution begins.
pub(crate) fn supports_plane_filter_source(source: &Path) -> Result<bool> {
    if crate::native_lossless::eligible(source)? { return Ok(true); }
    let input = BufReader::new(File::open(source)?);
    let Ok(mut reader) = NativeReader::software(input, usize::MAX) else { return Ok(false); };
    Ok(matches!(reader.read_frame_raw(), Ok(Some(frame)) if !matches!(frame, RawFrame::Rgb(_))))
}

pub(crate) fn supports_video_request(transform: &DecodeTransform) -> bool {
    if transform.fade.as_deref().is_some_and(|a|fvid_media::owned_fade::Fade::parse(a).is_err()) {return false;}
    if transform.framestep.as_deref().is_some_and(|args| fvid_media::owned_framestep::FrameStep::parse(args).is_err()) { return false; }
    if transform.grayworld.as_deref().is_some_and(|a| fvid_media::owned_timeline::Timeline::grayworld(a).is_err()) { return false; }
    if transform.monochrome.as_deref().is_some_and(|a| fvid_media::owned_monochrome::Monochrome::parse(a).is_err()) { return false; }
    if transform.colorize.as_deref().is_some_and(|a| fvid_media::owned_colorize::Colorize::parse(a).is_err()) { return false; }
    if transform.curves.as_deref().is_some_and(|a| fvid_media::owned_curves::Curves::parse(a).is_err()) {return false;}
    if transform.vignette.as_deref().is_some_and(|a| fvid_media::owned_vignette::Vignette::parse(a).is_err()) {return false;}
    if transform.smartblur.as_deref().is_some_and(|a| fvid_media::owned_smartblur::SmartBlur::parse(a).is_err()) {return false;}
    if transform.sab.as_deref().is_some_and(|a| fvid_media::owned_sab::Sab::parse(a).is_err()) {return false;}
    if transform.bitplanenoise.as_deref().is_some_and(|a| fvid_media::owned_bitplanenoise::BitPlaneNoise::parse(a).is_err()) {return false;}
    if transform.gradfun.as_deref().is_some_and(|a| fvid_media::owned_gradfun::GradFun::parse(a).is_err()) {return false;}
    if transform.lenscorrection.as_deref().is_some_and(|a| fvid_media::owned_lenscorrection::LensCorrection::parse(a).is_err()) {return false;}
    if transform.hqdn3d.as_deref().is_some_and(|a| fvid_media::owned_hqdn3d::HqDn3d::parse(a).is_err()) {return false;}
    if transform.boxblur.as_deref().is_some_and(|args| crate::native_boxblur::BoxBlurProgram::parse(args).is_err()) { return false; }
    matches!(transform, DecodeTransform {
        crop: _,
        vertical_flip: _,
        horizontal_flip: _,
        scale: _,
        epx: None,
        transpose: _,
        rotate: _,
        pad: _,
        burn_subs: None,
        overlay: _,
        yadif: None,
        bwdif: None,
        w3fdif: None,
        tblend: None,
        tmix: _,
        hqdn3d: _,
        gblur: _,
        eq: _,
        unsharp: _,
        hue: _,
        avgblur: _,
        boxblur: _,
        negate: _,
        edgedetect: None,
        sobel: _,
        prewitt: _,
        roberts: _,
        kirsch: _,
        scharr: _,
        atadenoise: None,
        owdenoise: None,
        vaguedenoiser: None,
        nlmeans: None,
        bm3d: None,
        dctdnoiz: None,
        fftdnoiz: None,
        smartblur: _,
        sab: _,
        bitplanenoise: _,
        gradfun: _,
        lenscorrection: _,
        bilateral: _,
        cas: _,
        vignette: _,
        curves: _,
        colorbalance: _,
        colorlevels: _,
        colorchannelmixer: _,
        deflicker: None,
        photosensitivity: None,
        monochrome: _,
        grayworld: _,
        drawbox: None,
        drawgrid: None,
        lagfun: _,
        amplify: None,
        deband: None,
        pixelize: _,
        removegrain: None,
        yaepblur: None,
        vibrance: _,
        dilation: _,
        erosion: _,
        colorize: _,
        exposure: _,
        chromashift: _,
        colorcontrast: _,
        colorcorrect: _,
        histeq: None,
        shuffleplanes: _,
        lutyuv: _,
        colorhold: _,
        fade: _,
        perspective: None,
        lumakey: None,
        chromakey: None,
        colorkey: None,
        despill: None,
        selectivecolor: None,
        stereo3d: None,
        field: None,
        hqx: None,
        xbr: None,
        il: None,
        super2xsai: None,
        kerndeint: None,
        phase: None,
        estdif: None,
        tinterlace: None,
        separatefields: None,
        weave: None,
        doubleweave: None,
        framepack: None,
        telecine: None,
        pullup: None,
        decimate: None,
        mpdecimate: None,
        framestep: _,
        tile: None,
        untile: None,
        shuffleframes: None,
        reverse: None,
        r#loop: None,
        thumbnail: None,
        freezedetect: None,
        pseudocolor: None,
        minterpolate: None,
        fps: None,
        colorspace: None,
        zscale: None,
        tonemap: None,
        pix_fmt: None,
        interval: _,
        input_format: None,
    }) && crate::native_pixels::PixelFilters::from_request(transform).is_ok()
}

/// Execute a shared video request without enabling `media` or any external codec.
/// Unsupported filters are errors, never ignored or routed to another backend.
pub fn decode_video_request(source: &Path, transform: &DecodeTransform) -> Result<DecodeStats> {
    if !supports_video_request(transform) {return Err(invalid("video request contains a filter not yet supported by the owned decoder"));}
    let interval = transform.interval.map(|(from, to)| {
        if from < 0 || to <= from { return Err(invalid("decode interval requires 0 <= from < to")); }
        Ok((std::time::Duration::from_micros(from as u64), std::time::Duration::from_micros(to as u64)))
    }).transpose()?;
    let geometry = crate::native_geometry::VideoGeometry {
        rotate: transform.rotate,
        crop: transform.crop.map(|r| [r.x, r.y, r.width, r.height]),
        horizontal_flip: transform.horizontal_flip,
        vertical_flip: transform.vertical_flip,
        scale: transform.scale.map(|r| [r.width as usize, r.height as usize]),
        transpose: transform.transpose.map(|r| crate::native_geometry::Transpose::parse(r.as_str())).transpose()?,
        pad: transform.pad.map(|r| [r.width as usize, r.height as usize, r.x as usize, r.y as usize]),
    };
    let filters = crate::native_pixels::PixelFilters::from_request(transform)?;
    let step = fvid_media::owned_framestep::FrameStep::parse(transform.framestep.as_deref().unwrap_or(""))
        .map_err(|e| invalid(&e))?;
    decode_video_pipeline_overlay_step(source, interval, &geometry, &filters, transform.overlay.as_ref(), step)
}

/// Detect ALAC in MP4 or Matroska by container contents, independent of filename suffix.
pub fn is_alac_source(path: &Path) -> Result<bool> {
    use std::io::{Read, Seek, SeekFrom};
    let mut input = File::open(path)?;
    let mut prefix = [0; 8];
    match input.read_exact(&mut prefix) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
        Err(e) => return Err(e.into()),
    }
    if prefix.starts_with(&[0x1a,0x45,0xdf,0xa3]) {
        input.seek(SeekFrom::Start(0))?;
        let reader=crate::container::webm::WebmReader::open(BufReader::new(input),Default::default())?;
        return Ok(reader.tracks.iter().any(|t|t.kind==2 && t.codec=="A_ALAC"));
    }
    if !crate::container::mp4::recognizes_prefix(&prefix) {return Ok(false);}
    input.seek(SeekFrom::Start(0))?;
    let reader = crate::container::mp4::Mp4Reader::open(BufReader::new(input), Default::default())?;
    Ok(reader
        .tracks()
        .iter()
        .any(|t| t.handler == *b"soun" && t.codec == *b"alac"))
}

fn is_mp4_audio_container(path: &Path) -> Result<bool> {
    use std::io::Read;
    let mut prefix = [0;8];
    match File::open(path)?.read_exact(&mut prefix) {
        Ok(()) => Ok(crate::container::mp4::recognizes_prefix(&prefix)),
        Err(e) if e.kind()==std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Admission for compressed sources with an owned export implementation.
/// Unsupported ALAC layouts and ambiguous default selection retain the adapter.
/// Packed WAVE is detected separately by native_pcm::is_wave.
pub fn is_owned_audio_source(path: &Path) -> Result<bool> {
    if is_aac_source(path)? { return Ok(true); }
    Ok((is_mp4_audio_container(path)? || crate::native_export::is_matroska_source(path)?) && audio_source_info_selected(path, None).is_ok())
}

/// Trim can select one supported audio track even when default decoding is
/// ambiguous. Execution still requires explicit selection if other streams exist.
pub fn is_owned_audio_trim_source(path: &Path) -> Result<bool> {
    if is_owned_audio_source(path)? { return Ok(true); }
    if !is_mp4_audio_container(path)? && !crate::native_export::is_matroska_source(path)? { return Ok(false); }
    let info = crate::native_probe::probe(path).map_err(|e| invalid(&e))?;
    Ok(info.streams.iter().any(|stream| stream.media_type == "audio"
        && audio_source_info_selected(path, Some(stream.index)).is_ok()))
}

pub(crate) fn is_aac_trim_source(path: &Path, selected: Option<usize>) -> Result<bool> {
    match selected {
        Some(index) => Ok(audio_source_info_selected(path, Some(index))?.codec == "aac"),
        None => Ok(is_aac_source(path)?),
    }
}

pub(crate) fn mp4_audio_index<R: std::io::Read + std::io::Seek>(
    reader: &crate::container::mp4::Mp4Reader<R>,
    selected: Option<usize>,
) -> Result<usize> {
    if selected.is_some() && !reader.refused().is_empty() {
        return Err(invalid(
            "cannot select an MP4 stream while some sample entries are unindexed",
        ));
    }
    let index = audio_index(
        reader.tracks().iter().map(|t| t.handler == *b"soun"),
        selected,
    )?;
    if !matches!(&reader.tracks()[index].codec, b"mp4a" | b"alac" | b"sowt" | b"twos" | b"fl32" | b"fl64" | b"in24" | b"in32" | b"raw ") {
        return Err(invalid("selected MP4 audio codec is not supported by the owned export"));
    }
    Ok(index)
}

pub struct AudioSourceInfo {
    /// None when the codec/container does not describe speaker positions.
    pub channel_mask: Option<u32>,
    pub stream_index: usize,
    pub sample_rate: u32,
    pub channels: u16,
    pub codec: &'static str,
}
pub fn audio_source_info_selected(
    source: &Path,
    selected: Option<usize>,
) -> Result<AudioSourceInfo> {
    if is_mp4_audio_container(source)? || crate::native_export::is_matroska_source(source)? {
        use std::io::{Read,Seek,SeekFrom};
        let mut input=File::open(source)?;let mut prefix=[0;4];input.read_exact(&mut prefix)?;input.seek(SeekFrom::Start(0))?;
        if prefix==[0x1a,0x45,0xdf,0xa3] {
            let reader=crate::container::webm::WebmReader::open(BufReader::new(input),Default::default())?;
            let index=matroska_audio_index(&reader,selected)?;let track=&reader.tracks[index];let decoder=crate::native_audio_decoder::PacketPcmDecoder::from_matroska(track)?;
            if track.sample_rate!=u64::from(decoder.sample_rate()) || track.channels!=u64::from(decoder.channels()) {return Err(invalid("Matroska audio geometry disagrees with configuration"));}
            return Ok(AudioSourceInfo {channel_mask:decoder.channel_mask(),stream_index:index,sample_rate:decoder.sample_rate(),channels:decoder.channels(),codec:match track.codec.as_str() {"A_ALAC"=>"alac","A_PCM/INT/LIT"|"A_PCM/INT/BIG" if track.bit_depth==8=>"pcm_u8","A_PCM/INT/LIT"=>"pcm_sle","A_PCM/INT/BIG"=>"pcm_sbe","A_PCM/FLOAT/IEEE"=>"pcm_fle",_=>"aac"}});
        }
        let reader = crate::container::mp4::Mp4Reader::open(
            BufReader::new(File::open(source)?),
            Default::default(),
        )?;
        let index = mp4_audio_index(&reader, selected)?;
        let track = &reader.tracks()[index];
        let decoder = crate::native_audio_decoder::PacketPcmDecoder::new(track)?;
        if track.sample_rate != decoder.sample_rate() || track.channels != decoder.channels() {
            return Err(invalid("MP4 container and decoder geometry disagree"));
        }
        Ok(AudioSourceInfo {
            channel_mask: decoder.channel_mask(),
            stream_index: index,
            sample_rate: decoder.sample_rate(),
            channels: decoder.channels(),
            codec: match &track.codec { b"alac"=>"alac",b"raw "=>"pcm_u8",b"sowt"=>"pcm_sle",b"twos"=>"pcm_sbe",b"in24"|b"in32"=>if track.configuration.first()==Some(&1) {"pcm_sle"} else {"pcm_sbe"},b"fl32"|b"fl64"=>"pcm_fle",_=>"aac" },
        })
    } else {
        let info = aac_source_info_selected(source, selected)?;
        Ok(AudioSourceInfo {
            channel_mask: Some(info.channel_mask),
            stream_index: info.stream_index,
            sample_rate: info.sample_rate,
            channels: info.channels,
            codec: "aac",
        })
    }
}

pub(crate) fn matroska_audio_index<R:std::io::Read+std::io::Seek>(reader:&crate::container::webm::WebmReader<R>,selected:Option<usize>)->Result<usize> {
    let index=audio_index(reader.tracks.iter().map(|t|t.kind==2),selected)?;
    if !matches!(reader.tracks[index].codec.as_str(),"A_AAC"|"A_ALAC"|"A_PCM/INT/LIT"|"A_PCM/INT/BIG"|"A_PCM/FLOAT/IEEE") {return Err(invalid("selected Matroska audio stream is not supported by the owned export"));}
    Ok(index)
}
