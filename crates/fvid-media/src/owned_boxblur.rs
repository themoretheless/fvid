//! Owned separable repeated box blur on planar YUV samples.
use crate::owned_expression::Expression as BoxBlurExpression;
use crate::owned_frame::GeometryFrame;
pub use fvid_control::error::{Error, Result};
use fvid_control::error::{buffer, invalid};
#[derive(Clone, Copy, Debug)]
pub struct BoxBlur {
    radius: [usize; 3],
    power: [u32; 3],
}
impl BoxBlur {
    /// Integer radii and pass counts, in luma/chroma/alpha option order.
    /// Use BoxBlurProgram for geometry-dependent radius expressions.
    pub fn parse(args: &str) -> Result<Self> {
        let names = [
            "luma_radius",
            "luma_power",
            "chroma_radius",
            "chroma_power",
            "alpha_radius",
            "alpha_power",
        ];
        let aliases = ["lr", "lp", "cr", "cp", "ar", "ap"];
        let mut values = [Some(2i32), Some(2), None, None, None, None];
        let mut positional = 0;
        if !args.is_empty() {
            for entry in args.split(':') {
                let (key, value) = if let Some(pair) = entry.split_once('=') {
                    pair
                } else {
                    let key = *names
                        .get(positional)
                        .ok_or_else(|| invalid("too many boxblur options"))?;
                    positional += 1;
                    (key, entry)
                };
                let index = names
                    .iter()
                    .zip(aliases)
                    .position(|(name, alias)| *name == key || alias == key)
                    .ok_or_else(|| invalid("unknown boxblur option"))?;
                let n = value
                    .parse::<i32>()
                    .map_err(|_| invalid("owned boxblur requires integer radii and powers"))?;
                if n < 0 && !(n == -1 && matches!(index, 3 | 5)) {
                    return Err(invalid("boxblur option out of range"));
                }
                values[index] = if n == -1 { None } else { Some(n) };
            }
        }
        let radius = values[0].unwrap() as usize;
        let power = values[1].unwrap() as u32;
        Ok(Self {
            radius: [
                radius,
                values[2].map_or(radius, |n| n as usize),
                values[4].map_or(radius, |n| n as usize),
            ],
            power: [
                power,
                values[3].map_or(power, |n| n as u32),
                values[5].map_or(power, |n| n as u32),
            ],
        })
    }

    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        self.apply_inner(frame, depth, false)
    }

    pub(crate) fn apply_validated(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        self.apply_inner(frame, depth, true)
    }

    fn apply_inner(self, frame: &mut GeometryFrame, depth: u8, prevalidated: bool) -> Result<()> {
        let [sx, sy] = frame
            .subsampling
            .ok_or_else(|| invalid("boxblur requires planar YUV"))?;
        if sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0 || !(8..=16).contains(&depth)
        {
            return Err(invalid("invalid boxblur geometry or depth"));
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let (cw, ch) = (frame.width.div_ceil(sx), frame.height.div_ceil(sy));
        let size = |w: usize, h: usize| {
            w.checked_mul(h)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("boxblur size overflow"))
        };
        let luma = size(frame.width, frame.height)?;
        let chroma = size(cw, ch)?;
        let total = chroma
            .checked_mul(2)
            .and_then(|n| n.checked_add(luma))
            .ok_or_else(|| invalid("boxblur storage overflow"))?;
        if total != frame.data.len()
            || (!prevalidated
                && bytes == 2
                && depth < 16
                && frame
                    .data
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|p| u16::from_le_bytes(*p) >= 1u16 << depth))
        {
            return Err(invalid("invalid boxblur sample storage"));
        }
        // The filter's radius contract uses floor chroma dimensions, even for
        // an odd-size source whose stored planes have a rounded-up last sample.
        for (index, (w, h)) in [
            (frame.width, frame.height),
            (frame.width / sx, frame.height / sy),
            (frame.width, frame.height),
        ]
        .into_iter()
        .enumerate()
        {
            if self.radius[index]
                .checked_mul(2)
                .is_none_or(|n| n >= w.min(h))
            {
                return Err(invalid(
                    "boxblur radius must be less than half the plane dimension",
                ));
            }
        }
        if (self.radius[0] == 0 || self.power[0] == 0)
            && (self.radius[1] == 0 || self.power[1] == 0)
        {
            return Ok(());
        }
        let mut a = Vec::<u16>::new();
        let mut b = Vec::<u16>::new();
        for v in [&mut a, &mut b] {
            v.try_reserve_exact(frame.width)
                .map_err(|_| invalid("boxblur scratch allocation failed"))?;
            v.resize(frame.width, 0);
        }
        let mut acc = Vec::<u64>::new();
        acc.try_reserve_exact(frame.width)
            .map_err(|_| invalid("boxblur scratch allocation failed"))?;
        acc.resize(frame.width, 0);
        let mut scratch = buffer(total)?;
        scratch.copy_from_slice(&frame.data);
        let mut base = 0;
        for (plane, (w, h)) in [(frame.width, frame.height), (cw, ch), (cw, ch)]
            .into_iter()
            .enumerate()
        {
            let plane_bytes = w * h * bytes;
            let p = usize::from(plane != 0);
            let radius = self.radius[p];
            let power = self.power[p];
            if radius != 0 && power != 0 {
                // Horizontal passes: sequential row-by-row in place inside scratch.
                for y in 0..h {
                    let row_start = base + y * w * bytes;
                    if bytes == 1 {
                        for (dst, &src) in a[..w]
                            .iter_mut()
                            .zip(&scratch[row_start..row_start + w])
                        {
                            *dst = u16::from(src);
                        }
                    } else {
                        for (dst, src) in a[..w]
                            .iter_mut()
                            .zip(scratch[row_start..row_start + w * 2].as_chunks::<2>().0)
                        {
                            *dst = u16::from_le_bytes(*src);
                        }
                    }
                    for _ in 0..power {
                        blur_line(&a[..w], &mut b[..w], radius, bytes);
                        std::mem::swap(&mut a, &mut b);
                    }
                    if bytes == 1 {
                        for (dst, &src) in scratch[row_start..row_start + w]
                            .iter_mut()
                            .zip(&a[..w])
                        {
                            *dst = src as u8;
                        }
                    } else {
                        for (dst, &src) in scratch[row_start..row_start + w * 2]
                            .as_chunks_mut::<2>()
                            .0
                            .iter_mut()
                            .zip(&a[..w])
                        {
                            *dst = src.to_le_bytes();
                        }
                    }
                }
                // Vertical passes: row-major sliding column accumulator ping-ponging scratch <-> frame.data.
                let mut src_is_scratch = true;
                for _ in 0..power {
                    if src_is_scratch {
                        blur_vertical_plane(
                            &scratch[base..base + plane_bytes],
                            &mut frame.data[base..base + plane_bytes],
                            w,
                            h,
                            radius,
                            bytes,
                            &mut acc[..w],
                        );
                    } else {
                        blur_vertical_plane(
                            &frame.data[base..base + plane_bytes],
                            &mut scratch[base..base + plane_bytes],
                            w,
                            h,
                            radius,
                            bytes,
                            &mut acc[..w],
                        );
                    }
                    src_is_scratch = !src_is_scratch;
                }
                if src_is_scratch {
                    frame.data[base..base + plane_bytes]
                        .copy_from_slice(&scratch[base..base + plane_bytes]);
                }
            }
            base += plane_bytes;
        }
        Ok(())
    }
}

