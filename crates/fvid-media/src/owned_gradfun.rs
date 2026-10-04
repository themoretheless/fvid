//! Owned gradient debanding: sliding rectangular sums, detail gating and Bayer dither.
//! Reference behavior: https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/vf_gradfun.c
use crate::owned_frame::GeometryFrame;
use std::cell::RefCell;
#[derive(Debug, Default)]
struct Scratch {
    geometry: Option<(usize, usize, u8)>,
    data: Vec<u64>,
}
type Result<T> = std::result::Result<T, String>;
#[derive(Debug)]
pub struct GradFun {
    scratch: RefCell<Scratch>,
    strength: f32,
    radius: usize,
    enable: crate::owned_timeline::Timeline,
}
impl GradFun {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid gradfun options".into());
        }
        let mut strength = 1.2f32;
        let mut radius = 16;
        let mut enable = Default::default();
        let mut position = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(v) = entry.split_once('=') {
                v
            } else {
                let key = *["strength", "radius"]
                    .get(position)
                    .ok_or("too many gradfun options")?;
                position += 1;
                (key, entry)
            };
            match key.trim() {
                "strength" => {
                    let v = crate::owned_expression::constant(value.trim())?;
                    if !v.is_finite() || !(0.51..=64.).contains(&v) {
                        return Err("gradfun strength must be from 0.51 to 64".into());
                    }
                    strength = v as f32;
                }
                "radius" => {
                    let v = crate::owned_expression::constant(value.trim())?;
                    if !v.is_finite() || !(4.0..=32.).contains(&v) || v.fract() != 0. {
                        return Err("gradfun radius must be an integer from 4 to 32".into());
                    }
                    radius = ((v as usize + 1) & !1).clamp(4, 32);
                }
                "enable" => {
                    enable = crate::owned_timeline::Timeline::grayworld(&format!(
                        "enable={}",
                        value.trim()
                    ))?
                }
                _ => return Err("unknown gradfun option".into()),
            }
        }
        Ok(Self {
            scratch: RefCell::default(),
            strength,
            radius,
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
        if n == 0 {
            self.scratch.borrow_mut().data.fill(0);
        }
        if !self.enable.enabled(n, t, frame.width, frame.height)? {
            return Ok(());
        }
        self.prepare(frame.width, frame.height, depth, n)?;
        let mut output = crate::owned_frame::buffer(frame.data.len())?;
        output.copy_from_slice(&frame.data);
        if let Some([sx, sy]) = frame.subsampling {
            let cr = (((self.radius / sx + self.radius / sy) / 2 + 1) & !1).clamp(4, 32);
            let mut offset = 0;
            let bytes = if depth == 8 { 1 } else { 2 };
            for (w, h, r) in [
                (frame.width, frame.height, self.radius),
                (frame.width.div_ceil(sx), frame.height.div_ceil(sy), cr),
                (frame.width.div_ceil(sx), frame.height.div_ceil(sy), cr),
            ] {
                let size = w * h * bytes;
                self.filter(
                    &frame.data[offset..offset + size],
                    &mut output[offset..offset + size],
                    w,
                    h,
                    depth,
                    r,
                    1,
                    0,
                )?;
                offset += size;
            }
        } else {
            for channel in 0..3 {
                self.filter(
                    &frame.data,
                    &mut output,
                    frame.width,
                    frame.height,
                    depth,
                    self.radius,
                    3,
                    channel,
                )?;
            }
        }
        frame.data = output;
        Ok(())
    }
    /// Standalone grayscale/GBR/alpha component; radius uses supplied sampling axes.
    pub fn apply_plane(
        &self,
        data: &mut [u8],
        width: usize,
        height: usize,
        depth: u8,
        sampling: [usize; 2],
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        let bytes = if depth == 8 { 1 } else { 2 };
        if width == 0
            || height == 0
            || !(8..=16).contains(&depth)
            || sampling.iter().any(|&s| ![1, 2, 4].contains(&s))
            || width.checked_mul(height).and_then(|v| v.checked_mul(bytes)) != Some(data.len())
        {
            return Err("invalid gradfun plane storage or geometry".into());
        }
        if depth > 8
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 >= 1u32 << depth)
        {
            return Err("gradfun sample exceeds precision".into());
        }
        if n == 0 {
            self.scratch.borrow_mut().data.fill(0);
        }
        if !self.enable.enabled(n, t, width, height)? {
            return Ok(());
        }
        let r =
            (((self.radius / sampling[0] + self.radius / sampling[1]) / 2 + 1) & !1).clamp(4, 32);
        self.prepare(width, height, depth, n)?;
        let mut output = crate::owned_frame::buffer(data.len())?;
        output.copy_from_slice(data);
        self.filter(data, &mut output, width, height, depth, r, 1, 0)?;
        data.copy_from_slice(&output);
        Ok(())
    }
    fn prepare(&self, w: usize, h: usize, depth: u8, n: u64) -> Result<()> {
        let mut scratch = self.scratch.borrow_mut();
        if scratch.geometry != Some((w, h, depth)) {
            let size = w
                .checked_add(15)
                .map(|v| v & !15)
                .and_then(|v| v.checked_mul(self.radius + 1))
                .map(|v| v / 2)
                .and_then(|v| v.checked_add(32))
                .ok_or("gradfun scratch overflow")?;
            let mut data = Vec::new();
            data.try_reserve_exact(size).map_err(|e| e.to_string())?;
            data.resize(size, 0);
            scratch.data = data;
            scratch.geometry = Some((w, h, depth));
        } else if n == 0 {
            scratch.data.fill(0);
        }
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
        if w.min(h) <= 2 * r {
            return Ok(());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let maximum = (1i64 << depth) - 1;
        let read = |x: usize, y: usize| -> u64 {
            let at = ((y * w + x) * stride + channel) * bytes;
            if bytes == 1 {
                input[at] as u64
            } else {
                u16::from_le_bytes([input[at], input[at + 1]]) as u64
            }
        };
        let half = w / 2;
        let mut columns = Vec::new();
        columns.try_reserve_exact(half).map_err(|e| e.to_string())?;
        columns.resize(half, 0u64);
        for y in 0..2 * r {
            for (x, sum) in columns.iter_mut().enumerate() {
                *sum += read(2 * x, y) + read(2 * x + 1, y);
            }
        }
        let mut dc = Vec::new();
        dc.try_reserve_exact(half + r).map_err(|e| e.to_string())?;
        dc.resize(half + r, 0u64);
        let bstride = (w + 15) / 16 * 8;
        let base = bstride + 32;
        let mask = (1u64 << (depth + 8)) - 1;
        let mut scratch = self.scratch.borrow_mut();
        let memory = &mut scratch.data;
        memory[16..base].fill(0);
        let mut initial = vec![0u64; half];
        for row in 0..r {
            for x in 0..half {
                let current = base + row * bstride + x;
                let previous = current - bstride;
                let value = (memory[previous]
                    + read(2 * x, 2 * row)
                    + read(2 * x + 1, 2 * row)
                    + read(2 * x, 2 * row + 1)
                    + read(2 * x + 1, 2 * row + 1))
                    & mask;
                initial[x] = value.wrapping_sub(memory[current]) & mask;
                memory[current] = value;
            }
        }
        let factor = (1u64 << 21) / (r * r) as u64;
        let threshold = (32768f32 / (self.strength * (1u32 << (depth - 8)) as f32)) as i64;
        let mut origin = 0usize;
        // The first computed window is centered one pair beyond the initial rows.
        // A 2r+1-row plane retains its unscaled initial column sums instead.
        if r + 1 >= h - r {
            for x in 0..half {
                dc[r / 2 + x] = initial[x];
            }
        }
        let mut y = r;
        loop {
            if y + 1 < h - r {
                for (x, sum) in columns.iter_mut().enumerate() {
                    *sum -= read(2 * x, origin)
                        + read(2 * x + 1, origin)
                        + read(2 * x, origin + 1)
                        + read(2 * x + 1, origin + 1);
                    *sum += read(2 * x, origin + 2 * r)
                        + read(2 * x + 1, origin + 2 * r)
                        + read(2 * x, origin + 2 * r + 1)
                        + read(2 * x + 1, origin + 2 * r + 1);
                }
                let row = y + r;
                let slot = (row / 2) % r;
                let previous = if slot == 0 { r - 1 } else { slot - 1 };
                for x in 0..half {
                    memory[base + slot * bstride + x] = (memory[base + previous * bstride + x]
                        + read(2 * x, row)
                        + read(2 * x + 1, row)
                        + read(2 * x, row + 1)
                        + read(2 * x + 1, row + 1))
                        & mask;
                }
                origin += 2;
                let mut sum = columns[..r].iter().sum::<u64>();
                for x in r..half {
                    sum += columns[x];
                    sum -= columns[x - r];
                    dc[x - r] = sum * factor >> 16;
                }
                for x in half..(w + r + 1) / 2 {
                    dc[x - r] = sum * factor >> 16;
                }
                // Expand the box result to sample coordinates with repeated left edge.
                dc.copy_within(0..half, r / 2);
                let first = dc[r / 2];
                dc[..r / 2].fill(first);
            }
            let start = if y == r { 0 } else { y };
            let end = (y + 2).min(h);
            for row in start..end {
                for x in 0..w {
                    let pixel = (read(x, row) as i64) << 7;
                    let delta = dc[x / 2] as i64 - pixel;
                    let weight = (127 - ((delta.abs() * threshold) >> 16)).max(0);
                    let adjustment = (weight * weight * delta) >> 14;
                    let mut dither = 0;
                    for bit in 0..3 {
                        let xb = (x >> bit) & 1;
                        let yb = (row >> bit) & 1;
                        dither += (((xb ^ yb) << 1) | xb) << (5 - 2 * bit);
                    }
                    let value =
                        ((pixel + adjustment + dither as i64) >> 7).clamp(0, maximum) as u16;
                    let at = ((row * w + x) * stride + channel) * bytes;
                    if bytes == 1 {
                        output[at] = value as u8
                    } else {
                        output[at..at + 2].copy_from_slice(&value.to_le_bytes())
                    }
                }
            }
            y += 2;
            if y >= h {
                break;
            }
        }
        Ok(())
    }
}
