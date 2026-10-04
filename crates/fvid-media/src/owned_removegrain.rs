//! Owned spatial grain removal. Each output reads the unchanged 3x3 source window.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Debug)]
pub struct RemoveGrain {
    modes: [u8; 4],
    enable: crate::owned_timeline::Timeline,
}
impl RemoveGrain {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid removegrain options".into());
        }
        let mut modes = [0; 4];
        let mut enable = Default::default();
        let mut position = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *["m0", "m1", "m2", "m3"]
                    .get(position)
                    .ok_or("too many removegrain options")?;
                position += 1;
                (key, entry)
            };
            let value = value.trim().trim_matches('\'');
            if key.trim() == "enable" {
                enable = crate::owned_timeline::Timeline::grayworld(&format!("enable={value}"))?;
                continue;
            }
            let plane = match key.trim() {
                "m0" => 0,
                "m1" => 1,
                "m2" => 2,
                "m3" => 3,
                _ => return Err("unknown removegrain option".into()),
            };
            let number = crate::owned_expression::constant(value)?;
            if !number.is_finite() || number.fract() != 0. || !(0.0..=24.).contains(&number) {
                return Err("removegrain mode must be an integer from 0 to 24".into());
            }
            modes[plane] = number as u8;
        }
        Ok(Self { modes, enable })
    }
    pub fn apply(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        crate::owned_overlay::validate_frame(frame, depth)?;
        if self.modes == [0; 4] || !self.enable.enabled(n, t, frame.width, frame.height)? {
            return Ok(());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let mut out = crate::owned_frame::buffer(frame.data.len())?;
        out.copy_from_slice(&frame.data);
        if let Some([sx, sy]) = frame.subsampling {
            let mut offset = 0;
            for (plane, (w, h)) in [
                (frame.width, frame.height),
                (frame.width.div_ceil(sx), frame.height.div_ceil(sy)),
                (frame.width.div_ceil(sx), frame.height.div_ceil(sy)),
            ]
            .into_iter()
            .enumerate()
            {
                let size = w * h * bytes;
                self.filter(
                    &frame.data[offset..offset + size],
                    &mut out[offset..offset + size],
                    w,
                    h,
                    depth,
                    plane,
                    3,
                    1,
                    0,
                );
                offset += size;
            }
        } else {
            for (plane, channel) in [1usize, 2, 0].into_iter().enumerate() {
                self.filter(
                    &frame.data,
                    &mut out,
                    frame.width,
                    frame.height,
                    depth,
                    plane,
                    3,
                    3,
                    channel,
                );
            }
        }
        frame.data = out;
        Ok(())
    }
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
        self.apply_plane_in_frame(data, w, h, depth, plane, [w, h], 4, n, t)
    }
    /// Full source dimensions keep timeline expressions shared by subsampled planes.
    pub fn apply_plane_in_frame(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        plane: usize,
        source: [usize; 2],
        active_planes: usize,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        let bytes = if depth == 8 { 1 } else { 2 };
        if !(1..=4).contains(&active_planes)
            || plane >= active_planes
            || source.contains(&0)
            || w > source[0]
            || h > source[1]
            || w == 0
            || h == 0
            || plane > 3
            || !(8..=16).contains(&depth)
            || w.checked_mul(h).and_then(|v| v.checked_mul(bytes)) != Some(data.len())
        {
            return Err("invalid removegrain plane geometry/storage".into());
        }
        if depth > 8
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 >= 1u32 << depth)
        {
            return Err("removegrain sample exceeds precision".into());
        }
        if self.modes[plane] == 0 || !self.enable.enabled(n, t, source[0], source[1])? {
            return Ok(());
        }
        let mut out = crate::owned_frame::buffer(data.len())?;
        out.copy_from_slice(data);
        self.filter(data, &mut out, w, h, depth, plane, active_planes, 1, 0);
        data.copy_from_slice(&out);
        Ok(())
    }
    fn filter(
        &self,
        input: &[u8],
        output: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        plane: usize,
        active_planes: usize,
        stride: usize,
        channel: usize,
    ) {
        let mode = self.modes[plane];
        if mode == 0 || w < 3 || h < 3 {
            return;
        }
        // Legacy modes 13..16 select parity across every active plane, including mixed modes.
        let skip_odd = self.modes[..active_planes]
            .iter()
            .any(|m| matches!(m, 13 | 15));
        let skip_even = self.modes[..active_planes]
            .iter()
            .any(|m| matches!(m, 14 | 16));
        let bytes = if depth == 8 { 1 } else { 2 };
        let read = |x: usize, y: usize| {
            let at = ((y * w + x) * stride + channel) * bytes;
            if bytes == 1 {
                input[at] as i32
            } else {
                u16::from_le_bytes([input[at], input[at + 1]]) as i32
            }
        };
        for y in 1..h - 1 {
            if (skip_odd && y % 2 == 1) || (skip_even && y % 2 == 0) {
                continue;
            }
            for x in 1..w - 1 {
                let a = [
                    read(x - 1, y - 1),
                    read(x, y - 1),
                    read(x + 1, y - 1),
                    read(x - 1, y),
                    read(x + 1, y),
                    read(x - 1, y + 1),
                    read(x, y + 1),
                    read(x + 1, y + 1),
                ];
                let value = sample(mode, read(x, y), a) as u16;
                let at = ((y * w + x) * stride + channel) * bytes;
                if bytes == 1 {
                    output[at] = value as u8;
                } else {
                    output[at..at + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
}
fn sample(mode: u8, c: i32, mut a: [i32; 8]) -> i32 {
    let pairs = [(a[0], a[7]), (a[1], a[6]), (a[2], a[5]), (a[3], a[4])];
    let low = pairs.map(|(a, b)| a.min(b));
    let high = pairs.map(|(a, b)| a.max(b));
    let width = std::array::from_fn::<_, 4, _>(|i| high[i] - low[i]);
    let clipped = std::array::from_fn::<_, 4, _>(|i| c.clamp(low[i], high[i]));
    let pick = |cost: [i32; 4]| {
        let min = *cost.iter().min().unwrap();
        [3, 1, 2, 0].into_iter().find(|&i| cost[i] == min).unwrap()
    };
    match mode {
        0 => c,
        1..=4 => {
            a.sort_unstable();
            let index = mode as usize - 1;
            c.clamp(a[index], a[7 - index])
        }
        5..=9 | 18 => {
            let costs = std::array::from_fn(|i| match mode {
                5 => (c - clipped[i]).abs(),
                6 => 2 * (c - clipped[i]).abs() + width[i],
                7 => (c - clipped[i]).abs() + width[i],
                8 => (c - clipped[i]).abs() + 2 * width[i],
                9 => width[i],
                _ => (c - pairs[i].0).abs().max((c - pairs[i].1).abs()),
            });
            clipped[pick(costs)]
        }
        10 => {
            let d = a.map(|v| (c - v).abs());
            let min = *d.iter().min().unwrap();
            a[[6, 7, 5, 1, 2, 0, 4, 3]
                .into_iter()
                .find(|&i| d[i] == min)
                .unwrap()]
        }
        11 | 12 => (4 * c + 2 * (a[1] + a[3] + a[4] + a[6]) + a[0] + a[2] + a[5] + a[7] + 8) / 16,
        13..=16 => {
            let costs = [width[0], width[1], width[2]];
            let min = *costs.iter().min().unwrap();
            let i = [1, 2, 0].into_iter().find(|&i| costs[i] == min).unwrap();
            if mode <= 14 {
                (pairs[i].0 + pairs[i].1 + 1) / 2
            } else {
                let avg = (2 * (a[1] + a[6]) + a[0] + a[2] + a[5] + a[7] + 4) / 8;
                avg.clamp(low[i], high[i])
            }
        }
        17 => {
            let l = *low.iter().max().unwrap();
            let h = *high.iter().min().unwrap();
            c.clamp(l.min(h), l.max(h))
        }
        19 => (a.iter().sum::<i32>() + 4) / 8,
        20 => (a.iter().sum::<i32>() + c + 4) / 9,
        21 | 22 => {
            let low = pairs.map(|(a, b)| (a + b + if mode == 22 { 1 } else { 0 }) / 2);
            let high = pairs.map(|(a, b)| (a + b + 1) / 2);
            c.clamp(*low.iter().min().unwrap(), *high.iter().max().unwrap())
        }
        23 | 24 => {
            let mut up = 0;
            let mut down = 0;
            for i in 0..4 {
                let u = c - high[i];
                let d = low[i] - c;
                up = up.max(u.min(if mode == 23 { width[i] } else { width[i] - u }));
                down = down.max(d.min(if mode == 23 { width[i] } else { width[i] - d }));
            }
            c - up + down
        }
        _ => unreachable!("validated removegrain mode"),
    }
}
