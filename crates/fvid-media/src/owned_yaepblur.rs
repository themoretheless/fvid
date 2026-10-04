//! Owned local-statistics edge-preserving blur using bounded-width rolling sums.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Debug)]
pub struct YaepBlur {
    radius: usize,
    planes: u8,
    sigma: u128,
    enable: crate::owned_timeline::Timeline,
}
impl YaepBlur {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid yaepblur options".into());
        }
        let mut radius = 3;
        let mut planes = 1;
        let mut sigma = 128;
        let mut enable = Default::default();
        let mut position = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(v) = entry.split_once('=') {
                v
            } else {
                let key = *["radius", "planes", "sigma"]
                    .get(position)
                    .ok_or("too many yaepblur options")?;
                position += 1;
                (key, entry)
            };
            let key = key.trim();
            let value = value.trim().trim_matches('\'');
            if key == "enable" {
                enable = crate::owned_timeline::Timeline::grayworld(&format!("enable={value}"))?;
                continue;
            }
            let v = crate::owned_expression::constant(value)?;
            let minimum = if matches!(key, "sigma" | "s") { 1. } else { 0. };
            let maximum = if matches!(key, "planes" | "p") {
                15.
            } else {
                i32::MAX as f64
            };
            if !v.is_finite() || v.fract() != 0. || !(minimum..=maximum).contains(&v) {
                return Err("invalid yaepblur integer parameter".into());
            }
            match key {
                "radius" | "r" => radius = v as usize,
                "planes" | "p" => planes = v as u8,
                "sigma" | "s" => sigma = v as u128,
                _ => return Err("unknown yaepblur option".into()),
            }
        }
        Ok(Self {
            radius,
            planes,
            sigma,
            enable,
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
        if self.radius == 0
            || self.planes == 0
            || !self.enable.enabled(n, t, frame.width, frame.height)?
        {
            return Ok(());
        }
        let radius = self.radius.min(frame.width.min(frame.height).div_ceil(2));
        let bytes = if depth == 8 { 1 } else { 2 };
        let mut output = crate::owned_frame::buffer(frame.data.len())?;
        output.copy_from_slice(&frame.data);
        if let Some([sx, sy]) = frame.subsampling {
            let mut offset = 0;
            for (p, (w, h)) in [
                (frame.width, frame.height),
                (frame.width.div_ceil(sx), frame.height.div_ceil(sy)),
                (frame.width.div_ceil(sx), frame.height.div_ceil(sy)),
            ]
            .into_iter()
            .enumerate()
            {
                let size = w * h * bytes;
                if self.planes & (1 << p) != 0 {
                    self.filter(
                        &frame.data[offset..offset + size],
                        &mut output[offset..offset + size],
                        w,
                        h,
                        depth,
                        radius,
                        1,
                        0,
                    )?;
                }
                offset += size;
            }
        } else {
            for (plane, channel) in [1usize, 2, 0].into_iter().enumerate() {
                if self.planes & (1 << plane) != 0 {
                    self.filter(
                        &frame.data,
                        &mut output,
                        frame.width,
                        frame.height,
                        depth,
                        radius,
                        3,
                        channel,
                    )?;
                }
            }
        }
        frame.data = output;
        Ok(())
    }
    /// Standalone component; plane indices use YUV/GBR/alpha order.
    pub fn apply_plane(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        plane: usize,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        self.apply_plane_in_frame(data, w, h, depth, plane, [w, h], n, t)
    }
    /// Supply full source dimensions so subsampled planes share the radius cap and timeline.
    pub fn apply_plane_in_frame(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        plane: usize,
        source: [usize; 2],
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        let bytes = if depth == 8 { 1 } else { 2 };
        if w == 0
            || h == 0
            || source.contains(&0)
            || w > source[0]
            || h > source[1]
            || plane > 3
            || !(8..=16).contains(&depth)
            || w.checked_mul(h).and_then(|v| v.checked_mul(bytes)) != Some(data.len())
        {
            return Err("invalid yaepblur plane geometry/storage".into());
        }
        if depth > 8
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 >= 1u32 << depth)
        {
            return Err("yaepblur sample exceeds precision".into());
        }
        if self.radius == 0
            || self.planes & (1 << plane) == 0
            || !self.enable.enabled(n, t, source[0], source[1])?
        {
            return Ok(());
        }
        let radius = self.radius.min(source[0].min(source[1]).div_ceil(2));
        let mut output = crate::owned_frame::buffer(data.len())?;
        self.filter(data, &mut output, w, h, depth, radius, 1, 0)?;
        data.copy_from_slice(&output);
        Ok(())
    }
    fn filter(
        &self,
        input: &[u8],
        output: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        r: usize,
        stride: usize,
        channel: usize,
    ) -> Result<()> {
        let bytes = if depth == 8 { 1 } else { 2 };
        let read = |x: usize, y: usize| -> u128 {
            let at = ((y * w + x) * stride + channel) * bytes;
            if bytes == 1 {
                input[at] as u128
            } else {
                u16::from_le_bytes([input[at], input[at + 1]]) as u128
            }
        };
        let mut columns = Vec::new();
        columns.try_reserve_exact(w).map_err(|e| e.to_string())?;
        columns.resize(w, (0u128, 0u128));
        let mut top = 0;
        let mut bottom = 0;
        for y in 0..h {
            let low = y.saturating_sub(r);
            let high = y.saturating_add(r).saturating_add(1).min(h);
            for row in top..low {
                for (x, (sum, square)) in columns.iter_mut().enumerate() {
                    let v = read(x, row);
                    *sum -= v;
                    *square -= v * v;
                }
            }
            for row in bottom..high {
                for (x, (sum, square)) in columns.iter_mut().enumerate() {
                    let v = read(x, row);
                    *sum += v;
                    *square += v * v;
                }
            }
            top = low;
            bottom = high;
            let mut left = 0;
            let mut right = (r + 1).min(w);
            let mut sum = 0;
            let mut square = 0;
            for &(s, q) in &columns[..right] {
                sum += s;
                square += q;
            }
            for x in 0..w {
                let count = ((bottom - top) * (right - left)) as u128;
                let mean = sum / count;
                let variance = (square - sum * sum / count) / count;
                let pixel = read(x, y);
                let value =
                    ((self.sigma * mean + variance * pixel) / (self.sigma + variance)) as u16;
                let at = ((y * w + x) * stride + channel) * bytes;
                if bytes == 1 {
                    output[at] = value as u8
                } else {
                    output[at..at + 2].copy_from_slice(&value.to_le_bytes())
                }
                let next_left = (x + 1).saturating_sub(r);
                let next_right = (x + 1).saturating_add(r).saturating_add(1).min(w);
                for &(s, q) in &columns[left..next_left] {
                    sum -= s;
                    square -= q;
                }
                for &(s, q) in &columns[right..next_right] {
                    sum += s;
                    square += q;
                }
                left = next_left;
                right = next_right;
            }
        }
        Ok(())
    }
}
