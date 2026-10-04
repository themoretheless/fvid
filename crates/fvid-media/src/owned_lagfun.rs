//! Owned persistent bright-pixel decay; history retains unrounded f32 samples.
use crate::owned_frame::GeometryFrame;
use std::cell::RefCell;
type Result<T> = std::result::Result<T, String>;
#[derive(Debug)]
pub struct LagFun {
    decay: f32,
    planes: u8,
    enable: crate::owned_timeline::Timeline,
    history: RefCell<Option<History>>,
}
#[derive(Debug)]
struct History {
    width: usize,
    height: usize,
    sub: Option<[usize; 2]>,
    depth: u8,
    values: Vec<f32>,
    last: u64,
}
impl LagFun {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid lagfun options".into());
        }
        let mut decay = 0.95f32;
        let mut planes = 15u8;
        let mut position = 0;
        let mut enable = crate::owned_timeline::Timeline::default();
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, text) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *["decay", "planes"]
                    .get(position)
                    .ok_or("too many lagfun options")?;
                position += 1;
                (key, entry)
            };
            if key.trim() == "enable" {
                enable =
                    crate::owned_timeline::Timeline::grayworld(&format!("enable={}", text.trim()))?;
                continue;
            }
            let value = crate::owned_expression::constant(text.trim())?;
            match key.trim() {
                "decay" if value.is_finite() && (0.0..=1.0).contains(&value) => {
                    decay = value as f32
                }
                "planes"
                    if value.is_finite()
                        && value.fract() == 0.0
                        && (0.0..=15.0).contains(&value) =>
                {
                    planes = value as u8
                }
                _ => return Err("invalid lagfun option or parameter".into()),
            }
        }
        Ok(Self {
            decay,
            planes,
            enable,
            history: RefCell::new(None),
        })
    }
    pub fn apply(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        if !(8..=16).contains(&depth) || frame.width == 0 || frame.height == 0 {
            return Err("invalid lagfun frame".into());
        }
        let luma = frame
            .width
            .checked_mul(frame.height)
            .ok_or("lagfun geometry overflow")?;
        let chroma = if let Some([sx, sy]) = frame.subsampling {
            if sx == 0 || sy == 0 {
                return Err("invalid lagfun chroma".into());
            }
            frame
                .width
                .div_ceil(sx)
                .checked_mul(frame.height.div_ceil(sy))
                .ok_or("lagfun chroma overflow")?
        } else {
            luma
        };
        let samples = chroma
            .checked_mul(2)
            .and_then(|v| v.checked_add(luma))
            .ok_or("lagfun storage overflow")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        if samples.checked_mul(bytes) != Some(frame.data.len()) {
            return Err("lagfun storage mismatch".into());
        }
        let max = (1u32 << depth) - 1;
        if depth > 8
            && frame
                .data
                .chunks_exact(2)
                .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) > max)
        {
            return Err("lagfun sample exceeds depth".into());
        }
        let enabled = self.enable.enabled(n, t, frame.width, frame.height)?;
        let mut state = self
            .history
            .try_borrow_mut()
            .map_err(|_| "lagfun history is already borrowed")?;
        if let Some(history) = state.as_ref().filter(|_| n != 0) {
            if (history.width, history.height, history.sub, history.depth)
                != (frame.width, frame.height, frame.subsampling, depth)
            {
                return Err("lagfun geometry changed within a stream".into());
            }
            if n <= history.last {
                return Err("lagfun frame index must advance or rewind to zero".into());
            }
        }
        if state.is_none() || n == 0 {
            let mut values = Vec::new();
            values
                .try_reserve_exact(samples)
                .map_err(|_| "lagfun history allocation failed")?;
            values.resize(samples, 0.);
            *state = Some(History {
                width: frame.width,
                height: frame.height,
                sub: frame.subsampling,
                depth,
                values,
                last: n,
            });
        }
        let history = state.as_mut().unwrap();
        for i in 0..samples {
            // Packed RGB uses the same plane masks as planar GBR: G, B, R.
            let plane = if frame.subsampling.is_none() {
                [2, 0, 1][i % 3]
            } else if i < luma {
                0
            } else if i < luma + chroma {
                1
            } else {
                2
            };
            if self.planes & (1 << plane) == 0 {
                continue;
            }
            let source = if bytes == 1 {
                f32::from(frame.data[i])
            } else {
                f32::from(u16::from_le_bytes([
                    frame.data[2 * i],
                    frame.data[2 * i + 1],
                ]))
            };
            let value = source.max(history.values[i] * self.decay);
            history.values[i] = value;
            if enabled {
                let output = value.round_ties_even() as u16;
                if bytes == 1 {
                    frame.data[i] = output as u8;
                } else {
                    frame.data[2 * i..2 * i + 2].copy_from_slice(&output.to_le_bytes());
                }
            }
        }
        history.last = n;
        Ok(())
    }
}
