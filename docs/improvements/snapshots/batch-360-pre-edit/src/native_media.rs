//! Media operations implemented by FVid's own container and codec pipeline.
//! This module is available without the legacy `media` feature or FFmpeg.
use fvid_media::owned_mp4_audio_packet_frames::frames as mp4_coded_packet_frames;
use fvid_media::owned_mp4_audio_normalization::opaque_positions as mp4_opaque_positions;
use crate::playback_native::{NativeReader, RawFrame};
use crate::{Result, invalid};
use fvid_media::owned_matroska_sample_clock::quantized_sample_matches as aac_quantized_sample_matches;
use fvid_media::owned_matroska_grid_clock::grid_sample_matches as aac_quantized_grid_matches;
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
            compositor.validate_main_with_geometry(&reader, Some(&frame), geometry, pts)?;
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
        if reader.insets() != [0; 4] || !geometry.is_identity() || !filters.is_empty() || compositor.is_some() {
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
            if let Some([sx, sy]) = output.subsampling {
                let base = match [sx, sy] {
                    [1, 1] => "yuv444p",
                    [2, 1] => "yuv422p",
                    [2, 2] => "yuv420p",
                    [1, 2] => "yuv440p",
                    [4, 1] => "yuv411p",
                    [4, 4] => "yuv410p",
                    _ => return Err(invalid("unrecognized decoded plane geometry")),
                };
                pixel_format = if depth == 8 {
                    base.into()
                } else {
                    format!("{base}{depth}le")
                };
            }
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
    pub(crate) fn check_admission(&self, _asc: &[u8]) -> Result<()> {
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
    pub(crate) fn packet_limit(&self) -> Option<u64> {
        self.max_packets
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
        if self.event.packets.is_multiple_of(256) { self.emit(false); }
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
    // Preserve indexed-source admission and full structural validation, then
    // use the same negotiated output clock as the sequential ADTS API.
    crate::container::adts::Aac::parse(data, limits)?;
    decode_adts_aac_reader(crate::container::adts::StreamReader::open(data)?, output, interval)
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
use crate::codec::config::AudioSpecificConfig as AdtsAudioConfig;
use crate::codec::aac_native::NativeAacDecoder as AdtsPacketDecoder;
use crate::codec::aac_ps_native::{NativePsAacDecoder as AdtsPsDecoder, InBandPsProbe as AdtsPsProbe};
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
    let reader = crate::container::mp4::Mp4Reader::open_indexed(
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

use crate::codec::config::aac_specific_config as mp4_probe_config;
use crate::container::mp4::Mp4Reader as Mp4TimelineReader;
use crate::native_audio_decoder::PacketPcmDecoder as Mp4TimelineDecoder;
use crate::container::audio_timeline::AudioTimeline as Mp4AudioSchedule;
use crate::native_audio_decoder::NativeAudioCheckpoint as Mp4AacCheckpoint;
use fvid_media::owned_mp4_audio_normalization::coefficients as mp4_normalization_coefficients;
type Mp4NormalizationTrack = crate::container::mp4::Track;
type Mp4PcmScalar = f32;
type Mp4OutputResampler = fvid_media::owned_resample::Resampler<Vec<u8>>;
#[allow(dead_code)] // Export uses only the arithmetic subset of the shared playback clock.
mod mp4_export_clock {
    use crate::{Error, Result, invalid};
    use crate::container::mp4::Edit;
    use std::{cmp::Ordering, time::Duration};
    include!("../crates/fvid-media/src/owned_mp4_edit_clock_impl.rs");
}
include!("../crates/fvid-media/src/owned_mp4_audio_timeline_impl.rs");

/// Decode Matroska AAC on its container presentation timeline. Decode negative
/// preroll for history, trim signed padding and preserve timestamp gaps as silence.
/// Intervals address container time starting at zero, including leading offsets.
/// Coded sample timing resolves quantization before audible windows are cropped.
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
use crate::codec::aac_ps_native::InBandPsProbe;
struct MatroskaTimelineDecoder(crate::native_audio_decoder::PacketPcmDecoder);
impl MatroskaTimelineDecoder {
    const SAMPLE_BYTES: usize = 4;
    fn with_in_band_ps(track: &crate::container::webm::Track) -> Result<Self> {
        Ok(Self(crate::native_audio_decoder::PacketPcmDecoder::Ps(Box::new(
            crate::codec::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(&track.codec_private,
                u32::try_from(track.sample_rate).map_err(|_| invalid("AAC output clock overflow"))?).map_err(|e| invalid(&e.0))?
        ))))
    }

    fn from_matroska(track: &crate::container::webm::Track) -> Result<Self> {
        Ok(Self(
            crate::native_audio_decoder::PacketPcmDecoder::from_matroska(track)?,
        ))
    }
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.0.channels()
    }
    fn delayed(&self) -> bool {
        self.0.delayed()
    }
    fn decode(&mut self, data: &[u8]) -> Result<Option<Vec<f32>>> {
        self.0.decode_delayed(data)
    }
    fn finish(&mut self) -> Result<Option<Vec<f32>>> {
        self.0.finish_delayed()
    }
}
include!("../crates/fvid-media/src/owned_matroska_audio_timeline_impl.rs");

/// Identify AAC in supported containers by their headers and track configuration.
/// This only detects the codec; decoding still validates the complete bitstream.
pub fn is_aac_source(path: &std::path::Path) -> std::io::Result<bool> {
    is_aac_source_with_metadata_limit(path, None)
}

/// Bound MP4 metadata while inspecting an audio route with copy controls.
pub fn is_aac_source_with_metadata_limit(path: &std::path::Path, max_controlled_bytes: Option<usize>) -> std::io::Result<bool> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut prefix = [0; 8];
    match file.read_exact(&mut prefix) {
        Ok(()) if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) => {
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start(0))?;
            let mut limits = crate::container::webm::Limits::default();
            let memory_packets = max_controlled_bytes
                .map(|bytes| bytes / std::mem::size_of::<crate::container::webm::Packet>());
            if let Some(maximum) = memory_packets {
                limits.packets = limits.packets.min(maximum);
            }
            let map_packet_limit = |error: crate::Error| {
                if memory_packets
                    .is_some_and(|max| max < crate::container::webm::Limits::default().packets)
                    && error
                        .to_string()
                        .contains("WebM packet count exceeds limit")
                {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "controlled memory budget exceeded: Matroska packet index limit",
                    )
                } else {
                    std::io::Error::other(error)
                }
            };
            let mut reader =
                crate::container::webm::WebmReader::open(file, limits).map_err(map_packet_limit)?;
            reader.scan_all().map_err(map_packet_limit)?;
            Ok(reader.tracks.iter().any(|track| track.kind == 2 && track.codec == "A_AAC"))
        }
        Ok(()) if crate::container::mp4::recognizes_prefix(&prefix) => {
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start(0))?;
            let limits = crate::container::mp4::Limits {
                metadata_bytes: crate::container::mp4::Limits::default().metadata_bytes
                    .min(max_controlled_bytes.unwrap_or(usize::MAX)),
                ..Default::default()
            };
            let reader = crate::container::mp4::Mp4Reader::open_indexed(file, limits)
                .map_err(std::io::Error::other)?;
            Ok(reader.tracks().iter().any(|track| track.handler == *b"soun" && track.codec == *b"mp4a"))
        }
        Ok(()) => Ok(!prefix.starts_with(b"ID3") && crate::container::adts::header(&prefix).is_some()),
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
/// Implicit PS is verified from MP4/Matroska payloads. Mono ADTS scans the
/// encoded stream before publishing its negotiated output clock and layout.
pub fn aac_source_info_selected(source: &Path, selected: Option<usize>) -> Result<AacSourceInfo> {
    aac_source_info_selected_with_options(source, selected, None, &fvid_media::CopyOptions::default())
}

