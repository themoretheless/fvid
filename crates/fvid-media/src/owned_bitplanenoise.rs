//! Owned bit-plane majority analysis and optional black/white visualization.
use crate::owned_frame::GeometryFrame;
use std::{cell::RefCell, collections::BTreeMap};
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct PlaneReport {
    pub plane: usize,
    pub bitplane: u8,
    pub matching: u64,
    pub samples: u64,
    pub score: f64,
}
impl PlaneReport {
    /// Exact integer counting, independently of the legacy single-precision score.
    pub fn precise_score(&self) -> f64 {
        2. * self.matching.min(self.samples - self.matching) as f64 / self.samples as f64
    }
}
#[derive(Clone, Debug)]
pub struct Report {
    pub planes: Vec<PlaneReport>,
}
impl Report {
    /// Existing metadata names and six-decimal presentation remain compatible.
    pub fn metadata(&self) -> BTreeMap<String, String> {
        self.planes
            .iter()
            .map(|r| {
                (
                    format!("lavfi.bitplanenoise.{}.{}", r.plane, r.bitplane),
                    format!("{:.6}", r.score),
                )
            })
            .collect()
    }
}
#[derive(Debug)]
pub struct BitPlaneNoise {
    bit: u8,
    visualize: bool,
    enable: crate::owned_timeline::Timeline,
    last: RefCell<Option<Report>>,
}
impl BitPlaneNoise {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid bitplanenoise options".into());
        }
        let mut bit = 1;
        let mut visualize = false;
        let mut enable = Default::default();
        let mut position = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, text) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *["bitplane", "filter"]
                    .get(position)
                    .ok_or("too many bitplanenoise options")?;
                position += 1;
                (key, entry)
            };
            let text = text.trim();
            match key.trim() {
                "bitplane" => {
                    let v = crate::owned_expression::constant(text)?;
                    if !v.is_finite() || !(1.0..=16.0).contains(&v) || v.fract() != 0. {
                        return Err("bitplane must be an integer from 1 to 16".into());
                    }
                    bit = v as u8;
                }
                "filter" => {
                    visualize = match text {
                        "0" | "false" | "off" | "no" => false,
                        "1" | "true" | "on" | "yes" => true,
                        _ => return Err("invalid bitplanenoise filter flag".into()),
                    }
                }
                "enable" => {
                    enable = crate::owned_timeline::Timeline::grayworld(&format!("enable={text}"))?
                }
                _ => return Err("unknown bitplanenoise option".into()),
            }
        }
        Ok(Self {
            bit,
            visualize,
            enable,
            last: RefCell::new(None),
        })
    }
    pub fn last_report(&self) -> Option<Report> {
        self.last.borrow().clone()
    }
    pub fn apply(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<Option<Report>> {
        crate::owned_overlay::validate_frame(frame, depth)?;
        if !self.enable.enabled(n, t, frame.width, frame.height)? {
            *self.last.borrow_mut() = None;
            return Ok(None);
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let mut output = if self.visualize {
            Some(crate::owned_frame::buffer(frame.data.len())?)
        } else {
            None
        };
        let mut planes = Vec::with_capacity(3);
        let mut offset = 0;
        if let Some([sx, sy]) = frame.subsampling {
            for (plane, (w, h)) in [
                (frame.width, frame.height),
                (frame.width.div_ceil(sx), frame.height.div_ceil(sy)),
                (frame.width.div_ceil(sx), frame.height.div_ceil(sy)),
            ]
            .into_iter()
            .enumerate()
            {
                let count = w
                    .checked_mul(h)
                    .and_then(|v| v.checked_mul(bytes))
                    .ok_or("bitplanenoise geometry overflow")?;
                planes.push(analyze(
                    &frame.data[offset..offset + count],
                    output.as_mut().map(|v| &mut v[offset..offset + count]),
                    w,
                    h,
                    depth,
                    plane,
                    self.bit,
                    1,
                    0,
                ));
                offset += count;
            }
        } else {
            for channel in 0..3 {
                planes.push(analyze(
                    &frame.data,
                    output.as_mut().map(|v| v.as_mut_slice()),
                    frame.width,
                    frame.height,
                    depth,
                    channel,
                    self.bit,
                    3,
                    channel,
                ));
            }
        }
        let report = Report { planes };
        if let Some(output) = output {
            frame.data = output;
        }
        *self.last.borrow_mut() = Some(report.clone());
        Ok(Some(report))
    }
    pub fn apply_plane(
        &self,
        data: &mut [u8],
        width: usize,
        height: usize,
        depth: u8,
        plane: usize,
        n: u64,
        t: Option<f64>,
    ) -> Result<Option<PlaneReport>> {
        if width == 0 || height == 0 || !(8..=16).contains(&depth) || plane > 3 {
            return Err("invalid bitplanenoise plane".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        if width.checked_mul(height).and_then(|v| v.checked_mul(bytes)) != Some(data.len()) {
            return Err("bitplanenoise plane storage mismatch".into());
        }
        if depth > 8
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 >= 1u32 << depth)
        {
            return Err("bitplanenoise sample exceeds precision".into());
        }
        if !self.enable.enabled(n, t, width, height)? {
            *self.last.borrow_mut() = None;
            return Ok(None);
        }
        let mut output = if self.visualize {
            Some(crate::owned_frame::buffer(data.len())?)
        } else {
            None
        };
        let report = analyze(
            data,
            output.as_deref_mut(),
            width,
            height,
            depth,
            plane,
            self.bit,
            1,
            0,
        );
        if let Some(output) = output {
            data.copy_from_slice(&output);
        }
        *self.last.borrow_mut() = Some(Report {
            planes: vec![report.clone()],
        });
        Ok(Some(report))
    }
}
fn analyze(
    source: &[u8],
    mut output: Option<&mut [u8]>,
    width: usize,
    height: usize,
    depth: u8,
    plane: usize,
    bitplane: u8,
    stride: usize,
    channel: usize,
) -> PlaneReport {
    let bytes = if depth == 8 { 1 } else { 2 };
    let mask = 1u16 << (bitplane - 1);
    let maximum = ((1u32 << depth) - 1) as u16;
    let read = |x: usize, y: usize| {
        let at = ((y * width + x) * stride + channel) * bytes;
        if bytes == 1 {
            u16::from(source[at])
        } else {
            u16::from_le_bytes([source[at], source[at + 1]])
        }
    };
    let mut matching = 0u64;
    for y in 0..height {
        let vertical = if y + 1 < height {
            y + 1
        } else {
            y.saturating_sub(1)
        };
        for x in 0..width {
            let reference = read(x, y) & mask;
            let neighbours = if x == 0 {
                let right = (x + 1).min(width - 1);
                [(right, y), (right, vertical), (x, vertical)]
            } else if x + 1 == width {
                [(x - 1, y), (x - 1, vertical), (x, vertical)]
            } else {
                [(x - 1, y), (x + 1, y), (x, vertical)]
            };
            let majority = neighbours
                .into_iter()
                .filter(|&(nx, ny)| read(nx, ny) & mask == reference)
                .count()
                >= 2;
            matching += u64::from(majority);
            if let Some(data) = output.as_deref_mut() {
                let at = ((y * width + x) * stride + channel) * bytes;
                let value = if majority { maximum } else { 0 };
                if bytes == 1 {
                    data[at] = value as u8;
                } else {
                    data[at..at + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
    let samples = (width as u64) * (height as u64);
    // Preserve the established float-score presentation while exposing the
    // actual integer count. A repeated f32 +1 saturates exactly at 2^24.
    let fraction = matching.min(1 << 24) as f32 / samples as f32;
    let score = 1. - 2. * f64::from(fraction - 0.5).abs();
    PlaneReport {
        plane,
        bitplane,
        matching,
        samples,
        score,
    }
}
