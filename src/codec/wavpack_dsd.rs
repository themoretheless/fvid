//! Owned DSD byte reconstruction: direct, static byte-context range coding,
//! and adaptive bitwise prediction. The PCM filter keeps history across blocks.
use super::*;
struct Input<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Input<'a> {
    fn byte(&mut self) -> Result<u8> {
        let v = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| invalid("WavPack DSD stream is truncated"))?;
        self.at += 1;
        Ok(v)
    }
    fn word(&mut self) -> Result<u32> {
        let mut v = 0;
        for _ in 0..4 {
            v = (v << 8) | u32::from(self.byte()?);
        }
        Ok(v)
    }
}
struct Range {
    low: u32,
    high: u32,
    value: u32,
}
impl Range {
    fn new(input: &mut Input<'_>) -> Result<Self> {
        Ok(Self {
            low: 0,
            high: u32::MAX,
            value: input.word()?,
        })
    }
    fn normalize(&mut self, input: &mut Input<'_>) -> Result<()> {
        while (self.low ^ self.high) & 0xff00_0000 == 0 {
            self.value = (self.value << 8) | u32::from(input.byte()?);
            self.low <<= 8;
            self.high = (self.high << 8) | 255;
        }
        Ok(())
    }
}
pub(super) fn rate(body: &[u8], base: u32) -> Result<u32> {
    if body.len() < 2 {
        return Err(invalid("WavPack DSD metadata is too short"));
    }
    if body[0] > 31 {
        return Err(invalid("WavPack DSD multiplier exceeds format range"));
    }
    base.checked_mul(1u32 << body[0])
        .filter(|&v| v != 0)
        .ok_or_else(|| invalid("WavPack DSD sample rate overflows timebase"))
}
pub(super) fn decode(block: &Block<'_>) -> Result<Vec<i32>> {
    let count = (block.block_samples as usize)
        .checked_mul(usize::from(block.coded_channels()))
        .ok_or_else(|| invalid("WavPack DSD sample count overflow"))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| invalid("cannot allocate WavPack DSD samples"))?;
    let body = block.dsd;
    let mut input = Input {
        bytes: body
            .get(2..)
            .ok_or_else(|| invalid("WavPack DSD metadata is too short"))?,
        at: 0,
    };
    match body[1] {
        0 => {
            if input.bytes.len() != count {
                return Err(invalid("WavPack raw DSD sample count mismatch"));
            }
            output.extend(input.bytes.iter().map(|&b| i32::from(b)));
        }
        1 => fast(&mut input, count, block.coded_channels() == 2, &mut output)?,
        3 => high(
            &mut input,
            block.block_samples as usize,
            block.coded_channels() == 2,
            &mut output,
        )?,
        _ => return Err(invalid("WavPack DSD coding mode is invalid")),
    }
    let crc = output.iter().fold(u32::MAX, |crc, &v| {
        crc.wrapping_mul(3).wrapping_add(v as u32)
    });
    if crc != block.crc {
        return Err(invalid("WavPack DSD checksum does not match samples"));
    }
    if block.flags & FALSE_STEREO != 0 {
        let mut expanded = Vec::new();
        expanded
            .try_reserve_exact(count * 2)
            .map_err(|_| invalid("cannot allocate WavPack DSD false stereo"))?;
        for v in output {
            expanded.extend_from_slice(&[v, v]);
        }
        return Ok(expanded);
    }
    Ok(output)
}
fn fast(input: &mut Input<'_>, count: usize, stereo: bool, out: &mut Vec<i32>) -> Result<()> {
    let history = input.byte()?;
    if history > 5 {
        return Err(invalid("WavPack DSD history exceeds format range"));
    }
    let bins = 1usize << history;
    let maximum = input.byte()?;
    let mut probabilities = Vec::new();
    probabilities
        .try_reserve_exact(bins * 256)
        .map_err(|_| invalid("cannot allocate WavPack DSD probabilities"))?;
    if maximum == 255 {
        for _ in 0..bins * 256 {
            probabilities.push(input.byte()?);
        }
    } else {
        while probabilities.len() < bins * 256 {
            let code = input.byte()?;
            if code == 0 {
                return Err(invalid("WavPack DSD probability table is incomplete"));
            }
            if code > maximum {
                let zeros = usize::from(code - maximum).min(bins * 256 - probabilities.len());
                probabilities.resize(probabilities.len() + zeros, 0);
            } else {
                probabilities.push(code);
            }
        }
        if input.byte()? != 0 {
            return Err(invalid("WavPack DSD probability terminator is invalid"));
        }
    }
    let mut cumulative = Vec::new();
    cumulative
        .try_reserve_exact(bins)
        .map_err(|_| invalid("cannot allocate WavPack DSD cumulative table"))?;
    let mut total = 0usize;
    for bin in probabilities.chunks_exact(256) {
        let mut sums = [0u32; 257];
        for (i, &p) in bin.iter().enumerate() {
            sums[i + 1] = sums[i] + u32::from(p);
        }
        total += sums[256] as usize;
        cumulative.push(sums);
    }
    if total > bins * 1280 {
        return Err(invalid(
            "WavPack DSD probability storage exceeds format bound",
        ));
    }
    let mut range = Range::new(input)?;
    let (mut p0, mut p1) = (0usize, 0usize);
    for _ in 0..count {
        let sums = &cumulative[p0];
        let total = sums[256];
        if total == 0 {
            return Err(invalid("WavPack DSD selected empty probability context"));
        }
        let mut step = range.high.wrapping_sub(range.low) / total;
        if step == 0 {
            range = Range::new(input)?;
            step = u32::MAX / total;
        }
        let position = range.value.wrapping_sub(range.low) / step;
        if position >= total {
            return Err(invalid("WavPack DSD range value exceeds probability table"));
        }
        let code = sums.partition_point(|&p| p <= position) - 1;
        range.low = range.low.wrapping_add(sums[code] * step);
        range.high = range
            .low
            .wrapping_add(u32::from(probabilities[p0 * 256 + code]) * step)
            .wrapping_sub(1);
        out.push(code as i32);
        if stereo {
            p0 = p1;
            p1 = code & (bins - 1);
        } else {
            p0 = code & (bins - 1);
        }
        range.normalize(input)?;
    }
    Ok(())
}
fn table(rate: u8) -> [i32; 256] {
    let mut out = [0; 256];
    let mut value = 0x808000i32;
    let mut rate = i32::from(rate) << 8;
    for _ in 0..((rate + 128) >> 8) {
        value += (0x10000 - value) >> 8;
    }
    for i in 0..128 {
        out[i] = value;
        out[255 - i] = 0x100ffff - value;
        if value > 0x10000 {
            rate += (rate * 20 + 128) >> 8;
            for _ in 0..((rate + 64) >> 7) {
                value += (0x10000 - value) >> 8;
            }
        }
    }
    out
}
#[derive(Clone, Copy)]
struct Predictor {
    filter: [i32; 6],
    factor: i32,
    value: i32,
    byte: u8,
}
impl Predictor {
    fn read(input: &mut Input<'_>) -> Result<Self> {
        let mut filter = [0; 6];
        for v in &mut filter[..5] {
            *v = i32::from(input.byte()?) << 12;
        }
        let a = input.byte()?;
        let b = input.byte()?;
        let factor = i32::from(i16::from_le_bytes([a, b]));
        let mut out = Self {
            filter,
            factor,
            value: 0,
            byte: 0,
        };
        out.predict();
        Ok(out)
    }
    fn predict(&mut self) {
        self.value = self.filter[0]
            .wrapping_sub(self.filter[4])
            .wrapping_add(self.filter[5].wrapping_mul(self.factor) >> 2);
    }
    fn update(&mut self, one: bool) {
        let bit = if one { -1 } else { 0 };
        self.byte = (self.byte << 1) | u8::from(one);
        self.value = self.value.wrapping_add(self.filter[5].wrapping_mul(8));
        let sign = ((self.value ^ bit) >> 31) | 1;
        let change = (self.value ^ self.value.wrapping_sub(self.filter[5].wrapping_mul(16))) >> 31;
        self.factor = self.factor.wrapping_add(sign & change);
        self.filter[0] =
            self.filter[0].wrapping_add(((bit & 0x100000).wrapping_sub(self.filter[0])) >> 6);
        self.filter[1] =
            self.filter[1].wrapping_add(((bit & 0x100000).wrapping_sub(self.filter[1])) >> 4);
        self.filter[2] =
            self.filter[2].wrapping_add(self.filter[1].wrapping_sub(self.filter[2]) >> 4);
        self.filter[3] =
            self.filter[3].wrapping_add(self.filter[2].wrapping_sub(self.filter[3]) >> 4);
        let delta = self.filter[3].wrapping_sub(self.filter[4]) >> 4;
        self.filter[4] = self.filter[4].wrapping_add(delta);
        self.filter[5] = self.filter[5].wrapping_add(delta.wrapping_sub(self.filter[5]) >> 3);
        self.predict();
    }
}
fn high(input: &mut Input<'_>, count: usize, stereo: bool, out: &mut Vec<i32>) -> Result<()> {
    let rate = input.byte()?;
    if input.byte()? != 20 {
        return Err(invalid("WavPack high DSD adaptation rate is invalid"));
    }
    let mut probabilities = table(rate);
    let mut predictors = vec![Predictor::read(input)?];
    if stereo {
        predictors.push(Predictor::read(input)?);
    }
    let mut range = Range::new(input)?;
    for _ in 0..count {
        for p in &mut predictors {
            p.predict();
        }
        for _ in 0..8 {
            for p in &mut predictors {
                let probability = &mut probabilities[((p.value >> 8) & 255) as usize];
                let split = range.low.wrapping_add(
                    (range.high.wrapping_sub(range.low) >> 8) * ((*probability >> 16) as u32),
                );
                let one = range.value <= split;
                if one {
                    range.high = split;
                    *probability += (0x10000fe - *probability) >> 8;
                } else {
                    range.low = split.wrapping_add(1);
                    *probability += (0x10000 - *probability) >> 8;
                }
                range.normalize(input)?;
                p.update(one);
            }
        }
        for p in &mut predictors {
            out.push(i32::from(p.byte));
            p.factor = p.factor.wrapping_sub(p.factor.wrapping_add(512) >> 10);
        }
    }
    Ok(())
}