#[inline]
fn reflect_coord(i: isize, len: usize) -> usize {
    if i < 0 {
        (-i - 1) as usize
    } else if i >= len as isize {
        (2 * len as isize - i - 1) as usize
    } else {
        i as usize
    }
}

fn blur_vertical_plane(
    src: &[u8],
    dst: &mut [u8],
    w: usize,
    h: usize,
    radius: usize,
    bytes: usize,
    acc: &mut [u64],
) {
    let length = 2 * radius + 1;
    let reciprocal = (65536 + length as u64 / 2) / length as u64;
    acc.fill(0);
    if bytes == 1 {
        for dy in -(radius as isize)..=radius as isize {
            let row = &src[reflect_coord(dy, h) * w..][..w];
            for (sum, &v) in acc.iter_mut().zip(row) {
                *sum += u64::from(v);
            }
        }
        for (out, &sum) in dst[..w].iter_mut().zip(acc.iter()) {
            *out = ((sum * reciprocal + 32768) >> 16) as u8;
        }
        for y in 1..h {
            let sub_row = &src[reflect_coord(y as isize - radius as isize - 1, h) * w..][..w];
            let add_row = &src[reflect_coord(y as isize + radius as isize, h) * w..][..w];
            let out_row = &mut dst[y * w..(y + 1) * w];
            for x in 0..w {
                let sum = acc[x] - u64::from(sub_row[x]) + u64::from(add_row[x]);
                acc[x] = sum;
                out_row[x] = ((sum * reciprocal + 32768) >> 16) as u8;
            }
        }
    } else {
        for dy in -(radius as isize)..=radius as isize {
            let row = src[reflect_coord(dy, h) * w * 2..][..w * 2]
                .as_chunks::<2>()
                .0;
            for (sum, pair) in acc.iter_mut().zip(row) {
                *sum += u64::from(u16::from_le_bytes(*pair));
            }
        }
        for (out, &sum) in dst[..w * 2].as_chunks_mut::<2>().0.iter_mut().zip(acc.iter()) {
            *out = (((sum * reciprocal + 32768) >> 16) as u16).to_le_bytes();
        }
        for y in 1..h {
            let sub_row = src[reflect_coord(y as isize - radius as isize - 1, h) * w * 2..][..w * 2]
                .as_chunks::<2>()
                .0;
            let add_row = src[reflect_coord(y as isize + radius as isize, h) * w * 2..][..w * 2]
                .as_chunks::<2>()
                .0;
            let out_row = dst[y * w * 2..(y + 1) * w * 2].as_chunks_mut::<2>().0;
            for x in 0..w {
                let sum = acc[x]
                    - u64::from(u16::from_le_bytes(sub_row[x]))
                    + u64::from(u16::from_le_bytes(add_row[x]));
                acc[x] = sum;
                out_row[x] = (((sum * reciprocal + 32768) >> 16) as u16).to_le_bytes();
            }
        }
    }
}

