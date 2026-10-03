/// Bytes of setup data that carry the fields this decoder reads.
const COOKIE_FIELDS: usize = 24;
/// Offsets of those fields, counted from the frame length the cookie starts with.
/// The one byte between the frame length and the sample depth is the compatible
/// version every writer this build has measured leaves zero; nothing else in the
/// header reads it, so it goes unread here too.
const FRAME_LENGTH: usize = 0;
const SAMPLE_SIZE: usize = 5;
const HISTORY_MULT: usize = 6;
const INITIAL_HISTORY: usize = 7;
const RICE_LIMIT: usize = 8;
const COOKIE_CHANNELS: usize = 9;

/// The frame length Apple's own headers give as the default and the only length
/// this decoder allocates for: both Apple's encoder and this build's reference one
/// write it, and a cookie claiming longer frames would cost memory per packet that
/// no sample in such a frame could fill.
const MAX_FRAME_LENGTH: usize = 4096;

/// The syntax elements this decoder reads. A frame is a run of them, and a single
/// channel is either a `SCE` or, for the low-frequency channel of a surround
/// layout, an `LFE` - the two are decoded the same way here.
const SINGLE_CHANNEL: u32 = 0;
const CHANNEL_PAIR: u32 = 1;
const LOW_FREQUENCY: u32 = 3;
const FRAME_END: u32 = 7;

/// The one prediction type besides plain linear prediction that has a spelling in
/// the bitstream: the reference decoder runs the predictor twice, once with a
/// fixed first-order pass over 31 coefficients.
const PREDICT_TWICE: u32 = 15;

/// A sample held to the top of a 32-bit word is a full scale of 2^31, whichever
/// depth the frame stored it at.
const FULL_SCALE: f32 = 1.0 / (1u64 << 31) as f32;

/// The cookie's fields, as far as the decoder of a frame needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Params {
    /// Samples one frame can hold, which bounds what an element may declare.
    frame_length: usize,
    /// Bits in a stored sample: 16, 20, 24 or 32.
    sample_size: u32,
    /// The cookie's Rice history multiplier, scaled by each channel's own.
    history_mult: u32,
    /// The Rice history a channel starts its element with.
    initial_history: u32,
    /// The largest Rice parameter a header may ask for.
    rice_limit: u32,
    /// The channel count the cookie states, which has to agree with the track's.
    channels: u32,
}

impl Params {
    /// Read the fields, and refuse a cookie that states a geometry no frame of this
    /// coding can hold.
    fn read(cookie: &[u8]) -> Result<Self> {
        if cookie.len() < COOKIE_FIELDS {
            return Err(invalid(
                "ALAC setup data is shorter than the cookie's fields",
            ));
        }
        let frame_length =
            u32::from_be_bytes(cookie[FRAME_LENGTH..4].try_into().unwrap_or([0; 4])) as usize;
        if frame_length == 0 || frame_length > MAX_FRAME_LENGTH {
            return Err(invalid(&format!(
                "ALAC cookie states a frame length of {frame_length}, outside the 1..={} samples this decoder holds",
                MAX_FRAME_LENGTH
            )));
        }
        let sample_size = u32::from(cookie[SAMPLE_SIZE]);
        if !matches!(sample_size, 16 | 20 | 24 | 32) {
            return Err(invalid(&format!("ALAC sample depth of {sample_size} bits")));
        }
        Ok(Self {
            frame_length,
            sample_size,
            history_mult: u32::from(cookie[HISTORY_MULT]),
            initial_history: u32::from(cookie[INITIAL_HISTORY]),
            rice_limit: u32::from(cookie[RICE_LIMIT]),
            channels: u32::from(cookie[COOKIE_CHANNELS]),
        })
    }
}

/// A channel's scratch for the frame being read: the residuals as they come off the
/// Rice stream, the samples those residuals predict, and the bits below the
/// residual's own width, which the frame codes plainly when it stores more than it
/// predicts.
#[derive(Default)]
struct Channel {
    residual: Vec<i32>,
    samples: Vec<i32>,
    low: Vec<u32>,
}

/// ALAC decoder: one packet in, one frame of interleaved f32 out.
pub struct AlacDecoder {
    params: Params,
    sample_rate: u32,
    channels: usize,
    channel: Vec<Channel>,
}

impl AlacDecoder {
    pub fn decode_pcm(&mut self, data:&[u8]) -> Result<Vec<f32>> {
        let mut samples=Vec::new();
        self.frame(data,&mut samples)?;
        Ok(samples)
    }

