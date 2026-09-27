//! RGB playback adapter for FVid's own Y4M, MP4/AVC/HEVC and WebM/VP9/AV1 readers.
use crate::color::{
    hdr::{ColourDescription, HdrMetadata},
    primaries::MatrixCoeff,
};
use crate::{
    codec::avc_picture::IntraPicture, container::mp4::Limits, invalid, playback::Y4mReader,
    playback_mp4::Mp4VideoReader, Result,
};
use std::{
    io::{BufRead, Read, Seek, SeekFrom},
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
        /// Degrees clockwise the container's transform turns the coded picture,
        /// so the size and the bytes the reader hands over are the ones to show.
        rotation: u16,
    },
}
/// Turn one picture by the quarter turn a container states. `bytes` is the size
/// of a pixel — 1 for a colour plane, 3 for packed RGB — and the two side turns
/// swap the axes, so the result is `height × width` for those.
pub fn rotate_plane(
    data: &[u8],
    width: usize,
    height: usize,
    rotation: u16,
    bytes: usize,
) -> Vec<u8> {
    let (w, h) = match rotation {
        90 | 270 => (height, width),
        _ => (width, height),
    };
    let mut out = vec![0u8; w * h * bytes];
    for (y, row) in out.chunks_exact_mut(w * bytes).enumerate() {
        for (x, pixel) in row.chunks_exact_mut(bytes).enumerate() {
            // Where this pixel was in the picture as decoded.
            let (sx, sy) = match rotation {
                90 => (y, height - 1 - x),
                180 => (width - 1 - x, height - 1 - y),
                270 => (width - 1 - y, x),
                _ => (x, y),
            };
            let from = (sy * width + sx) * bytes;
            pixel.copy_from_slice(&data[from..from + bytes]);
        }
    }
    out
}
/// The same picture turned to be shown upright. Both chroma planes turn with the
/// luma, so a 4:2:0 picture stays aligned with itself however it is set down.
pub fn rotate_planar8(planes: &Planar8, rotation: u16) -> Planar8 {
    let quarter = rotation == 90 || rotation == 270;
    Planar8 {
        width: if quarter { planes.height } else { planes.width },
        height: if quarter { planes.width } else { planes.height },
        chroma_width: if quarter {
            planes.chroma_height
        } else {
            planes.chroma_width
        },
        chroma_height: if quarter {
            planes.chroma_width
        } else {
            planes.chroma_height
        },
        y: rotate_plane(&planes.y, planes.width, planes.height, rotation, 1),
        cb: rotate_plane(
            &planes.cb,
            planes.chroma_width,
            planes.chroma_height,
            rotation,
            1,
        ),
        cr: rotate_plane(
            &planes.cr,
            planes.chroma_width,
            planes.chroma_height,
            rotation,
            1,
        ),
        colour: planes.colour,
    }
}
/// Restart a Matroska stream at the keyframe opening the stretch that holds
/// `target` and decode forward to the frame covering it, in the plane form the
/// decode thread stages. The pre-roll costs what decoding it costs.
fn seek_webm_to<R: Read + Seek>(
    reader: &mut crate::playback_webm::WebmVideoReader<R>,
    target: Duration,
) -> Result<Option<RawFrame>> {
    let nanos = target.as_nanos();
    let target = i64::try_from(nanos).map_err(|_| invalid("seek target overflow"))?;
    reader.seek_to_sync(target);
    let mut last = None;
    loop {
        let Some(planes) = reader.read_frame_planes()? else {
            return Ok(last);
        };
        let frame = RawFrame::Planar8(Arc::new(planes));
        if reader
            .frame_interval()
            .is_some_and(|(_, end, _)| end > nanos)
        {
            return Ok(Some(frame));
        }
        last = Some(frame);
    }
}

