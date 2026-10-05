//! Packed planar samples retain codec depth and explicit chroma subsampling.
use super::{AvcColour, Planar8};
use crate::{Result, invalid, native_geometry::GeometryFrame};

#[derive(Debug)]
pub struct PackedPlanar {
    pub frame: GeometryFrame,
    pub depth: u8,
    pub colour: AvcColour,
}
impl PackedPlanar {
    /// Validate storage before using it as a decoded planar picture.
    pub fn new(frame: GeometryFrame, depth: u8, colour: AvcColour) -> Result<Self> {
        let picture = Self {
            frame,
            depth,
            colour,
        };
        picture.layout()?;
        Ok(picture)
    }
    pub(crate) fn validate(&self) -> Result<()> {
        self.layout().map(|_| ())
    }
    fn layout(&self) -> Result<(usize, usize, usize, usize, usize)> {
        let f = &self.frame;
        let AvcColour { kr, kb, .. } = self.colour;
        if !kr.is_finite() || !kb.is_finite() || kr < 0.0 || kb < 0.0 || kr + kb >= 1.0 {
            return Err(invalid("invalid planar colour matrix"));
        }
        let [sx, sy] = f
            .subsampling
            .ok_or_else(|| invalid("planar frame needs chroma geometry"))?;
        if f.width == 0 || f.height == 0 || sx == 0 || sy == 0 || !(8..=16).contains(&self.depth) {
            return Err(invalid("invalid packed planar layout"));
        }
        let luma = f
            .width
            .checked_mul(f.height)
            .ok_or_else(|| invalid("planar size overflow"))?;
        let chroma = f
            .width
            .div_ceil(sx)
            .checked_mul(f.height.div_ceil(sy))
            .ok_or_else(|| invalid("chroma size overflow"))?;
        let bytes = if self.depth == 8 { 1 } else { 2 };
        let total = chroma
            .checked_mul(2)
            .and_then(|c| luma.checked_add(c))
            .and_then(|n| n.checked_mul(bytes));
        if total != Some(f.data.len()) {
            return Err(invalid("invalid packed planar storage"));
        }
        Ok((sx, sy, luma, chroma, bytes))
    }
    pub fn pixel_format(&self) -> Result<String> {
        self.layout()?;
        let name = match self.frame.subsampling {
            Some([1, 1]) => "444",
            Some([2, 1]) => "422",
            Some([2, 2]) => "420",
            Some([1, 2]) => "440",
            Some([4, 1]) => "411",
            Some([4, 4]) => "410",
            _ => return Err(invalid("unsupported planar pixel format name")),
        };
        Ok(if self.depth == 8 {
            format!("yuv{name}p")
        } else {
            format!("yuv{name}p{}le", self.depth)
        })
    }
    fn sample(&self, index: usize, bytes: usize) -> u16 {
        let at = index * bytes;
        if bytes == 1 {
            self.frame.data[at].into()
        } else {
            u16::from_le_bytes([self.frame.data[at], self.frame.data[at + 1]])
        }
    }
    /// Borrow the three encoded planes without narrowing samples or converting colour.
    pub(crate) fn plane_data(&self) -> Result<[&[u8]; 3]> {
        let (_, _, luma, chroma, bytes) = self.layout()?;
        let y_end = luma * bytes;
        let cb_end = y_end + chroma * bytes;
        Ok([
            &self.frame.data[..y_end],
            &self.frame.data[y_end..cb_end],
            &self.frame.data[cb_end..],
        ])
    }
    /// Preserve coded sample precision while removing the AVC/HEVC coded borders.
    pub(crate) fn from_picture(
        p: &crate::codec::avc_picture::IntraPicture,
        colour: AvcColour,
        budget: usize,
    ) -> Result<Self> {
        let [left, right, top, bottom] = p.crop;
        if left % 2 != 0 || top % 2 != 0 || !p.coded_width.is_multiple_of(2) || !p.coded_height.is_multiple_of(2) {
            return Err(invalid("unaligned coded planar crop"));
        }
        let width = p
            .coded_width
            .checked_sub(
                left.checked_add(right)
                    .ok_or_else(|| invalid("crop overflow"))?,
            )
            .filter(|&v| v != 0)
            .ok_or_else(|| invalid("invalid coded crop"))?;
        let height = p
            .coded_height
            .checked_sub(
                top.checked_add(bottom)
                    .ok_or_else(|| invalid("crop overflow"))?,
            )
            .filter(|&v| v != 0)
            .ok_or_else(|| invalid("invalid coded crop"))?;
        let cw = width.div_ceil(2);
        let ch = height.div_ceil(2);
        let length = width
            .checked_mul(height)
            .and_then(|n| {
                cw.checked_mul(ch)
                    .and_then(|c| c.checked_mul(2))
                    .and_then(|c| n.checked_add(c))
            })
            .and_then(|n| n.checked_mul(2))
            .filter(|&n| n <= budget)
            .ok_or_else(|| invalid("packed picture exceeds budget"))?;
        let mut data = Vec::new();
        data.try_reserve_exact(length)
            .map_err(|error| invalid(&error.to_string()))?;
        for (plane, stride, x, y, w, h) in [
            (&p.y, p.coded_width, left, top, width, height),
            (&p.cb, p.coded_width / 2, left / 2, top / 2, cw, ch),
            (&p.cr, p.coded_width / 2, left / 2, top / 2, cw, ch),
        ] {
            for row in 0..h {
                let start = (y + row)
                    .checked_mul(stride)
                    .and_then(|n| n.checked_add(x))
                    .ok_or_else(|| invalid("plane offset overflow"))?;
                let samples = plane
                    .get(
                        start
                            ..start
                                .checked_add(w)
                                .ok_or_else(|| invalid("plane size overflow"))?,
                    )
                    .ok_or_else(|| invalid("truncated coded plane"))?;
                data.extend(samples.iter().flat_map(|v| v.to_le_bytes()));
            }
        }
        Self::new(
            GeometryFrame {
                width,
                height,
                subsampling: Some([2, 2]),
                data,
            },
            p.bit_depth,
            colour,
        )
    }
    /// Rotate source-depth planes by a clockwise quarter turn without narrowing.
    pub fn rotated(&self, rotation: u16) -> Result<Self> {
        if ![0, 90, 180, 270].contains(&rotation) {
            return Err(invalid("invalid planar rotation"));
        }
        let (sx, sy, _, _, bytes) = self.layout()?;
        let data = self.plane_data()?;
        let w = self.frame.width;
        let h = self.frame.height;
        let cw = w.div_ceil(sx);
        let ch = h.div_ceil(sy);
        let quarter = matches!(rotation, 90 | 270);
        let mut output = Vec::with_capacity(self.frame.data.len());
        for (data, width, height) in [(data[0], w, h), (data[1], cw, ch), (data[2], cw, ch)] {
            output.extend(super::rotate_plane(data, width, height, rotation, bytes));
        }
        Self::new(
            GeometryFrame {
                width: if quarter { h } else { w },
                height: if quarter { w } else { h },
                subsampling: Some(if quarter { [sy, sx] } else { [sx, sy] }),
                data: output,
            },
            self.depth,
            self.colour,
        )
    }
    /// Explicit 8-bit presentation conversion; raw/export callers keep `frame`.
    pub fn to_planar8(&self, budget: usize) -> Result<Planar8> {
        let (sx, sy, luma, chroma, bytes) = self.layout()?;
        if luma + 2 * chroma > budget {
            return Err(invalid("planar presentation exceeds budget"));
        }
        let narrow = |start, len| -> Result<Vec<u8>> {
            let mut output = crate::buffer(len)?;
            for (i, out) in output.iter_mut().enumerate() {
                *out = (self.sample(start + i, bytes) >> (self.depth - 8)) as u8;
            }
            Ok(output)
        };
        Ok(Planar8 {
            width: self.frame.width,
            height: self.frame.height,
            chroma_width: self.frame.width.div_ceil(sx),
            chroma_height: self.frame.height.div_ceil(sy),
            y: narrow(0, luma)?,
            cb: narrow(luma, chroma)?,
            cr: narrow(luma + chroma, chroma)?,
            colour: self.colour,
        })
    }
    /// Convert at source precision, with the actual chroma axes and matrix.
    pub fn to_rgb(&self, rgb: &mut Vec<u8>, budget: usize) -> Result<()> {
        let (sx, sy, luma, chroma, bytes) = self.layout()?;
        let len = luma
            .checked_mul(3)
            .filter(|&n| n <= budget)
            .ok_or_else(|| invalid("RGB frame exceeds playback budget"))?;
        if rgb.len() != len {
            *rgb = crate::buffer(len)?;
        }
        let AvcColour { kr, kb, full } = self.colour;
        if !kr.is_finite() || !kb.is_finite() || kr < 0.0 || kb < 0.0 || kr + kb >= 1.0 {
            return Err(invalid("invalid planar colour matrix"));
        }
        let scale = f64::from(1u32 << (self.depth - 8));
        let max = f64::from((1u32 << self.depth) - 1);
        let (offset, yr, cr) = if full {
            (0.0, max, max)
        } else {
            (16.0 * scale, 219.0 * scale, 224.0 * scale)
        };
        let cw = self.frame.width.div_ceil(sx);
        for (i, out) in rgb.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let x = i % self.frame.width;
            let y = i / self.frame.width;
            let c = (y / sy) * cw + x / sx;
            let yy = (f64::from(self.sample(i, bytes)) - offset) / yr;
            let cb = (f64::from(self.sample(luma + c, bytes)) - 128.0 * scale) / cr;
            let cv = (f64::from(self.sample(luma + chroma + c, bytes)) - 128.0 * scale) / cr;
            let r = yy + 2.0 * (1.0 - kr) * cv;
            let b = yy + 2.0 * (1.0 - kb) * cb;
            let g = (yy - kr * r - kb * b) / (1.0 - kr - kb);
            for (out, v) in out.iter_mut().zip([r, g, b]) {
                *out = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        Ok(())
    }
}