// Symmetric 56-tap decimator, scaled to the signed 24-bit PCM domain.
const TAPS: [i32; 56] = [
    4, 17, 56, 147, 336, 692, 1315, 2337, 3926, 6281, 9631, 14216, 20275, 28021, 37619, 49155,
    62616, 77870, 94649, 112551, 131049, 149507, 167220, 183448, 197472, 208636, 216402, 220385,
    220385, 216402, 208636, 197472, 183448, 167220, 149507, 131049, 112551, 94649, 77870, 62616,
    49155, 37619, 28021, 20275, 14216, 9631, 6281, 3926, 2337, 1315, 692, 336, 147, 56, 17, 4,
];
pub(super) struct Filter {
    history: Vec<[u8; 7]>,
    initial: bool,
}
impl Filter {
    pub fn new(channels: u16) -> Result<Self> {
        let mut history = Vec::new();
        history
            .try_reserve_exact(usize::from(channels))
            .map_err(|_| invalid("cannot allocate DSD filter history"))?;
        history.resize(usize::from(channels), [0x55; 7]);
        Ok(Self {
            history,
            initial: true,
        })
    }
    pub fn duplicate(&self) -> Result<Self> {
        let mut next = Self::new(self.history.len() as u16)?;
        next.history.copy_from_slice(&self.history);
        next.initial = self.initial;
        Ok(next)
    }
    pub fn convert(&mut self, data: &[u8]) -> Result<Vec<f32>> {
        let sum: i64 = TAPS.iter().map(|&v| i64::from(v)).sum();
        let scale = f64::from((1 << 23) - 1) / sum as f64 * 16.0;
        let coefficients = TAPS.map(|v| (f64::from(v) * scale + 0.5).floor() as i32);
        let mut pcm = Vec::new();
        pcm.try_reserve_exact(data.len())
            .map_err(|_| invalid("cannot allocate DSD PCM"))?;
        let channels = self.history.len();
        for (i, &byte) in data.iter().enumerate() {
            let history = &mut self.history[i % channels];
            history.rotate_left(1);
            history[6] = byte;
            let mut value = 0i32;
            for (tap, &coefficient) in coefficients.iter().enumerate() {
                value += if history[tap / 8] & (0x80 >> (tap % 8)) != 0 {
                    coefficient
                } else {
                    -coefficient
                };
            }
            pcm.push(((value + 8) >> 4) as f32);
        }
        if self.initial {
            extrapolate(&mut pcm, self.history.len());
            self.initial = false;
        }
        for value in &mut pcm {
            *value *= 1.0 / (1u32 << 23) as f32;
        }
        Ok(pcm)
    }
}
fn extrapolate(samples: &mut [f32], channels: usize) {
    let frames = samples.len() / channels;
    if frames < 16 {
        return;
    }
    let maximum = 10.min((frames - 6) / 2);
    for c in 0..channels {
        let (mut left, mut right) = (0.0f32, 0.0f32);
        for period in 5..=maximum {
            let (mut first, mut second) = (0.0f32, 0.0f32);
            for i in 0..period {
                first += samples[(6 + i) * channels + c] / period as f32;
                second += samples[(6 + period + i) * channels + c] / period as f32;
            }
            let lratio = (6.0 + period as f32 / 2.0) / period as f32;
            let rratio = (period as f32 / 2.0) / period as f32;
            left += (first - second).mul_add(lratio, first);
            right += (first - second).mul_add(rratio, first);
        }
        left /= (maximum - 4) as f32;
        right /= (maximum - 4) as f32;
        let slope = (right - left) / 5.0;
        for i in 0..6 {
            samples[i * channels + c] =
                ((f64::from((i as f32).mul_add(slope, left)) + 0.5) as i32) as f32;
        }
    }
}

pub(super) fn bytes(frame: &frame::Frame<'_>, channels: u16) -> Result<Vec<u8>> {
    let first = &frame.blocks[0];
    if first.flags & DSD_DATA == 0 {
        return Err(invalid("WavPack frame is not DSD"));
    }
    let count = (first.block_samples as usize)
        .checked_mul(usize::from(channels))
        .ok_or_else(|| invalid("DSD output length overflow"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(count)
        .map_err(|_| invalid("cannot allocate interleaved DSD"))?;
    out.resize(count, 0);
    let mut offset = 0;
    for block in &frame.blocks {
        let samples = decode(block)?;
        for (row, values) in samples
            .chunks_exact(usize::from(block.channels))
            .enumerate()
        {
            for (c, &value) in values.iter().enumerate() {
                out[row * usize::from(channels) + offset + c] = value as u8;
            }
        }
        offset += usize::from(block.channels);
    }
    Ok(out)
}
