//! Owned crop, flip, quarter-turn, padding and nearest resize of sample planes.
//! No RGB conversion: high-bit-depth samples keep their original bytes.
use crate::{Result, invalid, playback_native::RawFrame};
use std::borrow::Cow;

/// Quarter-turn direction, optionally reflected vertically after the turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transpose {
    Clock,
    CClock,
    ClockFlip,
    CClockFlip,
}
impl Transpose {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "clock" => Ok(Self::Clock),
            "cclock" => Ok(Self::CClock),
            "clock_flip" => Ok(Self::ClockFlip),
            "cclock_flip" => Ok(Self::CClockFlip),
            _ => Err(invalid(
                "transpose must be clock, cclock, clock_flip, or cclock_flip",
            )),
        }
    }
    fn source(self, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
        match self {
            Self::Clock => (y, h - 1 - x),
            Self::CClock => (w - 1 - y, x),
            Self::ClockFlip => (w - 1 - y, h - 1 - x),
            Self::CClockFlip => (y, x),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VideoGeometry {
    /// x, y, width, height in luma pixels. Chroma alignment must be exact.
    pub crop: Option<[usize; 4]>,
    pub horizontal_flip: bool,
    pub vertical_flip: bool,
    pub scale: Option<[usize; 2]>,
    pub transpose: Option<Transpose>,
    /// Canvas width, height, x, y after transpose and before scale.
    /// Padding is black in the source range, with neutral chroma.
    pub pad: Option<[usize; 4]>,
}

#[derive(Debug)]
pub struct GeometryFrame {
    pub width: usize,
    pub height: usize,
    /// Horizontal/vertical luma samples per chroma sample; None for RGB.
    /// A quarter-turn swaps the axes (4:2:2 becomes 4:4:0).
    pub subsampling: Option<[usize; 2]>,
    /// Packed RGB or packed Y, Cb, Cr. Sample depth is unchanged;
    /// chroma axes follow `subsampling`.
    pub data: Vec<u8>,
}

impl VideoGeometry {
    pub fn is_identity(&self) -> bool {
        self.crop.is_none()
            && self.scale.is_none()
            && self.transpose.is_none()
            && self.pad.is_none()
            && !self.horizontal_flip
            && !self.vertical_flip
    }

    /// Crop, reflect, transpose, pad, then resize, in that order. Nearest sampling maps output
    /// pixel centres to the containing input pixel; ties choose the higher index.
    pub fn apply(&self, frame: &RawFrame, width: usize, height: usize) -> Result<GeometryFrame> {
        self.apply_with_sampling(frame, width, height, false)
    }
    /// Point sampling and pad colour compatible with the existing media adapter.
    /// Native `apply` retains its exact-centre and neutral-chroma contract.
    pub fn apply_media(
        &self,
        frame: &RawFrame,
        width: usize,
        height: usize,
    ) -> Result<GeometryFrame> {
        self.apply_with_sampling(frame, width, height, true)
    }
    fn apply_with_sampling(
        &self,
        frame: &RawFrame,
        width: usize,
        height: usize,
        media: bool,
    ) -> Result<GeometryFrame> {
        if width == 0 || height == 0 {
            return Err(invalid("empty video geometry"));
        }
        let (data, sx, sy, rgb) = match frame {
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            RawFrame::Surface { surface, colour } => {
                let p = crate::playback_native::surface_to_packed(surface, *colour)?;
                if (p.frame.width, p.frame.height) != (width, height) {
                    return Err(invalid("inconsistent surface dimensions"));
                }
                (Cow::Owned(p.frame.data), 2, 2, false)
            }
            RawFrame::Rgb(data) => (Cow::Borrowed(data.as_slice()), 1, 1, true),
            RawFrame::Avc { picture, .. } => {
                let [left, right, top, bottom] = picture.crop;
                if picture
                    .coded_width
                    .checked_sub(left)
                    .and_then(|n| n.checked_sub(right))
                    != Some(width)
                    || picture
                        .coded_height
                        .checked_sub(top)
                        .and_then(|n| n.checked_sub(bottom))
                        != Some(height)
                    || picture.coded_width % 2 != 0
                    || picture.coded_height % 2 != 0
                    || width % 2 != 0
                    || height % 2 != 0
                    || left % 2 != 0
                    || top % 2 != 0
                    || !(8..=16).contains(&picture.bit_depth)
                    || picture.coded_width.checked_mul(picture.coded_height)
                        != Some(picture.y.len())
                    || (picture.coded_width / 2).checked_mul(picture.coded_height / 2)
                        != Some(picture.cb.len())
                    || picture.cb.len() != picture.cr.len()
                {
                    return Err(invalid("invalid coded picture planes"));
                }
                if picture.dimensions() != (width, height) {
                    return Err(invalid("inconsistent picture dimensions"));
                }
                let mut data = Vec::new();
                picture.write_planar(&mut data)?;
                (Cow::Owned(data), 2, 2, false)
            }
            RawFrame::Planar(p) => {
                p.validate()?;
                if (p.frame.width, p.frame.height) != (width, height) {
                    return Err(invalid("inconsistent packed plane dimensions"));
                }
                let [sx, sy] = p
                    .frame
                    .subsampling
                    .ok_or_else(|| invalid("missing planar subsampling"))?;
                (Cow::Borrowed(p.frame.data.as_slice()), sx, sy, false)
            }
            RawFrame::Planar8(p) => {
                if (p.width, p.height) != (width, height) {
                    return Err(invalid("inconsistent plane dimensions"));
                }
                let sx = if p.chroma_width == width {
                    1
                } else if p.chroma_width == width.div_ceil(2) {
                    2
                } else {
                    return Err(invalid("unsupported chroma width"));
                };
                let sy = if p.chroma_height == height {
                    1
                } else if p.chroma_height == height.div_ceil(2) {
                    2
                } else {
                    return Err(invalid("unsupported chroma height"));
                };
                if width.checked_mul(height) != Some(p.y.len())
                    || p.chroma_width.checked_mul(p.chroma_height) != Some(p.cb.len())
                    || p.cb.len() != p.cr.len()
                {
                    return Err(invalid("invalid decoded plane lengths"));
                }
                let mut data = p.y.clone();
                data.extend_from_slice(&p.cb);
                data.extend_from_slice(&p.cr);
                (Cow::Owned(data), sx, sy, false)
            }
            RawFrame::Yuv {
                data,
                width: w,
                height: h,
                sx,
                sy,
                ..
            } => {
                if (*w, *h) != (width, height) || ![1, 2].contains(sx) || ![1, 2].contains(sy) {
                    return Err(invalid("invalid Y4M geometry"));
                }
                (Cow::Borrowed(data.as_slice()), *sx, *sy, false)
            }
        };
        let (depth, full) = match frame {
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            RawFrame::Surface { surface, .. } => (surface.depth(), surface.full_range()),
            RawFrame::Avc { picture, colour } => (picture.bit_depth, colour.full),
            RawFrame::Planar8(p) => (8, p.colour.full),
            RawFrame::Planar(p) => (p.depth, p.colour.full),
            // The legacy untagged Y4M variant represents limited-range samples.
            RawFrame::Yuv { .. } => (8, false),
            RawFrame::Rgb(_) => (8, true),
        };
        self.apply_samples(
            &data,
            width,
            height,
            (!rgb).then_some([sx, sy]),
            depth,
            full,
            media,
        )
    }

    /// Apply user geometry after the container's display rotation. The decoder
    /// keeps AVC/HEVC sample planes in coded orientation even when its reported
    /// dimensions describe the display, so normalize those planes first.
    pub fn apply_display(
        &self,
        frame: &RawFrame,
        width: usize,
        height: usize,
        rotation: u16,
    ) -> Result<GeometryFrame> {
        self.apply_display_with_sampling(frame, width, height, rotation, false)
    }
    /// Display orientation followed by the media-compatible spatial convention.
    pub fn apply_display_media(
        &self,
        frame: &RawFrame,
        width: usize,
        height: usize,
        rotation: u16,
    ) -> Result<GeometryFrame> {
        self.apply_display_with_sampling(frame, width, height, rotation, true)
    }
    fn apply_display_with_sampling(
        &self,
        frame: &RawFrame,
        width: usize,
        height: usize,
        rotation: u16,
        media: bool,
    ) -> Result<GeometryFrame> {
        if rotation == 0 {
            return self.apply_with_sampling(frame, width, height, media);
        }
        let display = match rotation {
            90 => Self {
                transpose: Some(Transpose::Clock),
                ..Default::default()
            },
            180 => Self {
                horizontal_flip: true,
                vertical_flip: true,
                ..Default::default()
            },
            270 => Self {
                transpose: Some(Transpose::CClock),
                ..Default::default()
            },
            _ => return Err(invalid("invalid container display rotation")),
        };
        let (w, h) = if rotation == 180 {
            (width, height)
        } else {
            (height, width)
        };
        let normalized = display.apply_with_sampling(frame, w, h, media)?;
        let (depth, full) = match frame {
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            RawFrame::Surface { surface, .. } => (surface.depth(), surface.full_range()),
            RawFrame::Avc { picture, colour } => (picture.bit_depth, colour.full),
            RawFrame::Planar8(p) => (8, p.colour.full),
            RawFrame::Planar(p) => (p.depth, p.colour.full),
            RawFrame::Rgb(_) => (8, true),
            RawFrame::Yuv { .. } => (8, false),
        };
        if self.is_identity() {
            return Ok(normalized);
        }
        self.apply_samples(
            &normalized.data,
            width,
            height,
            normalized.subsampling,
            depth,
            full,
            media,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_samples(
        &self,
        data: &[u8],
        width: usize,
        height: usize,
        subsampling: Option<[usize; 2]>,
        depth: u8,
        full: bool,
        media: bool,
    ) -> Result<GeometryFrame> {
        let rgb = subsampling.is_none();
        let [sx, sy] = subsampling.unwrap_or([1, 1]);
        let bytes = if rgb {
            3
        } else if depth == 8 {
            1
        } else {
            2
        };
        let [x, y, w, h] = self.crop.unwrap_or([0, 0, width, height]);
        let (tw, th) = if self.transpose.is_some() {
            (h, w)
        } else {
            (w, h)
        };
        let [canvas_w, canvas_h, pad_x, pad_y] = self.pad.unwrap_or([tw, th, 0, 0]);
        if pad_x.checked_add(tw).is_none_or(|n| n > canvas_w)
            || pad_y.checked_add(th).is_none_or(|n| n > canvas_h)
        {
            return Err(invalid("pad must contain the transformed picture"));
        }
        let (out_sx, out_sy) = if self.transpose.is_some() {
            (sy, sx)
        } else {
            (sx, sy)
        };
        let [ow, oh] = self.scale.unwrap_or([canvas_w, canvas_h]);
        if w == 0
            || h == 0
            || ow == 0
            || oh == 0
            || x.checked_add(w).is_none_or(|v| v > width)
            || y.checked_add(h).is_none_or(|v| v > height)
        {
            return Err(invalid("crop or scale lies outside video geometry"));
        }
        if x % sx != 0
            || y % sy != 0
            || w % sx != 0
            || h % sy != 0
            || ow % out_sx != 0
            || oh % out_sy != 0
            || canvas_w % out_sx != 0
            || canvas_h % out_sy != 0
            || pad_x % out_sx != 0
            || pad_y % out_sy != 0
        {
            return Err(invalid("video geometry must align with chroma samples"));
        }
        let shapes = if rgb {
            vec![(1, 1)]
        } else {
            vec![(1, 1), (sx, sy), (sx, sy)]
        };
        let mut result = Vec::new();
        let mut offset: usize = 0;
        for (plane, (dx, dy)) in shapes.into_iter().enumerate() {
            let pw = width.div_ceil(dx);
            let ph = height.div_ceil(dy);
            let size = pw
                .checked_mul(ph)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("plane size overflow"))?;
            let end = offset
                .checked_add(size)
                .ok_or_else(|| invalid("plane size overflow"))?;
            let input = data
                .get(offset..end)
                .ok_or_else(|| invalid("truncated sample plane"))?;
            let (odx, ody) = if self.transpose.is_some() {
                (dy, dx)
            } else {
                (dx, dy)
            };
            let (cw, ch, dw, dh) = (w / dx, h / dy, ow / odx, oh / ody);
            let out_size = dw
                .checked_mul(dh)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("output plane size overflow"))?;
            result
                .try_reserve(out_size)
                .map_err(|_| invalid("cannot allocate transformed frame"))?;
            let black: u16 = if rgb {
                0
            } else if media && !full {
                let code = if plane == 0 { 16u32 } else { 128 };
                ((code * ((1u32 << depth) - 1) + 127) / 255) as u16
            } else if plane != 0 {
                128u16 << (depth - 8)
            } else if full {
                0
            } else {
                16u16 << (depth - 8)
            };
            let fill = if rgb {
                [0, 0, 0]
            } else {
                let b = black.to_le_bytes();
                [b[0], b[1], 0]
            };
            for row in 0..dh {
                let py = sample_index(row, canvas_h / ody, dh, media);
                for col in 0..dw {
                    let px = sample_index(col, canvas_w / odx, dw, media);
                    if px < pad_x / odx
                        || py < pad_y / ody
                        || px - pad_x / odx >= tw / odx
                        || py - pad_y / ody >= th / ody
                    {
                        result.extend_from_slice(&fill[..bytes]);
                        continue;
                    }
                    let (tx, ty) = (px - pad_x / odx, py - pad_y / ody);
                    let (mut ix, mut iy) = self
                        .transpose
                        .map_or((tx, ty), |mode| mode.source(tx, ty, cw, ch));
                    if self.vertical_flip {
                        iy = ch - 1 - iy;
                    }
                    if self.horizontal_flip {
                        ix = cw - 1 - ix;
                    }
                    let at = ((y / dy + iy) * pw + x / dx + ix) * bytes;
                    result.extend_from_slice(&input[at..at + bytes]);
                }
            }
            offset = end;
        }
        if offset != data.len() {
            return Err(invalid("unexpected sample plane length"));
        }
        Ok(GeometryFrame {
            width: ow,
            height: oh,
            subsampling: (!rgb).then_some([out_sx, out_sy]),
            data: result,
        })
    }
}

fn centre(index: usize, input: usize, output: usize) -> usize {
    (((index as u128 * 2 + 1) * input as u128) / (output as u128 * 2)) as usize
}

fn sample_index(index: usize, input: usize, output: usize, media: bool) -> usize {
    if !media {
        return centre(index, input, output);
    }
    let increment = (((input as u128) << 16) + (output as u128 / 2)) / output as u128;
    (((index as u128 * increment + increment / 2) >> 16) as usize).min(input - 1)
}
