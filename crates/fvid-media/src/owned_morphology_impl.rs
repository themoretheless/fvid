
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MorphologyKind {
    Dilation,
    Erosion,
}
impl MorphologyKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dilation => "dilation",
            Self::Erosion => "erosion",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Morphology {
    kind: MorphologyKind,
    coordinates: u8,
    thresholds: [u16; 4],
}
impl Morphology {
    pub fn kind(self) -> MorphologyKind {
        self.kind
    }
    /// Constant expressions in coordinates:threshold0:threshold1:threshold2:threshold3.
    pub fn parse(kind: MorphologyKind, args: &str) -> Result<Self> {
        let mut result = Self {
            kind,
            coordinates: 255,
            thresholds: [u16::MAX; 4],
        };
        if args.is_empty() {
            return Ok(result);
        }
        let names = [
            "coordinates",
            "threshold0",
            "threshold1",
            "threshold2",
            "threshold3",
        ];
        let mut positional = 0;
        for entry in args.split(':') {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *names
                    .get(positional)
                    .ok_or_else(|| invalid("too many morphology options"))?;
                positional += 1;
                (key, entry)
            };
            let value = value.trim();
            let index = names
                .iter()
                .position(|name| *name == key.trim())
                .ok_or_else(|| invalid("unknown morphology option"))?;
            let maximum = if index == 0 { 255.0 } else { 65535.0 };
            let number = if let Some(hex) = value.strip_prefix("0x") {
                u16::from_str_radix(hex, 16).map(f64::from)
                    .map_err(|_| invalid("invalid morphology hexadecimal literal"))?
            } else {
                morphology_expression::Expression::parse(value)
                    .and_then(|expression| expression.evaluate(&[("default", maximum), ("min", 0.0), ("max", maximum)]))
                    .map_err(|_| invalid("morphology requires a constant numeric expression"))?
            };
            if !number.is_finite() || !(0.0..=maximum).contains(&number) {
                return Err(invalid("morphology option outside supported range"));
            }
            let number = number.round_ties_even() as u16;
            if index == 0 {
                result.coordinates = u8::try_from(number)
                    .map_err(|_| invalid("morphology coordinates must be 0..255"))?;
            } else {
                result.thresholds[index - 1] = number;
            }
        }
        Ok(result)
    }
    /// Reflect horizontal edges and clamp vertical edges. The mask enumerates
    /// the eight surrounding pixels in row order, skipping the center.
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let rgb = frame.subsampling.is_none();
        if !(8..=16).contains(&depth)
            || (rgb && depth != 8)
            || frame.width == 0
            || frame.height == 0
        {
            return Err(invalid("unsupported morphology geometry or depth"));
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let [sx, sy] = frame.subsampling.unwrap_or([1, 1]);
        if sx == 0 || sy == 0 {
            return Err(invalid("zero morphology subsampling"));
        }
        let cw = frame.width.div_ceil(sx);
        let ch = frame.height.div_ceil(sy);
        let size = |w: usize, h: usize| {
            w.checked_mul(h)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("morphology size overflow"))
        };
        let luma = size(frame.width, frame.height)?;
        let chroma = size(cw, ch)?;
        let expected = if rgb {
            luma.checked_mul(3)
        } else {
            chroma.checked_mul(2).and_then(|n| luma.checked_add(n))
        }
        .ok_or_else(|| invalid("morphology storage overflow"))?;
        let maximum = (1u32 << depth) - 1;
        if frame.data.len() != expected
            || (bytes == 2
                && frame
                    .data
                    .as_chunks::<2>().0.iter()
                    .any(|b| u32::from(u16::from_le_bytes([b[0], b[1]])) > maximum))
        {
            return Err(invalid("invalid morphology sample storage"));
        }
        if self.coordinates == 0 || self.thresholds.iter().all(|n| *n == 0) {
            return Ok(());
        }
        let mut out = Vec::new();
        out.try_reserve_exact(expected)
            .map_err(|_| invalid("cannot allocate morphology output"))?;
        out.extend_from_slice(&frame.data);
        for plane in 0..3 {
            // Match planar G,B,R option numbering while retaining RGB24 storage.
            let threshold = u32::from(self.thresholds[if rgb { [2, 0, 1][plane] } else { plane }]);
            if threshold == 0 {
                continue;
            }
            let (w, h, offset, stride) = if rgb {
                (frame.width, frame.height, plane, 3)
            } else if plane == 0 {
                (frame.width, frame.height, 0, bytes)
            } else {
                (cw, ch, luma + (plane - 1) * chroma, bytes)
            };
            let read = |x: usize, y: usize| {
                let at = offset + (y * w + x) * stride;
                if bytes == 1 {
                    u32::from(frame.data[at])
                } else {
                    u32::from(u16::from_le_bytes([frame.data[at], frame.data[at + 1]]))
                }
            };
            for y in 0..h {
                for x in 0..w {
                    let xs = [
                        if x == 0 { 1.min(w - 1) } else { x - 1 },
                        x,
                        if x + 1 == w {
                            w.saturating_sub(2)
                        } else {
                            x + 1
                        },
                    ];
                    let ys = [y.saturating_sub(1), y, (y + 1).min(h - 1)];
                    let center = read(x, y);
                    let mut value = center;
                    for (bit, (dx, dy)) in [
                        (0, 0),
                        (1, 0),
                        (2, 0),
                        (0, 1),
                        (2, 1),
                        (0, 2),
                        (1, 2),
                        (2, 2),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if self.coordinates & (1 << bit) != 0 {
                            let neighbor = read(xs[dx], ys[dy]);
                            value = match self.kind {
                                MorphologyKind::Dilation => value.max(neighbor),
                                MorphologyKind::Erosion => value.min(neighbor),
                            };
                        }
                    }
                    value = match self.kind {
                        MorphologyKind::Dilation => value.min((center + threshold).min(maximum)),
                        MorphologyKind::Erosion => value.max(center.saturating_sub(threshold)),
                    };
                    let at = offset + (y * w + x) * stride;
                    if bytes == 1 {
                        out[at] = value as u8;
                    } else {
                        out[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
                    }
                }
            }
        }
        frame.data = out;
        Ok(())
    }
}
