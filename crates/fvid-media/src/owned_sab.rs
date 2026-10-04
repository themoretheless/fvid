//! Owned shape-adaptive smoothing with Gaussian guidance and mirrored neighbours.
use crate::{owned_frame::GeometryFrame, owned_smartblur::SmartBlur};
use std::cell::RefCell;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
struct Plane {
    radius: f32,
    guidance: f32,
    strength: f32,
}
#[derive(Debug)]
pub struct Sab {
    plane: [Plane; 2],
    prefilter: SmartBlur,
    enable: crate::owned_timeline::Timeline,
    cache: RefCell<Option<Cache>>,
}
#[derive(Debug)]
struct Weights {
    side: usize,
    spatial: Vec<u32>,
    similarity: Vec<u32>,
    x: Vec<usize>,
    y: Vec<usize>,
}
#[derive(Debug)]
struct Cache {
    shape: (usize, usize, [usize; 2], u8),
    weights: [Weights; 2],
}
fn gaussian(sigma: f64, quality: f64) -> Vec<f64> {
    let count = (sigma * quality + 0.5) as usize | 1;
    let center = (count / 2) as f64;
    let mut samples: Vec<f64> = (0..count)
        .map(|i| {
            let distance = i as f64 - center;
            (-distance * distance / (2. * sigma * sigma)).exp()
                / (2. * sigma * std::f64::consts::PI).sqrt()
        })
        .collect();
    let total: f64 = samples.iter().sum();
    for sample in &mut samples {
        *sample /= total;
    }
    samples
}
fn neighbors(length: usize, side: usize) -> Result<Vec<usize>> {
    let count = length
        .checked_mul(side)
        .ok_or("sab neighbour geometry overflow")?;
    let mut result = Vec::new();
    result.try_reserve_exact(count).map_err(|e| e.to_string())?;
    let period = (length - 1)
        .checked_mul(2)
        .ok_or("sab reflection period overflow")?;
    for position in 0..length {
        for index in 0..side {
            let coordinate = position as i128 + index as i128 - (side / 2) as i128;
            let reflected = if period == 0 {
                0
            } else {
                let phase = coordinate.rem_euclid(period as i128) as usize;
                phase.min(period - phase)
            };
            result.push(reflected);
        }
    }
    Ok(result)
}
fn weights(plane: Plane, width: usize, height: usize, depth: u8) -> Result<Weights> {
    let spatial = gaussian(f64::from(plane.radius), 3.);
    let mut kernel = Vec::with_capacity(spatial.len() * spatial.len());
    for &vertical in &spatial {
        for &horizontal in &spatial {
            kernel.push((vertical * horizontal * 1024. + 0.5) as u32);
        }
    }
    // The similarity support is finite and scales with sample precision, while
    // the stored factors remain bounded integers. Retain the 8-bit normalized
    // Gaussian ratio for exact qualification of the established sample domain.
    let multiplier = (1u32 << (depth - 8)) as f64;
    let sigma = f64::from(plane.strength) * multiplier;
    let support = ((sigma * 5. + 0.5) as usize | 1) / 2;
    let maximum = (1usize << depth) - 1;
    let reference = if depth == 8 {
        Some(gaussian(sigma, 5.))
    } else {
        None
    };
    let mut similarity = Vec::new();
    similarity
        .try_reserve_exact(maximum + 1)
        .map_err(|e| e.to_string())?;
    for difference in 0..=maximum {
        let factor = if difference > support {
            0.
        } else if let Some(vector) = &reference {
            vector[support + difference] / vector[support]
        } else {
            (-(difference as f64).powi(2) / (2. * sigma * sigma)).exp()
        };
        similarity.push((factor * 4096. + 0.5) as u32);
    }
    let side = spatial.len();
    Ok(Weights {
        side,
        spatial: kernel,
        similarity,
        x: neighbors(width, side)?,
        y: neighbors(height, side)?,
    })
}
fn unquote(text: &str) -> Result<&str> {
    let text = text.trim();
    if let Some(mark) = text.chars().next().filter(|c| matches!(c, '\'' | '"')) {
        text.strip_prefix(mark)
            .and_then(|t| t.strip_suffix(mark))
            .ok_or("unclosed sab quote".into())
    } else {
        Ok(text)
    }
}
impl Sab {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 16384 || args.contains('\0') {
            return Err("invalid sab options".into());
        }
        let mut values = [[1.; 3], [-0.9; 3]];
        let mut enable = crate::owned_timeline::Timeline::default();
        let mut positional = 0;
        let names = ["lr", "lpfr", "ls", "cr", "cpfr", "cs"];
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let name = *names.get(positional).ok_or("too many sab options")?;
                positional += 1;
                (name, entry)
            };
            if key.trim() == "enable" {
                enable = crate::owned_timeline::Timeline::grayworld(&format!(
                    "enable={}",
                    unquote(value)?
                ))?;
                continue;
            }
            let index = match key.trim() {
                "lr" | "luma_radius" => 0,
                "lpfr" | "luma_pre_filter_radius" => 1,
                "ls" | "luma_strength" => 2,
                "cr" | "chroma_radius" => 3,
                "cpfr" | "chroma_pre_filter_radius" => 4,
                "cs" | "chroma_strength" => 5,
                _ => return Err("unknown sab option".into()),
            };
            let value = crate::owned_expression::constant(unquote(value)?)?;
            let minimum = if index < 3 { 0.1 } else { -0.9 };
            let maximum = [4., 2., 100.][index % 3];
            if !value.is_finite() || !(minimum..=maximum).contains(&value) {
                return Err("sab option outside its range".into());
            }
            values[index / 3][index % 3] = f64::from(value as f32);
        }
        for index in 0..3 {
            if values[1][index] < 0.1 {
                values[1][index] = values[0][index];
            }
        }
        let plane = values.map(|p| Plane {
            radius: p[0] as f32,
            guidance: p[1] as f32,
            strength: p[2] as f32,
        });
        let prefilter = SmartBlur::parse(&format!(
            "lr={}:cr={}",
            plane[0].guidance, plane[1].guidance
        ))?;
        Ok(Self {
            plane,
            prefilter,
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
        let sampling = frame.subsampling.ok_or("sab requires planar YUV")?;
        if !self.enable.enabled(n, t, frame.width, frame.height)? {
            return Ok(());
        }
        let shape = (frame.width, frame.height, sampling, depth);
        let (cw, ch) = (
            frame.width.div_ceil(sampling[0]),
            frame.height.div_ceil(sampling[1]),
        );
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|c| c.shape != shape) {
            *cache = Some(Cache {
                shape,
                weights: [
                    weights(self.plane[0], frame.width, frame.height, depth)?,
                    weights(self.plane[1], cw, ch, depth)?,
                ],
            });
        }
        let mut guidance = GeometryFrame {
            width: frame.width,
            height: frame.height,
            subsampling: frame.subsampling,
            data: crate::owned_frame::buffer(frame.data.len())?,
        };
        guidance.data.copy_from_slice(&frame.data);
        self.prefilter.apply(&mut guidance, depth, n, t)?;
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
                .ok_or("sab plane geometry overflow")?;
            filter_plane(
                &frame.data[offset..offset + length],
                &guidance.data[offset..offset + length],
                &mut output[offset..offset + length],
                width,
                height,
                depth,
                &cache.as_ref().unwrap().weights[component.min(1)],
            );
            offset += length;
        }
        frame.data = output;
        Ok(())
    }
    /// Standalone grayscale or chroma plane. There is no implicit RGB or alpha
    /// conversion; component 0 uses luma settings and 1 uses chroma settings.
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
        if width == 0 || height == 0 || !(8..=16).contains(&depth) || component > 1 {
            return Err("invalid sab plane".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        if width.checked_mul(height).and_then(|v| v.checked_mul(bytes)) != Some(data.len()) {
            return Err("sab plane storage mismatch".into());
        }
        if depth > 8
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 >= 1u32 << depth)
        {
            return Err("sab sample exceeds precision".into());
        }
        if !self.enable.enabled(n, t, width, height)? {
            return Ok(());
        }
        let mut guidance = crate::owned_frame::buffer(data.len())?;
        guidance.copy_from_slice(data);
        self.prefilter
            .apply_plane(&mut guidance, width, height, depth, component, n, t)?;
        let parameters = weights(self.plane[component], width, height, depth)?;
        let mut output = crate::owned_frame::buffer(data.len())?;
        filter_plane(
            data,
            &guidance,
            &mut output,
            width,
            height,
            depth,
            &parameters,
        );
        data.copy_from_slice(&output);
        Ok(())
    }
}
fn sample(data: &[u8], index: usize, depth: u8) -> u16 {
    if depth == 8 {
        u16::from(data[index])
    } else {
        u16::from_le_bytes([data[index * 2], data[index * 2 + 1]])
    }
}
fn filter_plane(
    source: &[u8],
    guidance: &[u8],
    output: &mut [u8],
    width: usize,
    height: usize,
    depth: u8,
    weights: &Weights,
) {
    for y in 0..height {
        for x in 0..width {
            let center = sample(guidance, y * width + x, depth);
            let mut numerator = 0u64;
            let mut denominator = 0u64;
            for dy in 0..weights.side {
                let row = weights.y[y * weights.side + dy] * width;
                for dx in 0..weights.side {
                    let at = row + weights.x[x * weights.side + dx];
                    let difference = center.abs_diff(sample(guidance, at, depth)) as usize;
                    let factor = u64::from(weights.similarity[difference])
                        * u64::from(weights.spatial[dy * weights.side + dx]);
                    numerator += u64::from(sample(source, at, depth)) * factor;
                    denominator += factor;
                }
            }
            // The center always has a positive spatial/similarity factor.
            let value = ((numerator + denominator / 2) / denominator) as u16;
            let at = y * width + x;
            if depth == 8 {
                output[at] = value as u8;
            } else {
                output[at * 2..at * 2 + 2].copy_from_slice(&value.to_le_bytes());
            }
        }
    }
}
