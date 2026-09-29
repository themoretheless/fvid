//! Owned crop, flip and nearest-neighbour resize of decoded sample planes.
//! No RGB conversion: high-bit-depth samples keep their original bytes.
use crate::{Result, invalid, playback_native::RawFrame};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, Default)]
pub struct VideoGeometry {
    /// x, y, width, height in luma pixels. Chroma alignment must be exact.
    pub crop: Option<[usize; 4]>,
    pub horizontal_flip: bool,
    pub vertical_flip: bool,
    pub scale: Option<[usize; 2]>,
}

#[derive(Debug)]
pub struct GeometryFrame {
    pub width: usize,
    pub height: usize,
    /// Packed RGB or packed Y, Cb, Cr, matching the source pixel format.
    pub data: Vec<u8>,
}

impl VideoGeometry {
    pub fn is_identity(&self) -> bool {
        self.crop.is_none() && self.scale.is_none() && !self.horizontal_flip && !self.vertical_flip
    }

    /// Crop, reflect, then resize, in that order. Nearest sampling maps output
    /// pixel centres to the containing input pixel; ties choose the higher index.
    pub fn apply(&self, frame: &RawFrame, width: usize, height: usize) -> Result<GeometryFrame> {
        if width == 0 || height == 0 {
            return Err(invalid("empty video geometry"));
        }
        let (data, sx, sy, bytes, rgb) = match frame {
            RawFrame::Rgb(data) => (Cow::Borrowed(data.as_slice()), 1, 1, 3, true),
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
                (
                    Cow::Owned(data),
                    2,
                    2,
                    if picture.bit_depth == 8 { 1 } else { 2 },
                    false,
                )
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
                (Cow::Owned(data), sx, sy, 1, false)
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
                (Cow::Borrowed(data.as_slice()), *sx, *sy, 1, false)
            }
        };
        let [x, y, w, h] = self.crop.unwrap_or([0, 0, width, height]);
        let [ow, oh] = self.scale.unwrap_or([w, h]);
        if w == 0
            || h == 0
            || ow == 0
            || oh == 0
            || x.checked_add(w).is_none_or(|v| v > width)
            || y.checked_add(h).is_none_or(|v| v > height)
        {
            return Err(invalid("crop or scale lies outside video geometry"));
        }
        if x % sx != 0 || y % sy != 0 || w % sx != 0 || h % sy != 0 || ow % sx != 0 || oh % sy != 0
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
        for (dx, dy) in shapes {
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
            let (cw, ch, dw, dh) = (w / dx, h / dy, ow / dx, oh / dy);
            let out_size = dw
                .checked_mul(dh)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("output plane size overflow"))?;
            result
                .try_reserve(out_size)
                .map_err(|_| invalid("cannot allocate transformed frame"))?;
            for row in 0..dh {
                let mut iy = centre(row, ch, dh);
                if self.vertical_flip {
                    iy = ch - 1 - iy;
                }
                for col in 0..dw {
                    let mut ix = centre(col, cw, dw);
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
            data: result,
        })
    }
}

fn centre(index: usize, input: usize, output: usize) -> usize {
    (((index as u128 * 2 + 1) * input as u128) / (output as u128 * 2)) as usize
}
