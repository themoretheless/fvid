
struct RangeWriter {
    low: u32,
    range: u32,
    pending: Option<u8>,
    carry_bytes: usize,
    output: Vec<u8>,
}
impl RangeWriter {
    fn new() -> Self {
        Self {
            low: 0,
            range: 0xff00,
            pending: None,
            carry_bytes: 0,
            output: Vec::new(),
        }
    }
    fn emit(&mut self, byte: u8) -> Result<()> {
        self.output
            .try_reserve(1)
            .map_err(|_| invalid("cannot allocate FFV1 packet"))?;
        self.output.push(byte);
        Ok(())
    }
    fn normalize(&mut self) -> Result<()> {
        if self.low <= 0xff00 || self.low >= 0x10000 {
            let carry = u8::from(self.low >= 0x10000);
            if let Some(byte) = self.pending {
                self.emit(byte.wrapping_add(carry))?;
            }
            while self.carry_bytes != 0 {
                self.emit(if carry == 0 { 255 } else { 0 })?;
                self.carry_bytes -= 1;
            }
            self.pending = Some((self.low >> 8) as u8);
        } else {
            self.carry_bytes += 1;
        }
        self.low = (self.low & 255) << 8;
        self.range <<= 8;
        Ok(())
    }
    fn bit(&mut self, state: &mut u8, one: bool) -> Result<()> {
        let split = (self.range * u32::from(*state)) >> 8;
        if one {
            self.low += self.range - split;
            self.range = split;
            *state = TRANSITION[usize::from(*state)];
        } else {
            self.range -= split;
            *state = (256 - u16::from(TRANSITION[256 - usize::from(*state)])) as u8;
        }
        if self.range < 256 {
            self.normalize()?;
        }
        Ok(())
    }
    fn integer(&mut self, state: &mut [u8; 32], value: i32, signed: bool) -> Result<()> {
        self.bit(&mut state[0], value == 0)?;
        if value == 0 {
            return Ok(());
        }
        let magnitude = value.unsigned_abs();
        let exponent = (31 - magnitude.leading_zeros()) as usize;
        for i in 0..=exponent {
            self.bit(&mut state[1 + i.min(9)], i < exponent)?;
        }
        for i in (0..exponent).rev() {
            self.bit(&mut state[22 + i.min(9)], (magnitude >> i) & 1 != 0)?;
        }
        if signed {
            self.bit(&mut state[11 + exponent.min(10)], value < 0)?;
        }
        Ok(())
    }
    fn finish(mut self) -> Result<Vec<u8>> {
        // Closed termination: the container supplies the packet byte length.
        self.low += 255;
        self.range = 255;
        self.normalize()?;
        self.range = 255;
        self.normalize()?;
        Ok(self.output)
    }
}

/// Encode one self-contained keyframe. Input is packed planar Y,Cb,Cr; samples
/// above 8 bits use little-endian u16. RGB and alpha require separate profiles.
/// Every frame resets probability models, so packets can be independently sought.
pub fn encode(frame: &GeometryFrame, depth: u8) -> Result<Vec<u8>> {
    if !(8..=16).contains(&depth) || frame.width == 0 || frame.height == 0 {
        return Err(invalid("unsupported FFV1 depth or empty geometry"));
    }
    let [sx, sy] = frame
        .subsampling
        .ok_or_else(|| invalid("FFV1 RGB encoding is not implemented"))?;
    if !matches!(
        (sx, sy),
        (1, 1) | (2, 1) | (2, 2) | (1, 2) | (4, 1) | (4, 4)
    ) {
        return Err(invalid("unsupported FFV1 chroma subsampling"));
    }
    let bytes = if depth == 8 { 1 } else { 2 };
    let cw = frame.width.div_ceil(sx);
    let ch = frame.height.div_ceil(sy);
    let size = |w: usize, h: usize| {
        w.checked_mul(h)
            .and_then(|n| n.checked_mul(bytes))
            .ok_or_else(|| invalid("FFV1 plane size overflow"))
    };
    let luma = size(frame.width, frame.height)?;
    let chroma = size(cw, ch)?;
    let expected = chroma
        .checked_mul(2)
        .and_then(|n| luma.checked_add(n))
        .ok_or_else(|| invalid("FFV1 storage overflow"))?;
    let maximum = (1u32 << depth) - 1;
    if frame.data.len() != expected
        || (bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|b| u32::from(u16::from_le_bytes([b[0], b[1]])) > maximum))
    {
        return Err(invalid("invalid FFV1 sample storage"));
    }
    let mut coder = RangeWriter::new();
    coder.bit(&mut 128, true)?; // keyframe has its own model
    let mut header = [128; 32];
    for value in [1, 1, 0, i32::from(depth)] {
        coder.integer(&mut header, value, false)?;
    }
    coder.bit(&mut header[0], true)?;
    coder.integer(&mut header, sx.trailing_zeros() as i32, false)?;
    coder.integer(&mut header, sy.trailing_zeros() as i32, false)?;
    coder.bit(&mut header[0], false)?;
    // Five all-zero context quantizers, each represented by one run of 128.
    for _ in 0..5 {
        coder.integer(&mut [128; 32], 127, false)?;
    }
    let mut states = [[128; 32]; 2];
    let mut offset = 0;
    for plane in 0..3 {
        let (w, h, len) = if plane == 0 {
            (frame.width, frame.height, luma)
        } else {
            (cw, ch, chroma)
        };
        let samples = &frame.data[offset..offset + len];
        let read = |x: usize, y: usize| -> i32 {
            let at = (y * w + x) * bytes;
            let value = if bytes == 1 {
                u16::from(samples[at])
            } else {
                u16::from_le_bytes([samples[at], samples[at + 1]])
            };
            if depth == 16 {
                i32::from(value as i16)
            } else {
                i32::from(value)
            }
        };
        for y in 0..h {
            for x in 0..w {
                let top = if y == 0 { 0 } else { read(x, y - 1) };
                let left = if x > 0 { read(x - 1, y) } else { top };
                let diagonal = if y == 0 {
                    0
                } else if x > 0 {
                    read(x - 1, y - 1)
                } else if y > 1 {
                    read(0, y - 2)
                } else {
                    0
                };
                let prediction = (left + top - diagonal).clamp(left.min(top), left.max(top));
                let difference = read(x, y) - prediction;
                let midpoint = 1i32 << (depth - 1);
                let residual = ((difference + midpoint) & maximum as i32) - midpoint;
                coder.integer(&mut states[usize::from(plane != 0)], residual, true)?;
            }
        }
        offset += len;
    }
    coder.finish()
}

// Default probability transitions specified by RFC 9043 section 3.8.1.5.
pub(super) const TRANSITION: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37,
    37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 56, 57, 58, 59,
    60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 75, 76, 77, 78, 79, 80, 81, 82,
    83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103,
    104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 114, 115, 116, 117, 118, 119, 120, 121,
    122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 133, 134, 135, 136, 137, 138, 139,
    140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 152, 153, 154, 155, 156, 157,
    158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 171, 172, 173, 174, 175,
    176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 190, 191, 192, 194,
    194, 195, 196, 197, 198, 199, 200, 201, 202, 202, 204, 205, 206, 207, 208, 209, 209, 210, 211,
    212, 213, 215, 215, 216, 217, 218, 219, 220, 220, 222, 223, 224, 225, 226, 227, 227, 229, 229,
    230, 231, 232, 234, 234, 235, 236, 237, 238, 239, 240, 241, 242, 243, 244, 245, 246, 247, 248,
    248, 0, 0, 0, 0, 0, 0, 0,
];