    /// Open for a stream of `channels` channels at this rate. `configuration` is
    /// the cookie's fields; a track that carries no setup data states no geometry
    /// and cannot be read.
    pub fn new(configuration: &[u8], sample_rate: u32, channels: u16) -> Result<Self> {
        let params = Self::validated_params(configuration, sample_rate, channels)?;
        let length = params.frame_length;
        Ok(Self {
            params,
            sample_rate,
            channels: usize::from(channels),
            channel: (0..usize::from(channels))
                .map(|_| Channel {
                    residual: vec![0; length],
                    samples: vec![0; length],
                    low: vec![0; length],
                })
                .collect(),
        })
    }

    fn validated_params(configuration: &[u8], sample_rate: u32, channels: u16) -> Result<Params> {
        let params = Params::read(configuration)?;
        if channels == 0 || u32::from(channels) != params.channels {
            return Err(invalid(&format!(
                "ALAC track of {} channels does not match its cookie's {}",
                channels, params.channels
            )));
        }
        if sample_rate == 0 {
            return Err(invalid("ALAC track states no sample rate"));
        }
        // Which output channel an element fills is settled by the layout for
        // anything past a pair: a 5.1 frame holds a pair, a single channel and
        // another pair in an order that is not the channel order, so placing the
        // elements one after another would hand the surround to the centre. The
        // layouts whose elements do travel in channel order are the ones a frame of
        // at most two channels can hold, and a wider track is refused by name.
        if channels > 2 {
            return Err(invalid(&format!(
                "ALAC track of {channels} channels has no element map here"
            )));
        }
        Ok(params)
    }
    /// Conservative retained decoder and interleaved PCM payload estimate.
    /// Includes three i32/u32 scratch planes and up to twice the output length
    /// for Vec growth. Excludes caller packet/I/O buffers and allocator headers.
    pub(crate) fn decode_admission_bytes(
        configuration: &[u8],
        sample_rate: u32,
        channels: u16,
    ) -> Result<usize> {
        let params = Self::validated_params(configuration, sample_rate, channels)?;
        params
            .frame_length
            .checked_mul(usize::from(channels))
            .and_then(|samples| samples.checked_mul(20))
            .and_then(|bytes| bytes.checked_add(usize::from(channels) * std::mem::size_of::<Channel>()))
            .and_then(|bytes| bytes.checked_add(16 * 1024))
            .ok_or_else(|| invalid("ALAC memory estimate overflow"))
    }

    /// Current audio specification (sample rate, channels).


    /// The frame's elements, read in the order the packet holds them and written
    /// into `out` at their track's channel positions.
    fn frame(&mut self, data: &[u8], out: &mut Vec<f32>) -> Result<()> {
        let mut bits = Bits::new(data);
        let params = self.params;
        let mut filled = 0usize;
        let mut count = 0usize;
        let mut ended = false;
        while bits.left() >= 3 {
            let element = bits.take(3);
            if element == FRAME_END {
                ended = true;
                break;
            }
            let width = match element {
                SINGLE_CHANNEL | LOW_FREQUENCY => 1,
                CHANNEL_PAIR => 2,
                other => {
                    return Err(invalid(&format!(
                        "ALAC syntax element {other} has no decoder here"
                    )));
                }
            };
            if filled + width > self.channels {
                return Err(invalid("ALAC element does not fit the track's channels"));
            }
            let samples = self.element(&mut bits, &params, filled, width, out)?;
            if count == 0 {
                count = samples;
            } else if count != samples {
                return Err(invalid("ALAC elements of one frame hold different counts"));
            }
            filled += width;
        }
        if !ended {
            return Err(invalid("ALAC frame has no end tag"));
        }
        if bits.past_end() {
            return Err(invalid("ALAC frame reads past its own bytes"));
        }
        if filled != self.channels || count == 0 {
            return Err(invalid("ALAC frame does not hold every channel"));
        }
        Ok(())
    }

