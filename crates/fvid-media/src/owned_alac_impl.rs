/// Bytes of setup data that carry the fields this decoder reads.
const COOKIE_FIELDS: usize = 24;
/// Offsets of those fields, counted from the frame length the cookie starts with.
/// The compatible version between frame length and depth must be zero.
const FRAME_LENGTH: usize = 0;
const COMPATIBLE_VERSION: usize = 4;
const SAMPLE_SIZE: usize = 5;
const HISTORY_MULT: usize = 6;
const INITIAL_HISTORY: usize = 7;
const RICE_LIMIT: usize = 8;
const COOKIE_CHANNELS: usize = 9;

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

// Older opaque cookies wrap the same fields in optional frma/alac atoms.
// Extract by checked headers; never search arbitrary bytes for codec markers.
fn cookie_fields(mut data: &[u8]) -> Result<&[u8]> {
    let format = data.get(4..8) == Some(b"frma");
    if format {
        if data.get(..4) != Some(&12u32.to_be_bytes()) || data.get(8..12) != Some(b"alac") {
            return Err(invalid("invalid ALAC legacy format atom"));
        }
        data = &data[12..];
    }
    if data.get(4..8) != Some(b"alac") {
        if format { return Err(invalid("invalid ALAC legacy configuration atom")); }
        return Ok(data);
    }
    let size = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
    if !matches!(size, 36 | 60) {
        return Err(invalid("invalid ALAC legacy configuration atom"));
    }
    if data.len() < size {
        return Err(invalid("truncated ALAC legacy configuration atom"));
    }
    if data[8..12] != [0; 4] {
        return Err(invalid("invalid ALAC legacy configuration atom"));
    }
    data = &data[12..];
    if matches!(data.len(), 32 | 56) {
        if data[data.len()-8..] != [0,0,0,8,0,0,0,0] {
            return Err(invalid("invalid ALAC legacy terminator"));
        }
        data = &data[..data.len()-8];
    }
    Ok(data)
}

