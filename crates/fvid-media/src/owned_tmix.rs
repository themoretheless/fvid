//! Owned trailing-frame mixing with replicated first-frame warmup.
use crate::owned_frame::GeometryFrame;
use std::{cell::RefCell, collections::VecDeque, sync::Arc};
type Result<T> = std::result::Result<T, String>;
#[derive(Debug)]
pub struct TemporalMix {
    weights: Vec<f32>,
    factor: f32,
    fast: bool,
    planes: u8,
    history: RefCell<Option<History>>,
}
#[derive(Debug)]
struct History {
    width: usize,
    height: usize,
    sub: Option<[usize; 2]>,
    depth: u8,
    last: u64,
    frames: VecDeque<Arc<Vec<u8>>>,
}
impl TemporalMix {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid tmix options".into());
        }
        let mut frames = 3usize;
        let mut text = "1 1 1";
        let mut scale = 0f32;
        let mut planes = 15u8;
        let mut position = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *["frames", "weights", "scale", "planes"]
                    .get(position)
                    .ok_or("too many tmix options")?;
                position += 1;
                (key, entry)
            };
            if key.trim() == "weights" {
                text = value.trim();
                if text.starts_with('\'') || text.starts_with('"') {
                    let quote = text.chars().next().unwrap();
                    text = text
                        .strip_prefix(quote)
                        .and_then(|v| v.strip_suffix(quote))
                        .ok_or("unclosed tmix weights quote")?;
                }
                continue;
            }
            let value = crate::owned_expression::constant(value.trim())?;
            match key.trim() {
                "frames"
                    if value.is_finite()
                        && value.fract() == 0.
                        && (1.0..=1024.0).contains(&value) =>
                {
                    frames = value as usize
                }
                "scale" if value.is_finite() && (0.0..=32767.0).contains(&value) => {
                    scale = value as f32
                }
                "planes"
                    if value.is_finite()
                        && value.fract() == 0.
                        && (0.0..=15.0).contains(&value) =>
                {
                    planes = value as u8
                }
                _ => return Err("invalid tmix option or parameter".into()),
            }
        }
        let mut weights = Vec::new();
        weights
            .try_reserve_exact(frames)
            .map_err(|_| "tmix weights allocation failed")?;
        for token in text
            .split(|c: char| c == '|' || c.is_ascii_whitespace())
            .filter(|v| !v.is_empty())
            .take(frames)
        {
            let value = token.parse::<f32>().map_err(|_| "invalid tmix weight")?;
            if !value.is_finite() {
                return Err("tmix weights must be finite".into());
            }
            weights.push(value);
        }
        let last = *weights.last().ok_or("tmix requires at least one weight")?;
        weights.resize(frames, last);
        let sum = weights.iter().copied().fold(0f32, |sum, w| sum + w);
        let fast = weights.iter().all(|&w| w == weights[0]) && (scale == 0. || scale == 1. / sum);
        let factor = if scale == 0. { 1. / sum } else { scale };
        if !fast && (!factor.is_finite() || !sum.is_finite()) {
            return Err("invalid tmix normalization".into());
        }
        Ok(Self {
            weights,
            factor,
            fast,
            planes,
            history: RefCell::new(None),
        })
    }
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8, n: u64) -> Result<()> {
        if !(8..=16).contains(&depth) || frame.width == 0 || frame.height == 0 {
            return Err("invalid tmix frame".into());
        }
        let y = frame
            .width
            .checked_mul(frame.height)
            .ok_or("tmix geometry overflow")?;
        let c = if let Some([sx, sy]) = frame.subsampling {
            if sx == 0 || sy == 0 {
                return Err("invalid tmix chroma".into());
            }
            frame
                .width
                .div_ceil(sx)
                .checked_mul(frame.height.div_ceil(sy))
                .ok_or("tmix chroma overflow")?
        } else {
            y
        };
        let samples = c
            .checked_mul(2)
            .and_then(|c| c.checked_add(y))
            .ok_or("tmix storage overflow")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let maximum = (1u32 << depth) - 1;
        if samples.checked_mul(bytes) != Some(frame.data.len()) {
            return Err("tmix storage mismatch".into());
        }
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) > maximum)
        {
            return Err("tmix sample exceeds depth".into());
        }
        let envelope = self
            .weights
            .iter()
            .map(|&w| f64::from(w).abs())
            .sum::<f64>()
            * f64::from(maximum);
        if !self.fast
            && (envelope > f32::MAX as f64
                || envelope * f64::from(self.factor).abs() > f32::MAX as f64)
        {
            return Err("tmix weighted accumulation overflow".into());
        }
        if self.weights.len() == 1 {
            return Ok(());
        }
        let mut state = self
            .history
            .try_borrow_mut()
            .map_err(|_| "tmix history is already borrowed")?;
        if let Some(h) = state.as_ref().filter(|_| n != 0) {
            if (h.width, h.height, h.sub, h.depth)
                != (frame.width, frame.height, frame.subsampling, depth)
            {
                return Err("tmix geometry changed within a stream".into());
            }
            if n <= h.last {
                return Err("tmix frame index must advance or rewind to zero".into());
            }
        }
        let mut raw = Vec::new();
        raw.try_reserve_exact(frame.data.len())
            .map_err(|_| "tmix frame allocation failed")?;
        raw.extend_from_slice(&frame.data);
        let raw = Arc::new(raw);
        if state.is_none() || n == 0 {
            let mut frames = VecDeque::new();
            frames
                .try_reserve_exact(self.weights.len())
                .map_err(|_| "tmix history allocation failed")?;
            frames.extend(std::iter::repeat_n(raw.clone(), self.weights.len()));
            *state = Some(History {
                width: frame.width,
                height: frame.height,
                sub: frame.subsampling,
                depth,
                last: n,
                frames,
            });
        } else {
            let h = state.as_mut().unwrap();
            h.frames.pop_front();
            h.frames.push_back(raw);
        }
        let history = state.as_mut().unwrap();
        let read = |data: &[u8], i: usize| -> u32 {
            if bytes == 1 {
                u32::from(data[i])
            } else {
                u32::from(u16::from_le_bytes([data[2 * i], data[2 * i + 1]]))
            }
        };
        for i in 0..samples {
            let plane = if frame.subsampling.is_none() || i < y {
                0
            } else if i < y + c {
                1
            } else {
                2
            };
            let value = if self.planes & (1 << plane) == 0 {
                read(&history.frames[0], i)
            } else if self.fast {
                let sum = history
                    .frames
                    .iter()
                    .map(|f| u64::from(read(f, i)))
                    .sum::<u64>();
                // A wide sum preserves bright samples even at the largest window.
                ((sum + self.weights.len() as u64 / 2) / self.weights.len() as u64) as u32
            } else {
                let sum = history
                    .frames
                    .iter()
                    .zip(&self.weights)
                    .fold(0f32, |sum, (f, w)| sum + read(f, i) as f32 * w);
                (sum * self.factor)
                    .round_ties_even()
                    .clamp(0., maximum as f32) as u32
            };
            if bytes == 1 {
                frame.data[i] = value as u8;
            } else {
                frame.data[2 * i..2 * i + 2].copy_from_slice(&(value as u16).to_le_bytes());
            }
        }
        history.last = n;
        Ok(())
    }
}