    /// One element: its header, its residuals, its prediction and, for a pair, the
    /// unmixing that turns mid and difference back into left and right. Returns the
    /// samples it holds.
    fn element(
        &mut self,
        bits: &mut Bits,
        params: &Params,
        filled: usize,
        width: usize,
        out: &mut Vec<f32>,
    ) -> Result<usize> {
        // The element's own instance number and the bits the format leaves unused.
        bits.skip(4);
        bits.skip(12);
        let declared = bits.bit();
        let mut extra_bits = u32::from(bits.take(2)) * 8;
        // A pair codes the mid channel, which can be a bit wider than either of the
        // two samples it stands for.
        let bps = params.sample_size as i32 - extra_bits as i32 + width as i32 - 1;
        if !(1..=32).contains(&bps) {
            return Err(invalid(&format!(
                "ALAC element codes residuals of {bps} bits, outside 1..=32"
            )));
        }
        let bps = bps as u32;
        let compressed = !bits.bit();
        let samples = if declared {
            bits.take(32) as usize
        } else {
            params.frame_length
        };
        if samples == 0 || samples > params.frame_length {
            return Err(invalid(&format!(
                "ALAC element holds {samples} samples, outside its frame's 1..={}",
                params.frame_length
            )));
        }

        let mut shift = 0u32;
        let mut weight = 0u32;
        if compressed {
            if params.rice_limit == 0 {
                return Err(invalid("ALAC cookie sets a Rice limit of zero"));
            }
            shift = bits.take(8);
            weight = bits.take(8);
            if width == 2 && weight != 0 && shift > 31 {
                return Err(invalid(
                    "ALAC stereo unmix shifts a sample by more than it holds",
                ));
            }
            let mut code = [[0i16; 32]; 2];
            let mut order = [0usize; 2];
            let mut quant = [0u32; 2];
            let mut mult = [0u32; 2];
            let mut twice = [false; 2];
            for ch in 0..width {
                let prediction = bits.take(4);
                quant[ch] = bits.take(4);
                mult[ch] = bits.take(3);
                order[ch] = bits.take(5) as usize;
                if quant[ch] == 0 {
                    return Err(invalid("ALAC predictor shifts its coefficients by zero"));
                }
                if order[ch] >= params.frame_length {
                    return Err(invalid("ALAC predictor order reaches past the frame"));
                }
                if prediction != 0 && prediction != PREDICT_TWICE {
                    return Err(invalid(&format!(
                        "ALAC prediction type {prediction} has no decoder here"
                    )));
                }
                // The coefficients travel last-in-first-out.
                for index in (0..order[ch]).rev() {
                    code[ch][index] = bits.signed(16) as i16;
                }
                twice[ch] = prediction == PREDICT_TWICE;
            }
            // Below the residual's width the frame codes the sample's low bits
            // plainly, one run per sample and channel, in sample order.
            if extra_bits > 0 {
                if bits.left() < samples * width * extra_bits as usize {
                    return Err(invalid("ALAC frame is short of its extra bits"));
                }
                for index in 0..samples {
                    for ch in 0..width {
                        self.channel[filled + ch].low[index] = bits.take(extra_bits);
                    }
                }
            }
            for ch in 0..width {
                let history_mult = mult[ch] * params.history_mult / 4;
                let channel = &mut self.channel[filled + ch];
                rice(
                    bits,
                    &mut channel.residual[..samples],
                    bps,
                    history_mult,
                    params,
                )?;
                if twice[ch] {
                    // The type with no coefficients of its own: a plain first-order
                    // pass over the residuals first, then the frame's real filter.
                    first_order(&mut channel.residual[..samples], bps);
                }
                predict(
                    &channel.residual[..samples],
                    &mut channel.samples[..samples],
                    bps,
                    &mut code[ch],
                    order[ch],
                    quant[ch],
                );
            }
        } else {
            // Not compressed: the samples themselves, at their stored width. A frame
            // that holds them plainly holds no residual to shorten, so it carries no
            // low bits and no unmixing either, whatever its header said.
            if bits.left() < samples * width * params.sample_size as usize {
                return Err(invalid("ALAC frame is short of its samples"));
            }
            for index in 0..samples {
                for ch in 0..width {
                    self.channel[filled + ch].samples[index] = bits.signed(params.sample_size);
                }
            }
            extra_bits = 0;
        }

        if width == 2 {
            let (left, right) = self.channel.split_at_mut(filled + 1);
            unmix(&mut left[filled], &mut right[0], samples, shift, weight);
            if extra_bits > 0 {
                let (left, right) = self.channel.split_at_mut(filled + 1);
                merge_low(&mut left[filled], samples, extra_bits);
                merge_low(&mut right[0], samples, extra_bits);
            }
        } else if extra_bits > 0 {
            merge_low(&mut self.channel[filled], samples, extra_bits);
        }

        out.resize(samples * self.channels, 0.0);
        for index in 0..samples {
            for ch in 0..width {
                let sample = self.channel[filled + ch].samples[index] as u32;
                out[index * self.channels + filled + ch] =
                    aligned(sample, params.sample_size) as f32 * FULL_SCALE;
            }
        }
        Ok(samples)
    }
}