/// What the fourcc heading an MP4/MOV video entry is called, spelled the way a
/// viewer's media information spells it rather than the way the container does.
/// The six tags are the four codec families the player builds a decoder for, so
/// no other reaches here: `Mp4VideoReader` refuses the entry before that.
pub fn mp4_codec(fourcc: &[u8; 4]) -> &'static str {
    match fourcc {
        b"avc1" | b"avc3" => "H.264",
        b"hvc1" | b"hev1" => "H.265",
        b"vp09" => "VP9",
        b"av01" => "AV1",
        _ => unreachable!(),
    }
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
        let rotation = track.rotation;
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
            rotation,
        })
    }
    /// The size the picture is shown at: the coded one, turned the way the
    /// container says it is stored.
    pub fn dimensions(&self) -> [usize; 2] {
        match self {
            Self::Y4m(r) => r.dimensions(),
            Self::Webm(r) => r.dimensions(),
            Self::Avc { dimensions, .. } => *dimensions,
        }
    }
    /// Degrees clockwise the container turns the coded picture to be shown
    /// upright. The reader applies it to the bytes it hands over, so a caller
    /// that shapes a `RawFrame` itself has to.
    pub fn rotation(&self) -> u16 {
        match self {
            Self::Y4m(_) | Self::Webm(_) => 0,
            Self::Avc { rotation, .. } => *rotation,
        }
    }
    /// How much wider a coded pixel is than it is tall, as the container states
    /// it. A stream that states nothing is drawn with square pixels.
    pub fn pixel_aspect(&self) -> (u32, u32) {
        match self {
            Self::Y4m(_) => (1, 1),
            Self::Webm(r) => r.pixel_aspect(),
            Self::Avc { source, .. } => source.track().pixel_aspect,
        }
    }
    /// The borders the container asks to be kept off screen, as pixel insets
    /// into the coded frame. Only Matroska has a way to state them; the others
    /// show the whole picture.
    pub fn insets(&self) -> [u32; 4] {
        match self {
            Self::Y4m(_) | Self::Avc { .. } => [0; 4],
            Self::Webm(r) => r.insets(),
        }
    }
    /// Which codec the picture is decoded from, named the way a viewer names it.
    /// A Y4M stream carries finished pixels, so the format it is written in is
    /// the answer.
    pub fn video_codec(&self) -> &'static str {
        match self {
            Self::Y4m(_) => "Y4M",
            Self::Webm(r) => r.codec(),
            Self::Avc { source, .. } => mp4_codec(&source.track().codec),
        }
    }
    /// The signal the source states its pictures in, which is what a grade is
    /// built from: the container's own description first, with the coding's VUI
    /// or sequence header filling any code the container left unspecified.
    ///
    /// A Y4M stream has nowhere to write primaries or a curve, so it states
    /// only what its converter assumes — BT.601 luma weights over a studio
    /// range — and a caller grades the rest by deciding what the picture is.
    /// The coding's half of the answer appears once a parameter set has been
    /// read, so asking after the first frame sees more than asking at open.
    pub fn colour(&self) -> ColourDescription {
        match self {
            Self::Y4m(_) => ColourDescription {
                matrix: MatrixCoeff::Bt601.code(),
                ..Default::default()
            },
            Self::Webm(r) => r.colour().filled_with(r.bitstream_colour()),
            Self::Avc { source, .. } => {
                source.track().colour.filled_with(source.bitstream_colour())
            }
        }
    }
    /// The light the source names for its pictures: a mastering display and a
    /// peak and average level, all empty where nothing is named. The container
    /// states them from its own box — MP4's `mdcv`/`ccll`, Matroska's `Colour`
    /// element — and the coding's SEI message or metadata OBU fills any half the
    /// box left out, which is the whole answer for a file whose HDR was written
    /// only in-band. An HEVC stream's messages are read from its `hvcC` as the
    /// decoder is built, so that half already stands at open; a stream that
    /// writes them only inside its packets states them once one has been decoded.
    pub fn hdr(&self) -> HdrMetadata {
        match self {
            Self::Y4m(_) => HdrMetadata::default(),
            Self::Webm(r) => r.hdr().filled_with(r.bitstream_hdr()),
            Self::Avc { source, .. } => source.track().hdr.filled_with(source.bitstream_hdr()),
        }
    }
    /// The cap this reader sizes a packed RGB picture by, so a caller that
    /// converts one of the reader's own plane frames asks the same question the
    /// reader does. A Y4M stream converts inside its reader, which checked the
    /// frame against the caller's limit when it opened and has no cap left to
    /// hand on.
    pub fn rgb_budget(&self) -> usize {
        match self {
            Self::Y4m(_) => usize::MAX,
            Self::Webm(r) => r.rgb_budget(),
            Self::Avc { rgb_budget, .. } => *rgb_budget,
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
            Self::Webm(r) => r
                .frame_interval()
                .map(|(start, _, scale)| (start as i64, scale)),
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
    /// Whether `seek` can position this stream: MP4 indexes its sync samples and
    /// Matroska marks every block's keyframe, so both can restart decoding at
    /// the preceding keyframe. Y4M carries no index to consult.
    pub fn seekable(&self) -> bool {
        matches!(self, Self::Avc { .. } | Self::Webm(_))
    }
    /// Position playback so the current frame contains `target` (clamped to the
    /// stream). Decoding restarts at the preceding sync sample and runs forward
    /// to the target, so the call takes as long as decoding that stretch.
    /// On failure the stream is rewound to the start.
    pub fn seek(&mut self, target: Duration) -> Result<()> {
        if let Self::Webm(reader) = self {
            let target = i64::try_from(target.as_nanos())
                .map_err(|_| invalid("seek target overflow"))
                .and_then(|target| reader.seek_to_frame(target));
            if target.is_err() {
                self.rewind()?;
            }
            return target;
        }
        let result = (|| {
            if let Some(raw) = self.seek_raw(target)? {
                if let Self::Avc {
                    rgb,
                    rgb_budget,
                    rotation,
                    dimensions,
                    ..
                } = self
                {
                    fill_rgb(raw, rgb, *rgb_budget, *rotation, *dimensions)?;
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
        if let Self::Webm(reader) = self {
            let result = seek_webm_to(reader, target);
            if result.is_err() {
                self.rewind()?;
            }
            return result;
        }
        let Self::Avc {
            source,
            media_start,
            resync,
            ..
        } = self
        else {
            return Err(invalid("seeking is only implemented for MP4 and Matroska"));
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
                    rgb,
                    rgb_budget,
                    rotation,
                    dimensions,
                    ..
                } = self
                else {
                    unreachable!()
                };
                fill_rgb(raw, rgb, *rgb_budget, *rotation, *dimensions)?;
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
            rotation,
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
        // The size the picture is shown at, which for a side turn is the coded
        // one standing on its head.
        *dimensions = if *rotation == 90 || *rotation == 270 {
            [h, w]
        } else {
            [w, h]
        };
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
/// Shape a decoded frame into the packed RGB the reader shows, turned the way the
/// container says its picture is stored. `dimensions` is the size the picture is
/// shown at, so the bytes and the size the reader reports agree.
fn fill_rgb(
    raw: RawFrame,
    rgb: &mut Vec<u8>,
    budget: usize,
    rotation: u16,
    dimensions: [usize; 2],
) -> Result<()> {
    match raw {
        RawFrame::Avc { picture, colour } => avc_to_rgb(&picture, colour, rgb, budget)?,
        RawFrame::Planar8(planes) => planar8_to_rgb(&planes, rgb, budget)?,
        RawFrame::Rgb(bytes) => *rgb = bytes,
        RawFrame::Yuv { .. } => unreachable!(),
    }
    if rotation != 0 {
        // The bytes are still the way they were coded, which for a side turn is
        // the shown size the other way round.
        let [w, h] = dimensions;
        let (cw, ch) = if rotation == 90 || rotation == 270 {
            (h, w)
        } else {
            (w, h)
        };
        let turned = rotate_plane(rgb, cw, ch, rotation, 3);
        *rgb = turned;
    }
    Ok(())
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
fn rgb_from_planes<T: Copy + Into<f32> + Sync>(
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
    // Each chroma sample is converted once and applied to its two luma columns.
    // Rows never share a write, so the row loop is cut between workers the way
    // the grade is: this measured 4.5 ms for a 1080p frame and 16.1 ms for a 4K
    // one on one thread, and 0.8 ms and 2.2 ms once cut, so what the frame waited
    // on was the arithmetic rather than the memory. Terms are applied in the same
    // order as the per-pixel formula (`luma - g_cr*cr - g_cb*cb`), and
    // `a_frame_split_between_workers_matches_its_own_strips` holds the cut to the
    // bytes the same rows give a single thread.
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
    let rows = |first: usize, span: &mut [u8]| {
        for (offset, line) in span.chunks_exact_mut(w * 3).enumerate() {
            let y = first + offset + p.y0;
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
    };
    let rgb = rgb.as_mut_slice();
    let workers = crate::span_workers(len);
    if workers < 2 {
        rows(0, rgb);
        return Ok(());
    }
    let rows_per_span = h.div_ceil(workers);
    std::thread::scope(|scope| {
        for (index, span) in rgb.chunks_mut(rows_per_span * w * 3).enumerate() {
            scope.spawn(move || rows(index * rows_per_span, span));
        }
    });
    Ok(())
}
/// A single rate-one edit can trim/offset the media timeline without changing
/// the decode sequence. Empty edits and repeated ranges need a richer scheduler.
fn playback_window(
    track: &crate::container::mp4::Track,
    movie_scale: u32,
) -> Result<(i64, Option<i64>)> {
    // Leading empty edits only delay the track's start; QuickTime writers
    // emit them routinely. Playback starts at the first media edit instead.
    let edits: Vec<_> = track
        .edits
        .iter()
        .skip_while(|edit| edit.media_time < 0)
        .collect();
    if edits.is_empty() {
        return Ok((0, None));
    }
    if movie_scale == 0 {
        return Err(invalid("MP4 movie timescale is zero"));
    }
    let ticks = |duration: u64| {
        u128::from(duration) * u128::from(track.timescale) / u128::from(movie_scale)
    };
    // Back-to-back edits that continue the media timeline play as one window.
    for pair in edits.windows(2) {
        let expected = i128::from(pair[0].media_time) + ticks(pair[0].duration) as i128;
        if pair[1].media_time < 0 || (i128::from(pair[1].media_time) - expected).abs() > 1 {
            return Err(invalid(
                "non-contiguous MP4 playback edits are not implemented",
            ));
        }
    }
    let edit = crate::container::mp4::Edit {
        duration: edits.iter().map(|edit| edit.duration).sum(),
        media_time: edits[0].media_time,
    };
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

    /// A frame cut between workers has to hold the same bytes as the same rows
    /// converted by a single thread, because no two rows share a write. The
    /// strips below are each exactly the size that keeps one worker, so they
    /// stand as the serial picture of the parallel one.
    #[test]
    fn a_frame_split_between_workers_matches_its_own_strips() {
        let (w, h) = (4096, 2160);
        let planes = Planar8 {
            width: w,
            height: h,
            chroma_width: w / 2,
            chroma_height: h / 2,
            y: (0..w * h).map(|i| (i % 251) as u8).collect(),
            cb: (0..w * h / 4).map(|i| (i * 3) as u8).collect(),
            cr: (0..w * h / 4).map(|i| (i * 7) as u8).collect(),
            colour: AvcColour::default(),
        };
        let mut whole = Vec::new();
        planar8_to_rgb(&planes, &mut whole, w * h * 3).unwrap();
        assert!(crate::span_workers(w * h * 3) > 1);
        let mut strips = Vec::new();
        for band in 0..h / 4 {
            let part = Planar8 {
                width: w,
                height: 4,
                chroma_width: w / 2,
                chroma_height: 2,
                y: planes.y[band * 4 * w..][..4 * w].to_vec(),
                cb: planes.cb[band * (w / 2)..][..2 * (w / 2)].to_vec(),
                cr: planes.cr[band * (w / 2)..][..2 * (w / 2)].to_vec(),
                colour: AvcColour::default(),
            };
            assert_eq!(crate::span_workers(part.width * part.height * 3), 1);
            let mut rgb = Vec::new();
            planar8_to_rgb(&part, &mut rgb, w * h * 3).unwrap();
            strips.extend_from_slice(&rgb);
        }
        assert_eq!(whole, strips);
    }

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
    /// The tags the player builds a decoder for, read out as the names a viewer
    /// shows rather than the four bytes the container stores.
    #[test]
    fn video_fourccs_read_as_the_names_a_viewer_shows() {
        for (fourcc, name) in [
            (b"avc1", "H.264"),
            (b"avc3", "H.264"),
            (b"hvc1", "H.265"),
            (b"hev1", "H.265"),
            (b"vp09", "VP9"),
            (b"av01", "AV1"),
        ] {
            assert_eq!(mp4_codec(&fourcc), name, "{fourcc:?}");
        }
    }
    #[test]
    fn edit_window_preserves_track_units_and_rounds_fractional_end_up() {
        use crate::container::mp4::{Edit, SampleIndex, Track};
        let mut track = Track {
            id: 1,
            handler: *b"vide",
            codec: *b"avc1",
            name: String::new(),
            language: String::new(),
            timescale: 12800,
            duration: 6144,
            width: 64,
            height: 64,
            channels: 0,
            sample_rate: 0,
            bit_depth: 0,
            configuration: vec![],
            edits: vec![],
            samples: SampleIndex::Expanded(vec![]),
            pixel_aspect: (1, 1),
            rotation: 0,
            colour: Default::default(),
            hdr: Default::default(),
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
        assert_eq!(playback_window(&track, 1000).unwrap(), (0, None));
        // A leading delay is skipped; contiguous edits join into one window.
        track.edits.push(Edit {
            duration: 240,
            media_time: 1024,
        });
        track.edits.push(Edit {
            duration: 240,
            media_time: 1024 + 3072,
        });
        assert_eq!(playback_window(&track, 1000).unwrap(), (1024, Some(7168)));
        track.edits.truncate(1);
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
    /// The shape of a pixel belongs to the container, not to the decoder: two
    /// files of one 64x64 source, each muxed to be drawn twice as wide as it is
    /// stored, and a file that says nothing.
    ///
    /// ```text
    /// ffmpeg -f lavfi -i testsrc2=size=64x64:rate=25:duration=0.4 -vf setsar=2 \
    ///   -c:v libx264 -pix_fmt yuv420p tests/fixtures/display/par-2x1.mp4
    /// ffmpeg -f lavfi -i testsrc2=size=64x64:rate=25:duration=0.4 -vf setsar=2 \
    ///   -c:v libvpx-vp9 -b:v 0 -crf 60 -deadline realtime \
    ///   tests/fixtures/display/par-2x1.webm
    /// ```
    #[test]
    fn an_item_states_the_shape_of_its_pixels() {
        let aspect = |bytes: &[u8]| {
            NativeReader::new(BufReader::new(Cursor::new(bytes)), 1 << 20)
                .unwrap()
                .pixel_aspect()
        };
        assert_eq!(
            aspect(include_bytes!("../tests/fixtures/display/par-2x1.mp4")),
            (2, 1)
        );
        assert_eq!(
            aspect(include_bytes!("../tests/fixtures/display/par-2x1.webm")),
            (2, 1)
        );
        // Nothing stated is square pixels, which is what the overwhelming
        // majority of files mean; a guessed shape would stretch all of them.
        assert_eq!(
            aspect(include_bytes!("../tests/fixtures/vp9/motion.webm")),
            (1, 1)
        );
        let raw = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\nFRAME\n";
        assert_eq!(
            NativeReader::new(BufReader::new(Cursor::new(raw)), 18)
                .unwrap()
                .pixel_aspect(),
            (1, 1)
        );
    }
    /// Every turn a container can ask for, on a grid whose pixels are
    /// distinguishable and whose edges are not: 2 wide, 3 tall, numbered as
    /// stored.
    ///
    /// ```text
    /// 1 2
    /// 3 4
    /// 5 6
    /// ```
    #[test]
    fn a_turn_moves_every_pixel_to_its_place() {
        let stored = [1u8, 2, 3, 4, 5, 6];
        assert_eq!(rotate_plane(&stored, 2, 3, 0, 1), stored);
        assert_eq!(rotate_plane(&stored, 2, 3, 90, 1), [5, 3, 1, 6, 4, 2]);
        assert_eq!(rotate_plane(&stored, 2, 3, 180, 1), [6, 5, 4, 3, 2, 1]);
        assert_eq!(rotate_plane(&stored, 2, 3, 270, 1), [2, 4, 6, 1, 3, 5]);
        // Four quarter turns, each over the size the previous one left, are the
        // picture again.
        let once = rotate_plane(&stored, 2, 3, 90, 1);
        let twice = rotate_plane(&once, 3, 2, 90, 1);
        let thrice = rotate_plane(&twice, 2, 3, 90, 1);
        assert_eq!(rotate_plane(&thrice, 3, 2, 90, 1), stored);
        // A pixel wider than one byte travels whole: no turn shears a channel
        // away from its two neighbours.
        let rgb: Vec<u8> = (1..=12).collect();
        assert_eq!(
            rotate_plane(&rgb, 2, 2, 90, 3),
            [7, 8, 9, 1, 2, 3, 10, 11, 12, 4, 5, 6]
        );
    }
    /// Planes turn with their own geometry: the luma grid and the two smaller
    /// chroma grids each go through the same turn, and the sizes that describe
    /// them change with them.
    #[test]
    fn planes_turn_with_their_own_geometry() {
        let y: Vec<u8> = (1..=24u8).collect();
        let cb: Vec<u8> = (101..=106u8).collect();
        let cr: Vec<u8> = (201..=206u8).collect();
        let planes = Planar8 {
            width: 4,
            height: 6,
            chroma_width: 2,
            chroma_height: 3,
            y: y.clone(),
            cb: cb.clone(),
            cr: cr.clone(),
            colour: AvcColour::default(),
        };
        let turned = rotate_planar8(&planes, 90);
        assert_eq!((turned.width, turned.height), (6, 4));
        assert_eq!((turned.chroma_width, turned.chroma_height), (3, 2));
        assert_eq!(turned.y, rotate_plane(&y, 4, 6, 90, 1));
        assert_eq!(turned.cb, rotate_plane(&cb, 2, 3, 90, 1));
        assert_eq!(turned.cr, rotate_plane(&cr, 2, 3, 90, 1));
        // A stated nothing leaves the bytes where they are.
        let same = rotate_planar8(&planes, 0);
        assert_eq!((same.y, same.cb, same.cr), (y.clone(), cb.clone(), cr));
        assert_eq!((same.width, same.height), (4, 6));
        // The colour the decoder carried travels with the pixels.
        assert_eq!(turned.colour.kr, planes.colour.kr);
        assert_eq!(turned.colour.kb, planes.colour.kb);
        assert_eq!(turned.colour.full, planes.colour.full);
    }
    /// The fixture of `an_item_states_the_shape_of_its_pixels` again with its
    /// `tkhd` transform set to a quarter turn, which is the shape of a file
    /// from a camera held sideways. No muxer here writes one: ffmpeg drops the
    /// rotation option it once had, so the header is patched by hand over the
    /// bytes it did write.
    #[test]
    fn a_turned_item_is_shown_upright() {
        const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/display/par-2x1.mp4");
        let mut upright = NativeReader::new(BufReader::new(Cursor::new(FIXTURE)), 1 << 20).unwrap();
        assert_eq!(upright.rotation(), 0);
        assert!(upright.read_frame().unwrap());
        assert_eq!(upright.dimensions(), [64, 64]);
        let rgb = upright.rgb().to_vec();
        let mut turned = FIXTURE.to_vec();
        let matrix = turned.windows(4).position(|w| w == b"tkhd").unwrap() + 4 + 40;
        for (at, value) in [(0, 0u32), (4, 0x0001_0000), (12, 0xFFFF_0000), (16, 0)] {
            turned[matrix + at..matrix + at + 4].copy_from_slice(&value.to_be_bytes());
        }
        let mut reader = NativeReader::new(BufReader::new(Cursor::new(turned)), 1 << 20).unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.rotation(), 90);
        // A square picture keeps its size through a quarter turn; the shape of
        // a stored pixel, which the turn stands on its side, does not.
        assert_eq!(reader.dimensions(), [64, 64]);
        assert_eq!(reader.pixel_aspect(), (1, 2));
        assert_eq!(reader.rgb(), rotate_plane(&rgb, 64, 64, 90, 3));
        // The test would pass on a symmetric picture without saying anything.
        assert_ne!(reader.rgb(), rgb.as_slice());
    }

    /// BT.601 is the only thing a Y4M stream can state about its colour: the
    /// format has no field for primaries or a curve, and the weights its
    /// converter applies are that matrix's. So the reader says what is known
    /// and leaves the rest for a caller to decide.
    #[test]
    fn a_y4m_stream_states_only_the_matrix_its_converter_uses() {
        let bytes = b"YUV4MPEG2 W2 H2 F60:1 Ip C420jpeg\nFRAME\n\x10\x20\x30\x40\x80\x80";
        let reader = NativeReader::without_memory_limit(Cursor::new(bytes.to_vec())).unwrap();
        assert_eq!(
            reader.colour(),
            ColourDescription {
                matrix: MatrixCoeff::Bt601.code(),
                ..Default::default()
            }
        );
        assert!(!reader.colour().is_hdr());
        assert!(reader.hdr().is_empty());
    }

    /// A real HDR10 file, muxed by FFmpeg from synthetic input: Main10 HEVC whose
    /// VUI names BT.2020 primaries, a PQ curve and the BT.2020-NCL matrix. The
    /// muxer wrote no `colr` atom for it — counted in the file's bytes, zero — so
    /// the whole statement comes out of the parameter set, which is the case a
    /// reader has to reach into the coding for.
    #[test]
    fn a_real_hdr10_files_signal_reaches_the_caller_that_grades_it() {
        use crate::color::{Primaries, Transfer};
        let data = include_bytes!("../tests/fixtures/hevc/hdr10.mp4").to_vec();
        let mut reader = NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
        let stated = reader.colour();
        assert_eq!(
            stated,
            ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: false,
            }
        );
        assert!(stated.is_hdr());
        assert_eq!(stated.primary_set(), Some(Primaries::BT2020));
        assert_eq!(stated.transfer_function(), Transfer::Pq);
        assert_eq!(stated.matrix_coefficients(), Some(MatrixCoeff::Bt2020Ncl));
        // Decoding a whole clip does not change what it is stated in.
        for _ in 0..5 {
            assert!(reader.read_frame_raw().unwrap().is_some());
        }
        assert_eq!(reader.colour(), stated);
    }

    /// The same file's light, which the container writes in no box at all: SEI
    /// 137 and 144, carried by the configuration record the decoder is built
    /// from. So the reader states them the moment the file is open — the point a
    /// tone map needs them, before a single packet has been decoded — and a
    /// container that named none has nothing to contradict them with.
    #[test]
    fn a_real_hdr10_files_light_reaches_the_caller_that_tone_maps_it() {
        let data = include_bytes!("../tests/fixtures/hevc/hdr10.mp4").to_vec();
        let reader = NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
        let hdr = reader.hdr();
        assert!(hdr.mastering.unwrap().is_hdr10());
        assert_eq!((hdr.light.max_cll, hdr.light.max_fall), (1_000.0, 400.0));
        // The stated peak, not a fallback, is what a tone map compresses from.
        assert_eq!(hdr.content_light(400.0).max_cll, 1_000.0);
    }

    /// A master that writes its volume and no content light, which is what most
    /// encoders produce: `x265` needs `--max-cll` spelled out, while
    /// `--master-display` alone is enough for HDR10. Nothing in the file states
    /// a content peak, so the peak the grade compresses from has to come from
    /// the authored volume — a caller that only knows its own 100-nit panel would
    /// otherwise treat a 1 000 cd/m² master as if it already fit.
    #[test]
    fn a_volume_without_a_content_light_grades_against_its_own_peak() {
        use crate::color::{DisplayTarget, Grade, Settings};
        let data = include_bytes!("../tests/fixtures/hevc/mdcv-only.mp4").to_vec();
        let mut reader = NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
        let hdr = reader.hdr();
        let volume = hdr.mastering.unwrap();
        assert!(volume.is_hdr10());
        assert_eq!(volume.max_luminance, 1_000.0);
        assert!(hdr.light.max_cll == 0.0 && hdr.light.max_fall == 0.0);
        assert_eq!(hdr.content_light(100.0).max_cll, 1_000.0);
        let grade = Grade::new(
            reader.colour(),
            &hdr,
            Settings::video(DisplayTarget::sdr(100.0)),
            None,
        );
        assert_eq!(grade.plan().content.max_cll, 1_000.0);
        // Decoding the packets that repeat the messages states the same light.
        for _ in 0..5 {
            assert!(reader.read_frame_raw().unwrap().is_some());
        }
        assert_eq!(reader.hdr(), hdr);
    }

    /// An HLG clip, stated the same way the HDR10 one is — in the parameter set,
    /// with no `colr` atom in the file to read — except that here the coding says
    /// everything and means by it something the reader must not over-read: HLG
    /// carries no mastering volume and no content light, because the format's
    /// scene light is normalised to whatever panel shows it. So the triple comes
    /// through and the light stays empty, which is what leaves the grade with the
    /// panel's own peak rather than a headroom the file never claimed.
    #[test]
    fn a_real_hlg_files_signal_reaches_the_caller_that_grades_it() {
        use crate::color::{Primaries, Transfer};
        let data = include_bytes!("../tests/fixtures/hevc/hlg.mp4").to_vec();
        let mut reader = NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
        let stated = reader.colour();
        assert_eq!(
            stated,
            ColourDescription {
                primaries: 9,
                transfer: 18,
                matrix: 9,
                full_range: false,
            }
        );
        assert!(stated.is_hdr());
        assert_eq!(stated.transfer_function(), Transfer::Hlg);
        assert_eq!(stated.primary_set(), Some(Primaries::BT2020));
        assert_eq!(stated.matrix_coefficients(), Some(MatrixCoeff::Bt2020Ncl));
        // Nothing about light is stated, in either direction.
        let hdr = reader.hdr();
        assert!(hdr.is_empty());
        assert!(hdr.mastering.is_none());
        // Decoding the whole clip changes neither half.
        for _ in 0..5 {
            assert!(reader.read_frame_raw().unwrap().is_some());
        }
        assert_eq!(reader.colour(), stated);
        assert!(reader.hdr().is_empty());
    }

    /// An AV1 track whose WebM header writes no `Colour` element: every code the
    /// container states is zero, which is silence rather than a value, and once a
    /// frame has been decoded the sequence header's own triple is what the reader
    /// hands over. AV1's default is 2/2/2, and H.273 numbers 2 *unspecified*
    /// rather than absent, so the zeroes become 2s and stay unknown: the reader
    /// reports what the stream says instead of guessing on its behalf.
    #[test]
    fn a_codings_own_statement_replaces_a_containers_silence() {
        let data = include_bytes!("../tests/fixtures/av1/ramp.webm").to_vec();
        let mut reader = NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
        assert_eq!(reader.colour(), ColourDescription::default());
        assert!(reader.read_frame_raw().unwrap().is_some());
        assert_eq!(
            reader.colour(),
            ColourDescription {
                primaries: 2,
                transfer: 2,
                matrix: 2,
                full_range: false,
            }
        );
        assert!(!reader.colour().is_hdr());
    }
}
