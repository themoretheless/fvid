//! Owned radial inverse mapping with quantized nearest/bilinear sampling.
use crate::owned_frame::GeometryFrame;
use std::cell::RefCell;
type Result<T> = std::result::Result<T, String>;
const UNIT: i128 = 1 << 24;
#[derive(Debug)]
struct Map {
    width: usize,
    height: usize,
    multipliers: Vec<i64>,
}
#[derive(Debug)]
pub struct LensCorrection {
    center: [f64; 2],
    coefficients: [i64; 2],
    bilinear: bool,
    fill: [u8; 4],
    enable: crate::owned_timeline::Timeline,
    maps: RefCell<Vec<Map>>,
}
#[derive(Clone, Copy, Debug)]
pub enum Component {
    Luma,
    Cb,
    Cr,
    Red,
    Green,
    Blue,
    Alpha,
}
impl LensCorrection {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid lenscorrection options".into());
        }
        let mut center = [0.5; 2];
        let mut coefficients = [0; 2];
        let mut bilinear = false;
        let mut fill = [0, 0, 0, 0];
        let mut enable = Default::default();
        let mut position = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(v) = entry.split_once('=') {
                v
            } else {
                let key = *["cx", "cy", "k1", "k2", "i", "fc"]
                    .get(position)
                    .ok_or("too many lenscorrection options")?;
                position += 1;
                (key, entry)
            };
            let value = value.trim().trim_matches('\'');
            match key.trim() {
                "cx" | "cy" => {
                    let v = crate::owned_expression::constant(value)?;
                    if !v.is_finite() || !(0.0..=1.).contains(&v) {
                        return Err("lenscorrection center must be from 0 to 1".into());
                    }
                    center[usize::from(key.trim() == "cy")] = v;
                }
                "k1" | "k2" => {
                    let v = crate::owned_expression::constant(value)?;
                    if !v.is_finite() || !(-1.0..=1.).contains(&v) {
                        return Err("lenscorrection coefficients must be from -1 to 1".into());
                    }
                    coefficients[usize::from(key.trim() == "k2")] = (v * UNIT as f64) as i64;
                }
                "i" => {
                    bilinear = match value {
                        "nearest" | "0" => false,
                        "bilinear" | "1" => true,
                        _ => {
                            let v = crate::owned_expression::constant(value)?;
                            if !v.is_finite() || !(0.0..=64.).contains(&v) || v.fract() != 0. {
                                return Err("invalid lenscorrection interpolation".into());
                            }
                            v != 0.
                        }
                    };
                }
                "fc" => fill = crate::owned_rgba::parse(value)?,
                "enable" => {
                    enable = crate::owned_timeline::Timeline::grayworld(&format!("enable={value}"))?
                }
                _ => return Err("unknown lenscorrection option".into()),
            }
        }
        Ok(Self {
            center,
            coefficients,
            bilinear,
            fill,
            enable,
            maps: RefCell::default(),
        })
    }
    fn map(&self, w: usize, h: usize) -> Result<()> {
        let mut maps = self.maps.borrow_mut();
        if maps.iter().any(|m| m.width == w && m.height == h) {
            return Ok(());
        }
        let size = w.checked_mul(h).ok_or("lenscorrection map overflow")?;
        let mut values = Vec::new();
        values.try_reserve_exact(size).map_err(|e| e.to_string())?;
        let center = [
            (self.center[0] * w as f64) as i128,
            (self.center[1] * h as f64) as i128,
        ];
        let inverse = (1i128 << 62) / (w as i128 * w as i128 + h as i128 * h as i128);
        for y in 0..h {
            for x in 0..w {
                let dx = x as i128 - center[0];
                let dy = y as i128 - center[1];
                let distance = ((dx * dx + dy * dy) * inverse + (1 << 31)) >> 32;
                let fourth = (distance * distance + (1 << 27)) >> 28;
                let multiplier = UNIT
                    + ((distance * self.coefficients[0] as i128
                        + fourth * self.coefficients[1] as i128
                        + (1 << 27))
                        >> 28);
                values.push(
                    i64::try_from(multiplier)
                        .map_err(|_| "lenscorrection radial multiplier overflow")?,
                );
            }
        }
        // Keep only the current frame's small set of plane geometries across playback.
        if maps.len() == 4 {
            maps.clear();
        }
        maps.push(Map {
            width: w,
            height: h,
            multipliers: values,
        });
        Ok(())
    }
    fn fill_sample(&self, component: Component, depth: u8) -> u16 {
        let [r, g, b, a] = self.fill.map(i64::from);
        let coefficient = |v: f64| (v * 1024. + 0.5) as i64;
        let sample = match component {
            Component::Luma => {
                (coefficient(0.21260 * 219. / 255.) * r
                    + coefficient(0.71520 * 219. / 255.) * g
                    + coefficient(0.07220 * 219. / 255.) * b
                    + 512
                    + (16 << 10))
                    >> 10
            }
            Component::Cb => {
                ((-coefficient(0.11457 * 224. / 255.) * r - coefficient(0.38543 * 224. / 255.) * g
                    + coefficient(0.5 * 224. / 255.) * b
                    + 511)
                    >> 10)
                    + 128
            }
            Component::Cr => {
                ((coefficient(0.5 * 224. / 255.) * r
                    - coefficient(0.45415 * 224. / 255.) * g
                    - coefficient(0.04585 * 224. / 255.) * b
                    + 511)
                    >> 10)
                    + 128
            }
            Component::Red => r,
            Component::Green => g,
            Component::Blue => b,
            Component::Alpha => a,
        };
        (sample << (depth - 8)) as u16
    }
    pub fn apply(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        crate::owned_overlay::validate_frame(frame, depth)?;
        if !self.enable.enabled(n, t, frame.width, frame.height)? {
            return Ok(());
        }
        let mut output = crate::owned_frame::buffer(frame.data.len())?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let mut offset = 0;
        if let Some([sx, sy]) = frame.subsampling {
            for (w, h, c) in [
                (frame.width, frame.height, Component::Luma),
                (
                    frame.width.div_ceil(sx),
                    frame.height.div_ceil(sy),
                    Component::Cb,
                ),
                (
                    frame.width.div_ceil(sx),
                    frame.height.div_ceil(sy),
                    Component::Cr,
                ),
            ] {
                let size = w * h * bytes;
                self.sample(
                    &frame.data[offset..offset + size],
                    &mut output[offset..offset + size],
                    w,
                    h,
                    depth,
                    1,
                    0,
                    c,
                )?;
                offset += size;
            }
        } else {
            for (channel, c) in [Component::Red, Component::Green, Component::Blue]
                .into_iter()
                .enumerate()
            {
                self.sample(
                    &frame.data,
                    &mut output,
                    frame.width,
                    frame.height,
                    depth,
                    3,
                    channel,
                    c,
                )?;
            }
        }
        frame.data = output;
        Ok(())
    }
    pub fn apply_plane(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        component: Component,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        let bytes = if depth == 8 { 1 } else { 2 };
        if w == 0
            || h == 0
            || !(8..=16).contains(&depth)
            || w.checked_mul(h).and_then(|v| v.checked_mul(bytes)) != Some(data.len())
        {
            return Err("invalid lenscorrection plane geometry/storage".into());
        }
        if depth > 8
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 >= 1u32 << depth)
        {
            return Err("lenscorrection sample exceeds precision".into());
        }
        if !self.enable.enabled(n, t, w, h)? {
            return Ok(());
        }
        let mut output = crate::owned_frame::buffer(data.len())?;
        self.sample(data, &mut output, w, h, depth, 1, 0, component)?;
        data.copy_from_slice(&output);
        Ok(())
    }
    fn sample(
        &self,
        input: &[u8],
        output: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        stride: usize,
        channel: usize,
        component: Component,
    ) -> Result<()> {
        self.map(w, h)?;
        let maps = self.maps.borrow();
        let map = maps
            .iter()
            .find(|m| m.width == w && m.height == h)
            .ok_or("lenscorrection map missing")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let center = [
            (self.center[0] * w as f64) as i128,
            (self.center[1] * h as f64) as i128,
        ];
        let mask = UNIT - 1;
        let fill = self.fill_sample(component, depth);
        let read = |x: usize, y: usize| -> u128 {
            let at = ((y * w + x) * stride + channel) * bytes;
            if bytes == 1 {
                input[at] as u128
            } else {
                u16::from_le_bytes([input[at], input[at + 1]]) as u128
            }
        };
        for y in 0..h {
            for x in 0..w {
                let m = map.multipliers[y * w + x] as i128;
                let delta = [x as i128 - center[0], y as i128 - center[1]];
                let source = [
                    center[0] + ((m * delta[0] + UNIT / 2) >> 24),
                    center[1] + ((m * delta[1] + UNIT / 2) >> 24),
                ];
                let value = if source[0] < 0
                    || source[1] < 0
                    || source[0] >= w as i128
                    || source[1] >= h as i128
                {
                    fill
                } else {
                    let (u, v) = (source[0] as usize, source[1] as usize);
                    if !self.bilinear {
                        read(u, v) as u16
                    } else {
                        let fraction = |d: i128| -> u128 {
                            if d >= 0 {
                                ((m * d + UNIT / 2) & mask) as u128
                            } else {
                                (mask - ((m * (-d) + UNIT / 2) & mask)) as u128
                            }
                        };
                        let a = fraction(delta[0]);
                        let b = fraction(delta[1]);
                        let limit = mask as u128;
                        let nextu = (u + 1).min(w - 1);
                        let nextv = (v + 1).min(h - 1);
                        let top = (limit - a) * read(u, v) + a * read(nextu, v);
                        let bottom = (limit - a) * read(u, nextv) + a * read(nextu, nextv);
                        (((limit - b) * top + b * bottom + (1u128 << 47)) >> 48)
                            .min((1u128 << depth) - 1) as u16
                    }
                };
                let at = ((y * w + x) * stride + channel) * bytes;
                if bytes == 1 {
                    output[at] = value as u8
                } else {
                    output[at..at + 2].copy_from_slice(&value.to_le_bytes())
                }
            }
        }
        Ok(())
    }
}
