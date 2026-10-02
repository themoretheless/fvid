
#[derive(Clone, Copy, Debug)]
pub struct BoxBlur {
    radius: [usize; 3],
    power: [u32; 3],
}
impl BoxBlur {
    /// Integer radii and pass counts, in luma/chroma/alpha option order.
    /// Expressions remain the responsibility of the legacy request adapter.
    pub fn parse(args: &str) -> Result<Self> {
        let names = [
            "luma_radius",
            "luma_power",
            "chroma_radius",
            "chroma_power",
            "alpha_radius",
            "alpha_power",
        ];
        let aliases = ["lr", "lp", "cr", "cp", "ar", "ap"];
        let mut values = [Some(2i32), Some(2), None, None, None, None];
        let mut positional = 0;
        if !args.is_empty() {
            for entry in args.split(':') {
                let (key, value) = if let Some(pair) = entry.split_once('=') {
                    pair
                } else {
                    let key = *names
                        .get(positional)
                        .ok_or_else(|| invalid("too many boxblur options"))?;
                    positional += 1;
                    (key, entry)
                };
                let index = names
                    .iter()
                    .zip(aliases)
                    .position(|(name, alias)| *name == key || alias == key)
                    .ok_or_else(|| invalid("unknown boxblur option"))?;
                let n = value
                    .parse::<i32>()
                    .map_err(|_| invalid("owned boxblur requires integer radii and powers"))?;
                if n < 0 && !(n == -1 && matches!(index, 3 | 5)) {
                    return Err(invalid("boxblur option out of range"));
                }
                values[index] = if n == -1 { None } else { Some(n) };
            }
        }
        let radius = values[0].unwrap() as usize;
        let power = values[1].unwrap() as u32;
        Ok(Self {
            radius: [
                radius,
                values[2].map_or(radius, |n| n as usize),
                values[4].map_or(radius, |n| n as usize),
            ],
            power: [
                power,
                values[3].map_or(power, |n| n as u32),
                values[5].map_or(power, |n| n as u32),
            ],
        })
    }

    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame
            .subsampling
            .ok_or_else(|| invalid("boxblur requires planar YUV"))?;
        if sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0 || !(8..=16).contains(&depth)
        {
            return Err(invalid("invalid boxblur geometry or depth"));
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let (cw, ch) = (frame.width.div_ceil(sx), frame.height.div_ceil(sy));
        let size = |w: usize, h: usize| {
            w.checked_mul(h)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("boxblur size overflow"))
        };
        let luma = size(frame.width, frame.height)?;
        let chroma = size(cw, ch)?;
        let total = chroma
            .checked_mul(2)
            .and_then(|n| n.checked_add(luma))
            .ok_or_else(|| invalid("boxblur storage overflow"))?;
        if total != frame.data.len()
            || (bytes == 2
                && frame
                    .data
                    .chunks_exact(2)
                    .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) >= 1u32 << depth))
        {
            return Err(invalid("invalid boxblur sample storage"));
        }
        // The filter's radius contract uses floor chroma dimensions, even for
        // an odd-size source whose stored planes have a rounded-up last sample.
        for (index, (w, h)) in [
            (frame.width, frame.height),
            (frame.width / sx, frame.height / sy),
            (frame.width, frame.height),
        ]
        .into_iter()
        .enumerate()
        {
            if self.radius[index]
                .checked_mul(2)
                .is_none_or(|n| n >= w.min(h))
            {
                return Err(invalid(
                    "boxblur radius must be less than half the plane dimension",
                ));
            }
        }
        if (self.radius[0] == 0 || self.power[0] == 0)
            && (self.radius[1] == 0 || self.power[1] == 0)
        {
            return Ok(());
        }
        let len = frame.width.max(frame.height);
        let mut a = Vec::<u16>::new();
        let mut b = Vec::<u16>::new();
        for v in [&mut a, &mut b] {
            v.try_reserve_exact(len)
                .map_err(|_| invalid("boxblur scratch allocation failed"))?;
            v.resize(len, 0);
        }
        let mut output = buffer(total)?;
        output.copy_from_slice(&frame.data);
        let mut base = 0;
        for (plane, (w, h)) in [(frame.width, frame.height), (cw, ch), (cw, ch)]
            .into_iter()
            .enumerate()
        {
            let p = usize::from(plane != 0);
            let radius = self.radius[p];
            let power = self.power[p];
            if radius != 0 && power != 0 {
                for vertical in [false, true] {
                    let (lines, length, step) = if vertical { (w, h, w) } else { (h, w, 1) };
                    for line in 0..lines {
                        let start = if vertical { line } else { line * w };
                        for i in 0..length {
                            let at = base + (start + i * step) * bytes;
                            a[i] = if bytes == 1 {
                                u16::from(output[at])
                            } else {
                                u16::from_le_bytes([output[at], output[at + 1]])
                            };
                        }
                        for _ in 0..power {
                            blur_line(&a[..length], &mut b[..length], radius, bytes);
                            std::mem::swap(&mut a, &mut b);
                        }
                        for i in 0..length {
                            let at = base + (start + i * step) * bytes;
                            if bytes == 1 {
                                output[at] = a[i] as u8;
                            } else {
                                output[at..at + 2].copy_from_slice(&a[i].to_le_bytes());
                            }
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

fn blur_line(input: &[u16], output: &mut [u16], radius: usize, bytes: usize) {
    let length = 2 * radius + 1;
    let reciprocal = (65536 + length as u64 / 2) / length as u64;
    let reflect = |i: i128| -> usize {
        if i < 0 {
            (-i - 1) as usize
        } else if i >= input.len() as i128 {
            (2 * input.len() as i128 - i - 1) as usize
        } else {
            i as usize
        }
    };
    let mut sum = 0u64;
    for i in -(radius as i128)..=radius as i128 {
        sum += u64::from(input[reflect(i)]);
    }
    for (x, value) in output.iter_mut().enumerate() {
        if x > 0 {
            sum -= u64::from(input[reflect(x as i128 - radius as i128 - 1)]);
            sum += u64::from(input[reflect(x as i128 + radius as i128)]);
        }
        let rounded = (sum * reciprocal + 32768) >> 16;
        // Quantize at each pass to the stored sample width, as the public
        // media filter does (not one division after the full 2D operation).
        *value = if bytes == 1 {
            u16::from(rounded as u8)
        } else {
            rounded as u16
        };
    }
}
