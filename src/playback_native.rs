//! RGB playback adapter for FVid's own Y4M, MP4/AVC/HEVC and WebM/VP9/AV1 readers.
use crate::{
    Result, codec::avc_picture::IntraPicture, container::mp4::Limits, invalid, playback::Y4mReader,
    playback_mp4::Mp4VideoReader,
};
use std::{
    io::{BufRead, Seek, SeekFrom},
    sync::Arc,
    time::Duration,
};

pub enum NativeReader<R> {
    Webm(crate::playback_webm::WebmVideoReader<R>),
    Y4m(Y4mReader<R>),
    Avc {
        source: Mp4VideoReader<R>,
        rgb: Vec<u8>,
        dimensions: [usize; 2],
        period: Duration,
        next_pts: i64,
        frame_start: i64,
        frames: u64,
        rgb_budget: usize,
        media_start: i64,
        media_end: Option<i64>,
        /// After a seek the next decoded frame defines the timeline position
        /// instead of having to continue the previous frame exactly.
        resync: bool,
    },
}
impl<R: BufRead + Seek> NativeReader<R> {
    /// Playback without an application-imposed memory cap. Format bounds and
    /// checked size arithmetic still apply; storage is allocated as needed.
    pub fn without_memory_limit(reader: R) -> Result<Self> {
        Self::new(reader, usize::MAX)
    }