/// The residuals of one channel, Rice-coded with a history the stream itself
/// updates, and with runs of equal samples held back and coded once as a length.
fn rice(bits: &mut Bits, out: &mut [i32], bps: u32, mult: u32, params: &Params) -> Result<()> {
    let count = out.len();
    let mut history = params.initial_history;
    let mut modifier = 0u32;
    let mut i = 0usize;
    while i < count {
        if bits.left() == 0 {
            return Err(invalid("ALAC frame ends in the middle of its residuals"));
        }
        let limit = params.rice_limit;
        let k = log2((history >> 9) + 3).min(limit);
        let value = scalar(bits, k, bps).wrapping_add(modifier);
        modifier = 0;
        // The residual travels as a magnitude with its sign folded into the last
        // bit of it.
        out[i] = ((value >> 1) as i32) ^ -((value & 1) as i32);
        if value > 0xffff {
            history = 0xffff;
        } else {
            history = history.wrapping_add(
                value
                    .wrapping_mul(mult)
                    .wrapping_sub(history.wrapping_mul(mult) >> 9),
            );
        }
        // A history this small says the residuals have stopped varying, and the
        // frame says so with a run length instead of one value per sample.
        if history < 128 && i + 1 < count {
            let k = (7 - log2(history) + ((history + 16) >> 6)).min(limit);
            let run = scalar(bits, k, 16);
            if run > 0 {
                if run as usize >= count - i {
                    return Err(invalid("ALAC zero run reaches past the end of its frame"));
                }
                out[i + 1..i + 1 + run as usize].fill(0);
                i += run as usize;
            }
            if run <= 0xffff {
                modifier = 1;
            }
            history = 0;
        }
        i += 1;
    }
    Ok(())
}

/// One Rice value: a run of ones capped at nine, then `k` bits folded into it the
/// format's own way - or, past the cap, the value's raw `bps` bits.
fn scalar(bits: &mut Bits, k: u32, bps: u32) -> u32 {
    let run = bits.unary();
    if run > 8 {
        return bits.take(bps);
    }
    if k == 1 {
        return run;
    }
    // Peeking before the value is scaled is what decides whether the remainder is
    // part of it at all: a remainder of 0 or 1 means the bits were the terminator.
    let rest = bits.peek(k);
    let value = (run << k) - run;
    if rest > 1 {
        bits.skip(k);
        value + rest - 1
    } else {
        bits.skip(k - 1);
        value
    }
}

/// The predictor's own first pass, over 31 coefficients it does not read: each
/// sample is the one before it plus its residual.
fn first_order(samples: &mut [i32], bps: u32) {
    for i in 1..samples.len() {
        samples[i] = sign_extend(samples[i - 1].wrapping_add(samples[i]), bps);
    }
}

/// Turn residuals back into samples with an order-`order` filter whose coefficients
/// the frame just read, adapting them as the residuals come in.
fn predict(
    residual: &[i32],
    out: &mut [i32],
    bps: u32,
    coefficients: &mut [i16; 32],
    order: usize,
    quant: u32,
) {
    let count = out.len();
    if count == 0 {
        return;
    }
    // The first sample is never predicted.
    out[0] = residual[0];
    if count <= 1 {
        return;
    }
    if order == 0 {
        out[1..].copy_from_slice(&residual[1..count]);
        return;
    }
    if order == 31 {
        for i in 1..count {
            out[i] = sign_extend(out[i - 1].wrapping_add(residual[i]), bps);
        }
        return;
    }
    // Until the filter has a full window behind it, every sample only adds its
    // residual to the one before.
    let mut i = 1usize;
    while i <= order && i < count {
        out[i] = sign_extend(out[i - 1].wrapping_add(residual[i]), bps);
        i += 1;
    }
    while i < count {
        // The window is the `order` samples before this one, and every one of them
        // enters the filter only as its distance from the oldest, which is held
        // outside the window and added back once.
        let start = i - order - 1;
        let base = out[start] as u32;
        let mut sum = 0u32;
        for j in 0..order {
            sum = sum.wrapping_add(
                (out[start + j + 1] as u32)
                    .wrapping_sub(base)
                    .wrapping_mul(coefficients[j] as u32),
            );
        }
        // The coefficients travel scaled by 2^quant, and the rounding is the
        // arithmetic shift's own.
        let rounded = (((sum as i32) as i64 + (1i64 << (quant - 1))) >> quant) as i32;
        let value = (rounded as u32)
            .wrapping_add(base)
            .wrapping_add(residual[i] as u32) as i32;
        out[i] = sign_extend(value, bps);

        // A residual that survived the round trip unchanged says the coefficients
        // that produced it were right; one that shrank says which of them to nudge.
        let mut error = residual[i] as u32;
        let sign = sign_only(error as i32);
        if sign != 0 {
            let mut j = 0usize;
            while j < order && (error.wrapping_mul(sign as u32) as i32) > 0 {
                let mut step = base.wrapping_sub(out[start + j + 1] as u32) as i32;
                let direction = sign_only(step) * sign;
                coefficients[j] = coefficients[j].wrapping_sub(direction as i16);
                step = (step as u32).wrapping_mul(direction as u32) as i32;
                error = error.wrapping_sub(((step >> quant) as u32).wrapping_mul((j + 1) as u32));
                j += 1;
            }
        }
        i += 1;
    }
}

