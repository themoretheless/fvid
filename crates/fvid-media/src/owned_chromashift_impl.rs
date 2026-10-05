
#[derive(Clone, Copy, Debug, Default)]
pub struct ChromaShift {
    shifts: [[i16; 2]; 2],
    wrap: bool,
}
impl ChromaShift {
    pub fn parse(args: &str) -> Result<Self> {
        let mut result = Self::default();
        if args.is_empty() {
            return Ok(result);
        }
        let names = ["cbh", "cbv", "crh", "crv", "edge"];
        let mut positional = 0;
        for entry in args.split(':') {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *names
                    .get(positional)
                    .ok_or_else(|| invalid("too many chromashift options"))?;
                positional += 1;
                (key, entry)
            };
            let index = names
                .iter()
                .position(|name| *name == key.trim())
                .ok_or_else(|| invalid("unknown chromashift option"))?;
            let (minimum, maximum, constants): (f64, f64, &[(&str, f64)]) = if index == 4 {
                (0.0, 1.0, &[("smear", 0.0), ("wrap", 1.0)])
            } else {
                (-255.0, 255.0, &[])
            };
            let mut variables = vec![("min", minimum), ("max", maximum), ("default", 0.0)];
            variables.extend_from_slice(constants);
            let number = chromashift_expression::Expression::parse(value.trim())
                .and_then(|expression| expression.evaluate(&variables))
                .map_err(|_| invalid("chromashift requires a constant numeric expression"))?;
            if !number.is_finite() || !(minimum..=maximum).contains(&number) {
                return Err(invalid("chromashift option outside supported range"));
            }
            let number = number.round_ties_even() as i16;
            if index == 4 {
                result.wrap = number != 0;
            } else {
                result.shifts[index / 2][index % 2] = number;
            }
        }
        Ok(result)
    }

    /// Validate before changing any samples. Luma and sample precision are retained.
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame
            .subsampling
            .ok_or_else(|| invalid("chromashift requires planar YUV"))?;
        if sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0 || !(8..=16).contains(&depth)
        {
            return Err(invalid("invalid chromashift geometry or depth"));
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let (cw, ch) = (frame.width.div_ceil(sx), frame.height.div_ceil(sy));
        let size = |w: usize, h: usize| {
            w.checked_mul(h)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("chromashift size overflow"))
        };
        let luma = size(frame.width, frame.height)?;
        let chroma = size(cw, ch)?;
        let total = chroma
            .checked_mul(2)
            .and_then(|n| n.checked_add(luma))
            .ok_or_else(|| invalid("chromashift storage overflow"))?;
        let max = (1u32 << depth) - 1;
        if frame.data.len() != total
            || (bytes == 2
                && frame
                    .data
                    .as_chunks::<2>().0.iter()
                    .any(|b| u32::from(u16::from_le_bytes([b[0], b[1]])) > max))
        {
            return Err(invalid("invalid chromashift sample storage"));
        }
        if self.shifts == [[0; 2]; 2] {
            return Ok(());
        }
        let mut scratch = buffer(chroma)?;
        let coordinate = |at: usize, shift: i16, len: usize| -> usize {
            // i128 also covers any representable usize dimension without overflow.
            let p = at as i128 - i128::from(shift);
            if self.wrap {
                p.rem_euclid(len as i128) as usize
            } else {
                p.clamp(0, len as i128 - 1) as usize
            }
        };
        for (plane, [dx, dy]) in self.shifts.into_iter().enumerate() {
            if dx == 0 && dy == 0 {
                continue;
            }
            let base = luma + plane * chroma;
            scratch.copy_from_slice(&frame.data[base..base + chroma]);
            for y in 0..ch {
                let row = coordinate(y, dy, ch) * cw;
                for x in 0..cw {
                    let from = (row + coordinate(x, dx, cw)) * bytes;
                    let to = base + (y * cw + x) * bytes;
                    frame.data[to..to + bytes].copy_from_slice(&scratch[from..from + bytes]);
                }
            }
        }
        Ok(())
    }
}