pub fn aac_source_info_selected_with_options(
    source: &Path,
    selected: Option<usize>,
    interval: Option<(Duration, Duration)>,
    options: &fvid_media::CopyOptions,
) -> Result<AacSourceInfo> {
    use std::io::{BufReader, Read, Seek, SeekFrom};
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    let mut control = DecodeProgress::new_with_limits(
        options.cancel.as_ref(),
        None,
        options.max_rss_bytes,
        options.max_packets,
    )?;
    control.check()?;
    let mut input = BufReader::new(std::fs::File::open(source)?);
    let mut prefix = [0; 8];
    input.read_exact(&mut prefix)?;
    input.seek(SeekFrom::Start(0))?;
    let (stream_index, asc, declared, in_band_ps) = if crate::container::mp4::recognizes_prefix(&prefix) {
        let limits = crate::container::mp4::Limits {
            metadata_bytes: crate::container::mp4::Limits::default().metadata_bytes
                .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
            ..Default::default()
        };
        let mut reader = crate::container::mp4::Mp4Reader::open_indexed(input, limits)?;
        reader.restrict_packet_bytes(options.max_packet_bytes);
        let index = mp4_aac_index(&reader, selected)?;
        if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes {
            let track = &reader.tracks()[index];
            let max_samples = mp4_active_samples(track, interval, options.max_packets);
            let exceeds = match &track.samples {
                crate::container::mp4::SampleIndex::Expanded(samples) => samples
                    .iter()
                    .take(max_samples)
                    .any(|s| (s.size as usize) > options.max_packet_bytes),
                crate::container::mp4::SampleIndex::Uniform(uniform) => {
                    uniform.count > 0
                        && max_samples > 0
                        && (uniform.bytes_per_frame as usize) > options.max_packet_bytes
                }
            };
            if exceeds {
                return Err(invalid("MP4 packet exceeds budget"));
            }
        }
        let in_band_ps = negotiate_mp4_ps_with_interval(&mut reader, index, interval, options.max_packets, || control.check())?;
        let source_track = &reader.tracks()[index];
        let first = source_track.samples.get(0).ok_or_else(|| invalid("MP4 audio has no samples"))?;
        let setup = source_track.decoder_setup(first.description_index)?;
        let track = &setup;
        (index, crate::codec::config::aac_specific_config(&track.configuration)?.to_vec(),
            (u64::from(track.sample_rate), u64::from(track.channels)), in_band_ps)
    } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let mut limits = crate::container::webm::Limits::default();
        let memory_packets = options
            .max_controlled_bytes
            .map(|bytes| bytes / std::mem::size_of::<crate::container::webm::Packet>());
        if let Some(maximum) = memory_packets {
            limits.packets = limits.packets.min(maximum);
        }
        let map_packet_limit = |error: crate::Error| {
            if memory_packets
                .is_some_and(|max| max < crate::container::webm::Limits::default().packets)
                && error
                    .to_string()
                    .contains("WebM packet count exceeds limit")
            {
                invalid("controlled memory budget exceeded: Matroska packet index limit")
            } else {
                error
            }
        };
        let mut reader =
            crate::container::webm::WebmReader::open(input, limits).map_err(map_packet_limit)?;
        reader.restrict_packet_bytes(options.max_packet_bytes);
        reader.scan_all().map_err(map_packet_limit)?;
        let index = matroska_aac_index(&reader, selected)?;
        if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes {
            let track_number = reader.tracks[index].number;
            let active_limit = matroska_active_track_packets(&reader, index, interval, options.max_packets);
            let mut track_packets = 0usize;
            for (packet_index, packet) in reader.packets.iter().enumerate() {
                if packet.track == track_number {
                    if track_packets >= active_limit {
                        break;
                    }
                    if reader.packet_payload_size(packet_index)? > options.max_packet_bytes {
                        return Err(invalid("WebM packet exceeds budget"));
                    }
                    track_packets += 1;
                }
            }
        }
        let in_band_ps = negotiate_matroska_ps_with_interval(&mut reader, index, interval, options.max_packets, || control.check())?;
        let track = &reader.tracks[index];
        (index, track.codec_private.clone(), (track.sample_rate, track.channels), in_band_ps)
    } else {
        if selected.is_some_and(|index| index != 0) { return Err(invalid("ADTS has only stream 0")); }
        let reader = crate::container::adts::StreamReader::open_with_packet_limit(
            input,
            options
                .max_packet_bytes
                .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
        )?;
        let negotiated = negotiate_adts_aac_reader(reader, interval, &mut control)?;
        return Ok(AacSourceInfo { stream_index: 0, sample_rate: negotiated.sample_rate(),
            channels: negotiated.channels(), channel_mask: negotiated.channel_mask() });
    };
    let parsed = crate::codec::config::AudioSpecificConfig::parse(&asc)?;
    if in_band_ps || parsed.ps_present == Some(true) {
        let rate = u32::try_from(declared.0).map_err(|_| invalid("AAC output clock overflow"))?;
        let decoder = crate::codec::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(&asc, rate)?;
        if !matches!(declared.1, 1 | 2) || decoder.sample_rate() != rate {
            return Err(invalid("PS AAC container geometry disagrees with decoded layout"));
        }
        return Ok(AacSourceInfo { stream_index, sample_rate: rate, channels: 2, channel_mask: decoder.channel_mask() });
    }
    let decoder = crate::codec::aac_native::NativeAacDecoder::new_with_output_rate(&asc,u32::try_from(declared.0).map_err(|_|invalid("AAC output clock overflow"))?)?;
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
    inspect_adts_with_options(source, &fvid_media::CopyOptions::default())
}

