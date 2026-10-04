//! Owned separable Gaussian/unsharp FIR filtering with contour thresholds.
use crate::owned_frame::GeometryFrame;
use std::cell::RefCell;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
struct Plane {
    radius: f32,
    strength: f32,
    threshold: i32,
}
#[derive(Debug)]
pub struct SmartBlur {
    planes: [Plane; 3],
    enable: crate::owned_timeline::Timeline,
    cache: RefCell<Option<Cache>>,
}
#[derive(Debug)]
struct Row {
    start: usize,
    weights: Vec<i32>,
}
#[derive(Debug)]
struct Axes {
    horizontal: Vec<Row>,
    vertical: Vec<Row>,
}
#[derive(Debug)]
struct Cache {
    shape: (usize, usize, [usize; 2]),
    planes: [Axes; 2],
}
fn rounded(value: i64, divisor: i64) -> i64 {
    if value < 0 {
        (value - divisor / 2) / divisor
    } else {
        (value + divisor / 2) / divisor
    }
}
fn kernel(plane: Plane) -> Vec<i64> {
    let sigma = f64::from(plane.radius);
    let size = (sigma * 3. + 0.5) as usize | 1;
    let middle = (size / 2) as f64;
    let mut weights: Vec<f64> = (0..size)
        .map(|i| {
            let distance = i as f64 - middle;
            (-distance * distance / (2. * sigma * sigma)).exp()
                / (2. * sigma * std::f64::consts::PI).sqrt()
        })
        .collect();
    let total: f64 = weights.iter().sum();
    for value in &mut weights {
        *value = *value / total * f64::from(plane.strength);
    }
    weights[size / 2] += 1. - f64::from(plane.strength);
    // A wide coefficient grid makes the later normalized integer quantizers
    // deterministic without relying on CPU-specific vector instructions.
    weights
        .into_iter()
        .map(|v| (v * ((1u64 << 54) as f64)) as i64)
        .collect()
}
fn rows(length: usize, coefficients: &[i64], quantum: i64) -> Result<Vec<Row>> {
    let center = coefficients.len() / 2;
    let cutoff = ((1u64 << 54) as f64 * 0.002) as i64;
    let mut left = 0;
    let mut consumed = 0i64;
    while left < center && consumed + coefficients[left].abs() <= cutoff {
        consumed += coefficients[left].abs();
        left += 1;
    }
    let mut right = coefficients.len();
    consumed = 0;
    while right > center + 1 && consumed + coefficients[right - 1].abs() <= cutoff {
        right -= 1;
        consumed += coefficients[right].abs();
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|e| e.to_string())?;
    for position in 0..length {
        let begin = position
            .saturating_add(left)
            .saturating_sub(center)
            .min(length - 1);
        let end = position
            .saturating_add(right - 1)
            .saturating_sub(center)
            .min(length - 1);
        let mut merged = vec![0i64; end - begin + 1];
        for (i, &coefficient) in coefficients.iter().enumerate().take(right).skip(left) {
            let at = position
                .saturating_add(i)
                .saturating_sub(center)
                .min(length - 1);
            merged[at - begin] += coefficient;
        }
        let total = merged.iter().sum::<i64>();
        let divisor = ((total + quantum / 2) / quantum).max(1);
        let mut remainder = 0i64;
        let mut weights = Vec::with_capacity(merged.len());
        for value in merged {
            let value = value + remainder;
            let quantized = rounded(value, divisor);
            remainder = value - quantized * divisor;
            weights.push(i32::try_from(quantized).map_err(|_| "smartblur coefficient overflow")?);
        }
        result.push(Row {
            start: begin,
            weights,
        });
    }
    Ok(result)
}
fn axes(width: usize, height: usize, plane: Plane) -> Result<Axes> {
    let coefficients = kernel(plane);
    Ok(Axes {
        horizontal: rows(width, &coefficients, 1 << 14)?,
        vertical: rows(height, &coefficients, 1 << 12)?,
    })
}
fn trim_quote(text: &str) -> Result<&str> {
    let text = text.trim();
    if let Some(mark) = text.chars().next().filter(|c| matches!(c, '\'' | '"')) {
        text.strip_prefix(mark)
            .and_then(|t| t.strip_suffix(mark))
            .ok_or("unclosed smartblur quote".into())
    } else {
        Ok(text)
    }
}
impl SmartBlur {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 16384 || args.contains('\0') {
            return Err("invalid smartblur options".into());
        }
        let mut values = [[1., 1., 0.], [-0.9, -2., -31.], [-0.9, -2., -31.]];
        let mut enable = crate::owned_timeline::Timeline::default();
        let mut positional = 0;
        let names = ["lr", "ls", "lt", "cr", "cs", "ct", "ar", "as", "at"];
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *names.get(positional).ok_or("too many smartblur options")?;
                positional += 1;
                (key, entry)
            };
            if key.trim() == "enable" {
                enable = crate::owned_timeline::Timeline::grayworld(&format!(
                    "enable={}",
                    trim_quote(value)?
                ))?;
                continue;
            }
            let index = match key.trim() {
                "lr" | "luma_radius" => 0,
                "ls" | "luma_strength" => 1,
                "lt" | "luma_threshold" => 2,
                "cr" | "chroma_radius" => 3,
                "cs" | "chroma_strength" => 4,
                "ct" | "chroma_threshold" => 5,
                "ar" | "alpha_radius" => 6,
                "as" | "alpha_strength" => 7,
                "at" | "alpha_threshold" => 8,
                _ => return Err("unknown smartblur option".into()),
            };
            let value = crate::owned_expression::constant(trim_quote(value)?)?;
            let kind = index % 3;
            let component = index / 3;
            let minimum = match kind {
                0 => {
                    if component == 0 {
                        0.1
                    } else {
                        -0.9
                    }
                }
                1 => {
                    if component == 0 {
                        -1.
                    } else {
                        -2.
                    }
                }
                _ => {
                    if component == 0 {
                        -30.
                    } else {
                        -31.
                    }
                }
            };
            let maximum = match kind {
                0 => 5.,
                1 => 1.,
                _ => 30.,
            };
            if !value.is_finite()
                || !(minimum..=maximum).contains(&value)
                || (kind == 2 && value.fract() != 0.)
            {
                return Err("smartblur option outside its range".into());
            }
            values[component][kind] = if kind == 2 {
                value
            } else {
                f64::from(value as f32)
            };
        }
        for component in 1..3 {
            for (kind, minimum) in [0.1, -1., -30.].into_iter().enumerate() {
                if values[component][kind] < minimum {
                    values[component][kind] = values[0][kind];
                }
            }
        }
        Ok(Self {
            planes: values.map(|v| Plane {
                radius: v[0] as f32,
                strength: v[1] as f32,
                threshold: v[2] as i32,
            }),
            enable,
            cache: RefCell::new(None),
        })
    }
    pub fn apply(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        crate::owned_overlay::validate_frame(frame, depth)?;
        let sampling = frame.subsampling.ok_or("smartblur requires planar YUV")?;
        if !self.enable.enabled(n, t, frame.width, frame.height)? {
            return Ok(());
        }
        let shape = (frame.width, frame.height, sampling);
        let (cw, ch) = (
            frame.width.div_ceil(sampling[0]),
            frame.height.div_ceil(sampling[1]),
        );
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|c| c.shape != shape) {
            *cache = Some(Cache {
                shape,
                planes: [
                    axes(frame.width, frame.height, self.planes[0])?,
                    axes(cw, ch, self.planes[1])?,
                ],
            });
        }
        // Produce all planes before committing a frame, so allocation failure
        // cannot leave luma processed but chroma unprocessed.
        let mut output = crate::owned_frame::buffer(frame.data.len())?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let mut offset = 0;
        for (component, (width, height)) in [(frame.width, frame.height), (cw, ch), (cw, ch)]
            .into_iter()
            .enumerate()
        {
            let length = width
                .checked_mul(height)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or("smartblur geometry overflow")?;
            filter_plane(
                &frame.data[offset..offset + length],
                &mut output[offset..offset + length],
                width,
                height,
                depth,
                self.planes[component.min(1)],
                &cache.as_ref().unwrap().planes[component.min(1)],
            )?;
            offset += length;
        }
        frame.data = output;
        Ok(())
    }
    /// Filter a standalone planar luma/chroma/alpha image. Component 0 is luma,
    /// 1 is chroma and 2 is alpha. No packed RGB/alpha conversion is implicit.
    pub fn apply_plane(
        &self,
        data: &mut [u8],
        width: usize,
        height: usize,
        depth: u8,
        component: usize,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        if width == 0 || height == 0 || !(8..=16).contains(&depth) || component > 2 {
            return Err("invalid smartblur plane".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        if width.checked_mul(height).and_then(|v| v.checked_mul(bytes)) != Some(data.len()) {
            return Err("smartblur plane storage mismatch".into());
        }
        if depth > 8
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 >= 1u32 << depth)
        {
            return Err("smartblur sample exceeds depth".into());
        }
        if !self.enable.enabled(n, t, width, height)? {
            return Ok(());
        }
        let mut output = crate::owned_frame::buffer(data.len())?;
        let axes = axes(width, height, self.planes[component])?;
        filter_plane(
            data,
            &mut output,
            width,
            height,
            depth,
            self.planes[component],
            &axes,
        )?;
        data.copy_from_slice(&output);
        Ok(())
    }
}
fn filter_plane(
    source: &[u8],
    output: &mut [u8],
    width: usize,
    height: usize,
    depth: u8,
    plane: Plane,
    axes: &Axes,
) -> Result<()> {
    let bytes = if depth == 8 { 1 } else { 2 };
    let sample = |i: usize| {
        if bytes == 1 {
            source[i] as i64
        } else {
            u16::from_le_bytes([source[i * 2], source[i * 2 + 1]]) as i64
        }
    };
    let count = width
        .checked_mul(height)
        .ok_or("smartblur plane overflow")?;
    let mut horizontal = Vec::new();
    horizontal
        .try_reserve_exact(count)
        .map_err(|e| e.to_string())?;
    let maximum = (1i64 << depth) - 1;
    let ceiling = (1i64 << (depth + 7)) - 1;
    for y in 0..height {
        for row in &axes.horizontal {
            let sum: i64 = row
                .weights
                .iter()
                .enumerate()
                .map(|(i, &weight)| sample(y * width + row.start + i) * i64::from(weight))
                .sum();
            horizontal.push((sum >> 7).min(ceiling));
        }
    }
    let threshold = i64::from(plane.threshold) * (1i64 << (depth - 8));
    for (y, row) in axes.vertical.iter().enumerate() {
        for x in 0..width {
            let sum: i64 = (1 << 18)
                + row
                    .weights
                    .iter()
                    .enumerate()
                    .map(|(i, &weight)| horizontal[(row.start + i) * width + x] * i64::from(weight))
                    .sum::<i64>();
            let filtered = (sum >> 19).clamp(0, maximum);
            let original = sample(y * width + x);
            let difference = original - filtered;
            let distance = difference.abs();
            let adjusted = if threshold > 0 {
                if distance > 2 * threshold {
                    original
                } else if distance > threshold {
                    original - difference.signum() * threshold
                } else {
                    filtered
                }
            } else if threshold < 0 {
                let edge = -threshold;
                if distance <= edge {
                    original
                } else if distance <= 2 * edge {
                    filtered + difference.signum() * edge
                } else {
                    filtered
                }
            } else {
                filtered
            };
            let at = (y * width + x) * bytes;
            if bytes == 1 {
                output[at] = adjusted as u8;
            } else {
                output[at..at + 2].copy_from_slice(&(adjusted as u16).to_le_bytes());
            }
        }
    }
    Ok(())
}
