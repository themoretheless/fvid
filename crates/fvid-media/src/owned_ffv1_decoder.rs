//! Owned FFV1 v0/v1/v3 decoding, persistent slice contexts and bounded storage.
//! RFC 9043 bitstream core is shared with native frontend playback.
use crate::owned_ffv1_encoder::TRANSITION;
use crate::owned_frame::GeometryFrame;
pub use fvid_control::error::{Error, Result};
use fvid_control::error::{buffer, invalid, unsupported};
pub(crate) const UNSUPPORTED_PREFIX: &str = "unsupported FFV1 capability: ";
struct Range<'a> {
    data: &'a [u8],
    at: usize,
    low: u32,
    range: u32,
    transitions: &'a [u8; 256],
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
            transitions: &TRANSITION,
        })
    }
    fn bit(&mut self, state: &mut u8) -> Result<bool> {
        if *state == 0 {
            // A zero-probability one is impossible; this context emits zero
            // without consuming or renormalizing the range interval.
            return Ok(false);
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
    rice: Option<[Vec<RiceModel>; 3]>,
    rgb: bool,
    has_alpha: bool,
    depth: u8,
    has_chroma: bool,
    shifts: [u8; 2],
    tables: Vec<[[i32; 256]; 5]>,
    transitions: [u8; 256],
    models: [Vec<[u8; 32]>; 3],
}
impl State {
    fn context_bytes(&self) -> usize {
        self.models.iter().map(|m| m.len() * 32).sum::<usize>()
            + self.rice.as_ref().map_or(0, |models| {
                models
                    .iter()
                    .map(|m| m.len() * std::mem::size_of::<RiceModel>())
                    .sum::<usize>()
            })
            + self.tables.len() * std::mem::size_of::<[[i32; 256]; 5]>()
            + std::mem::size_of::<Self>()
    }
}
/// One frame in planar Y/Cb/Cr or interleaved RGB, preserving sample depth.
pub struct Decoded {
    pub frame: GeometryFrame,
    /// Full-resolution alpha at `depth`, little-endian u16 above eight bits.
    pub alpha: Option<Vec<u8>>,
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
    v3: Option<Box<V3>>,
}
impl Decoder {
    /// Source chroma presence after a successfully decoded frame.
    pub fn monochrome(&self) -> Option<bool> {
        if let Some(v3) = self.v3.as_ref() {
            return v3.live.then_some(!v3.config.chroma);
        }
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
            v3: None,
        })
    }
    pub fn reset(&mut self) {
        self.state = None;
        if let Some(v3) = self.v3.as_mut() {
            v3.history.clear();
            v3.live = false;
        }
    }
    fn header(&self, r: &mut Range<'_>) -> Result<State> {
        let mut h = [128; 32];
        let version = r.integer(&mut h, false)?;
        if version > 1 {
            return Err(unsupported("FFV1 versions above 1 are not implemented"));
        }
        let coder = r.integer(&mut h, false)?;
        if !matches!(coder, 0 | 1 | 2) {
            return Err(unsupported("unsupported FFV1 entropy coder"));
        }
        let mut transitions = TRANSITION;
        if coder == 2 {
            for i in 1..256 {
                let n = i64::from(TRANSITION[i]) + i64::from(r.integer(&mut h, true)?);
                if !(0..=255).contains(&n) {
                    return Err(invalid("invalid FFV1 custom transition"));
                }
                transitions[i] = n as u8;
            }
        }
        let rgb = match r.integer(&mut h, false)? {
            0 => false,
            1 => true,
            _ => return Err(unsupported("unsupported FFV1 color space")),
        };
        let depth = if version == 0 {
            8
        } else {
            // RFC 9043 section 4.2.7: accept historical zero as eight bits.
            match r.integer(&mut h, false)? { 0 => 8, value => value }
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
        let has_alpha = r.bit(&mut h[0])?;
        if rgb && (!has_chroma || sx != 0 || sy != 0) {
            return Err(invalid("invalid FFV1 RGB chroma configuration"));
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
        let count = (scale as usize).div_ceil(2);
        let groups = if has_alpha { 3 } else { 2 };
        let bytes = count
            * groups
            * if coder == 0 {
                std::mem::size_of::<RiceModel>()
            } else {
                32
            }
            + groups * std::mem::size_of::<[[i32; 256]; 5]>()
            + std::mem::size_of::<State>();
        if bytes > self.budget {
            return Err(invalid("FFV1 contexts exceed memory budget"));
        }
        let mut models = [Vec::new(), Vec::new(), Vec::new()];
        for model in models.iter_mut().take(groups) {
            if coder == 0 {
                continue;
            }
            model
                .try_reserve_exact(count)
                .map_err(|_| invalid("cannot allocate FFV1 contexts"))?;
            model.resize(count, [128; 32]);
        }
        let rice = if coder == 0 {
            let mut models = [Vec::new(), Vec::new(), Vec::new()];
            for model in models.iter_mut().take(groups) {
                model
                    .try_reserve_exact(count)
                    .map_err(|_| invalid("cannot allocate FFV1 Rice contexts"))?;
                model.resize(count, RiceModel::default());
            }
            Some(models)
        } else {
            None
        };
        let mut quantizers = Vec::new();
        quantizers
            .try_reserve_exact(groups)
            .map_err(|_| invalid("cannot allocate FFV1 quantizers"))?;
        quantizers.resize(groups, tables);
        Ok(State {
            rice,
            has_alpha,
            rgb,
            depth: depth as u8,
            has_chroma,
            shifts: [sx as u8, sy as u8],
            tables: quantizers,
            transitions,
            models,
        })
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Decoded> {
        if let Some(mut v3) = self.v3.take() {
            let result = v3.decode(packet, self.width, self.height, self.budget);
            if result.is_err() {
                v3.history.clear();
                v3.live = false;
            }
            self.v3 = Some(v3);
            return result;
        }
        let old = self.state.take();
        let mut r = Range::new(packet)?;
        let keyframe = r.bit(&mut 128)?;
        let state = if keyframe {
            drop(old);
            self.header(&mut r)?
        } else {
            old.ok_or_else(|| invalid("FFV1 frame requires a preceding keyframe"))?
        };
        let transitions = state.transitions;
        r.transitions = &transitions;
        let mut rice = if state.rice.is_some() {
            r.bit(&mut 129)?;
            let start =
                r.at.checked_sub(1)
                    .filter(|&at| at <= packet.len())
                    .ok_or_else(|| invalid("truncated FFV1 Golomb Rice header"))?;
            Some(Rice {
                data: &packet[start..],
                at: 0,
            })
        } else {
            None
        };
        let (decoded, state) = decode_content(
            self.width,
            self.height,
            self.budget,
            &mut r,
            &mut rice,
            state,
            keyframe,
        )?;
        self.state = Some(state);
        Ok(decoded)
    }
}

fn decode_content(
    width: usize,
    height: usize,
    budget: usize,
    r: &mut Range<'_>,
    rice: &mut Option<Rice<'_>>,
    mut state: State,
    keyframe: bool,
) -> Result<(Decoded, State)> {
    if state.rgb {
        let (frame, alpha) = decode_rgb(r, rice, &mut state, width, height, budget)?;
        if let Some(rice) = rice.as_mut() {
            rice.finish()?;
        }
        let depth = state.depth;
        return Ok((
            Decoded {
                frame,
                alpha,
                depth,
                keyframe,
            },
            state,
        ));
    }
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
    let cw = width.div_ceil(sx);
    let ch = height.div_ceil(sy);
    let samples = width
        .checked_mul(height)
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
    let alpha_len = if state.has_alpha {
        width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(bytes))
            .ok_or_else(|| invalid("FFV1 alpha size overflow"))?
    } else {
        0
    };
    let storage = len
        .checked_add(alpha_len)
        .and_then(|n| n.checked_add(state.context_bytes()))
        .ok_or_else(|| invalid("FFV1 storage overflow"))?;
    if storage > budget {
        return Err(invalid("FFV1 frame exceeds memory budget"));
    }
    let mut data = buffer(len)?;
    let mut alpha = if state.has_alpha {
        Some(buffer(alpha_len)?)
    } else {
        None
    };
    let mut offset = 0;
    for plane in 0..if state.has_chroma { 3 } else { 1 } {
        let (w, h) = if plane == 0 {
            (width, height)
        } else {
            (cw, ch)
        };
        let length = w * h * bytes;
        decode_plane(
            r,
            rice,
            &mut state,
            &mut data[offset..offset + length],
            w,
            h,
            usize::from(plane != 0),
        )?;
        offset += length;
    }
    if let Some(alpha) = alpha.as_mut() {
        decode_plane(r, rice, &mut state, alpha, width, height, 2)?;
    }
    let depth = state.depth;
    if let Some(rice) = rice.as_mut() {
        rice.finish()?;
    }
    if !state.has_chroma {
        // Keep the public decoded-frame contract planar Y/Cb/Cr. The gray
        // luma is unchanged; synthesized full-resolution chroma is neutral.
        let neutral = (1u16 << (depth - 1)).to_le_bytes();
        for sample in data[offset..].chunks_exact_mut(bytes) {
            sample.copy_from_slice(&neutral[..bytes]);
        }
    }
    Ok((
        Decoded {
            frame: GeometryFrame {
                width: width,
                height: height,
                subsampling: Some([sx, sy]),
                data,
            },
            alpha,
            depth,
            keyframe,
        },
        state,
    ))
}

fn decode_plane(
    r: &mut Range<'_>,
    rice: &mut Option<Rice<'_>>,
    state: &mut State,
    data: &mut [u8],
    w: usize,
    h: usize,
    group: usize,
) -> Result<()> {
    let depth = state.depth;
    let bytes = if depth == 8 { 1 } else { 2 };
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
        let at = (y as usize * w + x as usize) * bytes;
        let value = if bytes == 1 {
            u16::from(data[at])
        } else {
            u16::from_le_bytes([data[at], data[at + 1]])
        };
        if depth == 16 {
            i32::from(value as i16)
        } else {
            i32::from(value)
        }
    };
    let mut run = RiceRun::default();
    for y in 0..h {
        run.line();
        for x in 0..w {
            let (xx, yy) = (x as isize, y as isize);
            let l = read(data, xx - 1, yy);
            let t = read(data, xx, yy - 1);
            let tl = read(data, xx - 1, yy - 1);
            let tr = read(data, xx + 1, yy - 1);
            let ll = read(data, xx - 2, yy);
            let tt = read(data, xx, yy - 2);
            let context: i32 = state.tables[group]
                .iter()
                .zip([l - tl, tl - t, t - tr, ll - l, tt - t])
                .map(|(table, d)| table[(d & 255) as usize])
                .sum();
            let mut residual = if let Some(rice) = rice.as_mut() {
                rice.sample(
                    &mut state.rice.as_mut().unwrap()[group],
                    context,
                    depth,
                    &mut run,
                    x,
                    w,
                )?
            } else {
                let probabilities = state.models[group]
                    .get_mut(context.unsigned_abs() as usize)
                    .ok_or_else(|| invalid("invalid FFV1 context"))?;
                r.integer(probabilities, true)?
            };
            if residual.unsigned_abs() > (1u32 << (depth - 1)) {
                return Err(invalid("FFV1 residual exceeds sample depth"));
            }
            if context < 0 {
                residual = -residual;
            }
            let prediction = (l + t - tl).clamp(l.min(t), l.max(t));
            let value = ((prediction + residual) & ((1i32 << depth) - 1)) as u16;
            let at = (y * w + x) * bytes;
            data[at] = value as u8;
            if bytes == 2 {
                data[at + 1] = (value >> 8) as u8;
            }
        }
    }
    Ok(())
}