pub fn inspect_adts_with_options<R: std::io::Read>(
    source: R,
    options: &fvid_media::CopyOptions,
) -> Result<AdtsInfo> {
    inspect_adts_with_interval_and_options(source, None, options)
}

pub fn inspect_adts_with_interval_and_options<R: std::io::Read>(
    source: R,
    interval: Option<(Duration, Duration)>,
    options: &fvid_media::CopyOptions,
) -> Result<AdtsInfo> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let mut reader = crate::container::adts::StreamReader::open_with_packet_limit(
        source,
        options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
    )?;
    let config = crate::codec::config::AacConfig::parse(reader.audio_specific_config())?;
    let mut info = AdtsInfo { packets: 0, payload_bytes: 0, sample_frames: 0,
        sample_rate: config.sample_rate, channels: u16::from(config.channels) };
    let core_to = match interval {
        Some((_, to)) => adts_sample_boundary(to, config.sample_rate)?,
        None => u64::MAX,
    };
    while info.sample_frames < core_to && options.max_packets.is_none_or(|limit| info.packets < limit) {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err(invalid("media operation cancelled"));
        }
        fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
        let Some(packet) = reader.next_packet()? else {
            break;
        };
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
    let automatic = if selected.is_none() { reader.automatic_track_indices(&[*b"soun"]) } else { Vec::new() };
    let index = audio_index(reader.tracks().iter().enumerate().map(|(i, t)| t.handler == *b"soun" && (selected.is_some() || automatic.contains(&i))),selected)?;
    if reader.tracks()[index].description_for_sample(0).is_none_or(|d|d.codec != *b"mp4a") { return Err(invalid("selected MP4 audio stream is not AAC")); }
    Ok(index)
}
pub(crate) fn matroska_aac_index<R: std::io::Read + std::io::Seek>(reader: &crate::container::webm::WebmReader<R>, selected: Option<usize>) -> Result<usize> {
    let preferred = if selected.is_none() { reader.preferred_track_numbers(&reader.tracks.iter().filter(|t| t.kind == 2).map(|t| t.number).collect::<Vec<_>>()) } else { Vec::new() };
    let index = audio_index(reader.tracks.iter().map(|t| t.kind == 2 && (selected.is_some() || preferred.contains(&t.number))),selected)?;
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
    if transform.deband.as_deref().is_some_and(|a| fvid_media::owned_deband::Deband::parse(a).is_err()) {return false;}
    if transform.perspective.as_deref().is_some_and(|a| fvid_media::owned_perspective::Perspective::parse(a).is_err()) {return false;}
    if transform.gradfun.as_deref().is_some_and(|a| fvid_media::owned_gradfun::GradFun::parse(a).is_err()) {return false;}
    if transform.lenscorrection.as_deref().is_some_and(|a| fvid_media::owned_lenscorrection::LensCorrection::parse(a).is_err()) {return false;}
    if transform.drawbox.as_deref().is_some_and(|a| fvid_media::owned_draw::Draw::box_filter(a).is_err()) {return false;}
    if transform.drawgrid.as_deref().is_some_and(|a| fvid_media::owned_draw::Draw::grid_filter(a).is_err()) {return false;}
    if transform.removegrain.as_deref().is_some_and(|a| fvid_media::owned_removegrain::RemoveGrain::parse(a).is_err()) {return false;}
    if transform.yaepblur.as_deref().is_some_and(|a| fvid_media::owned_yaepblur::YaepBlur::parse(a).is_err()) {return false;}
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
        deband: _,
        perspective: _,
        gradfun: _,
        lenscorrection: _,
        removegrain: _,
        yaepblur: _,
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
        drawbox: _,
        drawgrid: _,
        lagfun: _,
        amplify: None,
        pixelize: _,
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
    let reader = crate::container::mp4::Mp4Reader::open_indexed(BufReader::new(input), Default::default())?;
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
    is_owned_audio_source_with_options(path, &fvid_media::CopyOptions::default())
}

