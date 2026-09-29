//! Owned pointwise operations on packed decoded sample planes.
use crate::{Result, invalid, native_geometry::GeometryFrame};

/// Invert every colour sample. The legacy `1` option includes alpha; native
/// RGB24 and YUV frames have no alpha, so both modes produce the same pixels.
#[derive(Clone, Copy, Debug)]
pub struct Negate;

impl Negate {
    pub fn parse(args: &str) -> Result<Self> {
        match args {
            "" | "0" | "1" => Ok(Self),
            _ => Err(invalid("negate args must be empty, 0, or 1 (alpha)")),
        }
    }

    /// Invert against the sample maximum, including for limited-range YUV.
    /// High-depth planes contain little-endian u16 samples. Validate the whole
    /// buffer before mutation, so malformed input remains unchanged on error.
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if !(8..=16).contains(&depth) || (frame.subsampling.is_none() && depth != 8) {
            return Err(invalid("unsupported negate sample depth"));
        }
        if depth == 8 {
            for sample in &mut frame.data {
                *sample = 255 - *sample;
            }
        } else {
            let max = ((1u32 << depth) - 1) as u16;
            if frame.data.len() % 2 != 0
                || frame
                    .data
                    .chunks_exact(2)
                    .any(|s| u16::from_le_bytes([s[0], s[1]]) > max)
            {
                return Err(invalid("invalid negate sample storage"));
            }
            for sample in frame.data.chunks_exact_mut(2) {
                let value = max - u16::from_le_bytes([sample[0], sample[1]]);
                sample.copy_from_slice(&value.to_le_bytes());
            }
        }
        Ok(())
    }
}