fn decode_rgb(
    r: &mut Range<'_>,
    rice: &mut Option<Rice<'_>>,
    state: &mut State,
    width: usize,
    height: usize,
    budget: usize,
) -> Result<(GeometryFrame, Option<Vec<u8>>)> {
    let count = width
        .checked_mul(height)
        .ok_or_else(|| invalid("FFV1 RGB size overflow"))?;
    let bytes = if state.depth == 8 { 1 } else { 2 };
    let length = count
        .checked_mul(3 * bytes)
        .ok_or_else(|| invalid("FFV1 RGB size overflow"))?;
    let components = if state.has_alpha { 4 } else { 3 };
    let alpha_length = if state.has_alpha { count * bytes } else { 0 };
    let storage = count
        .checked_mul(components * 4)
        .and_then(|n| n.checked_add(length))
        .and_then(|n| n.checked_add(alpha_length))
        .and_then(|n| n.checked_add(state.context_bytes()))
        .ok_or_else(|| invalid("FFV1 RGB storage overflow"))?;
    if storage > budget {
        return Err(invalid("FFV1 RGB frame exceeds memory budget"));
    }
    let mut planes: [Vec<i32>; 4] = std::array::from_fn(|_| Vec::new());
    for p in planes.iter_mut().take(components) {
        p.try_reserve_exact(count)
            .map_err(|_| invalid("FFV1 RGB allocation failed"))?;
        p.resize(count, 0);
    }
    let read = |p: &[i32], x: isize, y: isize| {
        if y < 0 || x < -1 {
            return 0;
        }
        if x == -1 {
            if y == 0 {
                0
            } else {
                p[(y as usize - 1) * width]
            }
        } else {
            p[y as usize * width + (x as usize).min(width - 1)]
        }
    };
    let bias = 1i32 << state.depth;
    let mask = (bias << 1) - 1;
    let mut runs = [RiceRun::default(); 4];
    for y in 0..height {
        for (component, p) in planes.iter_mut().take(components).enumerate() {
            let group = if component == 3 {
                2
            } else {
                usize::from(component != 0)
            };
            runs[component].line();
            for x in 0..width {
                let (xx, yy) = (x as isize, y as isize);
                let l = read(p, xx - 1, yy);
                let t = read(p, xx, yy - 1);
                let tl = read(p, xx - 1, yy - 1);
                let tr = read(p, xx + 1, yy - 1);
                let ll = read(p, xx - 2, yy);
                let tt = read(p, xx, yy - 2);
                let context: i32 = state.tables[group]
                    .iter()
                    .zip([l - tl, tl - t, t - tr, ll - l, tt - t])
                    .map(|(table, d)| table[(d & 255) as usize])
                    .sum();
                let mut residual = if let Some(rice) = rice.as_mut() {
                    rice.sample(
                        &mut state.rice.as_mut().unwrap()[group],
                        context,
                        state.depth + 1,
                        &mut runs[component],
                        x,
                        width,
                    )?
                } else {
                    let probabilities = state.models[group]
                        .get_mut(context.unsigned_abs() as usize)
                        .ok_or_else(|| invalid("invalid FFV1 RGB context"))?;
                    r.integer(probabilities, true)?
                };
                if residual.unsigned_abs() > bias as u32 {
                    return Err(invalid("FFV1 RGB residual exceeds sample depth"));
                }
                if context < 0 {
                    residual = -residual;
                }
                let prediction = (l + t - tl).clamp(l.min(t), l.max(t));
                p[y * width + x] = (prediction + residual) & mask;
            }
        }
    }
    let mut data = buffer(length)?;
    let mut alpha = if state.has_alpha {
        Some(buffer(alpha_length)?)
    } else {
        None
    };
    for i in 0..count {
        let cb = planes[1][i] - bias;
        let cr = planes[2][i] - bias;
        let base = planes[0][i] - ((cb + cr) >> 2);
        let rgb = if (9..=15).contains(&state.depth) && !state.has_alpha {
            [cr + base, cb + base, base]
        } else {
            [cr + base, base, cb + base]
        };
        for (channel, value) in rgb.into_iter().enumerate() {
            if !(0..bias).contains(&value) {
                return Err(invalid("FFV1 RGB reconstruction exceeds sample depth"));
            }
            let at = (i * 3 + channel) * bytes;
            data[at] = value as u8;
            if bytes == 2 {
                data[at + 1] = (value >> 8) as u8;
            }
        }
        if let Some(alpha) = alpha.as_mut() {
            let value = planes[3][i];
            if !(0..bias).contains(&value) {
                return Err(invalid("FFV1 alpha reconstruction exceeds sample depth"));
            }
            let at = i * bytes;
            alpha[at] = value as u8;
            if bytes == 2 {
                alpha[at + 1] = (value >> 8) as u8;
            }
        }
    }
    Ok((
        GeometryFrame {
            width,
            height,
            subsampling: None,
            data,
        },
        alpha,
    ))
}
// RFC 9043 sections 3.8.2 and 3.8.1.1.1. No implicit payload padding.
#[derive(Clone, Copy)]
struct RiceModel {
    count: i32,
    sum: i32,
    drift: i32,
    bias: i32,
}
impl Default for RiceModel {
    fn default() -> Self {
        Self {
            count: 1,
            sum: 4,
            drift: 0,
            bias: 0,
        }
    }
}
#[derive(Clone, Copy, Default)]
struct RiceRun {
    index: usize,
    mode: u8,
    left: usize,
}
impl RiceRun {
    fn line(&mut self) {
        self.mode = 0;
        self.left = 0;
    }
}
const LOG2_RUN: [u8; 41] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13,
    14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
];
struct Rice<'a> {
    data: &'a [u8],
    at: usize,
}
impl Rice<'_> {
    fn bits(&mut self, count: u8) -> Result<u32> {
        let mut value = 0;
        for _ in 0..count {
            let byte = *self
                .data
                .get(self.at / 8)
                .ok_or_else(|| invalid("truncated FFV1 Golomb Rice payload"))?;
            value = (value << 1) | u32::from((byte >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Ok(value)
    }
    fn scalar(&mut self, model: &mut RiceModel, bits: u8) -> Result<i32> {
        let mut k = 0;
        let mut scale = model.count;
        while scale < model.sum {
            scale *= 2;
            k += 1;
        }
        let mut unsigned = None;
        for prefix in 0..12 {
            if self.bits(1)? != 0 {
                unsigned = Some((prefix << k) + self.bits(k)?);
                break;
            }
        }
        let unsigned = if let Some(value) = unsigned {
            value
        } else {
            let value = self.bits(bits)? + 11;
            if value < 12u32 << k {
                return Err(invalid("noncanonical FFV1 Golomb Rice escape"));
            }
            value
        };
        let mut v = if unsigned & 1 == 0 {
            (unsigned >> 1) as i32
        } else {
            -((unsigned >> 1) as i32) - 1
        };
        if 2 * model.drift < -model.count {
            v = -1 - v;
        }
        let mask = (1i32 << bits) - 1;
        let value = (v + model.bias) & mask;
        let result = if value & (1 << (bits - 1)) != 0 {
            value - (1 << bits)
        } else {
            value
        };
        model.sum += v.abs();
        model.drift += v;
        if model.count == 128 {
            model.count >>= 1;
            model.sum >>= 1;
            model.drift >>= 1;
        }
        model.count += 1;
        if model.drift <= -model.count {
            model.bias = (model.bias - 1).max(-128);
            model.drift = (model.drift + model.count).max(-model.count + 1);
        } else if model.drift > 0 {
            model.bias = (model.bias + 1).min(127);
            model.drift = (model.drift - model.count).min(0);
        }
        Ok(result)
    }
    fn sample(
        &mut self,
        models: &mut [RiceModel],
        context: i32,
        bits: u8,
        run: &mut RiceRun,
        x: usize,
        width: usize,
    ) -> Result<i32> {
        let model = models
            .get_mut(context.unsigned_abs() as usize)
            .ok_or_else(|| invalid("invalid FFV1 Rice context"))?;
        if run.mode == 0 && context == 0 {
            run.mode = 1;
        }
        if run.mode == 0 {
            return self.scalar(model, bits);
        }
        if run.left == 0 && run.mode == 1 {
            let log = *LOG2_RUN
                .get(run.index)
                .ok_or_else(|| invalid("invalid FFV1 run index"))?;
            if self.bits(1)? != 0 {
                run.left = 1usize << log;
                if run.left <= width - x {
                    run.index += 1;
                }
            } else {
                run.left = self.bits(log)? as usize;
                run.index = run.index.saturating_sub(1);
                run.mode = 2;
            }
        }
        if run.left != 0 {
            run.left -= 1;
            return Ok(0);
        }
        run.mode = 0;
        let value = self.scalar(model, bits)?;
        Ok(if value >= 0 { value + 1 } else { value })
    }
    fn finish(&mut self) -> Result<()> {
        while self.at % 8 != 0 {
            if self.bits(1)? != 0 {
                return Err(invalid("nonzero FFV1 Golomb Rice padding"));
            }
        }
        // Remaining full bytes are reserved in versions 0/1, not sample data.
        Ok(())
    }
}
/// One independently initialized FFV1 v3 quantizer/context set.
pub struct QuantizationSet {
    tables: [[i32; 256]; 5],
    initial: Vec<[u8; 32]>,
}
impl QuantizationSet {
    pub fn context_count(&self) -> usize {
        self.initial.len()
    }
    pub fn quantizers(&self) -> &[[i32; 256]; 5] {
        &self.tables
    }
    pub fn initial_states(&self) -> &[[u8; 32]] {
        &self.initial
    }
}
/// Validated configuration record, independent of per-frame adaptation history.
pub struct Configuration {
    micro_version: u32,
    coder: u8,
    rgb: bool,
    depth: u8,
    chroma: bool,
    alpha: bool,
    shifts: [u8; 2],
    grid: [u32; 2],
    sets: Vec<QuantizationSet>,
    transitions: [u8; 256],
    ec: bool,
    intra: bool,
    memory: usize,
}
fn ffv1_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in data {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = (crc << 1)
                ^ if crc & 0x8000_0000 != 0 {
                    0x04c1_1db7
                } else {
                    0
                };
        }
    }
    crc
}
impl Configuration {
    /// CRC is verified before any range-coded parameter is read or allocated.
    pub fn parse(data: &[u8], budget: usize) -> Result<Self> {
        if data.len() < 6 {
            return Err(invalid("truncated FFV1 configuration record"));
        }
        if ffv1_crc(data) != 0 {
            return Err(invalid("FFV1 configuration CRC mismatch"));
        }
        let mut r = Range::new(&data[..data.len() - 4])?;
        let mut h = [128; 32];
        if r.integer(&mut h, false)? != 3 {
            return Err(unsupported("FFV1 configuration requires version 3"));
        }
        let micro_version = r.integer(&mut h, false)? as u32;
        if micro_version < 4 {
            return Err(unsupported("experimental FFV1 v3 micro-version"));
        }
        let coder = r.integer(&mut h, false)?;
        if !(0..=2).contains(&coder) {
            return Err(unsupported("unsupported FFV1 entropy coder"));
        }
        let mut transitions = TRANSITION;
        if coder == 2 {
            for i in 1..256 {
                let value = i64::from(TRANSITION[i]) + i64::from(r.integer(&mut h, true)?);
                if !(0..=255).contains(&value) {
                    return Err(invalid("invalid FFV1 custom transition"));
                }
                transitions[i] = value as u8;
            }
        }
        let rgb = match r.integer(&mut h, false)? {
            0 => false,
            1 => true,
            _ => return Err(unsupported("unsupported FFV1 color space")),
        };
        let depth = match r.integer(&mut h, false)? {
            0 => 8,
            n @ 8..=16 => n as u8,
            _ => return Err(unsupported("unsupported FFV1 sample depth")),
        };
        let chroma = r.bit(&mut h[0])?;
        let sx = r.integer(&mut h, false)?;
        let sy = r.integer(&mut h, false)?;
        if sx > 4 || sy > 4 {
            return Err(invalid("invalid FFV1 chroma shift"));
        }
        let alpha = r.bit(&mut h[0])?;
        if rgb && (!chroma || sx != 0 || sy != 0) {
            return Err(invalid("invalid FFV1 RGB chroma configuration"));
        }
        let grid = [
            r.integer(&mut h, false)? as u32 + 1,
            r.integer(&mut h, false)? as u32 + 1,
        ];
        let count = r.integer(&mut h, false)? as usize;
        if !(1..=8).contains(&count) {
            return Err(invalid("invalid FFV1 quantizer set count"));
        }
        let mut memory = std::mem::size_of::<Self>()
            .checked_add(count * std::mem::size_of::<QuantizationSet>())
            .ok_or_else(|| invalid("FFV1 configuration storage overflow"))?;
        if memory > budget {
            return Err(invalid("FFV1 configuration exceeds memory budget"));
        }
        let mut sets = Vec::new();
        sets.try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate FFV1 quantizer sets"))?;
        for _ in 0..count {
            let mut tables = [[0; 256]; 5];
            let mut scale = 1i32;
            for table in &mut tables {
                let mut model = [128; 32];
                let mut at = 0;
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
                    .filter(|&n| n <= 65535)
                    .ok_or_else(|| invalid("FFV1 context count overflow"))?;
            }
            let contexts = (scale as usize).div_ceil(2);
            memory = memory
                .checked_add(contexts * 32)
                .ok_or_else(|| invalid("FFV1 configuration storage overflow"))?;
            if memory > budget {
                return Err(invalid("FFV1 configuration exceeds memory budget"));
            }
            let mut initial = Vec::new();
            initial
                .try_reserve_exact(contexts)
                .map_err(|_| invalid("cannot allocate FFV1 initial states"))?;
            initial.resize(contexts, [128; 32]);
            sets.push(QuantizationSet { tables, initial });
        }
        // Each probability position has an independent scalar coding context.
        let mut states = [[128; 32]; 32];
        for set in &mut sets {
            if r.bit(&mut h[0])? {
                for j in 0..set.initial.len() {
                    for k in 0..32 {
                        let predictor = if j == 0 { 128 } else { set.initial[j - 1][k] };
                        let delta = r.integer(&mut states[k], true)?;
                        set.initial[j][k] = ((i64::from(predictor) + i64::from(delta)) & 255) as u8;
                    }
                }
            }
        }
        let ec = match r.integer(&mut h, false)? {
            0 => false,
            1 => true,
            _ => return Err(unsupported("unsupported FFV1 error correction mode")),
        };
        let intra = match r.integer(&mut h, false)? {
            0 => false,
            1 => true,
            _ => return Err(unsupported("unsupported FFV1 intra constraint")),
        };
        Ok(Self {
            micro_version,
            coder: coder as u8,
            rgb,
            depth,
            chroma,
            alpha,
            shifts: [sx as u8, sy as u8],
            grid,
            sets,
            transitions,
            ec,
            intra,
            memory,
        })
    }
    pub fn micro_version(&self) -> u32 {
        self.micro_version
    }
    pub fn coder_type(&self) -> u8 {
        self.coder
    }
    pub fn rgb(&self) -> bool {
        self.rgb
    }
    pub fn depth(&self) -> u8 {
        self.depth
    }
    pub fn chroma(&self) -> bool {
        self.chroma
    }
    pub fn alpha(&self) -> bool {
        self.alpha
    }
    pub fn chroma_shifts(&self) -> [u8; 2] {
        self.shifts
    }
    pub fn slice_grid(&self) -> [u32; 2] {
        self.grid
    }
    pub fn quantization_sets(&self) -> &[QuantizationSet] {
        &self.sets
    }
    pub fn transitions(&self) -> &[u8; 256] {
        &self.transitions
    }
    pub fn slice_crc(&self) -> bool {
        self.ec
    }
    pub fn intra(&self) -> bool {
        self.intra
    }
    pub fn memory_bytes(&self) -> usize {
        self.memory
    }
    /// Index slices backwards from their footers, verifying CRC and error status.
    /// No sample data is read or published by this framing operation.
    pub fn slices<'a>(&self, packet: &'a [u8], budget: usize) -> Result<Vec<Slice<'a>>> {
        let footer = if self.ec { 8 } else { 3 };
        let max = u64::from(self.grid[0]) * u64::from(self.grid[1]);
        let start_of = |end: usize| -> Result<usize> {
            if end < footer {
                return Err(invalid("truncated FFV1 slice footer"));
            }
            let at = end - footer;
            let length = usize::from(packet[at]) << 16
                | usize::from(packet[at + 1]) << 8
                | usize::from(packet[at + 2]);
            if length < 2 || length > at {
                return Err(invalid("invalid FFV1 slice size"));
            }
            Ok(at - length)
        };
        let mut count = 0usize;
        let mut end = packet.len();
        while end != 0 {
            let start = start_of(end)?;
            let size_at = end - footer;
            if self.ec {
                if ffv1_crc(&packet[start..end]) != 0 {
                    return Err(invalid("FFV1 slice CRC mismatch"));
                }
                if packet[size_at + 3] != 0 {
                    return Err(invalid("FFV1 slice reports damaged content"));
                }
            }
            if count as u64 >= max {
                return Err(invalid("FFV1 slice count exceeds raster"));
            }
            count += 1;
            let bytes = count
                .checked_mul(std::mem::size_of::<Slice<'_>>())
                .and_then(|n| n.checked_add(self.memory))
                .ok_or_else(|| invalid("FFV1 slice index storage overflow"))?;
            if bytes > budget {
                return Err(invalid("FFV1 slice index exceeds memory budget"));
            }
            end = start;
        }
        if count == 0 {
            return Err(invalid("FFV1 frame contains no slices"));
        }
        let mut slices = Vec::new();
        slices
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate FFV1 slice index"))?;
        end = packet.len();
        while end != 0 {
            let start = start_of(end)?;
            slices.push(Slice {
                payload: &packet[start..end - footer],
                offset: start,
            });
            end = start;
        }
        slices.reverse();
        Ok(slices)
    }
}
pub struct Slice<'a> {
    pub payload: &'a [u8],
    pub offset: usize,
}
struct SliceHistory {
    region: [usize; 4],
    quantizers: [usize; 3],
    state: State,
}
struct V3 {
    config: Configuration,
    history: Vec<Option<Box<SliceHistory>>>,
    live: bool,
}
impl Decoder {
    pub fn from_configuration(
        width: usize,
        height: usize,
        private: &[u8],
        budget: usize,
    ) -> Result<Self> {
        let mut decoder = Self::new(width, height, budget)?;
        if private.is_empty() {
            return Ok(decoder);
        }
        let config = Configuration::parse(private, budget)?;
        if config
            .memory
            .checked_add(std::mem::size_of::<V3>() - std::mem::size_of::<Configuration>())
            .is_none_or(|n| n > budget)
        {
            return Err(invalid("FFV1 configuration exceeds memory budget"));
        }
        if config.grid[0] as usize > width || config.grid[1] as usize > height {
            return Err(invalid("FFV1 slice raster exceeds frame dimensions"));
        }
        decoder.v3 = Some(Box::new(V3 {
            config,
            history: Vec::new(),
            live: false,
        }));
        Ok(decoder)
    }
}
impl Configuration {
    fn change_quantizers(
        &self,
        state: &mut State,
        previous: [usize; 3],
        selected: [usize; 3],
        allowance: usize,
    ) -> Result<()> {
        if previous == selected {
            return Ok(());
        }
        let groups = if self.alpha { 3 } else { 2 };
        let mut sizes = [0usize; 3];
        let mut grow = [false; 3];
        for i in 0..groups {
            let before = self.sets[previous[i]].context_count();
            let after = self.sets[selected[i]].context_count();
            grow[i] = after > before;
            sizes[i] = if grow[i] {
                after
            } else if let Some(rice) = state.rice.as_ref() {
                rice[i].len()
            } else {
                state.models[i].len()
            };
        }
        let memory = std::mem::size_of::<State>()
            + state.tables.len() * std::mem::size_of::<[[i32; 256]; 5]>()
            + sizes.iter().sum::<usize>()
                * if self.coder == 0 {
                    std::mem::size_of::<RiceModel>()
                } else {
                    32
                };
        if memory > allowance {
            return Err(invalid(
                "FFV1 changed quantizer contexts exceed memory budget",
            ));
        }
        for i in 0..groups {
            if grow[i] {
                // An expanded bank is a new bank, not a prefix copied from the
                // old adaptation. Shrinking/equal-sized banks retain history.
                if let Some(rice) = state.rice.as_mut() {
                    rice[i] = Vec::new();
                    rice[i]
                        .try_reserve_exact(sizes[i])
                        .map_err(|_| invalid("cannot allocate FFV1 changed Rice contexts"))?;
                    rice[i].resize(sizes[i], RiceModel::default());
                } else {
                    state.models[i] = Vec::new();
                    state.models[i]
                        .try_reserve_exact(sizes[i])
                        .map_err(|_| invalid("cannot allocate FFV1 changed range contexts"))?;
                    state.models[i].extend_from_slice(&self.sets[selected[i]].initial);
                }
            }
            state.tables[i] = self.sets[selected[i]].tables;
        }
        Ok(())
    }
    fn state_for(&self, selected: [usize; 3], allowance: usize) -> Result<State> {
        let groups = if self.alpha { 3 } else { 2 };
        let contexts = selected
            .iter()
            .take(groups)
            .map(|&i| self.sets[i].context_count())
            .sum::<usize>();
        let bytes = std::mem::size_of::<State>()
            + groups * std::mem::size_of::<[[i32; 256]; 5]>()
            + contexts
                * if self.coder == 0 {
                    std::mem::size_of::<RiceModel>()
                } else {
                    32
                };
        if bytes > allowance {
            return Err(invalid("FFV1 slice contexts exceed memory budget"));
        }
        let mut models = [Vec::new(), Vec::new(), Vec::new()];
        let mut rice = if self.coder == 0 {
            Some([Vec::new(), Vec::new(), Vec::new()])
        } else {
            None
        };
        let mut tables = Vec::new();
        tables
            .try_reserve_exact(groups)
            .map_err(|_| invalid("cannot allocate FFV1 slice quantizers"))?;
        for i in 0..groups {
            let set = &self.sets[selected[i]];
            tables.push(set.tables);
            if let Some(rice) = rice.as_mut() {
                rice[i]
                    .try_reserve_exact(set.context_count())
                    .map_err(|_| invalid("cannot allocate FFV1 slice Rice contexts"))?;
                rice[i].resize(set.context_count(), RiceModel::default());
            } else {
                models[i]
                    .try_reserve_exact(set.context_count())
                    .map_err(|_| invalid("cannot allocate FFV1 slice contexts"))?;
                models[i].extend_from_slice(&set.initial);
            }
        }
        Ok(State {
            rice,
            has_alpha: self.alpha,
            rgb: self.rgb,
            depth: self.depth,
            has_chroma: self.chroma,
            shifts: self.shifts,
            tables,
            transitions: self.transitions,
            models,
        })
    }
}
impl V3 {
    fn decode(
        &mut self,
        packet: &[u8],
        width: usize,
        height: usize,
        budget: usize,
    ) -> Result<Decoded> {
        let slices = self.config.slices(packet, budget)?;
        let mut first = Range::new(slices[0].payload)?;
        first.transitions = &self.config.transitions;
        let keyframe = first.bit(&mut 128)?;
        let mut first = Some(first);
        if self.config.intra && !keyframe {
            return Err(invalid("FFV1 intra stream contains a nonkeyframe"));
        }
        if !keyframe && !self.live {
            return Err(invalid("FFV1 frame requires a preceding keyframe"));
        }
        let factors = if self.config.chroma {
            self.config.shifts.map(|s| 1usize << s)
        } else {
            [1, 1]
        };
        let bytes = if self.config.depth == 8 { 1 } else { 2 };
        let chroma = width
            .div_ceil(factors[0])
            .checked_mul(height.div_ceil(factors[1]))
            .ok_or_else(|| invalid("FFV1 frame size overflow"))?;
        let samples = width
            .checked_mul(height)
            .and_then(|n| {
                if self.config.rgb {
                    n.checked_mul(3)
                } else {
                    chroma.checked_mul(2).and_then(|c| n.checked_add(c))
                }
            })
            .ok_or_else(|| invalid("FFV1 frame size overflow"))?;
        let length = samples
            .checked_mul(bytes)
            .ok_or_else(|| invalid("FFV1 frame size overflow"))?;
        let alpha_length = if self.config.alpha {
            width
                .checked_mul(height)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("FFV1 alpha size overflow"))?
        } else {
            0
        };
        let cells = (self.config.grid[0] as usize)
            .checked_mul(self.config.grid[1] as usize)
            .ok_or_else(|| invalid("FFV1 raster size overflow"))?;
        let base = cells
            .checked_mul(2 * std::mem::size_of::<Option<Box<SliceHistory>>>() + 1)
            .and_then(|n| n.checked_add(length))
            .and_then(|n| n.checked_add(alpha_length))
            .and_then(|n| n.checked_add(self.config.memory))
            .and_then(|n| {
                n.checked_add(std::mem::size_of::<V3>() - std::mem::size_of::<Configuration>())
            })
            .and_then(|n| n.checked_add(slices.len() * std::mem::size_of::<Slice<'_>>()))
            .ok_or_else(|| invalid("FFV1 v3 storage overflow"))?;
        if base > budget {
            return Err(invalid("FFV1 v3 frame exceeds memory budget"));
        }
        let mut old = std::mem::take(&mut self.history);
        if keyframe {
            old.clear();
        }
        let overhead = std::mem::size_of::<SliceHistory>() - std::mem::size_of::<State>();
        let mut model_bytes = old
            .iter()
            .flatten()
            .try_fold(0usize, |sum, h| {
                sum.checked_add(h.state.context_bytes() + overhead)
            })
            .ok_or_else(|| invalid("FFV1 slice state storage overflow"))?;
        if model_bytes > budget - base {
            return Err(invalid("FFV1 slice contexts exceed memory budget"));
        }
        let mut history = Vec::new();
        history
            .try_reserve_exact(cells)
            .map_err(|_| invalid("cannot allocate FFV1 slice history"))?;
        history.resize_with(cells, || None);
        let mut covered = Vec::new();
        covered
            .try_reserve_exact(cells)
            .map_err(|_| invalid("cannot allocate FFV1 slice coverage"))?;
        covered.resize(cells, false);
        let mut output = GeometryFrame {
            width,
            height,
            subsampling: if self.config.rgb { None } else { Some(factors) },
            data: buffer(length)?,
        };
        let mut alpha = if self.config.alpha {
            Some(buffer(alpha_length)?)
        } else {
            None
        };
        for (index, slice) in slices.iter().enumerate() {
            let mut r = if index == 0 {
                // Keep the interval after the frame keyframe flag.
                first.take().unwrap()
            } else {
                let mut r = Range::new(slice.payload)?;
                r.transitions = &self.config.transitions;
                r
            };
            let mut h = [128; 32];
            let x = r.integer(&mut h, false)? as usize;
            let y = r.integer(&mut h, false)? as usize;
            let sw = r.integer(&mut h, false)? as usize + 1;
            let sh = r.integer(&mut h, false)? as usize + 1;
            let [gw, gh] = self.config.grid.map(|n| n as usize);
            if x.checked_add(sw).is_none_or(|n| n > gw) || y.checked_add(sh).is_none_or(|n| n > gh)
            {
                return Err(invalid("FFV1 slice region exceeds raster"));
            }
            let quantizers = [
                r.integer(&mut h, false)? as usize,
                r.integer(&mut h, false)? as usize,
                if self.config.alpha {
                    r.integer(&mut h, false)? as usize
                } else {
                    0
                },
            ];
            if quantizers.iter().any(|&i| i >= self.config.sets.len()) {
                return Err(invalid("invalid FFV1 slice quantizer index"));
            }
            if r.integer(&mut h, false)? > 3 {
                return Err(invalid("invalid FFV1 picture structure"));
            }
            let _sar = [r.integer(&mut h, false)?, r.integer(&mut h, false)?];
            for row in y..y + sh {
                for col in x..x + sw {
                    let slot = &mut covered[row * gw + col];
                    if *slot {
                        return Err(invalid("overlapping FFV1 slices"));
                    }
                    *slot = true;
                }
            }
            let region = [x, y, sw, sh];
            let slot = y * gw + x;
            let state = if keyframe {
                let allowance = budget
                    .checked_sub(base)
                    .and_then(|n| n.checked_sub(model_bytes))
                    .and_then(|n| n.checked_sub(overhead))
                    .ok_or_else(|| invalid("FFV1 slice contexts exceed memory budget"))?;
                let state = self.config.state_for(quantizers, allowance)?;
                model_bytes = model_bytes
                    .checked_add(state.context_bytes() + overhead)
                    .ok_or_else(|| invalid("FFV1 slice context storage overflow"))?;
                state
            } else {
                let previous = old
                    .get_mut(slot)
                    .and_then(Option::take)
                    .ok_or_else(|| invalid("FFV1 nonkeyframe changes slice layout"))?;
                if previous.region != region {
                    return Err(invalid("FFV1 nonkeyframe changes slice layout"));
                }
                let mut state = previous.state;
                let before = state.context_bytes();
                let other = model_bytes - before;
                let allowance = budget
                    .checked_sub(base)
                    .and_then(|n| n.checked_sub(other))
                    .ok_or_else(|| invalid("FFV1 slice contexts exceed memory budget"))?;
                self.config.change_quantizers(
                    &mut state,
                    previous.quantizers,
                    quantizers,
                    allowance,
                )?;
                model_bytes = other + state.context_bytes();
                state
            };
            let left = (x as u64 * width as u64 / gw as u64) as usize;
            let top = (y as u64 * height as u64 / gh as u64) as usize;
            let right = ((x + sw) as u64 * width as u64 / gw as u64) as usize;
            let bottom = ((y + sh) as u64 * height as u64 / gh as u64) as usize;
            let mut rice = if self.config.coder == 0 {
                r.bit(&mut 129)?;
                let at =
                    r.at.checked_sub(1)
                        .filter(|&at| at <= slice.payload.len())
                        .ok_or_else(|| invalid("truncated FFV1 Golomb Rice slice header"))?;
                Some(Rice {
                    data: &slice.payload[at..],
                    at: 0,
                })
            } else {
                None
            };
            let allowance = budget - base - (model_bytes - state.context_bytes());
            let (decoded, state) = decode_content(
                right - left,
                bottom - top,
                allowance,
                &mut r,
                &mut rice,
                state,
                keyframe,
            )?;
            merge_slice(&mut output, &decoded.frame, left, top, bytes)?;
            match (alpha.as_mut(), decoded.alpha.as_ref()) {
                (Some(target), Some(source)) => {
                    let sw = right - left;
                    let sh = bottom - top;
                    if source.len() != sw * sh * bytes {
                        return Err(invalid("invalid FFV1 slice alpha storage"));
                    }
                    for row in 0..sh {
                        let from = row * sw * bytes;
                        let to = ((top + row) * width + left) * bytes;
                        target[to..to + sw * bytes]
                            .copy_from_slice(&source[from..from + sw * bytes]);
                    }
                }
                (None, None) => {}
                _ => return Err(invalid("FFV1 slice alpha layout mismatch")),
            }

            history[slot] = Some(Box::new(SliceHistory {
                region,
                quantizers,
                state,
            }));
        }
        if covered.iter().any(|&v| !v) {
            return Err(invalid("FFV1 frame has missing slices"));
        }
        if old.iter().any(Option::is_some) {
            return Err(invalid("FFV1 nonkeyframe changes slice layout"));
        }
        self.history = history;
        self.live = true;
        Ok(Decoded {
            frame: output,
            alpha,
            depth: self.config.depth,
            keyframe,
        })
    }
}
fn merge_slice(
    output: &mut GeometryFrame,
    slice: &GeometryFrame,
    x: usize,
    y: usize,
    bytes: usize,
) -> Result<()> {
    if output.subsampling != slice.subsampling {
        return Err(invalid("FFV1 slice layout mismatch"));
    }
    let [sx, sy] = output.subsampling.unwrap_or([1, 1]);
    let mut source_offset = 0;
    let mut target_offset = 0;
    for component in 0..if output.subsampling.is_none() { 1 } else { 3 } {
        let (dx, dy, pixel) = if output.subsampling.is_none() {
            (1, 1, 3 * bytes)
        } else if component == 0 {
            (1, 1, bytes)
        } else {
            (sx, sy, bytes)
        };
        let (w, h) = (slice.width.div_ceil(dx), slice.height.div_ceil(dy));
        let (tw, th) = (output.width.div_ceil(dx), output.height.div_ceil(dy));
        let (ox, oy) = (x / dx, y / dy);
        if ox.checked_add(w).is_none_or(|n| n > tw) || oy.checked_add(h).is_none_or(|n| n > th) {
            return Err(invalid("FFV1 slice pixels exceed output frame"));
        }
        for row in 0..h {
            let from = source_offset + row * w * pixel;
            let to = target_offset + ((oy + row) * tw + ox) * pixel;
            output.data[to..to + w * pixel].copy_from_slice(&slice.data[from..from + w * pixel]);
        }
        source_offset += w * h * pixel;
        target_offset += tw * th * pixel;
    }
    Ok(())
}

#[cfg(test)]
mod gray_tests {
    use super::*;
    #[test]
    fn gray_fixtures_preserve_luma_and_expand_neutral_chroma() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 10, 16] {
            let mut decoder = Decoder::new(4, 3, 1 << 20).unwrap();
            for index in 0..2 {
                let packet =
                    std::fs::read(root.join(format!("ffv1-gray-{depth}-{index}.packet"))).unwrap();
                let samples =
                    std::fs::read(root.join(format!("ffv1-gray-{depth}-{index}.gray"))).unwrap();
                let decoded = decoder.decode(&packet).unwrap();
                assert_eq!(decoded.depth, depth);
                assert!(decoded.keyframe);
                assert_eq!(decoded.frame.subsampling, Some([1, 1]));
                assert_eq!(&decoded.frame.data[..samples.len()], samples);
                let bytes = if depth == 8 { 1 } else { 2 };
                let neutral = (1u16 << (depth - 1)).to_le_bytes();
                assert_eq!(decoded.frame.data.len(), samples.len() * 3);
                for pixel in decoded.frame.data[samples.len()..].chunks_exact(bytes) {
                    assert_eq!(pixel, &neutral[..bytes]);
                }
                assert_eq!(
                    crate::owned_ffv1_encoder::encode_gray(4, 3, &samples, depth).unwrap(),
                    packet
                );
                assert!(Decoder::new(4, 3, 1).unwrap().decode(&packet).is_err());
            }
        }
    }
    #[test]
    fn gray_encoding_validates_storage_depth_and_geometry() {
        use crate::owned_ffv1_encoder::encode_gray;
        assert!(encode_gray(0, 3, &[], 8).is_err());
        assert!(encode_gray(4, 3, &[0; 11], 8).is_err());
        assert!(encode_gray(4, 3, &[0; 12], 7).is_err());
        assert!(encode_gray(usize::MAX, 2, &[], 8).is_err());
        assert!(encode_gray(1, 1, &[0xff, 0xff], 10).is_err());
    }
}