impl Params {
    /// Read the fields, and refuse a cookie that states a geometry no frame of this
    /// coding can hold.
    fn read(cookie: &[u8]) -> Result<Self> {
        let cookie = cookie_fields(cookie)?;
        if cookie.len() < COOKIE_FIELDS {
            return Err(invalid(
                "ALAC setup data is shorter than the cookie's fields",
            ));
        }
        if cookie[COMPATIBLE_VERSION] != 0 {
            return Err(invalid(&format!("unsupported ALAC compatible version {}", cookie[COMPATIBLE_VERSION])));
        }
        let frame_length =
            u32::from_be_bytes(cookie[FRAME_LENGTH..4].try_into().unwrap_or([0; 4])) as usize;
        if frame_length == 0 {
            return Err(invalid("ALAC cookie states a frame length of zero"));
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

// Apple ALAC cookie channel-layout tags describe source element order.
// No `chan` block means discrete channels above stereo, never guessed surround.
fn channel_order(configuration:&[u8],channels:u16)->Result<(u32,[usize;8])>{
    let configuration = cookie_fields(configuration)?;
    if !(1..=8).contains(&channels){return Err(invalid("ALAC supports 1..=8 channels"));}
    let mut order=[0;8];for (i,v)in order.iter_mut().enumerate(){*v=i;}
    if configuration.len()==24{return Ok((match channels{1=>4,2=>3,_=>0},order));}
    if configuration.len()!=48{return Err(invalid("invalid ALAC channel layout size"));}
    let layout=configuration.get(24..48).ok_or_else(||invalid("truncated ALAC channel layout"))?;
    if u32::from_be_bytes(layout[..4].try_into().unwrap())!=24 || &layout[4..8]!=b"chan" || layout[8..12]!=[0;4] || layout[16..24]!=[0;8]{return Err(invalid("invalid ALAC channel layout block"));}
    let tag=u32::from_be_bytes(layout[12..16].try_into().unwrap());
    if tag&65535!=u32::from(channels){return Err(invalid("ALAC channel layout count disagrees with cookie"));}
    let (mask,map): (u32,&[usize])=match (tag>>16,channels){
        (100,1)=>(4,&[0]),(101,2)=>(3,&[0,1]),
        (113,3)=>(7,&[1,2,0]),(116,4)=>(0x107,&[1,2,0,3]),
        (120,5)=>(0x37,&[1,2,0,3,4]),(124,6)=>(0x3f,&[1,2,0,5,3,4]),
        (142,7)=>(0x13f,&[1,2,0,6,3,4,5]),(127,8)=>(0xff,&[3,4,0,7,5,6,1,2]),
        _=>return Err(invalid("unsupported ALAC channel layout tag")),
    };
    order[..map.len()].copy_from_slice(map);Ok((mask,order))
}

struct ElementHeader {
    extra_bits:u32,
    bps:u32,
    compressed:bool,
    samples:usize,
}
fn element_header(bits:&mut Bits,params:&Params,width:usize)->Result<ElementHeader> {
        // The element's own instance number and the bits the format leaves unused.
        bits.skip(4);
        if bits.take(12) != 0 {
            return Err(invalid("ALAC element has nonzero reserved header bits"));
        }
        let declared = bits.bit();
        let shifted_bytes = bits.take(2);
        if shifted_bytes == 3 {
            return Err(invalid("ALAC element uses reserved shifted-byte code 3"));
        }
        let extra_bits = shifted_bytes * 8;
        // A pair codes the mid channel, which can be a bit wider than either of the
        // two samples it stands for.
        let compressed = !bits.bit();
        let bps = if compressed {params.sample_size as i32 - extra_bits as i32 + width as i32 - 1}else{params.sample_size as i32};
        if !(1..=32).contains(&bps) {
            return Err(invalid(&format!(
                "ALAC element codes residuals of {bps} bits, outside 1..=32"
            )));
        }
        let bps = bps as u32;
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

    if bits.past_end(){return Err(invalid("ALAC frame reads past its own bytes"));}
    Ok(ElementHeader{extra_bits,bps,compressed,samples})
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

fn resize_samples<T: Default + Clone>(plane: &mut Vec<T>, count: usize) -> Result<()> {
    if count > plane.len() {
        plane.try_reserve_exact(count - plane.len())
            .map_err(|e| invalid(&format!("ALAC sample allocation failed: {e}")))?;
    }
    plane.resize(count, T::default());
    Ok(())
}

// Reconstruction stays integer; only the final output representation differs.
trait AlacSample: Copy + Default {
    fn from_aligned_i32(sample: i32) -> Self;
}
impl AlacSample for i32 {
    fn from_aligned_i32(sample: i32) -> Self { sample }
}
impl AlacSample for f32 {
    fn from_aligned_i32(sample: i32) -> Self { sample as f32 * FULL_SCALE }
}

/// ALAC decoder: one packet in, one frame of interleaved PCM out.
pub struct AlacDecoder {
    params: Params,
    sample_rate: u32,
    channels: usize,
    order:[usize;8],
    speaker_mask:u32,
    channel: Vec<Channel>,
}

// Ancillary elements carry no audio and may precede or separate channel elements.
fn skip_ancillary(bits: &mut Bits, tag: u32) -> Result<bool> {
    let bytes = match tag {
        6 => {
            let count = bits.take(4);
            if count == 15 { 14 + bits.take(8) } else { count }
        }
        4 => {
            bits.skip(4); // element instance
            let aligned = bits.bit();
            let mut count = bits.take(8);
            if count == 255 { count += bits.take(8); }
            if aligned { bits.skip(((8 - bits.at % 8) % 8) as u32); }
            count
        }
        _ => return Ok(false),
    };
    if bits.past_end() || bytes as usize > bits.left() / 8 {
        return Err(invalid("ALAC ancillary element exceeds packet"));
    }
    bits.skip(bytes * 8);
    Ok(true)
}

impl AlacDecoder {
    pub fn channel_mask(&self)->u32{self.speaker_mask}
    pub fn configuration_mask(configuration:&[u8],sample_rate:u32,channels:u16)->Result<u32>{
        Self::validated_params(configuration,sample_rate,channels)?;
        Ok(channel_order(configuration,channels)?.0)
    }

    /// Coded sample count from the first audio element header, without PCM
    /// reconstruction or scratch allocation. Full frame/body checks remain in decode.
    pub fn packet_samples(configuration:&[u8],sample_rate:u32,channels:u16,data:&[u8])->Result<usize>{
        let params=Self::validated_params(configuration,sample_rate,channels)?;
        let mut bits=Bits::new(data);
        let width = loop {
            if bits.left() < 3 { return Err(invalid("ALAC packet has no audio element")); }
            let tag = bits.take(3);
            if skip_ancillary(&mut bits, tag)? { continue; }
            break match tag { SINGLE_CHANNEL | LOW_FREQUENCY => 1, CHANNEL_PAIR => 2,
                other => return Err(invalid(&format!("ALAC syntax element {other} has no decoder here"))) };
        };
        if width>usize::from(channels){return Err(invalid("ALAC element does not fit the track's channels"));}
        Ok(element_header(&mut bits,&params,width)?.samples)
    }

    pub fn decode_pcm(&mut self, data:&[u8]) -> Result<Vec<f32>> {
        let mut samples=Vec::new();
        self.frame(data,&mut samples)?;
        Ok(samples)
    }

    /// Lossless signed PCM, left-aligned in 32-bit words (S32 full scale).
    /// Source depths below 32 have zero padding in the low bits. Channel order
    /// is identical to decode_pcm; full-depth source words never pass through f32.
    pub fn decode_pcm_i32(&mut self, data: &[u8]) -> Result<Vec<i32>> {
        let mut samples = Vec::new();
        self.frame(data, &mut samples)?;
        Ok(samples)
    }

    /// Open for a stream of `channels` channels at this rate. `configuration` is
    /// the cookie's fields; a track that carries no setup data states no geometry
    /// and cannot be read.
    pub fn new(configuration: &[u8], sample_rate: u32, channels: u16) -> Result<Self> {
        let params = Self::validated_params(configuration, sample_rate, channels)?;
        let (speaker_mask,order)=channel_order(configuration,channels)?;
        Ok(Self {
            params,
            sample_rate,
            channels: usize::from(channels),
            order,speaker_mask,
            channel: (0..usize::from(channels))
                .map(|_| Channel::default())
                .collect(),
        })
    }

    fn validated_params(configuration: &[u8], sample_rate: u32, channels: u16) -> Result<Params> {
        let configuration = cookie_fields(configuration)?;
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
        let cookie_rate = u32::from_be_bytes(configuration[20..24].try_into().unwrap());
        if cookie_rate != sample_rate {
            return Err(invalid("ALAC cookie and track sample rates disagree"));
        }
        channel_order(configuration,channels)?;
        Ok(params)
    }
    /// Conservative retained decoder and interleaved PCM payload estimate.
    /// Includes three i32/u32 scratch planes and up to twice the output length
    /// for Vec growth. Excludes caller packet/I/O buffers and allocator headers.
    pub fn decode_admission_bytes(
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

    /// The frame's elements, read in the order the packet holds them and written
    /// into `out` at their track's channel positions.
    fn frame<T: AlacSample>(&mut self, data: &[u8], out: &mut Vec<T>) -> Result<()> {
        let mut bits = Bits::new(data);
        let params = self.params;
        let mut filled = 0usize;
        let mut count = 0usize;
        let mut ended = false;
        while bits.left() >= 3 {
            let element = bits.take(3);
            if skip_ancillary(&mut bits, element)? { continue; }
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
        if self.order[..self.channels].iter().enumerate().any(|(i,&source)|i!=source){
            for row in out.chunks_exact_mut(self.channels){let mut original=[T::default();8];original[..self.channels].copy_from_slice(row);for (i,v)in row.iter_mut().enumerate(){*v=original[self.order[i]];}}
        }
        Ok(())
    }

    /// One element: its header, its residuals, its prediction and, for a pair, the
    /// unmixing that turns mid and difference back into left and right. Returns the
    /// samples it holds.
    fn element<T: AlacSample>(
        &mut self,
        bits: &mut Bits,
        params: &Params,
        filled: usize,
        width: usize,
        out: &mut Vec<T>,
    ) -> Result<usize> {
        let ElementHeader{mut extra_bits,bps,compressed,samples}=element_header(bits,params,width)?;

        let mut shift = 0u32;
        let mut weight = 0i32;
        if compressed {
            if params.rice_limit == 0 {
                return Err(invalid("ALAC cookie sets a Rice limit of zero"));
            }
            shift = bits.take(8);
            // The matrix coefficient is signed8, unlike its unsigned shift.
            weight = bits.signed(8);
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
                // Order 31 selects first-order integration; it has no 31-sample warmup.
                if order[ch] != 31 && order[ch] >= params.frame_length {
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
            if extra_bits > 0 && samples > bits.left() / width / extra_bits as usize {
                return Err(invalid("ALAC frame is short of its extra bits"));
            }
            // Allocate for the actual packet, not the cookie's maximum frame length.
            for channel in &mut self.channel[filled..filled + width] {
                resize_samples(&mut channel.residual, samples)?;
                resize_samples(&mut channel.samples, samples)?;
                if extra_bits > 0 { resize_samples(&mut channel.low, samples)?; }
            }
            // Below the residual's width the frame codes the sample's low bits
            // plainly, one run per sample and channel, in sample order.
            if extra_bits > 0 {
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
            if samples > bits.left() / width / params.sample_size as usize {
                return Err(invalid("ALAC frame is short of its samples"));
            }
            for channel in &mut self.channel[filled..filled + width] {
                resize_samples(&mut channel.samples, samples)?;
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

        let output_samples = samples.checked_mul(self.channels)
            .ok_or_else(|| invalid("ALAC PCM size overflow"))?;
        resize_samples(out, output_samples)?;
        for index in 0..samples {
            for ch in 0..width {
                let sample = self.channel[filled + ch].samples[index] as u32;
                out[index * self.channels + filled + ch] =
                    T::from_aligned_i32(aligned(sample, params.sample_size));
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
        let coded = scalar(bits, k, bps);
        let value = coded.wrapping_add(modifier);
        modifier = 0;
        // The residual travels as a magnitude with its sign folded into the last
        // bit of it.
        out[i] = ((value >> 1) as i32) ^ -((value & 1) as i32);
        // The escape/clamp decision uses the coded magnitude before the
        // post-run modifier, while history accumulation uses the restored value.
        if coded > 0xffff {
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
            // Run codes retain their history-derived suffix width. The cookie
            // masks the divisor, not the number of suffix bits; zero history
            // has 32 leading zeros and therefore selects k=8.
            let k = history.leading_zeros() - 24 + ((history + 16) >> 6);
            let mask = if limit >= 32 { u32::MAX } else { (1u32 << limit) - 1 };
            let divisor = ((1u32 << k) - 1) & mask;
            let run = scalar_with_divisor(bits, k, 16, divisor);
            if run > 0 {
                if run as usize >= count - i {
                    return Err(invalid("ALAC zero run reaches past the end of its frame"));
                }
                out[i + 1..i + 1 + run as usize].fill(0);
                i += run as usize;
            }
            if run < 0xffff {
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
    scalar_with_divisor(bits, k, bps, (1u32 << k) - 1)
}

fn scalar_with_divisor(bits: &mut Bits, k: u32, bps: u32, divisor: u32) -> u32 {
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
    let value = run * divisor;
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
        // ALAC rounds in the same wrapping 32-bit accumulator as the products.
        // Widening before the bias changes the sign at the signed boundary.
        // Quantization zero means unity scaling and no rounding bias.
        let bias = if quant == 0 { 0 } else { 1u32 << (quant - 1) };
        let rounded = (sum.wrapping_add(bias) as i32) >> quant;
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

/// Invert the weighted matrix: R = U - floor(V * weight / 2^shift), L = R + V.
/// The coefficient is signed; zero instead denotes independent channel planes.
fn unmix(left: &mut Channel, right: &mut Channel, count: usize, shift: u32, weight: i32) {
    if weight == 0 {
        return;
    }
    for i in 0..count {
        let mid = left.samples[i] as u32;
        let difference = right.samples[i] as u32;
        let mixed = ((difference.wrapping_mul(weight as u32)) as i32) >> shift;
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
