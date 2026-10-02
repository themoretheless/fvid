#[derive(Clone, Copy, Debug)]
pub struct Pixelize {
    width: usize,
    height: usize,
    mode: u8,
    planes: u8,
}
impl Pixelize {
    pub fn parse(args: &str) -> Result<Self> {
        let mut values = [16usize, 16, 0, 15];
        let mut positional = 0;
        if !args.is_empty() {
            for option in args.split(':') {
                let (key, value) = if let Some(pair) = option.split_once('=') {
                    pair
                } else {
                    let key = *["width", "height", "mode", "planes"]
                        .get(positional)
                        .ok_or_else(|| invalid("too many pixelize options"))?;
                    positional += 1;
                    (key, option)
                };
                let index = match key {
                    "width" | "w" => 0,
                    "height" | "h" => 1,
                    "mode" | "m" => 2,
                    "planes" | "p" => 3,
                    _ => return Err(invalid("unknown pixelize option")),
                };
                values[index] = if index == 2 {
                    match value {
                        "avg" => 0,
                        "min" => 1,
                        "max" => 2,
                        _ => value
                            .parse()
                            .map_err(|_| invalid("invalid pixelize mode"))?,
                    }
                } else {
                    value
                        .parse()
                        .map_err(|_| invalid("invalid pixelize integer"))?
                };
            }
        }
        if !(1..=1024).contains(&values[0])
            || !(1..=1024).contains(&values[1])
            || values[2] > 2
            || values[3] > 15
        {
            return Err(invalid("pixelize option outside supported range"));
        }
        Ok(Self {
            width: values[0],
            height: values[1],
            mode: values[2] as u8,
            planes: values[3] as u8,
        })
    }
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame
            .subsampling
            .ok_or_else(|| invalid("pixelize requires planar YUV"))?;
        if !matches!(sx, 1 | 2 | 4)
            || !matches!(sy, 1 | 2 | 4)
            || frame.width == 0
            || frame.height == 0
            || !(8..=16).contains(&depth)
        {
            return Err(invalid("invalid pixelize geometry or depth"));
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let cw = frame.width.div_ceil(sx);
        let ch = frame.height.div_ceil(sy);
        let luma = frame
            .width
            .checked_mul(frame.height)
            .ok_or_else(|| invalid("pixelize size overflow"))?;
        let chroma = cw
            .checked_mul(ch)
            .ok_or_else(|| invalid("pixelize size overflow"))?;
        let total = chroma
            .checked_mul(2)
            .and_then(|n| n.checked_add(luma))
            .and_then(|n| n.checked_mul(bytes))
            .ok_or_else(|| invalid("pixelize storage overflow"))?;
        if frame.data.len() != total {
            return Err(invalid("invalid pixelize plane storage"));
        }
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|s| u32::from(u16::from_le_bytes([s[0], s[1]])) >= 1u32 << depth)
        {
            return Err(invalid("pixelize sample exceeds depth"));
        }
        let (bw, bh) = ((self.width / sx).max(1), (self.height / sy).max(1));
        for (p, offset, w, h, bw, bh) in [
            (0, 0, frame.width, frame.height, bw * sx, bh * sy),
            (1, luma, cw, ch, bw, bh),
            (2, luma + chroma, cw, ch, bw, bh),
        ] {
            if self.planes & (1 << p) == 0 {
                continue;
            }
            for y in (0..h).step_by(bh) {
                for x in (0..w).step_by(bw) {
                    let ew = (x + bw).min(w);
                    let eh = (y + bh).min(h);
                    let mut sum = 0u64;
                    let mut low = u16::MAX;
                    let mut high = 0u16;
                    for yy in y..eh {
                        for xx in x..ew {
                            let at = (offset + yy * w + xx) * bytes;
                            let value = if bytes == 1 {
                                u16::from(frame.data[at])
                            } else {
                                u16::from_le_bytes([frame.data[at], frame.data[at + 1]])
                            };
                            sum += u64::from(value);
                            low = low.min(value);
                            high = high.max(value);
                        }
                    }
                    let fill = match self.mode {
                        0 => (sum / ((ew - x) * (eh - y)) as u64) as u16,
                        1 => low,
                        _ => high,
                    };
                    for yy in y..eh {
                        for xx in x..ew {
                            let at = (offset + yy * w + xx) * bytes;
                            frame.data[at] = fill as u8;
                            if bytes == 2 {
                                frame.data[at + 1] = (fill >> 8) as u8;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