/// Spatial edge operators compatible with the media filter option names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GradientKind {
    Sobel,
    Prewitt,
    Roberts,
    Kirsch,
    Scharr,
}
impl GradientKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sobel => "sobel",
            Self::Prewitt => "prewitt",
            Self::Roberts => "roberts",
            Self::Kirsch => "kirsch",
            Self::Scharr => "scharr",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Gradient {
    kind: GradientKind,
    planes: u8,
    scale: f32,
    delta: f32,
}
impl Gradient {
    /// Literal `planes:scale:delta` or named options. Numeric expressions are
    /// not evaluated here; the full media adapter retains its expression path.
    pub fn parse(kind: GradientKind, args: &str) -> Result<Self> {
        let mut result = Self {
            kind,
            planes: 15,
            scale: 1.0,
            delta: 0.0,
        };
        if args.is_empty() {
            return Ok(result);
        }
        let mut positional = 0usize;
        for entry in args.split(':') {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *["planes", "scale", "delta"]
                    .get(positional)
                    .ok_or_else(|| invalid("too many gradient options"))?;
                positional += 1;
                (key, entry)
            };
            let value = value.trim();
            match key.trim() {
                "planes" => {
                    let parsed = if let Some(hex) = value.strip_prefix("0x") {
                        u8::from_str_radix(hex, 16)
                    } else {
                        value.parse()
                    }
                    .map_err(|_| invalid("gradient planes requires an integer 0..15"))?;
                    if parsed > 15 {
                        return Err(invalid("gradient planes must be 0..15"));
                    }
                    result.planes = parsed;
                }
                "scale" | "delta" => {
                    let number = value
                        .parse::<f32>()
                        .map_err(|_| invalid("gradient options require numeric literals"))?;
                    let lower = if key.trim() == "scale" { 0.0 } else { -65535.0 };
                    if !number.is_finite() || !(lower..=65535.0).contains(&number) {
                        return Err(invalid("gradient option outside supported range"));
                    }
                    if key.trim() == "scale" {
                        result.scale = number;
                    } else {
                        result.delta = number;
                    }
                }
                _ => return Err(invalid("unknown gradient option")),
            }
        }
        Ok(result)
    }
    pub fn kind(self) -> GradientKind {
        self.kind
    }
    /// Retain format/depth and copy unselected planes exactly. Source validation
    /// precedes allocation/mutation, so an error leaves the frame unchanged.
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if !(8..=16).contains(&depth)
            || (frame.subsampling.is_none() && depth != 8)
            || frame.width == 0
            || frame.height == 0
        {
            return Err(invalid("unsupported gradient frame geometry or depth"));
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let rgb = frame.subsampling.is_none();
        let [sx, sy] = frame.subsampling.unwrap_or([1, 1]);
        if sx == 0 || sy == 0 {
            return Err(invalid("zero gradient subsampling"));
        }
        let count = |w: usize, h: usize| {
            w.checked_mul(h)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("gradient plane size overflow"))
        };
        let luma = count(frame.width, frame.height)?;
        let cw = frame.width.div_ceil(sx);
        let ch = frame.height.div_ceil(sy);
        let chroma = count(cw, ch)?;
        let expected = if rgb {
            luma.checked_mul(3)
        } else {
            chroma.checked_mul(2).and_then(|n| luma.checked_add(n))
        }
        .ok_or_else(|| invalid("gradient storage overflow"))?;
        let max = (1u32 << depth) - 1;
        if frame.data.len() != expected
            || (bytes == 2
                && frame
                    .data
                    .chunks_exact(2)
                    .any(|v| u32::from(u16::from_le_bytes([v[0], v[1]])) > max))
        {
            return Err(invalid("invalid gradient sample storage"));
        }
        if self.planes & 7 == 0 {
            return Ok(());
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(expected)
            .map_err(|_| invalid("cannot allocate gradient output"))?;
        output.extend_from_slice(&frame.data);
        for plane in 0..3 {
            // Planar RGB filters number components G, B, R. Interleaved RGB
            // still uses that mask while preserving the caller's RGB24 layout.
            let mask = if rgb { [2, 0, 1][plane] } else { plane };
            if self.planes & (1 << mask) == 0 {
                continue;
            }
            let (w, h, offset, stride) = if rgb {
                (frame.width, frame.height, plane, 3)
            } else if plane == 0 {
                (frame.width, frame.height, 0, bytes)
            } else {
                (cw, ch, luma + (plane - 1) * chroma, bytes)
            };
            let sample = |x: usize, y: usize| {
                let at = offset + (y * w + x) * stride;
                if bytes == 1 {
                    i32::from(frame.data[at])
                } else {
                    i32::from(u16::from_le_bytes([frame.data[at], frame.data[at + 1]]))
                }
            };
            for y in 0..h {
                for x in 0..w {
                    let axes = |at: usize, len: usize| {
                        [
                            if at == 0 { 1.min(len - 1) } else { at - 1 },
                            at,
                            if at + 1 == len {
                                len.saturating_sub(2)
                            } else {
                                at + 1
                            },
                        ]
                    };
                    let xs = axes(x, w);
                    let ys = axes(y, h);
                    let mut grid = [0i32; 9];
                    for row in 0..3 {
                        for col in 0..3 {
                            grid[row * 3 + col] = sample(xs[col], ys[row]);
                        }
                    }
                    let magnitude = match self.kind {
                        GradientKind::Kirsch => {
                            let ring = [
                                grid[0], grid[1], grid[2], grid[3], grid[5], grid[6], grid[7],
                                grid[8],
                            ];
                            let total = ring.iter().sum::<i32>();
                            (0..8)
                                .map(|i| {
                                    8 * (ring[i] + ring[(i + 1) % 8] + ring[(i + 2) % 8])
                                        - 3 * total
                                })
                                .max()
                                .unwrap()
                                .unsigned_abs() as f32
                        }
                        GradientKind::Roberts => {
                            let a = (grid[0] - grid[1]) as f32;
                            let b = (grid[4] - grid[3]) as f32;
                            (a * a + b * b).sqrt()
                        }
                        kind => {
                            let (outer, middle, divisor) = match kind {
                                GradientKind::Sobel => (1, 2, 1.0),
                                GradientKind::Prewitt => (1, 1, 1.0),
                                _ => (47, 162, 256.0),
                            };
                            let a = (outer * (grid[6] + grid[8] - grid[0] - grid[2])
                                + middle * (grid[7] - grid[1]))
                                as f32
                                / divisor;
                            let b = (outer * (grid[2] + grid[8] - grid[0] - grid[6])
                                + middle * (grid[5] - grid[3]))
                                as f32
                                / divisor;
                            (a * a + b * b).sqrt()
                        }
                    };
                    let value = (magnitude * self.scale + self.delta).clamp(0.0, max as f32) as u16;
                    let at = offset + (y * w + x) * stride;
                    if bytes == 1 {
                        output[at] = value as u8;
                    } else {
                        output[at..at + 2].copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        frame.data = output;
        Ok(())
    }
}

/// Native filter order matches the public media request, independent of CLI
/// flag order: inversion, Sobel, Prewitt, Roberts, Kirsch, Scharr, dilation, erosion, chroma shift.
#[derive(Default)]
pub struct PixelFilters {
    pub negate: Option<Negate>,
    pub chromashift: Option<crate::native_chromashift::ChromaShift>,
    pub gradients: Vec<Gradient>,
    pub morphology: Vec<crate::native_morphology::Morphology>,
}
impl PixelFilters {
    pub fn from_request(request: &crate::media_info::DecodeTransform) -> Result<Self> {
        let mut result = Self {
            negate: request.negate.as_deref().map(Negate::parse).transpose()?,
            chromashift: request
                .chromashift
                .as_deref()
                .map(crate::native_chromashift::ChromaShift::parse)
                .transpose()?,
            gradients: Vec::new(),
            morphology: Vec::new(),
        };
        for (kind, args) in [
            (GradientKind::Sobel, &request.sobel),
            (GradientKind::Prewitt, &request.prewitt),
            (GradientKind::Roberts, &request.roberts),
            (GradientKind::Kirsch, &request.kirsch),
            (GradientKind::Scharr, &request.scharr),
        ] {
            if let Some(args) = args {
                result.gradients.push(Gradient::parse(kind, args)?);
            }
        }
        use crate::native_morphology::{Morphology, MorphologyKind};
        for (kind, args) in [
            (MorphologyKind::Dilation, &request.dilation),
            (MorphologyKind::Erosion, &request.erosion),
        ] {
            if let Some(args) = args {
                result.morphology.push(Morphology::parse(kind, args)?);
            }
        }
        Ok(result)
    }
    pub fn is_empty(&self) -> bool {
        self.chromashift.is_none()
            && self.negate.is_none()
            && self.gradients.is_empty()
            && self.morphology.is_empty()
    }
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if let Some(negate) = self.negate {
            negate.apply(frame, depth)?;
        }
        for filter in &self.gradients {
            filter.apply(frame, depth)?;
        }
        for filter in &self.morphology {
            filter.apply(frame, depth)?;
        }
        if let Some(filter) = self.chromashift {
            filter.apply(frame, depth)?;
        }
        Ok(())
    }
}
