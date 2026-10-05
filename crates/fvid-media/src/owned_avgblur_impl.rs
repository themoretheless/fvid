
#[derive(Clone, Copy, Debug)]
pub struct AverageBlur {
    radius: [usize; 2],
    planes: u8,
}
impl AverageBlur {
    pub fn parse(args: &str) -> Result<Self> {
        let mut values = [1usize, 15, 0];
        let names = ["sizeX", "planes", "sizeY"];
        let mut positional = 0;
        if !args.is_empty() {
            for entry in args.split(':') {
                let (key, value) = if let Some(pair) = entry.split_once('=') {
                    pair
                } else {
                    let key = *names
                        .get(positional)
                        .ok_or_else(|| invalid("too many avgblur options"))?;
                    positional += 1;
                    (key, entry)
                };
                let i = names
                    .iter()
                    .position(|n| *n == key)
                    .ok_or_else(|| invalid("unknown avgblur option"))?;
                let n = value
                    .parse::<usize>()
                    .map_err(|_| invalid("avgblur requires integer options"))?;
                let valid = match i {
                    0 => (1..=1024).contains(&n),
                    1 => n <= 15,
                    _ => n <= 1024,
                };
                if !valid {
                    return Err(invalid("avgblur option out of range"));
                }
                values[i] = n;
            }
        }
        Ok(Self {
            radius: [
                values[0],
                if values[2] == 0 { values[0] } else { values[2] },
            ],
            planes: values[1] as u8,
        })
    }

    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame
            .subsampling
            .ok_or_else(|| invalid("avgblur requires planar YUV"))?;
        if sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0 || !(8..=16).contains(&depth)
        {
            return Err(invalid("invalid avgblur geometry or depth"));
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let cw = frame.width.div_ceil(sx);
        let ch = frame.height.div_ceil(sy);
        let area = |w: usize, h: usize| {
            w.checked_mul(h)
                .ok_or_else(|| invalid("avgblur size overflow"))
        };
        let luma = area(frame.width, frame.height)?;
        let chroma = area(cw, ch)?;
        let total = chroma
            .checked_mul(2)
            .and_then(|n| n.checked_add(luma))
            .and_then(|n| n.checked_mul(bytes))
            .ok_or_else(|| invalid("avgblur storage overflow"))?;
        if total != frame.data.len()
            || (bytes == 2
                && frame
                    .data
                    .as_chunks::<2>().0.iter()
                    .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) >= 1u32 << depth))
        {
            return Err(invalid("invalid avgblur sample storage"));
        }
        let rx = self.radius[0].min(cw / 2);
        let ry = self.radius[1].min(ch / 2);
        if self.planes & 7 == 0 || (rx == 0 && ry == 0) {
            return Ok(());
        }
        let divisor = ((2 * rx + 1) * (2 * ry + 1)) as u64;
        let mut output = buffer(total)?;
        output.copy_from_slice(&frame.data);
        let mut columns = Vec::<u64>::new();
        columns
            .try_reserve_exact(frame.width)
            .map_err(|_| invalid("avgblur scratch allocation failed"))?;
        columns.resize(frame.width, 0);
        let mut base = 0;
        for (plane, (w, h)) in [(frame.width, frame.height), (cw, ch), (cw, ch)]
            .into_iter()
            .enumerate()
        {
            if self.planes & (1 << plane) != 0 {
                let sample = |x: usize, y: usize| -> u64 {
                    let at = base + (y * w + x) * bytes;
                    if bytes == 1 {
                        u64::from(frame.data[at])
                    } else {
                        u64::from(u16::from_le_bytes([frame.data[at], frame.data[at + 1]]))
                    }
                };
                for (x, column) in columns[..w].iter_mut().enumerate() {
                    *column = sample(x, 0) * ry as u64;
                    for y in 0..=ry {
                        *column += sample(x, y);
                    }
                }
                for y in 0..h {
                    if y > 0 {
                        for (x, column) in columns[..w].iter_mut().enumerate() {
                            *column = *column - sample(x, y.saturating_sub(ry + 1))
                                + sample(x, (y + ry).min(h - 1));
                        }
                    }
                    let mut sum = columns[0] * rx as u64 + columns[..=rx].iter().sum::<u64>();
                    for x in 0..w {
                        if x > 0 {
                            sum = sum - columns[x.saturating_sub(rx + 1)]
                                + columns[(x + rx).min(w - 1)];
                        }
                        let value = (sum / divisor) as u16;
                        let at = base + (y * w + x) * bytes;
                        if bytes == 1 {
                            output[at] = value as u8;
                        } else {
                            output[at..at + 2].copy_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
            base += w * h * bytes;
        }
        frame.data = output;
        Ok(())
    }
}
