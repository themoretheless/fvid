struct Range<'a> {
    data: &'a [u8],
    at: usize,
    low: u32,
    range: u32,
    transitions: [u8; 256],
}
impl<'a> Range<'a> {
    fn new(data: &'a [u8]) -> Result<Self> {
        if data.len() < 2 {
            return Err(invalid("truncated FFV1 range header"));
        }
        let low = u32::from(u16::from_be_bytes([data[0], data[1]]));
        if low >= 0xff00 {
            return Err(invalid("invalid FFV1 range header"));
        }
        Ok(Self {
            data,
            at: 2,
            low,
            range: 0xff00,
            transitions: TRANSITION,
        })
    }
    fn bit(&mut self, state: &mut u8) -> Result<bool> {
        if *state == 0 {
            return Err(invalid("invalid FFV1 probability state"));
        }
        let split = (self.range * u32::from(*state)) >> 8;
        self.range -= split;
        let bit = self.low >= self.range;
        if bit {
            self.low -= self.range;
            self.range = split;
            *state = self.transitions[usize::from(*state)];
        } else {
            *state = (256 - u16::from(self.transitions[256 - usize::from(*state)])) as u8;
        }
        if self.range == 0 {
            return Err(invalid("invalid FFV1 range interval"));
        }
        if self.range < 256 {
            // Closed termination supplies zero padding, not unlimited input.
            if self.at >= self.data.len().saturating_add(2) {
                return Err(invalid("truncated FFV1 range payload"));
            }
            self.range <<= 8;
            self.low = (self.low << 8) + u32::from(self.data.get(self.at).copied().unwrap_or(0));
            self.at += 1;
        }
        Ok(bit)
    }
    fn integer(&mut self, s: &mut [u8; 32], signed: bool) -> Result<i32> {
        if self.bit(&mut s[0])? {
            return Ok(0);
        }
        let mut e = 0usize;
        while self.bit(&mut s[1 + e.min(9)])? {
            e += 1;
            if e > 30 {
                return Err(invalid("FFV1 integer overflow"));
            }
        }
        let mut value = 1i32;
        for i in (0..e).rev() {
            value = (value << 1) | i32::from(self.bit(&mut s[22 + i.min(9)])?);
        }
        if signed && self.bit(&mut s[11 + e.min(10)])? {
            value = -value;
        }
        Ok(value)
    }
}
struct State {
    depth: u8,
    has_chroma: bool,
    shifts: [u8; 2],
    tables: [[i32; 256]; 5],
    transitions: [u8; 256],
    models: [Vec<[u8; 32]>; 2],
}
/// One frame in packed planar Y, Cb, Cr order, preserving sample depth.
pub struct Decoded {
    pub frame: GeometryFrame,
    pub depth: u8,
    pub keyframe: bool,
}
/// Dimensions and memory limit are supplied by the container/caller.
/// An error invalidates adaptation history; recovery requires a keyframe.
pub struct Decoder {
    width: usize,
    height: usize,
    budget: usize,
    state: Option<State>,
}
impl Decoder {
    /// Source chroma presence after a successfully decoded frame.
    pub fn monochrome(&self) -> Option<bool> {
        self.state.as_ref().map(|state| !state.has_chroma)
    }
    pub fn new(width: usize, height: usize, budget: usize) -> Result<Self> {
        if width == 0
            || height == 0
            || width.checked_mul(height).is_none()
            || width > isize::MAX as usize
            || height > isize::MAX as usize
        {
            return Err(invalid("invalid FFV1 dimensions"));
        }
        Ok(Self {
            width,
            height,
            budget,
            state: None,
        })
    }
    pub fn reset(&mut self) {
        self.state = None;
    }
    fn header(&self, r: &mut Range<'_>) -> Result<State> {
        let mut h = [128; 32];
        let version = r.integer(&mut h, false)?;
        if version > 1 {
            return Err(unsupported("FFV1 versions above 1 are not implemented"));
        }
        let coder = r.integer(&mut h, false)?;
        if !matches!(coder, 1 | 2) {
            return Err(unsupported("FFV1 Golomb Rice decoding is not implemented"));
        }
        let mut transitions = TRANSITION;
        if coder == 2 {
            for i in 1..256 {
                let n = i64::from(TRANSITION[i]) + i64::from(r.integer(&mut h, true)?);
                if !(1..=255).contains(&n) {
                    return Err(invalid("invalid FFV1 custom transition"));
                }
                transitions[i] = n as u8;
            }
        }
        if r.integer(&mut h, false)? != 0 {
            return Err(unsupported("FFV1 RGB decoding is not implemented"));
        }
        let depth = if version == 0 {
            8
        } else {
            r.integer(&mut h, false)?
        };
        if !(8..=16).contains(&depth) {
            return Err(unsupported("unsupported FFV1 sample depth"));
        }
        let has_chroma = r.bit(&mut h[0])?;
        let sx = r.integer(&mut h, false)?;
        let sy = r.integer(&mut h, false)?;
        if sx > 4 || sy > 4 {
            return Err(invalid("invalid FFV1 chroma shift"));
        }
        if r.bit(&mut h[0])? {
            return Err(unsupported("FFV1 alpha decoding is not implemented"));
        }
        let mut tables = [[0; 256]; 5];
        let mut scale = 1i32;
        for table in &mut tables {
            let mut model = [128; 32];
            let mut at = 0usize;
            let mut value = 0;
            while at < 128 {
                let run = r.integer(&mut model, false)? as usize + 1;
                if run > 128 - at {
                    return Err(invalid("invalid FFV1 quantizer run"));
                }
                table[at..at + run].fill(scale * value);
                at += run;
                value += 1;
            }
            for i in 1..128 {
                table[256 - i] = -table[i];
            }
            table[128] = -table[127];
            scale = scale
                .checked_mul(2 * value - 1)
                .filter(|v| *v <= 32768)
                .ok_or_else(|| invalid("FFV1 context count overflow"))?;
        }
        let count = (scale as usize + 1) / 2;
        let bytes = count * 64 + std::mem::size_of::<State>();
        if bytes > self.budget {
            return Err(invalid("FFV1 contexts exceed memory budget"));
        }
        let mut models = [Vec::new(), Vec::new()];
        for model in &mut models {
            model
                .try_reserve_exact(count)
                .map_err(|_| invalid("cannot allocate FFV1 contexts"))?;
            model.resize(count, [128; 32]);
        }
        Ok(State {
            depth: depth as u8,
            has_chroma,
            shifts: [sx as u8, sy as u8],
            tables,
            transitions,
            models,
        })
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Decoded> {
        let old = self.state.take();
        let mut r = Range::new(packet)?;
        let keyframe = r.bit(&mut 128)?;
        let mut state = if keyframe {
            drop(old);
            self.header(&mut r)?
        } else {
            old.ok_or_else(|| invalid("FFV1 frame requires a preceding keyframe"))?
        };
        r.transitions = state.transitions;
        let sx = if state.has_chroma {
            1usize << state.shifts[0]
        } else {
            1
        };
        let sy = if state.has_chroma {
            1usize << state.shifts[1]
        } else {
            1
        };
        let cw = self.width.div_ceil(sx);
        let ch = self.height.div_ceil(sy);
        let samples = self
            .width
            .checked_mul(self.height)
            .and_then(|n| {
                cw.checked_mul(ch)
                    .and_then(|c| c.checked_mul(2))
                    .and_then(|c| n.checked_add(c))
            })
            .ok_or_else(|| invalid("FFV1 frame size overflow"))?;
        let bytes = if state.depth == 8 { 1 } else { 2 };
        let len = samples
            .checked_mul(bytes)
            .ok_or_else(|| invalid("FFV1 frame size overflow"))?;
        let storage = len
            .checked_add(state.models[0].len() * 64 + std::mem::size_of::<State>())
            .ok_or_else(|| invalid("FFV1 storage overflow"))?;
        if storage > self.budget {
            return Err(invalid("FFV1 frame exceeds memory budget"));
        }
        let mut data = buffer(len)?;
        let mut offset = 0;
        for plane in 0..if state.has_chroma { 3 } else { 1 } {
            let (w, h) = if plane == 0 {
                (self.width, self.height)
            } else {
                (cw, ch)
            };
            let model = &mut state.models[usize::from(plane != 0)];
            let read = |data: &[u8], x: isize, y: isize| -> i32 {
                if y < 0 || x < -1 {
                    return 0;
                }
                let (x, y) = if x == -1 {
                    if y == 0 {
                        return 0;
                    }
                    (0, y - 1)
                } else {
                    (x.min(w as isize - 1), y)
                };
                let at = offset + (y as usize * w + x as usize) * bytes;
                let v = if bytes == 1 {
                    u16::from(data[at])
                } else {
                    u16::from_le_bytes([data[at], data[at + 1]])
                };
                if state.depth == 16 {
                    i32::from(v as i16)
                } else {
                    i32::from(v)
                }
            };
            for y in 0..h {
                for x in 0..w {
                    let (xx, yy) = (x as isize, y as isize);
                    let l = read(&data, xx - 1, yy);
                    let t = read(&data, xx, yy - 1);
                    let tl = read(&data, xx - 1, yy - 1);
                    let tr = read(&data, xx + 1, yy - 1);
                    let ll = read(&data, xx - 2, yy);
                    let tt = read(&data, xx, yy - 2);
                    let mut context = 0i32;
                    for (table, d) in
                        state
                            .tables
                            .iter()
                            .zip([l - tl, tl - t, t - tr, ll - l, tt - t])
                    {
                        context += table[(d & 255) as usize];
                    }
                    let probabilities = model
                        .get_mut(context.unsigned_abs() as usize)
                        .ok_or_else(|| invalid("invalid FFV1 context"))?;
                    let mut residual = r.integer(probabilities, true)?;
                    if residual.unsigned_abs() > (1u32 << (state.depth - 1)) {
                        return Err(invalid("FFV1 residual exceeds sample depth"));
                    }
                    if context < 0 {
                        residual = -residual;
                    }
                    let prediction = (l + t - tl).clamp(l.min(t), l.max(t));
                    let v = ((prediction + residual) & ((1i32 << state.depth) - 1)) as u16;
                    let at = offset + (y * w + x) * bytes;
                    data[at] = v as u8;
                    if bytes == 2 {
                        data[at + 1] = (v >> 8) as u8;
                    }
                }
            }
            offset += w * h * bytes;
        }
        let depth = state.depth;
        if !state.has_chroma {
            // Keep the public decoded-frame contract planar Y/Cb/Cr. The gray
            // luma is unchanged; synthesized full-resolution chroma is neutral.
            let neutral = (1u16 << (depth - 1)).to_le_bytes();
            for sample in data[offset..].chunks_exact_mut(bytes) {
                sample.copy_from_slice(&neutral[..bytes]);
            }
        }
        self.state = Some(state);
        Ok(Decoded {
            frame: GeometryFrame {
                width: self.width,
                height: self.height,
                subsampling: Some([sx, sy]),
                data,
            },
            depth,
            keyframe,
        })
    }
}
