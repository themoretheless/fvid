//! Owned inverse projective mapping with quantized linear or cubic sampling.
use crate::{owned_expression::Expression, owned_frame::GeometryFrame};
use std::cell::RefCell;
type Result<T> = std::result::Result<T, String>;
#[derive(Debug)]
struct Map {
    width: usize,
    height: usize,
    frame: u64,
    points: Vec<[i64; 2]>,
}
#[derive(Debug)]
pub struct Perspective {
    corners: Vec<Expression>,
    cubic: bool,
    destination: bool,
    per_frame: bool,
    enable: crate::owned_timeline::Timeline,
    map: RefCell<Option<Map>>,
    coefficients: [[i64; 4]; 256],
}
#[derive(Clone, Copy)]
struct Plane {
    width: usize,
    height: usize,
    offset: usize,
    stride: usize,
    sub: [usize; 2],
}
impl Perspective {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid perspective options".into());
        }
        let mut corners: Vec<_> = ["0", "0", "W", "0", "0", "H", "W", "H"]
            .into_iter()
            .map(Expression::parse)
            .collect::<Result<_>>()?;
        let keys = [
            "x0",
            "y0",
            "x1",
            "y1",
            "x2",
            "y2",
            "x3",
            "y3",
            "interpolation",
            "sense",
            "eval",
        ];
        let (mut cubic, mut destination, mut per_frame) = (false, false, false);
        let mut enable = Default::default();
        let mut position = 0;
        for part in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(pair) = part.split_once('=') {
                pair
            } else {
                let key = *keys.get(position).ok_or("too many perspective options")?;
                position += 1;
                (key, part)
            };
            let key = key.trim();
            let value = value.trim().trim_matches('\'');
            if let Some(index) = keys[..8].iter().position(|v| *v == key) {
                let expr = Expression::parse(value)?;
                expr.evaluate(&[("W", 17.), ("H", 13.), ("in", 1.), ("on", 1.)])?;
                corners[index] = expr;
            } else {
                let flag = |first: &str, second: &str| -> Result<bool> {
                    match value {
                        v if v == first || v == "0" => Ok(false),
                        v if v == second || v == "1" => Ok(true),
                        _ => match crate::owned_expression::constant(value)? {
                            0. => Ok(false),
                            1. => Ok(true),
                            _ => Err(format!("invalid perspective {key}")),
                        },
                    }
                };
                match key {
                    "interpolation" => cubic = flag("linear", "cubic")?,
                    "sense" => destination = flag("source", "destination")?,
                    "eval" => per_frame = flag("init", "frame")?,
                    "enable" => {
                        enable =
                            crate::owned_timeline::Timeline::grayworld(&format!("enable={value}"))?
                    }
                    _ => return Err("unknown perspective option".into()),
                }
            }
        }
        let coefficients = std::array::from_fn(|i| {
            let fraction = i as f64 / 256.;
            let weights: [f64; 4] = std::array::from_fn(|j| cubic_weight(j as f64 - 1. - fraction));
            let sum = weights.iter().sum::<f64>();
            weights.map(|v| (v / sum * 2048.).round_ties_even() as i64)
        });
        Ok(Self {
            corners,
            cubic,
            destination,
            per_frame,
            enable,
            map: RefCell::new(None),
            coefficients,
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
        if let Some(sub) = frame.subsampling {
            self.apply_planar(
                &mut frame.data,
                frame.width,
                frame.height,
                sub,
                depth,
                false,
                n,
                t,
            )
        } else {
            self.apply_rgb(&mut frame.data, frame.width, frame.height, depth, 3, n, t)
        }
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
            return Err("invalid perspective geometry".into());
        }
        let y = w.checked_mul(h).ok_or("perspective dimensions overflow")?;
        let cw = w.div_ceil(sub[0]);
        let ch = h.div_ceil(sub[1]);
        let c = cw
            .checked_mul(ch)
            .ok_or("perspective dimensions overflow")?;
        let second = y.checked_add(c).ok_or("perspective storage overflow")?;
        let mut planes = vec![
            Plane {
                width: w,
                height: h,
                offset: 0,
                stride: 1,
                sub: [1, 1],
            },
            Plane {
                width: cw,
                height: ch,
                offset: y,
                stride: 1,
                sub,
            },
            Plane {
                width: cw,
                height: ch,
                offset: second,
                stride: 1,
                sub,
            },
        ];
        if alpha {
            planes.push(Plane {
                width: w,
                height: h,
                offset: second
                    .checked_add(c)
                    .ok_or("perspective storage overflow")?,
                stride: 1,
                sub: [1, 1],
            });
        }
        self.process(data, w, h, depth, &planes, n, t)
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
            return Err("invalid perspective RGB channels".into());
        }
        let planes: Vec<_> = (0..channels)
            .map(|offset| Plane {
                width: w,
                height: h,
                offset,
                stride: channels,
                sub: [1, 1],
            })
            .collect();
        self.process(data, w, h, depth, &planes, n, t)
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
            w,
            h,
            depth,
            &[Plane {
                width: w,
                height: h,
                offset: 0,
                stride: 1,
                sub: [1, 1],
            }],
            n,
            t,
        )
    }
    fn prepare(&self, w: usize, h: usize, n: u64) -> Result<()> {
        let mut cache = self.map.borrow_mut();
        if cache
            .as_ref()
            .is_some_and(|m| m.width == w && m.height == h && (!self.per_frame || m.frame == n))
        {
            return Ok(());
        }
        let counter = if self.per_frame { n as f64 + 1. } else { 1. };
        let variables = [
            ("W", w as f64),
            ("H", h as f64),
            ("in", counter),
            ("on", counter),
        ];
        let mut corner = [[0.; 2]; 4];
        for (i, p) in corner.iter_mut().enumerate() {
            for (j, v) in p.iter_mut().enumerate() {
                *v = self.corners[i * 2 + j].evaluate(&variables)?;
                if !v.is_finite() {
                    return Err("nonfinite perspective corner".into());
                }
            }
        }
        let rectangle = [
            [0., 0.],
            [w as f64, 0.],
            [0., h as f64],
            [w as f64, h as f64],
        ];
        let (from, to) = if self.destination {
            (corner, rectangle)
        } else {
            (rectangle, corner)
        };
        let matrix = homography(from, to)?;
        let size = w.checked_mul(h).ok_or("perspective map overflow")?;
        let mut points = Vec::new();
        points.try_reserve_exact(size).map_err(|e| e.to_string())?;
        for y in 0..h {
            for x in 0..w {
                let x = x as f64;
                let y = y as f64;
                let denominator = matrix[6] * x + matrix[7] * y + 1.;
                if denominator == 0. || !denominator.is_finite() {
                    return Err("perspective projection crosses a pole".into());
                }
                let coordinate = |a: usize| -> Result<i64> {
                    let v =
                        256. * (matrix[a] * x + matrix[a + 1] * y + matrix[a + 2]) / denominator;
                    // Bound conversion to the reference coordinate representation, not media allocation.
                    if !v.is_finite() || v < i32::MIN as f64 || v > i32::MAX as f64 {
                        return Err("perspective coordinate exceeds signed mapping range".into());
                    }
                    Ok(v.round_ties_even() as i64)
                };
                points.push([coordinate(0)?, coordinate(3)?]);
            }
        }
        *cache = Some(Map {
            width: w,
            height: h,
            frame: n,
            points,
        });
        Ok(())
    }
    fn process(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        planes: &[Plane],
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        if w == 0
            || h == 0
            || w > i32::MAX as usize
            || h > i32::MAX as usize
            || !(8..=16).contains(&depth)
        {
            return Err("invalid perspective dimensions/precision".into());
        }
        let bytes = usize::from(depth > 8) + 1;
        let maximum = (1u32 << depth) - 1;
        let mut extent = 0;
        for p in planes {
            let end = p
                .width
                .checked_mul(p.height)
                .and_then(|v| v.checked_sub(1))
                .and_then(|v| v.checked_mul(p.stride))
                .and_then(|v| v.checked_add(p.offset))
                .and_then(|v| v.checked_add(1))
                .ok_or("perspective storage overflow")?;
            extent = extent.max(end);
        }
        if extent.checked_mul(bytes) != Some(data.len())
            || depth > 8
                && data
                    .chunks_exact(2)
                    .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 > maximum)
        {
            return Err("invalid perspective sample storage".into());
        }
        if !self.enable.enabled(n, t, w, h)? {
            return Ok(());
        }
        self.prepare(w, h, n)?;
        let cache = self.map.borrow();
        let map = cache.as_ref().unwrap();
        let mut output = crate::owned_frame::buffer(data.len())?;
        for p in planes {
            for y in 0..p.height {
                for x in 0..p.width {
                    let [u, v] = map.points[y * p.sub[1] * w + x * p.sub[0]];
                    let u = u / p.sub[0] as i64 - (i64::from(u < 0 && u % p.sub[0] as i64 != 0));
                    let v = v / p.sub[1] as i64 - (i64::from(v < 0 && v % p.sub[1] as i64 != 0));
                    let (ix, iy) = (u >> 8, v >> 8);
                    let (fx, fy) = ((u & 255) as usize, (v & 255) as usize);
                    let read = |x: i64, y: i64| -> i64 {
                        let x = x.clamp(0, p.width as i64 - 1) as usize;
                        let y = y.clamp(0, p.height as i64 - 1) as usize;
                        let at = (p.offset + (y * p.width + x) * p.stride) * bytes;
                        if bytes == 1 {
                            data[at] as i64
                        } else {
                            u16::from_le_bytes([data[at], data[at + 1]]) as i64
                        }
                    };
                    let value = if self.cubic {
                        let mut sum = 0i64;
                        for dy in 0..4 {
                            for dx in 0..4 {
                                sum += self.coefficients[fx][dx]
                                    * self.coefficients[fy][dy]
                                    * read(ix + dx as i64 - 1, iy + dy as i64 - 1);
                            }
                        }
                        (sum + (1 << 21)) >> 22
                    } else {
                        let fx = fx as i64;
                        let fy = fy as i64;
                        let top = (256 - fx) * read(ix, iy) + fx * read(ix + 1, iy);
                        let bottom = (256 - fx) * read(ix, iy + 1) + fx * read(ix + 1, iy + 1);
                        ((256 - fy) * top + fy * bottom + (1 << 15)) >> 16
                    }
                    .clamp(0, maximum as i64);
                    let at = (p.offset + (y * p.width + x) * p.stride) * bytes;
                    if bytes == 1 {
                        output[at] = value as u8
                    } else {
                        output[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
                    }
                }
            }
        }
        data.copy_from_slice(&output);
        Ok(())
    }
}
fn cubic_weight(distance: f64) -> f64 {
    let d = distance.abs();
    if d < 1. {
        1. - 2.4 * d * d + 1.4 * d * d * d
    } else if d < 2. {
        2.4 - 4.8 * d + 3. * d * d - 0.6 * d * d * d
    } else {
        0.
    }
}
/// Solve the eight projective constraints with partial-pivot elimination.
/// Each point contributes u=(a*x+b*y+c)/(g*x+h*y+1), and the analogous v equation.
fn homography(from: [[f64; 2]; 4], to: [[f64; 2]; 4]) -> Result<[f64; 8]> {
    let mut equations = [[0.; 9]; 8];
    for i in 0..4 {
        let [x, y] = from[i];
        let [u, v] = to[i];
        equations[2 * i] = [x, y, 1., 0., 0., 0., -u * x, -u * y, u];
        equations[2 * i + 1] = [0., 0., 0., x, y, 1., -v * x, -v * y, v];
    }
    for column in 0..8 {
        let pivot = (column..8)
            .max_by(|&a, &b| {
                equations[a][column]
                    .abs()
                    .total_cmp(&equations[b][column].abs())
            })
            .unwrap();
        if equations[pivot][column] == 0. || !equations[pivot][column].is_finite() {
            return Err("singular perspective corners".into());
        }
        equations.swap(pivot, column);
        let scale = equations[column][column];
        for v in &mut equations[column][column..] {
            *v /= scale;
        }
        for row in 0..8 {
            if row != column {
                let factor = equations[row][column];
                for index in column..9 {
                    equations[row][index] -= factor * equations[column][index];
                }
            }
        }
    }
    let result = std::array::from_fn(|i| equations[i][8]);
    if result.iter().any(|v| !v.is_finite()) {
        return Err("nonfinite perspective matrix".into());
    }
    Ok(result)
}
