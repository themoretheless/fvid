//! Portable subtitle alpha masks and integer YUV composition, without libavfilter.
//! Rasterization uses the same bundled Ubuntu font as the player; no font lookup.
use crate::owned_frame::GeometryFrame;
use ab_glyph::{Font, FontRef, ScaleFont, point};

pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub alpha: Vec<u8>,
}
/// Center plain text at the lower edge, preserving explicit line breaks.
/// This primitive is not an ASS layout or complex-script shaping engine.
pub fn rasterize(text: &str, width: usize, height: usize, size: f32) -> Result<Mask, String> {
    if width == 0 || height == 0 || !size.is_finite() || !(4.0..=256.0).contains(&size) {
        return Err("invalid subtitle raster dimensions or font size".into());
    }
    let len = width
        .checked_mul(height)
        .ok_or("subtitle raster size overflow")?;
    let mut alpha = Vec::new();
    alpha.try_reserve_exact(len).map_err(|e| e.to_string())?;
    alpha.resize(len, 0);
    let font =
        FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).map_err(|e| e.to_string())?;
    let scaled = font.as_scaled(size);
    let lines: Vec<_> = text.lines().collect();
    let line_height = scaled.height() + scaled.line_gap();
    let top = height as f32 - size * 0.5 - lines.len() as f32 * line_height;
    for (line, text) in lines.iter().enumerate() {
        let mut glyphs = Vec::new();
        let mut advance = 0.0;
        let mut previous = None;
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            if id.0 == 0 && !ch.is_whitespace() {
                return Err(format!("subtitle font has no glyph for {ch:?}"));
            }
            if let Some(prev) = previous {
                advance += scaled.kern(prev, id);
            }
            let glyph = id.with_scale_and_position(size, point(advance, 0.0));
            advance += scaled.h_advance(id);
            glyphs.push(glyph);
            previous = Some(id);
        }
        let x = (width as f32 - advance) * 0.5;
        let baseline = top + line as f32 * line_height + scaled.ascent();
        for mut glyph in glyphs {
            glyph.position.x += x;
            glyph.position.y += baseline;
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i64 + i64::from(gx);
                    let py = bounds.min.y as i64 + i64::from(gy);
                    if px >= 0 && py >= 0 && px < width as i64 && py < height as i64 {
                        let slot = &mut alpha[py as usize * width + px as usize];
                        let a = (coverage * 255.0).round() as u8;
                        *slot =
                            255 - ((u16::from(255 - *slot) * u16::from(255 - a) + 127) / 255) as u8;
                    }
                });
            }
        }
    }
    Ok(Mask {
        width,
        height,
        alpha,
    })
}
/// Blend white text into planar YUV while neutralizing covered chroma.
/// Chroma coverage is averaged over its actual luma footprint, including odd edges.
pub fn composite_white(
    frame: &mut GeometryFrame,
    depth: u8,
    full_range: bool,
    mask: &Mask,
) -> Result<(), String> {
    crate::owned_overlay::validate_frame(frame, depth)?;
    if mask.width != frame.width
        || mask.height != frame.height
        || mask.alpha.len()
            != frame
                .width
                .checked_mul(frame.height)
                .ok_or("subtitle mask overflow")?
    {
        return Err("subtitle mask geometry mismatch".into());
    }
    let [sx, sy] = frame
        .subsampling
        .ok_or("subtitle compositor requires planar YUV")?;
    let width = frame.width;
    let height = frame.height;
    let bytes = if depth == 8 { 1 } else { 2 };
    let max = (1u32 << depth) - 1;
    let white = if full_range {
        max
    } else {
        235u32 << (depth - 8)
    };
    let neutral = 1u32 << (depth - 1);
    let blend = |data: &mut [u8], sample: usize, target: u32, alpha: u32| {
        let offset = sample * bytes;
        let old = if bytes == 1 {
            u32::from(data[offset])
        } else {
            u32::from(u16::from_le_bytes([data[offset], data[offset + 1]]))
        };
        let value = ((old * (255 - alpha) + target * alpha + 127) / 255).min(max);
        if bytes == 1 {
            data[offset] = value as u8;
        } else {
            data[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
        }
    };
    for (index, &alpha) in mask.alpha.iter().enumerate() {
        blend(&mut frame.data, index, white, u32::from(alpha));
    }
    let cw = width.div_ceil(sx);
    let ch = height.div_ceil(sy);
    for y in 0..ch {
        for x in 0..cw {
            let mut sum = 0u32;
            let mut count = 0u32;
            for ly in y * sy..((y + 1) * sy).min(height) {
                for lx in x * sx..((x + 1) * sx).min(width) {
                    sum += u32::from(mask.alpha[ly * width + lx]);
                    count += 1;
                }
            }
            let coverage = (sum + count / 2) / count;
            for plane in 0..2 {
                blend(
                    &mut frame.data,
                    width * height + plane * cw * ch + y * cw + x,
                    neutral,
                    coverage,
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_font_raster_is_deterministic_and_supports_cyrillic() {
        let a = rasterize("FVid\nПривет", 160, 80, 16.0).unwrap();
        let b = rasterize("FVid\nПривет", 160, 80, 16.0).unwrap();
        assert_eq!(a.alpha, b.alpha);
        assert!(a.alpha.iter().any(|&v| v > 0 && v < 255));
        assert!(a.alpha[..160 * 20].iter().all(|&v| v == 0));
        assert!(rasterize("x", usize::MAX, 2, 16.0).is_err());
    }
    #[test]
    fn odd_chroma_footprints_and_ten_bit_luma_blend_exactly() {
        for depth in [8, 10] {
            let mut data = Vec::new();
            for value in [
                16, 16, 16, 16, 16, 16, 16, 16, 16, 64, 64, 64, 64, 192, 192, 192, 192,
            ] {
                let v = (value as u16) << (depth - 8);
                if depth == 8 {
                    data.push(v as u8);
                } else {
                    data.extend(v.to_le_bytes());
                }
            }
            let mut frame = GeometryFrame {
                width: 3,
                height: 3,
                subsampling: Some([2, 2]),
                data,
            };
            let mask = Mask {
                width: 3,
                height: 3,
                alpha: vec![0, 0, 0, 0, 0, 0, 0, 0, 255],
            };
            composite_white(&mut frame, depth, false, &mask).unwrap();
            let values: Vec<u16> = if depth == 8 {
                frame.data.iter().map(|&v| u16::from(v)).collect()
            } else {
                frame
                    .data
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect()
            };
            let scale = 1 << (depth - 8);
            assert_eq!(values[8], 235 * scale);
            assert_eq!(values[12], 128 * scale);
            assert_eq!(values[16], 128 * scale);
            assert_eq!(values[9], 64 * scale);
            assert_eq!(values[0], 16 * scale);
        }
    }
}
