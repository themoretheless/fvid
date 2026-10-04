//! Owned directional four-neighbor debanding with deterministic spatial sampling.
use crate::owned_frame::GeometryFrame;
use std::cell::RefCell;
type Result<T> = std::result::Result<T, String>;
#[derive(Debug)]
struct Map {
    width: usize,
    height: usize,
    points: Vec<[i64; 2]>,
}
#[derive(Clone, Copy)]
struct Plane {
    width: usize,
    height: usize,
    offset: usize,
    stride: usize,
}
#[derive(Debug)]
pub struct Deband {
    threshold: [f32; 4],
    range: i32,
    direction: f32,
    blur: bool,
    coupling: bool,
    enable: crate::owned_timeline::Timeline,
    map: RefCell<Option<Map>>,
}
impl Deband {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid deband options".into());
        }
        let mut threshold = [0.02; 4];
        let mut range = 16;
        let mut direction = std::f64::consts::TAU as f32;
        let mut blur = true;
        let mut coupling = false;
        let mut enable = Default::default();
        let mut position = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *[
                    "1thr",
                    "2thr",
                    "3thr",
                    "4thr",
                    "range",
                    "direction",
                    "blur",
                    "coupling",
                ]
                .get(position)
                .ok_or("too many deband options")?;
                position += 1;
                (key, entry)
            };
            let key = key.trim();
            let value = value.trim().trim_matches('\'');
            if key == "enable" {
                enable = crate::owned_timeline::Timeline::grayworld(&format!("enable={value}"))?;
                continue;
            }
            if matches!(key, "blur" | "b" | "coupling" | "c") {
                let v = match value {
                    "1" | "true" | "yes" | "on" => true,
                    "0" | "false" | "no" | "off" => false,
                    _ => return Err("invalid deband boolean flag".into()),
                };
                if matches!(key, "blur" | "b") {
                    blur = v;
                } else {
                    coupling = v;
                }
                continue;
            }
            let v = crate::owned_expression::constant(value)?;
            if !v.is_finite() {
                return Err("deband parameter must be finite".into());
            }
            match key {
                "1thr" | "2thr" | "3thr" | "4thr" => {
                    if !(0.00003..=0.5).contains(&v) {
                        return Err("deband threshold must be from 0.00003 to 0.5".into());
                    }
                    threshold[key.as_bytes()[0] as usize - b'1' as usize] = v as f32;
                }
                "range" | "r" => {
                    if v.fract() != 0. || v < i32::MIN as f64 || v > i32::MAX as f64 {
                        return Err("invalid deband range".into());
                    }
                    range = v as i32;
                }
                "direction" | "d" => {
                    if v.abs() > std::f64::consts::TAU {
                        return Err("invalid deband direction".into());
                    }
                    direction = v as f32;
                }
                _ => return Err("unknown deband option".into()),
            }
        }
        Ok(Self {
            threshold,
            range,
            direction,
            blur,
            coupling,
            enable,
            map: RefCell::default(),
        })
    }
    pub fn coupled(&self) -> bool {
        self.coupling
    }
    pub fn needs_equal_planes(args: Option<&str>) -> bool {
        args.and_then(|a| Self::parse(a).ok())
            .is_some_and(|v| v.coupled())
    }
    pub fn apply(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        crate::owned_overlay::validate_frame(frame, depth)?;
        let planes = if let Some([sx, sy]) = frame.subsampling {
            let size = frame.width * frame.height;
            let cw = frame.width.div_ceil(sx);
            let ch = frame.height.div_ceil(sy);
            vec![
                Plane {
                    width: frame.width,
                    height: frame.height,
                    offset: 0,
                    stride: 1,
                },
                Plane {
                    width: cw,
                    height: ch,
                    offset: size,
                    stride: 1,
                },
                Plane {
                    width: cw,
                    height: ch,
                    offset: size + cw * ch,
                    stride: 1,
                },
            ]
        } else {
            [1usize, 2, 0]
                .map(|offset| Plane {
                    width: frame.width,
                    height: frame.height,
                    offset,
                    stride: 3,
                })
                .to_vec()
        };
        self.process(&mut frame.data, depth, &planes, n, t)
    }
    pub fn apply_planar(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        sub: [usize; 2],
        depth: u8,
        alpha: bool,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        if w == 0 || h == 0 || sub.iter().any(|v| !matches!(v, 1 | 2 | 4)) {
            return Err("invalid deband planar geometry".into());
        }
        let size = w.checked_mul(h).ok_or("deband dimensions overflow")?;
        let cw = w.div_ceil(sub[0]);
        let ch = h.div_ceil(sub[1]);
        let chroma = cw.checked_mul(ch).ok_or("deband dimensions overflow")?;
        let mut planes = vec![
            Plane {
                width: w,
                height: h,
                offset: 0,
                stride: 1,
            },
            Plane {
                width: cw,
                height: ch,
                offset: size,
                stride: 1,
            },
            Plane {
                width: cw,
                height: ch,
                offset: size
                    .checked_add(chroma)
                    .ok_or("deband dimensions overflow")?,
                stride: 1,
            },
        ];
        if alpha {
            planes.push(Plane {
                width: w,
                height: h,
                offset: size
                    .checked_add(chroma.checked_mul(2).ok_or("deband dimensions overflow")?)
                    .ok_or("deband dimensions overflow")?,
                stride: 1,
            });
        }
        self.process(data, depth, &planes, n, t)
    }
    pub fn apply_rgb(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        channels: usize,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        if !matches!(channels, 3 | 4) {
            return Err("invalid deband RGB channel count".into());
        }
        let mut planes: Vec<_> = [1usize, 2, 0]
            .into_iter()
            .map(|offset| Plane {
                width: w,
                height: h,
                offset,
                stride: channels,
            })
            .collect();
        if channels == 4 {
            planes.push(Plane {
                width: w,
                height: h,
                offset: 3,
                stride: channels,
            });
        }
        self.process(data, depth, &planes, n, t)
    }
    pub fn apply_gray(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        self.process(
            data,
            depth,
            &[Plane {
                width: w,
                height: h,
                offset: 0,
                stride: 1,
            }],
            n,
            t,
        )
    }
    fn prepare(&self, w: usize, h: usize) -> Result<()> {
        let mut map = self.map.borrow_mut();
        if map.as_ref().is_some_and(|m| m.width == w && m.height == h) {
            return Ok(());
        }
        let size = w.checked_mul(h).ok_or("deband map overflow")?;
        let mut points = Vec::new();
        points.try_reserve_exact(size).map_err(|e| e.to_string())?;
        for y in 0..h {
            for x in 0..w {
                // Preserve the fused coordinate arithmetic qualified against the reference.
                let random =
                    (((x as f32).mul_add(12.9898, y as f32 * 78.233)).sin() * 43758.545).fract();
                let random = if random < 0. { random + 1. } else { random };
                let direction = if self.direction < 0. {
                    -self.direction
                } else {
                    random * self.direction
                };
                let distance = if self.range < 0 {
                    -(self.range as i64)
                } else {
                    (random * self.range as f32) as i64
                };
                points.push([
                    (direction.cos() * distance as f32) as i64,
                    (direction.sin() * distance as f32) as i64,
                ]);
            }
        }
        *map = Some(Map {
            width: w,
            height: h,
            points,
        });
        Ok(())
    }
    fn process(
        &self,
        data: &mut [u8],
        depth: u8,
        planes: &[Plane],
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        if !(8..=16).contains(&depth) || planes.is_empty() || planes.len() > 4 {
            return Err("invalid deband precision/layout".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let max = (1u32 << depth) - 1;
        let mut samples = 0;
        for p in planes {
            if p.width == 0
                || p.height == 0
                || p.width > i32::MAX as usize
                || p.height > i32::MAX as usize
            {
                return Err("invalid deband plane dimensions".into());
            }
            let end = p
                .width
                .checked_mul(p.height)
                .and_then(|v| v.checked_sub(1))
                .and_then(|v| v.checked_mul(p.stride))
                .and_then(|v| v.checked_add(p.offset))
                .and_then(|v| v.checked_add(1))
                .ok_or("deband plane storage overflow")?;
            samples = samples.max(end);
        }
        if samples.checked_mul(bytes) != Some(data.len())
            || depth > 8
                && data
                    .chunks_exact(2)
                    .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 > max)
        {
            return Err("invalid deband sample storage/precision".into());
        }
        let w = planes[0].width;
        let h = planes[0].height;
        if planes.iter().any(|p| p.width > w || p.height > h)
            || self.coupling && planes.iter().any(|p| p.width != w || p.height != h)
        {
            return Err("coupled deband requires equal-sized planes; subsampled format negotiation is not yet owned".into());
        }
        if !self.enable.enabled(n, t, w, h)? {
            return Ok(());
        }
        self.prepare(w, h)?;
        let map = self.map.borrow();
        let map = map.as_ref().unwrap();
        let mut output = crate::owned_frame::buffer(data.len())?;
        output.copy_from_slice(data);
        let thresholds = self.threshold.map(|v| (max as f32 * v) as i32);
        let read = |p: Plane, x: usize, y: usize| -> i32 {
            let at = (p.offset + (y * p.width + x) * p.stride) * bytes;
            if bytes == 1 {
                data[at] as i32
            } else {
                u16::from_le_bytes([data[at], data[at + 1]]) as i32
            }
        };
        let evaluate = |p: Plane, index: usize, x: usize, y: usize| {
            let [dx, dy] = map.points[y * w + x];
            let refs = [(dx, dy), (dx, -dy), (-dx, -dy), (-dx, dy)].map(|(dx, dy)| {
                read(
                    p,
                    (x as i64 + dx).clamp(0, p.width as i64 - 1) as usize,
                    (y as i64 + dy).clamp(0, p.height as i64 - 1) as usize,
                )
            });
            let src = read(p, x, y);
            let avg = refs.iter().sum::<i32>() / 4;
            let pass = if self.blur {
                (src - avg).abs() < thresholds[index]
            } else {
                refs.iter().all(|v| (src - v).abs() < thresholds[index])
            };
            (avg, pass)
        };
        let write = |output: &mut [u8], p: Plane, x: usize, y: usize, value: i32| {
            let at = (p.offset + (y * p.width + x) * p.stride) * bytes;
            if bytes == 1 {
                output[at] = value as u8;
            } else {
                output[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
            }
        };
        if self.coupling {
            for y in 0..h {
                for x in 0..w {
                    let mut avg = [0; 4];
                    let mut all = true;
                    for (index, p) in planes.iter().copied().enumerate() {
                        let (value, pass) = evaluate(p, index, x, y);
                        avg[index] = value;
                        all &= pass;
                    }
                    if all {
                        for (index, p) in planes.iter().copied().enumerate() {
                            write(&mut output, p, x, y, avg[index]);
                        }
                    }
                }
            }
        } else {
            for (index, p) in planes.iter().copied().enumerate() {
                for y in 0..p.height {
                    for x in 0..p.width {
                        let (value, pass) = evaluate(p, index, x, y);
                        if pass {
                            write(&mut output, p, x, y, value);
                        }
                    }
                }
            }
        }
        data.copy_from_slice(&output);
        Ok(())
    }
}