pub fn is_owned_audio_source_with_options(
    path: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    is_owned_audio_source_with_interval_and_options(path, None, options)
}

pub fn is_owned_audio_source_with_interval_and_options(
    path: &Path,
    interval: Option<(Duration, Duration)>,
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    if is_aac_source_with_metadata_limit(path, options.max_controlled_bytes)? {
        return Ok(true);
    }
    Ok(
        (is_mp4_audio_container(path)? || crate::native_export::is_matroska_source(path)?)
            && audio_source_info_selected_with_interval_and_options(path, None, interval, options).is_ok(),
    )
}

/// Trim can select one supported audio track even when default decoding is
/// ambiguous. Execution still requires explicit selection if other streams exist.
pub fn is_owned_audio_trim_source(path: &Path) -> Result<bool> {
    is_owned_audio_trim_source_with_options(path, &fvid_media::CopyOptions::default())
}

pub fn is_owned_audio_trim_source_with_options(
    path: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    is_owned_audio_trim_source_with_interval_and_options(path, None, options)
}

pub fn is_owned_audio_trim_source_with_interval_and_options(
    path: &Path,
    interval: Option<(Duration, Duration)>,
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    if is_owned_audio_source_with_interval_and_options(path, interval, options)? {
        return Ok(true);
    }
    if !is_mp4_audio_container(path)? && !crate::native_export::is_matroska_source(path)? {
        return Ok(false);
    }
    let info = crate::native_probe::probe_with_options(path, options).map_err(|e| invalid(&e))?;
    Ok(info.streams.iter().any(|stream| {
        stream.media_type == "audio"
            && audio_source_info_selected_with_interval_and_options(path, Some(stream.index), interval, options).is_ok()
    }))
}