    pub fn new(mut reader: R, budget: usize) -> Result<Self> {
        let start = reader.stream_position()?;
        let mut prefix = [0u8; 9];
        let mut length = 0;
        while length < prefix.len() {
            let count = reader.read(&mut prefix[length..])?;
            if count == 0 {
                break;
            }
            length += count;
        }
        reader.seek(SeekFrom::Start(start))?;
        if &prefix == b"YUV4MPEG2" {
            return Ok(Self::Y4m(Y4mReader::new(reader, budget)?));
        }
        if length >= 4 && prefix[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
            return Ok(Self::Webm(crate::playback_webm::WebmVideoReader::open(
                reader, budget,
            )?));
        }
        let signature = &prefix[..length];
        if signature.starts_with(&[0xff, 0xd8, 0xff]) {
            return Err(invalid("JPEG image: still-image playback is not supported"));
        }
        if signature.starts_with(b"\x89PNG\r\n\x1a\n")
            || signature.starts_with(b"GIF87a")
            || signature.starts_with(b"GIF89a")
        {
            return Err(invalid("image file: still-image playback is not supported"));
        }
        // Dispatch only recognizable ISO BMFF/QuickTime box headers. A file
        // with another signature must not be diagnosed as a corrupt MP4.
        if length < 8
            || !matches!(
                &prefix[4..8],
                b"ftyp" | b"styp" | b"moov" | b"mdat" | b"free" | b"skip" | b"wide" | b"uuid"
            )
        {
            return Err(invalid(
                "unrecognized video format; supported containers: MP4/MOV, WebM/Matroska and Y4M",
            ));
        }
        let rgb_budget = budget / 4;
        let source = Mp4VideoReader::open(reader, Limits::default(), budget - rgb_budget)?;
        let track = source.track();
        let (media_start, media_end) = playback_window(track, source.movie_timescale())?;
        Ok(Self::Avc {
            source,
            rgb: Vec::new(),
            dimensions: [0; 2],
            period: Duration::ZERO,
            next_pts: 0,
            frame_start: 0,
            frames: 0,
            rgb_budget,
            media_start,
            media_end,
            resync: false,
        })
    }
    pub fn dimensions(&self) -> [usize; 2] {
        match self {
            Self::Y4m(r) => r.dimensions(),
            Self::Webm(r) => r.dimensions(),
            Self::Avc { dimensions, .. } => *dimensions,
        }
    }
    pub fn rgb(&self) -> &[u8] {
        match self {
            Self::Y4m(r) => r.rgb(),
            Self::Webm(r) => r.rgb(),
            Self::Avc { rgb, .. } => rgb,
        }
    }
    pub fn frame_period(&self) -> Duration {
        match self {
            Self::Y4m(r) => r.frame_period(),
            Self::Webm(r) => r.frame_period(),
            Self::Avc { period, .. } => *period,
        }
    }
    /// Exact half-open interval of the current frame, in source clock ticks.
    /// No accumulated floating-point or nanosecond rounding is involved.
    pub fn frame_interval(&self) -> Option<(u128, u128, u32)> {
        match self {
            Self::Webm(r) => r.frame_interval(),
            Self::Y4m(reader) => {
                let count = reader.frames_read();
                if count == 0 {
                    return None;
                }
                let (num, den) = reader.frame_rate();
                Some((
                    u128::from(count - 1) * u128::from(den),
                    u128::from(count) * u128::from(den),
                    num,
                ))
            }
            Self::Avc {
                source,
                frame_start,
                next_pts,
                frames,
                ..
            } => {
                if *frames == 0 {
                    return None;
                }
                Some((
                    *frame_start as u128,
                    *next_pts as u128,
                    source.track().timescale,
                ))
            }
        }
    }
    /// Current frame's presentation timestamp in track timescale units.
    /// Returns None if no frame has been read yet or for formats without PTS.
    pub fn current_pts(&self) -> Option<(i64, u32)> {
        match self {
            Self::Webm(r) => r.frame_interval().map(|(start, _, scale)| (start as i64, scale)),
            Self::Y4m(reader) => {
                let count = reader.frames_read();
                if count == 0 {
                    return None;
                }
                let (num, den) = reader.frame_rate();
                Some(((count - 1) as i64 * den as i64, num))
            }
            Self::Avc {
                source,
                frame_start,
                ..
            } => {
                if source.track().timescale == 0 {
                    return None;
                }
                Some((*frame_start, source.track().timescale))
            }
        }
    }
    /// Total playable length when the container declares one (MP4 track
    /// duration, clipped by its edit window, or the WebM timeline). Y4M
    /// streams carry no up-front length, so they report `None`.
    pub fn duration(&self) -> Option<Duration> {
        match self {
            Self::Y4m(_) => None,
            Self::Webm(r) => r.duration(),
            Self::Avc {
                source,
                media_start,
                media_end,
                ..
            } => {
                let track = source.track();
                if track.timescale == 0 {
                    return None;
                }
                let end = media_end.map_or(i128::from(track.duration), |end| i128::from(end));
                let ticks = end - i128::from(*media_start);
                if ticks <= 0 {
                    return None;
                }
                let nanos = ticks * 1_000_000_000 / i128::from(track.timescale);
                u64::try_from(nanos).ok().map(Duration::from_nanos)
            }
        }
    }
    pub fn rewind(&mut self) -> Result<()> {
        match self {
            Self::Y4m(r) => r.rewind(),
            Self::Webm(r) => {
                r.rewind();
                Ok(())
            }
            Self::Avc {
                source,
                next_pts,
                frame_start,
                frames,
                resync,
                ..
            } => {
                source.rewind();
                *next_pts = 0;
                *frame_start = 0;
                *frames = 0;
                *resync = false;
                Ok(())
            }
        }
    }
    /// Whether frames come from the platform's hardware decoder (VideoToolbox).
    pub fn hardware_accelerated(&self) -> bool {
        match self {
            Self::Avc { source, .. } => source.hardware_accelerated(),
            _ => false,
        }
    }
    /// Whether `seek` can position this stream; only containers with a sample
    /// index and sync samples (MP4) support it.
    pub fn seekable(&self) -> bool {
        matches!(self, Self::Avc { .. })
    }
    /// Position playback so the current frame contains `target` (clamped to the
    /// stream). Decoding restarts at the preceding sync sample and runs forward
    /// to the target, so the call takes as long as decoding that stretch.
    /// On failure the stream is rewound to the start.
    pub fn seek(&mut self, target: Duration) -> Result<()> {
        let result = (|| {
            if let Some(raw) = self.seek_raw(target)? {
                if let Self::Avc {
                    rgb, rgb_budget, ..
                } = self
                {
                    match raw {
                        RawFrame::Avc { picture, colour } => {
                            avc_to_rgb(&picture, colour, rgb, *rgb_budget)?
                        }
                        RawFrame::Planar8(planes) => planar8_to_rgb(&planes, rgb, *rgb_budget)?,
                        RawFrame::Rgb(bytes) => *rgb = bytes,
                        RawFrame::Yuv { .. } => unreachable!(),
                    }
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.rewind()?;
        }
        result
    }
    /// Seek with the same pixel representation used by continuous playback.
    /// Intermediate reference pictures are decoded without RGB conversion.
    /// `rgb()` remains unchanged; use `seek` when a packed RGB result is needed.
    pub fn seek_raw(&mut self, target: Duration) -> Result<Option<RawFrame>> {
        let Self::Avc {
            source,
            media_start,
            resync,
            ..
        } = self
        else {
            return Err(invalid("seeking is only implemented for MP4"));
        };
        let timescale = source.track().timescale;
        let ticks = i128::from(timescale) * target.as_nanos() as i128 / 1_000_000_000;
        let ticks = i64::try_from(ticks)
            .ok()
            .and_then(|t| t.checked_add(*media_start))
            .ok_or_else(|| invalid("seek target overflow"))?;
        source.seek_to_sync(ticks);
        *resync = true;
        let result = (|| {
            let mut last = None;
            loop {
                let Some(raw) = self.read_frame_raw()? else {
                    return Ok(last);
                };
                last = Some(raw);
                let Self::Avc {
                    next_pts,
                    media_start,
                    ..
                } = self
                else {
                    unreachable!()
                };
                if i128::from(*next_pts) + i128::from(*media_start) > i128::from(ticks) {
                    return Ok(last);
                }
            }
        })();
        if result.is_err() {
            self.rewind()?;
        }
        result
    }
    pub fn read_frame(&mut self) -> Result<bool> {
        match self {
            Self::Y4m(r) => r.read_frame(),
            Self::Webm(r) => r.read_frame(),
            Self::Avc { .. } => {
                let Some(raw) = self.advance_avc()? else {
                    return Ok(false);
                };
                let Self::Avc {
                    rgb, rgb_budget, ..
                } = self
                else {
                    unreachable!()
                };
                match raw {
                    RawFrame::Avc { picture, colour } => {
                        avc_to_rgb(&picture, colour, rgb, *rgb_budget)?
                    }
                    RawFrame::Planar8(planes) => planar8_to_rgb(&planes, rgb, *rgb_budget)?,
                    RawFrame::Rgb(bytes) => *rgb = bytes,
                    RawFrame::Yuv { .. } => unreachable!(),
                }
                Ok(true)
            }
        }
    }
    /// `read_frame` without the RGB conversion. The decoded picture (or, for
    /// readers that convert internally, the RGB bytes) is handed back so
    /// another thread can run `RawFrame::into_rgb` while decoding continues.
    /// For AVC, WebM and Y4M, `rgb()` is stale after this call.
    pub fn read_frame_raw(&mut self) -> Result<Option<RawFrame>> {
        match self {
            Self::Y4m(reader) => Ok(if reader.read_frame_raw()? {
                let (sx, sy) = reader.subsampling();
                let width = reader.width();
                let height = reader.height();
                let luma_len = width * height;
                let chroma_len = luma_len / sx / sy;
                let chroma_width = width / sx;
                let chroma_height = height / sy;
                // Split the contiguous YUV buffer into separate planes so the
                // GPU shader can do the colour conversion without a CPU pass.
                let yuv = reader.yuv();
                let y = yuv[..luma_len].to_vec();
                let cb = yuv[luma_len..luma_len + chroma_len].to_vec();
                let cr = yuv[luma_len + chroma_len..].to_vec();
                Some(RawFrame::Planar8(Arc::new(Planar8 {
                    width,
                    height,
                    chroma_width,
                    chroma_height,
                    y,
                    cb,
                    cr,
                    colour: AvcColour {
                        kr: 0.299,
                        kb: 0.114,
                        full: false,
                    },
                })))
            } else {
                None
            }),
            Self::Webm(reader) => Ok(reader
                .read_frame_planes()?
                .map(|planes| RawFrame::Planar8(Arc::new(planes)))),
            Self::Avc { .. } => self.advance_avc(),
        }
    }
    /// Decode the next AVC frame and update the timeline; conversion is separate.
    fn advance_avc(&mut self) -> Result<Option<RawFrame>> {
        let Self::Avc {
            source,
            dimensions,
            period,
            next_pts,
            frame_start,
            frames,
            media_start,
            media_end,
            resync,
            ..
        } = self
        else {
            return Ok(None);
        };
        if !*resync
            && media_end.is_some_and(|end| {
                i128::from(*next_pts) + i128::from(*media_start) >= i128::from(end)
            })
        {
            return Ok(None);
        }
        let frame = loop {
            let Some(mut frame) = source.read_frame()? else {
                return Ok(None);
            };
            let begin = frame.presentation_time.ticks;
            let end = begin
                .checked_add(frame.duration.ticks)
                .ok_or_else(|| invalid("video timestamp overflow"))?;
            if end <= *media_start {
                continue;
            }
            if media_end.is_some_and(|limit| begin >= limit) {
                return Ok(None);
            }
            let clipped_begin = begin.max(*media_start);
            let clipped_end = media_end.map_or(end, |limit| end.min(limit));
            frame.presentation_time.ticks = clipped_begin
                .checked_sub(*media_start)
                .ok_or_else(|| invalid("video edit timestamp overflow"))?;
            frame.duration.ticks = clipped_end
                .checked_sub(clipped_begin)
                .ok_or_else(|| invalid("video edit duration overflow"))?;
            break frame;
        };
        if *resync {
            *next_pts = frame.presentation_time.ticks;
            *resync = false;
        }
        if frame.presentation_time.ticks != *next_pts || frame.duration.ticks <= 0 {
            return Err(invalid(
                "non-contiguous AVC presentation timestamps are not implemented",
            ));
        }
        let nanos = frame.duration.nanoseconds()?;
        let nanos = u64::try_from(nanos)
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid("invalid video frame duration"))?;
        let colour = source.active_colour()?;
        let (w, h) = frame.picture.dimensions();
        *dimensions = [w, h];
        *period = Duration::from_nanos(nanos);
        *frame_start = frame.presentation_time.ticks;
        *next_pts = frame
            .presentation_time
            .ticks
            .checked_add(frame.duration.ticks)
            .ok_or_else(|| invalid("video timestamp overflow"))?;
        *frames += 1;
        Ok(Some(match frame.planes8 {
            Some(planes) => RawFrame::Planar8(planes),
            None => RawFrame::Avc {
                picture: frame.picture,
                colour,
            },
        }))
    }
}

/// Colour interpretation of a decoded AVC picture: VUI matrix coefficients
/// and whether samples use the full range.
#[derive(Clone, Copy, Debug)]
pub struct AvcColour {
    pub kr: f64,
    pub kb: f64,
    pub full: bool,
}
impl Default for AvcColour {
    fn default() -> Self {
        Self {
            kr: 0.299,
            kb: 0.114,
            full: false,
        }
    }
}
impl AvcColour {
    pub fn from_hevc_vui(vui: Option<&crate::codec::hevc_vui::Vui>) -> Result<Self> {
        let signal = vui.and_then(|v| v.signal.as_ref());
        let full = signal.is_some_and(|s| s.full_range);
        let matrix = signal.and_then(|s| s.colour).map_or(2, |c| c[2]);
        let (kr, kb) = match matrix {
            1 => (0.2126, 0.0722),
            2 | 5 | 6 => (0.299, 0.114),
            9 => (0.2627, 0.0593),
            _ => {
                return Err(invalid(
                    "HEVC colour matrix is not implemented for playback",
                ));
            }
        };
        Ok(Self { kr, kb, full })
    }
    /// Matrix coefficients and range from the VUI; BT.601 when unspecified.
    pub fn from_vui(vui: Option<&crate::codec::avc::Vui>) -> Result<Self> {
        let signal = vui.and_then(|v| v.video_signal);
        let full = signal.is_some_and(|(_, full, _)| full);
        let matrix = signal
            .and_then(|(_, _, colour)| colour)
            .map(|c| c[2])
            .unwrap_or(2);
        let (kr, kb) = match matrix {
            1 => (0.2126, 0.0722),
            2 | 5 | 6 => (0.299, 0.114),
            _ => return Err(invalid("AVC colour matrix is not implemented for playback")),
        };
        Ok(Self { kr, kb, full })
    }
}
/// The visible picture as packed 8-bit 4:2:0 planes, ready for GPU upload.
pub struct Planar8 {
    pub width: usize,
    pub height: usize,
    pub chroma_width: usize,
    pub chroma_height: usize,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
    pub colour: AvcColour,
}
/// Crop the picture and narrow its samples to 8 bits without colour conversion.
pub fn avc_to_planar8(p: &IntraPicture, colour: AvcColour) -> Planar8 {
    coded_planes_to_planar8(
        &p.y,
        &p.cb,
        &p.cr,
        p.coded_width,
        p.coded_height,
        p.crop,
        p.bit_depth,
        colour,
    )
}
/// Build packed 8-bit 4:2:0 planes from coded 16-bit planes and a crop window.
/// `crop` is `[left, right, top, bottom]`; chroma stride is `coded_width / 2`.
#[allow(clippy::too_many_arguments)]
pub fn coded_planes_to_planar8(
    y: &[u16],
    cb: &[u16],
    cr: &[u16],
    coded_width: usize,
    coded_height: usize,
    crop: [usize; 4],
    depth: u8,
    colour: AvcColour,
) -> Planar8 {
    let w = coded_width - crop[0] - crop[1];
    let h = coded_height - crop[2] - crop[3];
    let shift = depth.saturating_sub(8);
    let (x0, y0) = (crop[0], crop[2]);
    let (cx0, cy0) = (x0 / 2, y0 / 2);
    let chroma_width = (x0 + w).div_ceil(2) - cx0;
    let chroma_height = (y0 + h).div_ceil(2) - cy0;
    let narrow = |plane: &[u16], stride: usize, x: usize, y: usize, width: usize, height: usize| {
        let mut out = Vec::with_capacity(width * height);
        for row in 0..height {
            out.extend(
                plane[(y + row) * stride + x..][..width]
                    .iter()
                    .map(|&v| (v >> shift) as u8),
            );
        }
        out
    };
    Planar8 {
        width: w,
        height: h,
        chroma_width,
        chroma_height,
        y: narrow(y, coded_width, x0, y0, w, h),
        cb: narrow(cb, coded_width / 2, cx0, cy0, chroma_width, chroma_height),
        cr: narrow(cr, coded_width / 2, cx0, cy0, chroma_width, chroma_height),
        colour,
    }
}
/// Convert an interleaved planar Y4M frame (`luma`, then `Cb`, then `Cr`) to
/// packed 8-bit RGB using BT.601 limited range. `sx`/`sy` are the chroma
/// subsampling factors (1 or 2). `out` is resized only when the size changes.
#[allow(clippy::too_many_arguments)]
pub fn yuv_to_rgb(
    data: &[u8],
    luma_len: usize,
    chroma_len: usize,
    width: usize,
    height: usize,
    sx: usize,
    sy: usize,
    out: &mut Vec<u8>,
) {
    let len = width * height * 3;
    if out.len() != len {
        out.resize(len, 0);
    }
    for (py, line) in out.chunks_exact_mut(width * 3).enumerate() {
        let uv_row = (py / sy) * (width / sx);
        for (px, pixel) in line.chunks_exact_mut(3).enumerate() {
            let uv = uv_row + px / sx;
            let y = i32::from(data[py * width + px]) - 16;
            let u = i32::from(data[luma_len + uv]) - 128;
            let v = i32::from(data[luma_len + chroma_len + uv]) - 128;
            pixel[0] = ((298 * y + 409 * v + 128) >> 8).clamp(0, 255) as u8;
            pixel[1] = ((298 * y - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
            pixel[2] = ((298 * y + 516 * u + 128) >> 8).clamp(0, 255) as u8;
        }
    }
}
/// A decoded frame before RGB conversion.
pub enum RawFrame {
    Rgb(Vec<u8>),
    Avc {
        picture: Arc<IntraPicture>,
        colour: AvcColour,
    },
    /// Hardware decoder output: already cropped 8-bit planes.
    Planar8(Arc<Planar8>),
    /// Uncompressed Y4M frame (luma, then Cb, then Cr) for off-thread conversion.
    Yuv {
        data: Vec<u8>,
        luma_len: usize,
        chroma_len: usize,
        width: usize,
        height: usize,
        sx: usize,
        sy: usize,
    },
}
impl RawFrame {
    /// Packed 8-bit RGB of the visible picture area.
    pub fn into_rgb(self, budget: usize) -> Result<Vec<u8>> {
        let mut rgb = Vec::new();
        match self {
            RawFrame::Rgb(rgb) => return Ok(rgb),
            RawFrame::Avc { picture, colour } => avc_to_rgb(&picture, colour, &mut rgb, budget)?,
            RawFrame::Planar8(planes) => planar8_to_rgb(&planes, &mut rgb, budget)?,
            RawFrame::Yuv {
                data,
                luma_len,
                chroma_len,
                width,
                height,
                sx,
                sy,
            } => yuv_to_rgb(&data, luma_len, chroma_len, width, height, sx, sy, &mut rgb),
        }
        Ok(rgb)
    }
}
/// Convert the cropped 4:2:0 picture to packed 8-bit RGB into `rgb`, which is
/// resized when the picture size changes.
pub fn avc_to_rgb(
    p: &IntraPicture,
    colour: AvcColour,
    rgb: &mut Vec<u8>,
    budget: usize,
) -> Result<()> {
    let (w, h) = p.dimensions();
    let (x0, y0) = (p.crop[0], p.crop[2]);
    let source = PlaneSource {
        y: &p.y,
        cb: &p.cb,
        cr: &p.cr,
        luma_stride: p.coded_width,
        chroma_stride: p.coded_width / 2,
        x0,
        y0,
        chroma_x0: x0 / 2,
        width: w,
        height: h,
        bit_depth: p.bit_depth,
        colour,
    };
    rgb_from_planes(source, rgb, budget)
}
/// `avc_to_rgb` for packed 8-bit planes.
pub fn planar8_to_rgb(p: &Planar8, rgb: &mut Vec<u8>, budget: usize) -> Result<()> {
    let source = PlaneSource {
        y: &p.y,
        cb: &p.cb,
        cr: &p.cr,
        luma_stride: p.width,
        chroma_stride: p.chroma_width,
        x0: 0,
        y0: 0,
        chroma_x0: 0,
        width: p.width,
        height: p.height,
        bit_depth: 8,
        colour: p.colour,
    };
    rgb_from_planes(source, rgb, budget)
}
/// 4:2:0 planes of any sample width with the visible window to convert.
struct PlaneSource<'a, T> {
    y: &'a [T],
    cb: &'a [T],
    cr: &'a [T],
    luma_stride: usize,
    chroma_stride: usize,
    x0: usize,
    y0: usize,
    chroma_x0: usize,
    width: usize,
    height: usize,
    bit_depth: u8,
    colour: AvcColour,
}
fn rgb_from_planes<T: Copy + Into<f32>>(
    p: PlaneSource<'_, T>,
    rgb: &mut Vec<u8>,
    budget: usize,
) -> Result<()> {
    let AvcColour { kr, kb, full } = p.colour;
    let (w, h) = (p.width, p.height);
    let len = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(3))
        .filter(|n| *n <= budget)
        .ok_or_else(|| invalid("RGB frame exceeds playback budget"))?;
    if rgb.len() != len {
        *rgb = crate::buffer(len)?;
    }
    let scale = f64::from(1u32 << (p.bit_depth - 8));
    let (y_offset, y_range, c_range) = if full {
        (
            0.0,
            f64::from((1u32 << p.bit_depth) - 1),
            f64::from((1u32 << p.bit_depth) - 1),
        )
    } else {
        (16.0 * scale, 219.0 * scale, 224.0 * scale)
    };
    // Fold the range normalisation and the 255 output scale into
    // per-component gains so each pixel is three multiply-adds.
    // G = Y - kr*(2-2kr)/(1-kr-kb) * Cr - kb*(2-2kb)/(1-kr-kb) * Cb.
    let y_gain = (255.0 / y_range) as f32;
    let c_gain = 255.0 / c_range;
    let r_cr = (2.0 * (1.0 - kr) * c_gain) as f32;
    let b_cb = (2.0 * (1.0 - kb) * c_gain) as f32;
    let g_cr = (kr * 2.0 * (1.0 - kr) / (1.0 - kr - kb) * c_gain) as f32;
    let g_cb = (kb * 2.0 * (1.0 - kb) / (1.0 - kr - kb) * c_gain) as f32;
    let (y_offset, c_offset) = (y_offset as f32, (128.0 * scale) as f32);
    // Single-threaded on purpose: spreading this over threads measured
    // slower than the plain loop on a 3-megapixel frame. Each chroma
    // sample is converted once and applied to its two luma columns.
    // Terms are applied in the same order as the per-pixel formula
    // (`luma - g_cr*cr - g_cb*cb`), so results stay bit-identical.
    let store = |pixel: &mut [u8], luma: T, t: (f32, f32, f32, f32)| {
        let luma = (luma.into() - y_offset) * y_gain;
        pixel[0] = (luma + t.0).round().clamp(0.0, 255.0) as u8;
        pixel[1] = (luma - t.1 - t.2).round().clamp(0.0, 255.0) as u8;
        pixel[2] = (luma + t.3).round().clamp(0.0, 255.0) as u8;
    };
    let chroma_terms = |cb: T, cr: T| {
        let cb = cb.into() - c_offset;
        let cr = cr.into() - c_offset;
        (r_cr * cr, g_cr * cr, g_cb * cb, b_cb * cb)
    };
    let odd_start = p.x0 % 2 == 1;
    for (row, line) in rgb.chunks_exact_mut(w * 3).enumerate() {
        let y = row + p.y0;
        let luma_row = &p.y[y * p.luma_stride + p.x0..][..w];
        // Chroma rows follow the picture row (crop included), as before.
        let chroma_row = (y / 2) * p.chroma_stride + p.chroma_x0;
        let cb_row = &p.cb[chroma_row..];
        let cr_row = &p.cr[chroma_row..];
        let mut col = 0;
        let mut chroma = 0;
        if odd_start && w > 0 {
            let t = chroma_terms(cb_row[0], cr_row[0]);
            store(&mut line[..3], luma_row[0], t);
            col = 1;
            chroma = 1;
        }
        while col + 1 < w {
            let t = chroma_terms(cb_row[chroma], cr_row[chroma]);
            store(&mut line[col * 3..][..3], luma_row[col], t);
            store(&mut line[col * 3 + 3..][..3], luma_row[col + 1], t);
            col += 2;
            chroma += 1;
        }
        if col < w {
            let t = chroma_terms(cb_row[chroma], cr_row[chroma]);
            store(&mut line[col * 3..][..3], luma_row[col], t);
        }
    }
    Ok(())
}
/// A single rate-one edit can trim/offset the media timeline without changing
/// the decode sequence. Empty edits and repeated ranges need a richer scheduler.
fn playback_window(
    track: &crate::container::mp4::Track,
    movie_scale: u32,
) -> Result<(i64, Option<i64>)> {
    if track.edits.is_empty() {
        return Ok((0, None));
    }
    if track.edits.len() != 1 || track.edits[0].media_time < 0 || movie_scale == 0 {
        return Err(invalid(
            "multiple or empty MP4 playback edits are not implemented",
        ));
    }
    let edit = &track.edits[0];
    let numerator = u128::from(edit.duration) * u128::from(track.timescale);
    // The scheduler uses integral track ticks. Round the exclusive endpoint
    // upward so a positive fractional interval retains its final sample.
    // The extension is strictly less than one media tick, never a full frame.
    let duration = i64::try_from(numerator.div_ceil(u128::from(movie_scale)))
        .map_err(|_| invalid("MP4 edit duration overflow"))?;
    if duration <= 0 {
        return Err(invalid("empty MP4 playback edit"));
    }
    let end = edit
        .media_time
        .checked_add(duration)
        .ok_or_else(|| invalid("MP4 edit endpoint overflow"))?;
    Ok((edit.media_time, Some(end)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};
    #[test]
    fn images_and_unknown_data_are_not_dispatched_as_mp4() {
        for data in [
            b"\xff\xd8\xff\xe0\x00\x10JFIF".as_slice(),
            b"\x89PNG\r\n\x1a\n",
            b"GIF89a",
            b"not video",
            b"",
        ] {
            let error = match NativeReader::new(Cursor::new(data), 1 << 20) {
                Ok(_) => panic!("non-video accepted"),
                Err(error) => error.to_string(),
            };
            assert!(!error.contains("box"), "{error}");
            assert!(
                error.contains("image") || error.contains("unrecognized video"),
                "{error}"
            );
        }
    }
    #[test]
    fn webm_signature_never_enters_mp4_parser() {
        let bytes = [0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 0x01];
        let error = match NativeReader::new(Cursor::new(bytes), 1 << 20) {
            Ok(_) => panic!("truncated WebM accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("EBML"));
        assert!(!error.contains("box"));
    }
    #[test]
    fn edit_window_preserves_track_units_and_rounds_fractional_end_up() {
        use crate::container::mp4::{Edit, Track};
        let mut track = Track {
            id: 1,
            handler: *b"vide",
            codec: *b"avc1",
            timescale: 12800,
            duration: 6144,
            width: 64,
            height: 64,
            channels: 0,
            sample_rate: 0,
            configuration: vec![],
            edits: vec![],
            samples: vec![],
        };
        assert_eq!(playback_window(&track, 1000).unwrap(), (0, None));
        track.edits.push(Edit {
            duration: 480,
            media_time: 1024,
        });
        assert_eq!(playback_window(&track, 1000).unwrap(), (1024, Some(7168)));
        track.edits[0].duration = 1;
        assert_eq!(playback_window(&track, 1000).unwrap(), (1024, Some(1037)));
        track.edits[0].duration = 0;
        assert!(playback_window(&track, 1000).is_err());
        track.edits[0].duration = 480;
        track.edits[0].media_time = -1;
        assert!(playback_window(&track, 1000).is_err());
        track.edits[0].media_time = i64::MAX;
        assert!(playback_window(&track, 1000).is_err());
        track.edits[0].media_time = 0;
        track.edits.push(track.edits[0].clone());
        assert!(playback_window(&track, 1000).is_err());
    }
    #[test]
    fn format_detection_handles_small_buffers_and_y4m_rewind() {
        let mut bytes = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\nFRAME\n".to_vec();
        bytes.extend_from_slice(&[16, 235, 16, 235, 128, 128]);
        let mut reader =
            NativeReader::new(BufReader::with_capacity(1, Cursor::new(bytes)), 18).unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.dimensions(), [2, 2]);
        assert_eq!(
            reader.rgb(),
            &[0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255]
        );
        assert_eq!(reader.frame_period(), Duration::from_millis(40));
        assert!(!reader.read_frame().unwrap());
        reader.rewind().unwrap();
        assert!(reader.read_frame().unwrap());
    }
}
