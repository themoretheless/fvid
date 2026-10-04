//! Owned radial cos-fourth lens shading with persistent deterministic dithering.
use crate::{owned_expression::Expression, owned_frame::GeometryFrame};
use std::cell::RefCell;
type Result<T> = std::result::Result<T, String>;
/// Explicit source metadata for expressions and non-square sample geometry.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub n: u64,
    pub t: Option<f64>,
    pub pts: Option<f64>,
    pub rate: Option<f64>,
    pub time_base: Option<f64>,
    pub sample_aspect: f64,
}
impl Clock {
    pub fn square(n: u64, t: Option<f64>) -> Self {
        Self {
            n,
            t,
            pts: None,
            rate: None,
            time_base: None,
            sample_aspect: 1.,
        }
    }
}
#[derive(Debug)]
pub struct Vignette {
    expression: [Expression; 3],
    backward: bool,
    per_frame: bool,
    dither: bool,
    aspect: f64,
    enable: crate::owned_timeline::Timeline,
    state: RefCell<Option<State>>,
}
#[derive(Debug)]
struct State {
    geometry: (usize, usize, f64),
    parameter: [f64; 3],
    mask: Vec<f32>,
    random: u32,
    last: Option<u64>,
}
fn unquote(text: &str) -> Result<&str> {
    let text = text.trim();
    if let Some(mark) = text.chars().next().filter(|c| matches!(c, '\'' | '"')) {
        text.strip_prefix(mark)
            .and_then(|t| t.strip_suffix(mark))
            .ok_or("unclosed vignette quote".into())
    } else {
        Ok(text)
    }
}
impl Vignette {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 16384 || args.contains('\0') {
            return Err("invalid vignette options".into());
        }
        let mut filter = Self {
            expression: [
                Expression::parse("PI/5")?,
                Expression::parse("w/2")?,
                Expression::parse("h/2")?,
            ],
            backward: false,
            per_frame: false,
            dither: true,
            aspect: 1.,
            enable: Default::default(),
            state: RefCell::new(None),
        };
        let mut position = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *["angle", "x0", "y0", "mode", "eval", "dither", "aspect"]
                    .get(position)
                    .ok_or("too many vignette options")?;
                position += 1;
                (key, entry)
            };
            let value = unquote(value)?;
            match key.trim() {
                "a" | "angle" | "x0" | "y0" => {
                    let index = match key.trim() {
                        "x0" => 1,
                        "y0" => 2,
                        _ => 0,
                    };
                    let expression = Expression::parse(value)?;
                    expression.evaluate(&[
                        ("w", 1.),
                        ("h", 1.),
                        ("n", 0.),
                        ("t", 0.),
                        ("pts", 0.),
                        ("r", 25.),
                        ("tb", 0.04),
                    ])?;
                    filter.expression[index] = expression;
                }
                "mode" => {
                    filter.backward = match value {
                        "0" | "forward" => false,
                        "1" | "backward" => true,
                        _ => return Err("unknown vignette mode".into()),
                    }
                }
                "eval" => {
                    filter.per_frame = match value {
                        "0" | "init" => false,
                        "1" | "frame" => true,
                        _ => return Err("unknown vignette evaluation mode".into()),
                    }
                }
                "dither" => {
                    filter.dither = match value {
                        "0" | "false" | "off" | "no" => false,
                        "1" | "true" | "on" | "yes" => true,
                        _ => return Err("invalid vignette dither".into()),
                    }
                }
                "aspect" => {
                    filter.aspect = crate::owned_expression::constant(value)?;
                    if !filter.aspect.is_finite() || filter.aspect <= 0. {
                        return Err("vignette aspect must be finite and positive".into());
                    }
                }
                "enable" => {
                    filter.enable =
                        crate::owned_timeline::Timeline::grayworld(&format!("enable={value}"))?
                }
                _ => return Err("unknown vignette option".into()),
            }
        }
        Ok(filter)
    }
    /// Convenience for square pixels and n/t expressions. r/pts/tb expressions
    /// require apply_clock with actual metadata, rather than invented defaults.
    pub fn apply(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        self.apply_clock(frame, depth, Clock::square(n, t))
    }
    pub fn apply_clock(&self, frame: &mut GeometryFrame, depth: u8, clock: Clock) -> Result<()> {
        crate::owned_overlay::validate_frame(frame, depth)?;
        if !(8..=16).contains(&depth)
            || !clock.sample_aspect.is_finite()
            || clock.sample_aspect <= 0.
        {
            return Err("invalid vignette depth or sample aspect".into());
        }
        if !self
            .enable
            .enabled(clock.n, clock.t, frame.width, frame.height)?
        {
            return Ok(());
        }
        let variables = |initial: bool| {
            [
                ("w", frame.width as f64),
                ("h", frame.height as f64),
                ("n", if initial { f64::NAN } else { clock.n as f64 }),
                (
                    "t",
                    if initial {
                        f64::NAN
                    } else {
                        clock.t.unwrap_or(f64::NAN)
                    },
                ),
                (
                    "pts",
                    if initial {
                        f64::NAN
                    } else {
                        clock.pts.unwrap_or(f64::NAN)
                    },
                ),
                ("r", clock.rate.unwrap_or(f64::NAN)),
                ("tb", clock.time_base.unwrap_or(f64::NAN)),
            ]
        };
        let geometry = (frame.width, frame.height, clock.sample_aspect);
        let mut state = self.state.borrow_mut();
        let reset = state
            .as_ref()
            .is_none_or(|s| s.geometry != geometry || s.last.is_some_and(|last| clock.n <= last));
        let parameters = if !reset
            && !self.per_frame
            && state
                .as_ref()
                .unwrap()
                .parameter
                .iter()
                .all(|v| v.is_finite())
        {
            state.as_ref().unwrap().parameter
        } else {
            let mut value = [0.; 3];
            for (index, expression) in self.expression.iter().enumerate() {
                value[index] = expression.evaluate(&variables(!self.per_frame))?;
            }
            if value.iter().any(|v| v.is_nan()) {
                for (index, expression) in self.expression.iter().enumerate() {
                    value[index] = expression.evaluate(&variables(false))?;
                }
            }
            if value.iter().any(|v| !v.is_finite()) {
                return Err("vignette expressions require finite source metadata".into());
            }
            value[0] = (value[0] as f32).clamp(0., std::f32::consts::FRAC_PI_2) as f64;
            value
        };
        // Stateful n/t defaults fall back to per-frame evaluation, matching init
        // expressions whose initial result is undefined.
        let dynamic = self
            .expression
            .iter()
            .any(|e| e.evaluate(&variables(true)).is_ok_and(|v| v.is_nan()));
        let parameters = if dynamic && !self.per_frame {
            let mut p = [0.; 3];
            for (i, e) in self.expression.iter().enumerate() {
                p[i] = e.evaluate(&variables(false))?;
            }
            if p.iter().any(|v| !v.is_finite()) {
                return Err("vignette expressions require finite source metadata".into());
            }
            p[0] = (p[0] as f32).clamp(0., std::f32::consts::FRAC_PI_2) as f64;
            p
        } else {
            parameters
        };
        if reset || state.as_ref().unwrap().parameter != parameters {
            let count = frame
                .width
                .checked_mul(frame.height)
                .ok_or("vignette geometry overflow")?;
            let mut mask = Vec::new();
            mask.try_reserve_exact(count).map_err(|e| e.to_string())?;
            let (xs, ys) = if clock.sample_aspect > 1. {
                ((clock.sample_aspect / self.aspect) as f32, 1.)
            } else {
                (1., (self.aspect / clock.sample_aspect) as f32)
            };
            if !xs.is_finite() || !ys.is_finite() {
                return Err("vignette aspect exceeds floating point geometry".into());
            }
            let radius = (frame.width as f64 / 2.).hypot(frame.height as f64 / 2.);
            for y in 0..frame.height {
                for x in 0..frame.width {
                    let dx = ((x as f64 - parameters[1]) * xs as f64).trunc();
                    let dy = ((y as f64 - parameters[2]) * ys as f64).trunc();
                    let distance = dx.hypot(dy) / radius;
                    let factor = if distance > 1. {
                        0.
                    } else {
                        let c = (parameters[0] * distance).cos();
                        (c * c) * (c * c)
                    };
                    mask.push(if self.backward {
                        (1. / factor) as f32
                    } else {
                        factor as f32
                    });
                }
            }
            let random = if reset {
                0
            } else {
                state.as_ref().unwrap().random
            };
            *state = Some(State {
                geometry,
                parameter: parameters,
                mask,
                random,
                last: None,
            });
        }
        let state = state.as_mut().unwrap();
        let maximum = ((1u32 << depth) - 1) as f64;
        let center = ((1u32 << (depth - 1)) - 1) as f32;
        let bytes = if depth == 8 { 1 } else { 2 };
        let mut offset = 0;
        let shapes = if let Some([sx, sy]) = frame.subsampling {
            vec![
                (frame.width, frame.height, 1, 1, false, 1),
                (
                    frame.width.div_ceil(sx),
                    frame.height.div_ceil(sy),
                    sx,
                    sy,
                    true,
                    1,
                ),
                (
                    frame.width.div_ceil(sx),
                    frame.height.div_ceil(sy),
                    sx,
                    sy,
                    true,
                    1,
                ),
            ]
        } else {
            vec![(frame.width, frame.height, 1, 1, false, 3)]
        };
        for (width, height, sx, sy, chroma, channels) in shapes {
            for y in 0..height {
                for x in 0..width {
                    for _ in 0..channels {
                        let value = if bytes == 1 {
                            frame.data[offset] as u16
                        } else {
                            u16::from_le_bytes([frame.data[offset], frame.data[offset + 1]])
                        };
                        let factor = state.mask[y * sy * frame.width + x * sx];
                        let adjusted = if chroma {
                            factor * (value as f32 - center)
                        } else {
                            factor * value as f32
                        };
                        let noise = if self.dither {
                            let v = state.random as f64 / 4294967296.;
                            state.random =
                                state.random.wrapping_mul(1664525).wrapping_add(1013904223);
                            v
                        } else {
                            0.
                        };
                        let result = ((if chroma { adjusted + center } else { adjusted }) as f64
                            + noise)
                            .clamp(0., maximum) as u16;
                        if bytes == 1 {
                            frame.data[offset] = result as u8;
                        } else {
                            frame.data[offset..offset + 2].copy_from_slice(&result.to_le_bytes());
                        }
                        offset += bytes;
                    }
                }
            }
        }
        state.last = Some(clock.n);
        Ok(())
    }
}