fn blur_line(input: &[u16], output: &mut [u16], radius: usize, bytes: usize) {
    let length = 2 * radius + 1;
    let reciprocal = (65536 + length as u64 / 2) / length as u64;
    let reflect = |i: i128| -> usize {
        if i < 0 {
            (-i - 1) as usize
        } else if i >= input.len() as i128 {
            (2 * input.len() as i128 - i - 1) as usize
        } else {
            i as usize
        }
    };
    let mut sum = 0u64;
    for i in -(radius as i128)..=radius as i128 {
        sum += u64::from(input[reflect(i)]);
    }
    for (x, value) in output.iter_mut().enumerate() {
        if x > 0 {
            sum -= u64::from(input[reflect(x as i128 - radius as i128 - 1)]);
            sum += u64::from(input[reflect(x as i128 + radius as i128)]);
        }
        let rounded = (sum * reciprocal + 32768) >> 16;
        // Quantize at each pass to the stored sample width, as the public
        // media filter does (not one division after the full 2D operation).
        *value = if bytes == 1 {
            u16::from(rounded as u8)
        } else {
            rounded as u16
        };
    }
}

/// Radius expressions evaluated against source plane dimensions.
#[derive(Debug)]
pub struct BoxBlurProgram {
    radius: [BoxBlurExpression; 3],
    power: [u32; 3],
}
impl BoxBlurProgram {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err(invalid("invalid boxblur options"));
        }
        let names = [
            "luma_radius",
            "luma_power",
            "chroma_radius",
            "chroma_power",
            "alpha_radius",
            "alpha_power",
        ];
        let aliases = ["lr", "lp", "cr", "cp", "ar", "ap"];
        let mut radii: [Option<BoxBlurExpression>; 3] = [None, None, None];
        let mut powers = [Some(2u32), None, None];
        let mut positional = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, text) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *names
                    .get(positional)
                    .ok_or_else(|| invalid("too many boxblur options"))?;
                positional += 1;
                (key, entry)
            };
            let index = names
                .iter()
                .zip(aliases)
                .position(|(name, alias)| *name == key.trim() || alias == key.trim())
                .ok_or_else(|| invalid("unknown boxblur option"))?;
            let text = text.trim();
            if index % 2 == 1 {
                let n = text
                    .parse::<i32>()
                    .map_err(|_| invalid("boxblur powers must be integers"))?;
                if n < 0 && !(n == -1 && index != 1) {
                    return Err(invalid("boxblur power out of range"));
                }
                powers[index / 2] = if n == -1 { None } else { Some(n as u32) };
            } else {
                let text = if text.starts_with('\'') || text.starts_with('"') {
                    let quote = text.chars().next().unwrap();
                    text.strip_prefix(quote)
                        .and_then(|v| v.strip_suffix(quote))
                        .ok_or_else(|| invalid("unclosed boxblur expression quote"))?
                } else {
                    text
                };
                let expr = BoxBlurExpression::parse(text).map_err(|e| invalid(&e))?;
                expr.evaluate(&[
                    ("w", 32.),
                    ("h", 32.),
                    ("cw", 16.),
                    ("ch", 16.),
                    ("hsub", 2.),
                    ("vsub", 2.),
                ])
                .map_err(|e| invalid(&e))?;
                radii[index / 2] = Some(expr);
            }
        }
        let luma = radii[0]
            .take()
            .unwrap_or(BoxBlurExpression::parse("2").map_err(|e| invalid(&e))?);
        let chroma = radii[1].take().unwrap_or(luma.clone());
        let alpha = radii[2].take().unwrap_or(luma.clone());
        let power = powers[0].unwrap();
        Ok(Self {
            radius: [luma, chroma, alpha],
            power: [
                power,
                powers[1].unwrap_or(power),
                powers[2].unwrap_or(power),
            ],
        })
    }
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        self.resolve(frame)?.apply(frame, depth)
    }

    pub(crate) fn apply_validated(&self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        self.resolve(frame)?.apply_validated(frame, depth)
    }

    fn resolve(&self, frame: &GeometryFrame) -> Result<BoxBlur> {
        let [sx, sy] = frame
            .subsampling
            .ok_or_else(|| invalid("boxblur requires planar YUV"))?;
        if sx == 0 || sy == 0 {
            return Err(invalid("invalid boxblur subsampling"));
        }
        let vars = [
            ("w", frame.width as f64),
            ("h", frame.height as f64),
            ("cw", (frame.width / sx) as f64),
            ("ch", (frame.height / sy) as f64),
            ("hsub", sx as f64),
            ("vsub", sy as f64),
        ];
        let mut radius = [0; 3];
        for (i, expr) in self.radius.iter().enumerate() {
            let value = expr.evaluate(&vars).map_err(|e| invalid(&e))?;
            if !value.is_finite() || value.trunc() < 0. || value.trunc() > i32::MAX as f64 {
                return Err(invalid("boxblur radius expression out of range"));
            }
            radius[i] = value.trunc() as usize;
        }
        Ok(BoxBlur {
            radius,
            power: self.power,
        })
    }
}