/// A pair holds the mid channel and the difference of the two: the left sample is
/// the difference with the weighted mid taken off, the right one both of them.
fn unmix(left: &mut Channel, right: &mut Channel, count: usize, shift: u32, weight: u32) {
    if weight == 0 {
        return;
    }
    for i in 0..count {
        let mid = left.samples[i] as u32;
        let difference = right.samples[i] as u32;
        let mixed = ((difference.wrapping_mul(weight)) as i32) >> shift;
        let a = mid.wrapping_sub(mixed as u32);
        let b = difference.wrapping_add(a);
        left.samples[i] = b as i32;
        right.samples[i] = a as i32;
    }
}

/// Put back the low bits the residuals were shortened by: a sample is its predicted
/// value with the plainly coded bits below it.
fn merge_low(channel: &mut Channel, count: usize, extra_bits: u32) {
    for i in 0..count {
        channel.samples[i] = ((channel.samples[i] as u32) << extra_bits | channel.low[i]) as i32;
    }
}

/// The stored sample set into the top of a 32-bit word, which is how the reference
/// holds every depth: 16-bit samples travel in a word of their own and wider ones
/// keep only the bits their depth has, so anything above the depth wraps away.
fn aligned(sample: u32, sample_size: u32) -> i32 {
    sample.wrapping_shl(32 - sample_size) as i32
}

/// Sign-extend the low `bits` of a word, which is how the format names the width a
/// residual or a sample is held to.
fn sign_extend(value: i32, bits: u32) -> i32 {
    (value << (32 - bits)) >> (32 - bits)
}

fn sign_only(value: i32) -> i32 {
    (value > 0) as i32 - (value < 0) as i32
}

/// The index of a word's highest set bit, and zero for zero, which is what the
/// reference's table returns.
fn log2(value: u32) -> u32 {
    if value == 0 {
        0
    } else {
        31 - value.leading_zeros()
    }
}

/// The format's own bit order: the most significant bit of each byte first, with no
/// alignment between samples. Reads past the end give zeros and leave the overrun
/// for the caller to see, so a short packet decodes the same every time and is still
/// refused.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    end: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            at: 0,
            end: data.len() * 8,
        }
    }

    /// Bits still in the packet, counted as none once the reader is past the end.
    fn left(&self) -> usize {
        self.end.saturating_sub(self.at)
    }

    fn past_end(&self) -> bool {
        self.at > self.end
    }

    fn take(&mut self, bits: u32) -> u32 {
        let value = self.read(bits, self.at);
        self.at += bits as usize;
        value
    }

    fn peek(&self, bits: u32) -> u32 {
        self.read(bits, self.at)
    }

    fn skip(&mut self, bits: u32) {
        self.at += bits as usize;
    }

    fn bit(&mut self) -> bool {
        self.take(1) != 0
    }

    /// A signed value of `bits` bits, up to the 32 a sample can be.
    fn signed(&mut self, bits: u32) -> i32 {
        sign_extend(self.take(bits) as i32, bits)
    }

    /// How many ones come before the next zero, counted at most nine times: the
    /// ninth is not a run length but the mark that a raw value follows.
    fn unary(&mut self) -> u32 {
        let mut run = 0;
        while run < 9 && self.bit() {
            run += 1;
        }
        run
    }

    fn read(&self, bits: u32, from: usize) -> u32 {
        let mut value = 0u32;
        let mut at = from;
        let mut left = bits;
        while left > 0 {
            let in_byte = 8 - at % 8;
            let grab = left.min(in_byte as u32) as usize;
            let byte = u32::from(self.data.get(at / 8).copied().unwrap_or(0));
            value = (value << grab) | ((byte >> (in_byte - grab)) & ((1 << grab) - 1));
            at += grab;
            left -= grab as u32;
        }
        value
    }
}