pub(crate) fn is_aac_trim_source(path: &Path, selected: Option<usize>) -> Result<bool> {
    is_aac_trim_source_with_options(path, selected, &fvid_media::CopyOptions::default())
}

pub(crate) fn is_aac_trim_source_with_options(
    path: &Path,
    selected: Option<usize>,
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    is_aac_trim_source_with_interval_and_options(path, selected, None, options)
}

pub(crate) fn is_aac_trim_source_with_interval_and_options(
    path: &Path,
    selected: Option<usize>,
    interval: Option<(Duration, Duration)>,
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    match selected {
        Some(index) => Ok(audio_source_info_selected_with_interval_and_options(path, Some(index), interval, options)?.codec == "aac"),
        None => Ok(is_aac_source_with_metadata_limit(path, options.max_controlled_bytes)?),
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
    let automatic = if selected.is_none() {
        reader.automatic_track_indices(&[*b"soun"])
    } else {
        Vec::new()
    };
    let index = audio_index(
        reader
            .tracks()
            .iter()
            .enumerate()
            .map(|(i, t)| t.handler == *b"soun" && (selected.is_some() || automatic.contains(&i))),
        selected,
    )?;
    if !matches!(&reader.tracks()[index].codec, b"mp4a" | b"alac" | b"sowt" | b"twos" | b"fl32" | b"fl64" | b"in24" | b"in32" | b"raw " | b"ima4" | b"ms\x00\x11" | b"ms\x00\x02" | b"MAC3" | b"MAC6") {
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
    audio_source_info_selected_with_options(source, selected, &fvid_media::CopyOptions::default())
}

pub fn audio_source_info_selected_with_options(
    source: &Path,
    selected: Option<usize>,
    options: &fvid_media::CopyOptions,
) -> Result<AudioSourceInfo> {
    audio_source_info_selected_with_interval_and_options(source, selected, None, options)
}

pub fn audio_source_info_selected_with_interval_and_options(
    source: &Path,
    selected: Option<usize>,
    interval: Option<(Duration, Duration)>,
    options: &fvid_media::CopyOptions,
) -> Result<AudioSourceInfo> {
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    let control = DecodeProgress::new_with_limits(
        options.cancel.as_ref(),
        None,
        options.max_rss_bytes,
        options.max_packets,
    )?;
    control.check()?;
    if is_mp4_audio_container(source)? || crate::native_export::is_matroska_source(source)? {
        use std::io::{Read,Seek,SeekFrom};
        let mut input=File::open(source)?;let mut prefix=[0;4];input.read_exact(&mut prefix)?;input.seek(SeekFrom::Start(0))?;
        if prefix==[0x1a,0x45,0xdf,0xa3] {
            let mut limits = crate::container::webm::Limits::default();
            let memory_packets = options
                .max_controlled_bytes
                .map(|bytes| bytes / std::mem::size_of::<crate::container::webm::Packet>());
            if let Some(maximum) = memory_packets {
                limits.packets = limits.packets.min(maximum);
            }
            let map_packet_limit = |error: crate::Error| {
                if memory_packets
                    .is_some_and(|max| max < crate::container::webm::Limits::default().packets)
                    && error
                        .to_string()
                        .contains("WebM packet count exceeds limit")
                {
                    invalid("controlled memory budget exceeded: Matroska packet index limit")
                } else {
                    error
                }
            };
            let mut reader = crate::container::webm::WebmReader::open(BufReader::new(input), limits)
                .map_err(map_packet_limit)?;
            reader.restrict_packet_bytes(options.max_packet_bytes);
            reader.scan_all().map_err(map_packet_limit)?;
            let index=matroska_audio_index(&reader,selected)?;
            if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes {
                let track_number = reader.tracks[index].number;
                let active_limit = matroska_active_track_packets(&reader, index, interval, options.max_packets);
                let mut track_packets = 0usize;
                for (packet_index, packet) in reader.packets.iter().enumerate() {
                    if packet.track == track_number {
                        if track_packets >= active_limit {
                            break;
                        }
                        if reader.packet_payload_size(packet_index)? > options.max_packet_bytes {
                            return Err(invalid("WebM packet exceeds budget"));
                        }
                        track_packets += 1;
                    }
                }
            }
            let in_band_ps=negotiate_matroska_ps_with_interval(&mut reader,index,interval,options.max_packets,||control.check())?;
            let track=&reader.tracks[index];
            let decoder=if in_band_ps {MatroskaTimelineDecoder::with_in_band_ps(track)?.0} else {crate::native_audio_decoder::PacketPcmDecoder::from_matroska(track)?};
            if track.sample_rate!=u64::from(decoder.sample_rate()) || (!in_band_ps && track.channels!=u64::from(decoder.channels())) {return Err(invalid("Matroska audio geometry disagrees with configuration"));}
            return Ok(AudioSourceInfo {
                channel_mask: decoder.channel_mask(),
                stream_index: index,
                sample_rate: decoder.sample_rate(),
                channels: decoder.channels(),
                codec: match (track.codec.as_str(), track.bit_depth) {
                    ("A_ALAC", _) => "alac",
                    ("A_PCM/INT/LIT" | "A_PCM/INT/BIG", 8) => "pcm_u8",
                    ("A_PCM/INT/LIT", 16) => "pcm_s16le",
                    ("A_PCM/INT/LIT", 24) => "pcm_s24le",
                    ("A_PCM/INT/LIT", 32) => "pcm_s32le",
                    ("A_PCM/INT/BIG", 16) => "pcm_s16be",
                    ("A_PCM/INT/BIG", 24) => "pcm_s24be",
                    ("A_PCM/INT/BIG", 32) => "pcm_s32be",
                    ("A_PCM/FLOAT/IEEE", 32) => "pcm_f32le",
                    ("A_PCM/FLOAT/IEEE", 64) => "pcm_f64le",
                    _ => "aac",
                },
            });
        }
        let mut reader = crate::container::mp4::Mp4Reader::open_indexed(
            BufReader::new(File::open(source)?),
            crate::container::mp4::Limits {
                metadata_bytes: crate::container::mp4::Limits::default().metadata_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                ..Default::default()
            },
        )?;
        reader.restrict_packet_bytes(options.max_packet_bytes);
        let index = mp4_audio_index(&reader, selected)?;
        reader.tracks()[index].ensure_constant_description()?;
        if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes {
            let track = &reader.tracks()[index];
            let max_samples = mp4_active_samples(track, interval, options.max_packets);
            let exceeds = match &track.samples {
                crate::container::mp4::SampleIndex::Expanded(samples) => samples
                    .iter()
                    .take(max_samples)
                    .any(|s| (s.size as usize) > options.max_packet_bytes),
                crate::container::mp4::SampleIndex::Uniform(uniform) => {
                    uniform.count > 0
                        && max_samples > 0
                        && (uniform.bytes_per_frame as usize) > options.max_packet_bytes
                }
            };
            if exceeds {
                return Err(invalid("MP4 packet exceeds budget"));
            }
        }
        let in_band_ps = negotiate_mp4_ps_with_interval(&mut reader,index,interval,options.max_packets,||control.check())?;
        let source_track = &reader.tracks()[index];
        let first = source_track.samples.get(0).ok_or_else(|| invalid("MP4 audio has no samples"))?;
        let setup = source_track.decoder_setup(first.description_index)?;
        let track = &setup;
        let decoder = if in_band_ps {crate::native_audio_decoder::PacketPcmDecoder::with_in_band_ps(track)?} else {crate::native_audio_decoder::PacketPcmDecoder::new(track)?};
        if track.sample_rate != decoder.sample_rate() || (!in_band_ps && track.channels != decoder.channels()) {
            return Err(invalid("MP4 container and decoder geometry disagree"));
        }
        Ok(AudioSourceInfo {
            channel_mask: decoder.channel_mask(),
            stream_index: index,
            sample_rate: decoder.sample_rate(),
            channels: decoder.channels(),
            codec: match (&track.codec, track.configuration.first() == Some(&1)) {
                (b"alac", _) => "alac",
                (b"MAC3", _) => "mace3",
                (b"MAC6", _) => "mace6",
                (b"ima4", _) => "adpcm_ima_qt",
                (b"ms\x00\x11", _) => "adpcm_ima_wav",
                (b"ms\x00\x02", _) => "adpcm_ms",
                (b"raw ", _) => "pcm_u8",
                (b"sowt", _) => "pcm_s16le",
                (b"twos", _) => "pcm_s16be",
                (b"in24", true) => "pcm_s24le",
                (b"in24", false) => "pcm_s24be",
                (b"in32", true) => "pcm_s32le",
                (b"in32", false) => "pcm_s32be",
                (b"fl32", false) => "pcm_f32be",
                (b"fl32", true) => "pcm_f32le",
                (b"fl64", false) => "pcm_f64be",
                (b"fl64", true) => "pcm_f64le",
                _ => "aac",
            },
        })
    } else {
        let info = aac_source_info_selected_with_options(source, selected, interval, options)?;
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
    let preferred=if selected.is_none() {reader.preferred_track_numbers(&reader.tracks.iter().filter(|t|t.kind==2).map(|t|t.number).collect::<Vec<_>>())} else {Vec::new()};
    let index=audio_index(reader.tracks.iter().map(|t|t.kind==2&&(selected.is_some()||preferred.contains(&t.number))),selected)?;
    if !matches!(reader.tracks[index].codec.as_str(),"A_AAC"|"A_ALAC"|"A_PCM/INT/LIT"|"A_PCM/INT/BIG"|"A_PCM/FLOAT/IEEE") {return Err(invalid("selected Matroska audio stream is not supported by the owned export"));}
    Ok(index)
}

/// Keep legacy chroma negotiation until the owned format conversion graph is qualified.
pub fn supports_deband_source(source:&Path,args:Option<&str>)->Result<bool>{
 if !fvid_media::owned_deband::Deband::needs_equal_planes(args){return Ok(true);}
 let Ok(file)=File::open(source) else{return Ok(false);};
 let Ok(mut reader)=NativeReader::software(BufReader::new(file),usize::MAX) else{return Ok(false);};
 let Ok(Some(raw))=reader.read_frame_raw() else{return Ok(false);};
 let [w,h]=reader.dimensions();
 let frame=crate::native_geometry::VideoGeometry::default().apply(&raw,w,h)?;
 Ok(frame.subsampling.is_none()||frame.subsampling==Some([1,1]))
}
