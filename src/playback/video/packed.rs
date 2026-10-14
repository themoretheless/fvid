//! Packed samples retain codec depth: planar YUV or interleaved identity RGB.
use super::{AvcColour, Planar8};
use crate::{Result, invalid, native_geometry::GeometryFrame};

#[derive(Debug)]
pub struct PackedPlanar {
    pub frame: GeometryFrame,
    /// Unassociated, full-resolution opacity at `depth` (LE words above 8 bit).
    pub alpha: Option<Vec<u8>>,
    pub depth: u8,
    pub colour: AvcColour,
}
impl PackedPlanar {
    /// Validate storage before using it as a decoded planar picture.
    pub fn new(frame: GeometryFrame, depth: u8, colour: AvcColour) -> Result<Self> {
        Self::with_alpha(frame, depth, colour, None)
    }
    pub fn with_alpha(
        frame: GeometryFrame,
        depth: u8,
        colour: AvcColour,
        alpha: Option<Vec<u8>>,
    ) -> Result<Self> {
        let picture = Self {
            frame,
            alpha,
            depth,
            colour,
        };
        picture.validate()?;
        Ok(picture)
    }
    pub(crate) fn validate(&self) -> Result<()> {
        self.layout()?;
        if let Some(alpha) = &self.alpha {
            let bytes = if self.depth == 8 { 1 } else { 2 };
            if self
                .frame
                .width
                .checked_mul(self.frame.height)
                .and_then(|n| n.checked_mul(bytes))
                != Some(alpha.len())
            {
                return Err(invalid("invalid packed alpha storage"));
            }
            let max = (1u32 << self.depth) - 1;
            if bytes == 2
                && alpha
                    .chunks_exact(2)
                    .any(|s| u32::from(u16::from_le_bytes([s[0], s[1]])) > max)
            {
                return Err(invalid("alpha sample exceeds coded depth"));
            }
        }
        Ok(())
    }
    fn layout(&self) -> Result<(usize, usize, usize, usize, usize)> {
        let f = &self.frame;
        let AvcColour { kr, kb, .. } = self.colour;
        if !kr.is_finite() || !kb.is_finite() || kr < 0.0 || kb < 0.0 || kr + kb >= 1.0 {
            return Err(invalid("invalid planar colour matrix"));
        }
        if f.subsampling.is_none() {
            let luma = f
                .width
                .checked_mul(f.height)
                .filter(|_| f.width != 0 && f.height != 0 && (8..=16).contains(&self.depth))
                .ok_or_else(|| invalid("invalid packed RGB layout"))?;
            let bytes = if self.depth == 8 { 1 } else { 2 };
            if luma.checked_mul(3).and_then(|n| n.checked_mul(bytes)) != Some(f.data.len()) {
                return Err(invalid("invalid packed RGB storage"));
            }
            return Ok((0, 0, luma, 0, bytes));
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
        if self.alpha.is_some() {
            return Err(invalid(
                "separate alpha storage needs explicit RGBA conversion",
            ));
        }
        self.layout()?;
        if self.frame.subsampling.is_none() {
            // Storage uses 8-bit bytes or LE words; `depth` retains coded precision.
            return Ok(if self.depth == 8 { "rgb24" } else { "rgb48le" }.into());
        }
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
        if self.alpha.is_some() {
            return Err(invalid("three-plane presentation cannot discard alpha"));
        }
        self.colour_plane_data()
    }
    fn colour_plane_data(&self) -> Result<[&[u8]; 3]> {
        if self.frame.subsampling.is_none() {
            return Err(invalid("RGB storage is interleaved, not YUV planes"));
        }
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
        let depth=p.bit_depth.max(p.chroma_bit_depth);
        if !(8..=16).contains(&p.bit_depth) || !(8..=16).contains(&p.chroma_bit_depth) {
            return Err(invalid("invalid AVC component depth"));
        }
        let bytes=if depth==8 {1}else{2};
        let [left, right, top, bottom] = p.crop;
        if left % 2 != 0
            || top % 2 != 0
            || !p.coded_width.is_multiple_of(2)
            || !p.coded_height.is_multiple_of(2)
        {
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
            .and_then(|n| n.checked_mul(bytes))
            .filter(|&n| n <= budget)
            .ok_or_else(|| invalid("packed picture exceeds budget"))?;
        let mut data = Vec::new();
        data.try_reserve_exact(length)
            .map_err(|error| invalid(&error.to_string()))?;
        for (plane, stride, x, y, w, h, source_depth) in [
            (&p.y, p.coded_width, left, top, width, height,p.bit_depth),
            (&p.cb, p.coded_width / 2, left / 2, top / 2, cw, ch,p.chroma_bit_depth),
            (&p.cr, p.coded_width / 2, left / 2, top / 2, cw, ch,p.chroma_bit_depth),
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
                for &v in samples {
                    if u32::from(v)>=(1u32<<source_depth) {return Err(invalid("AVC sample exceeds component depth"));}
                    let normalized=v<<(depth-source_depth);
                    if bytes==1 {data.push(normalized as u8);} else {data.extend_from_slice(&normalized.to_le_bytes());}
                }
            }
        }
        Self::new(
            GeometryFrame {
                width,
                height,
                subsampling: Some([2, 2]),
                data,
            },
            depth,
            colour,
        )
    }
    /// Rotate source-depth planes by a clockwise quarter turn without narrowing.
    pub fn rotated(&self, rotation: u16) -> Result<Self> {
        self.validate()?;
        if ![0, 90, 180, 270].contains(&rotation) {
            return Err(invalid("invalid planar rotation"));
        }
        let (sx, sy, _, _, bytes) = self.layout()?;
        let alpha = self
            .alpha
            .as_ref()
            .map(|a| super::rotate_plane(a, self.frame.width, self.frame.height, rotation, bytes));
        if self.frame.subsampling.is_none() {
            let quarter = matches!(rotation, 90 | 270);
            return Self::with_alpha(
                GeometryFrame {
                    width: if quarter {
                        self.frame.height
                    } else {
                        self.frame.width
                    },
                    height: if quarter {
                        self.frame.width
                    } else {
                        self.frame.height
                    },
                    subsampling: None,
                    data: super::rotate_plane(
                        &self.frame.data,
                        self.frame.width,
                        self.frame.height,
                        rotation,
                        3 * bytes,
                    ),
                },
                self.depth,
                self.colour,
                alpha,
            );
        }
        let data = self.colour_plane_data()?;
        let w = self.frame.width;
        let h = self.frame.height;
        let cw = w.div_ceil(sx);
        let ch = h.div_ceil(sy);
        let quarter = matches!(rotation, 90 | 270);
        let mut output = Vec::with_capacity(self.frame.data.len());
        for (data, width, height) in [(data[0], w, h), (data[1], cw, ch), (data[2], cw, ch)] {
            output.extend(super::rotate_plane(data, width, height, rotation, bytes));
        }
        Self::with_alpha(
            GeometryFrame {
                width: if quarter { h } else { w },
                height: if quarter { w } else { h },
                subsampling: Some(if quarter { [sy, sx] } else { [sx, sy] }),
                data: output,
            },
            self.depth,
            self.colour,
            alpha,
        )
    }
    /// Explicit 8-bit presentation conversion; raw/export callers keep `frame`.
    pub fn to_planar8(&self, budget: usize) -> Result<Planar8> {
        if self.alpha.is_some() {
            return Err(invalid("planar8 conversion cannot discard alpha"));
        }
        if self.frame.subsampling.is_none() {
            return Err(invalid("RGB samples cannot be borrowed as YUV planes"));
        }
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
        if self.alpha.is_some() {
            return Err(invalid("RGB conversion cannot discard alpha; use RGBA"));
        }
        self.colour_to_rgb(rgb, budget)
    }
    /// Explicit presentation onto the player's opaque black backdrop.
    pub fn to_rgb_on_black(&self, rgb: &mut Vec<u8>, budget: usize) -> Result<()> {
        self.validate()?;
        self.colour_to_rgb(rgb, budget)?;
        if let Some(alpha) = &self.alpha {
            let max = (1u32 << self.depth) - 1;
            for (i, pixel) in rgb.chunks_exact_mut(3).enumerate() {
                let a = if self.depth == 8 {
                    u32::from(alpha[i])
                } else {
                    u32::from(u16::from_le_bytes([alpha[2 * i], alpha[2 * i + 1]]))
                };
                for channel in pixel {
                    *channel = ((u32::from(*channel) * a + max / 2) / max) as u8;
                }
            }
        }
        Ok(())
    }
    /// Convert to unassociated 8-bit RGBA. Color beneath zero opacity is retained.
    /// The budget includes both the output and the intermediate RGB buffer.
    pub fn to_rgba(&self, rgba: &mut Vec<u8>, budget: usize) -> Result<()> {
        self.validate()?;
        let count = self
            .frame
            .width
            .checked_mul(self.frame.height)
            .ok_or_else(|| invalid("RGBA frame size overflow"))?;
        let size = count
            .checked_mul(4)
            .filter(|_| count.checked_mul(7).is_some_and(|n| n <= budget))
            .ok_or_else(|| invalid("RGBA frame exceeds playback budget"))?;
        let mut rgb = Vec::new();
        self.colour_to_rgb(&mut rgb, budget - size)?;
        if rgba.len() != size {
            *rgba = crate::buffer(size)?;
        }
        let max = (1u32 << self.depth) - 1;
        for (i, (rgb, out)) in rgb
            .chunks_exact(3)
            .zip(rgba.chunks_exact_mut(4))
            .enumerate()
        {
            out[..3].copy_from_slice(rgb);
            out[3] = match &self.alpha {
                None => 255,
                Some(a) => {
                    let value = if self.depth == 8 {
                        u32::from(a[i])
                    } else {
                        u32::from(u16::from_le_bytes([a[2 * i], a[2 * i + 1]]))
                    };
                    ((value * 255 + max / 2) / max) as u8
                }
            };
        }
        Ok(())
    }
    fn colour_to_rgb(&self, rgb: &mut Vec<u8>, budget: usize) -> Result<()> {
        let (sx, sy, luma, chroma, bytes) = self.layout()?;
        let len = luma
            .checked_mul(3)
            .filter(|&n| n <= budget)
            .ok_or_else(|| invalid("RGB frame exceeds playback budget"))?;
        if rgb.len() != len {
            *rgb = crate::buffer(len)?;
        }
        if self.frame.subsampling.is_none() {
            let max = (1u32 << self.depth) - 1;
            for (i, out) in rgb.iter_mut().enumerate() {
                let value = u32::from(self.sample(i, bytes));
                if value > max {
                    return Err(invalid("RGB sample exceeds coded depth"));
                }
                *out = ((value * 255 + max / 2) / max) as u8;
            }
            return Ok(());
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

/// Preserve HEVC chroma geometry and component precision for display.
/// Lower-depth components are lifted exactly to the highest component depth.
pub fn hevc_picture(
    p: &crate::codec::hevc_picture::Picture,
    format: u8,
    colour: AvcColour,
) -> Result<PackedPlanar> {
    let depth = if format == 0 {
        p.depth[0]
    } else {
        p.depth[0].max(p.depth[1])
    };
    let sub = match format {
        0 | 1 => [2, 2],
        2 => [2, 1],
        3 => [1, 1],
        _ => return Err(invalid("invalid HEVC display chroma format")),
    };
    let [left, right, top, bottom] = p.crop.map(|v| v as usize);
    let width = (p.dimensions[0] as usize)
        .checked_sub(left + right)
        .ok_or_else(|| invalid("HEVC display crop exceeds width"))?;
    let height = (p.dimensions[1] as usize)
        .checked_sub(top + bottom)
        .ok_or_else(|| invalid("HEVC display crop exceeds height"))?;
    let mut data = Vec::new();
    for component in 0..3 {
        let [sx, sy] = if component == 0 { [1, 1] } else { sub };
        let (w, h) = (width.div_ceil(sx), height.div_ceil(sy));
        let source_depth = p.depth[usize::from(component != 0)];
        let plane = &p.planes[component];
        let stride = plane.dimensions()[0];
        for y in 0..h {
            for x in 0..w {
                let v = if format == 0 && component != 0 {
                    1u16 << (depth - 1)
                } else {
                    let index = (top / sy + y) * stride + left / sx + x;
                    let sample = *plane
                        .samples()
                        .get(index)
                        .ok_or_else(|| invalid("HEVC display plane is truncated"))?;
                    sample << (depth - source_depth)
                };
                if depth == 8 {
                    data.push(v as u8);
                } else {
                    data.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
    }
    PackedPlanar::new(
        GeometryFrame {
            width,
            height,
            subsampling: Some(sub),
            data,
        },
        depth,
        colour,
    )
}

/// Preserve VP9 plane strides, chroma geometry and full sample depth.
pub(crate) fn vp9_picture(
    p: &crate::codec::vp9_picture::Picture,
    colour: AvcColour,
    rgb: bool,
) -> Result<PackedPlanar> {
    if rgb {
        if p.subsampling != [false, false] {
            return Err(invalid("invalid VP9 RGB subsampling"));
        }
        let frame = fvid_media::owned_frame::pack_gbr(
            p.size,
            p.depth,
            p.planes
                .each_ref()
                .map(|plane| (plane.width, plane.height, plane.samples.as_slice())),
        )
        .map_err(|e| invalid(&e))?;
        return PackedPlanar::new(frame, p.depth, colour);
    }
    let [width, height] = p.size.map(|n| n as usize);
    let factors = p.subsampling.map(|s| 1usize << usize::from(s));
    let bytes = if p.depth == 8 { 1 } else { 2 };
    let mut data = Vec::new();
    for (i, plane) in p.planes.iter().enumerate() {
        let w = if i == 0 {
            width
        } else {
            width.div_ceil(factors[0])
        };
        let h = if i == 0 {
            height
        } else {
            height.div_ceil(factors[1])
        };
        if w > plane.width || h > plane.height || plane.samples.len() != plane.width * plane.height
        {
            return Err(invalid("invalid VP9 plane storage"));
        }
        for y in 0..h {
            for &sample in &plane.samples[y * plane.width..y * plane.width + w] {
                if bytes == 1 {
                    data.push(sample as u8);
                } else {
                    data.extend_from_slice(&sample.to_le_bytes());
                }
            }
        }
    }
    PackedPlanar::new(
        GeometryFrame {
            width,
            height,
            subsampling: Some(factors),
            data,
        },
        p.depth,
        colour,
    )
}

/// Pack visible AV1 rows with their own strides and explicit chroma geometry.
/// Monochrome pictures get neutral chroma for the renderer, at the coded depth.
pub(crate) fn av1_picture(
    p: &crate::codec::av1_picture::Picture,
    signal: &crate::codec::av1_sequence::Color,
    colour: AvcColour,
) -> Result<PackedPlanar> {
    if !(8..=16).contains(&p.depth) {
        return Err(invalid("invalid AV1 planar depth"));
    }
    let [width, height] = p.size.map(|n| n as usize);
    if signal.matrix == 0 && !signal.monochrome {
        if signal.subsampling != [false, false] || !signal.full_range {
            return Err(invalid("invalid AV1 identity RGB geometry or range"));
        }
        let bytes = if p.depth == 8 { 1 } else { 2 };
        let length = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(3 * bytes))
            .ok_or_else(|| invalid("AV1 RGB size overflow"))?;
        for plane in &p.planes {
            if plane.width < width
                || plane.height < height
                || plane.width.checked_mul(plane.height) != Some(plane.samples.len())
            {
                return Err(invalid("invalid AV1 RGB plane storage"));
            }
        }
        let mut data = Vec::new();
        data.try_reserve_exact(length)
            .map_err(|_| invalid("AV1 RGB allocation failed"))?;
        let max = (1u32 << p.depth) - 1;
        for y in 0..height {
            for x in 0..width {
                for channel in [2, 0, 1] {
                    let plane = &p.planes[channel];
                    let value = plane.samples[y * plane.width + x];
                    if u32::from(value) > max {
                        return Err(invalid("RGB sample exceeds coded depth"));
                    }
                    if bytes == 1 {
                        data.push(value as u8);
                    } else {
                        data.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        return PackedPlanar::new(
            GeometryFrame {
                width,
                height,
                subsampling: None,
                data,
            },
            p.depth,
            colour,
        );
    }
    let factors = if signal.monochrome {
        [2, 2]
    } else {
        signal.subsampling.map(|s| 1usize << usize::from(s))
    };
    let bytes = if p.depth == 8 { 1 } else { 2 };
    let chroma = width
        .div_ceil(factors[0])
        .checked_mul(height.div_ceil(factors[1]))
        .ok_or_else(|| invalid("AV1 chroma size overflow"))?;
    let count = width
        .checked_mul(height)
        .and_then(|n| chroma.checked_mul(2).and_then(|c| n.checked_add(c)))
        .and_then(|n| n.checked_mul(bytes))
        .ok_or_else(|| invalid("AV1 planar size overflow"))?;
    let mut data = Vec::new();
    data.try_reserve_exact(count)
        .map_err(|_| invalid("AV1 planar allocation failed"))?;
    for (i, plane) in p.planes.iter().enumerate() {
        let w = if i == 0 {
            width
        } else {
            width.div_ceil(factors[0])
        };
        let h = if i == 0 {
            height
        } else {
            height.div_ceil(factors[1])
        };
        let neutral = signal.monochrome && i != 0;
        if !neutral
            && (w > plane.width
                || h > plane.height
                || plane.width.checked_mul(plane.height) != Some(plane.samples.len()))
        {
            return Err(invalid("invalid AV1 plane storage"));
        }
        for y in 0..h {
            for x in 0..w {
                let sample = if neutral {
                    1u16 << (p.depth - 1)
                } else {
                    plane.samples[y * plane.width + x]
                };
                if bytes == 1 {
                    data.push(
                        u8::try_from(sample).map_err(|_| invalid("8-bit AV1 sample overflow"))?,
                    );
                } else {
                    data.extend_from_slice(&sample.to_le_bytes());
                }
            }
        }
    }
    PackedPlanar::new(
        GeometryFrame {
            width,
            height,
            subsampling: Some(factors),
            data,
        },
        p.depth,
        colour,
    )
}

/// Composite unassociated display RGBA codes onto opaque black.
pub fn rgba_to_rgb_on_black(rgba: &[u8], budget: usize) -> Result<Vec<u8>> {
    if rgba.len() % 4 != 0 {
        return Err(invalid("invalid RGBA display storage"));
    }
    let size = (rgba.len() / 4)
        .checked_mul(3)
        .filter(|&n| n <= budget)
        .ok_or_else(|| invalid("RGB composite exceeds playback budget"))?;
    let mut rgb = crate::buffer(size)?;
    for (source, out) in rgba.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
        for (color, out) in source[..3].iter().zip(out) {
            *out = ((u32::from(*color) * u32::from(source[3]) + 127) / 255) as u8;
        }
    }
    Ok(rgb)
}
