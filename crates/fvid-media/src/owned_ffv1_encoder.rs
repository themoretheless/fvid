//! Owned FFV1 version 1 range-coded, all-intra YCbCr/RGB encoder.
//! Bitstream semantics: RFC 9043. One slice and a single residual context
//! per luma/chroma model; quantization here selects contexts, not sample loss.
use crate::owned_frame::GeometryFrame;
pub use fvid_control::error::{Error, Result};
use fvid_control::error::{buffer, invalid};
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
        if self.output.len() == self.output.capacity() {
            let chunk = self.output.capacity().clamp(4096, 65536);
            if self.output.try_reserve(chunk).is_err() {
                self.output
                    .try_reserve(1)
                    .map_err(|_| invalid("cannot allocate FFV1 packet"))?;
            }
        }
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
/// above 8 bits use little-endian u16. Interleaved RGB selects the v1 RCT path;
/// alpha still requires a separate profile.
/// Every frame resets probability models, so packets can be independently sought.
pub fn encode(frame: &GeometryFrame, depth: u8) -> Result<Vec<u8>> {
    if !(8..=16).contains(&depth) || frame.width == 0 || frame.height == 0 {
        return Err(invalid("unsupported FFV1 depth or empty geometry"));
    }
    let Some(subsampling) = frame.subsampling else {
        return encode_rgb(frame, depth);
    };
    encode_planes(
        frame.width,
        frame.height,
        &frame.data,
        depth,
        subsampling,
        true,
        None,
    )
}

/// Encode a self-contained monochrome keyframe without inventing chroma samples.
/// Samples above eight bits are stored in little-endian u16 words.
pub fn encode_gray(width: usize, height: usize, samples: &[u8], depth: u8) -> Result<Vec<u8>> {
    encode_planes(width, height, samples, depth, [1, 1], false, None)
}
/// Encode color and an independent full-resolution alpha plane at the same depth.
pub fn encode_with_alpha(frame: &GeometryFrame, depth: u8, alpha: &[u8]) -> Result<Vec<u8>> {
    if let Some(subsampling) = frame.subsampling {
        encode_planes(
            frame.width,
            frame.height,
            &frame.data,
            depth,
            subsampling,
            true,
            Some(alpha),
        )
    } else {
        encode_rgb_with_alpha(frame, depth, alpha)
    }
}
pub fn encode_gray_with_alpha(
    width: usize,
    height: usize,
    samples: &[u8],
    alpha: &[u8],
    depth: u8,
) -> Result<Vec<u8>> {
    encode_planes(width, height, samples, depth, [1, 1], false, Some(alpha))
}
fn validate_alpha(alpha: &[u8], length: usize, depth: u8) -> Result<()> {
    if alpha.len() != length
        || (depth > 8
            && alpha
                .chunks_exact(2)
                .any(|s| u32::from(u16::from_le_bytes([s[0], s[1]])) >= 1u32 << depth))
    {
        return Err(invalid("invalid FFV1 alpha sample storage"));
    }
    Ok(())
}
fn encode_planes(
    width: usize,
    height: usize,
    data: &[u8],
    depth: u8,
    [sx, sy]: [usize; 2],
    has_chroma: bool,
    alpha: Option<&[u8]>,
) -> Result<Vec<u8>> {
    if !(8..=16).contains(&depth) || width == 0 || height == 0 {
        return Err(invalid("unsupported FFV1 depth or empty geometry"));
    }
    if !matches!(
        (sx, sy),
        (1, 1) | (2, 1) | (2, 2) | (1, 2) | (4, 1) | (4, 4)
    ) {
        return Err(invalid("unsupported FFV1 chroma subsampling"));
    }
    let bytes = if depth == 8 { 1 } else { 2 };
    let cw = width.div_ceil(sx);
    let ch = height.div_ceil(sy);
    let size = |w: usize, h: usize| {
        w.checked_mul(h)
            .and_then(|n| n.checked_mul(bytes))
            .ok_or_else(|| invalid("FFV1 plane size overflow"))
    };
    let luma = size(width, height)?;
    let chroma = size(cw, ch)?;
    let expected = if has_chroma {
        chroma
            .checked_mul(2)
            .and_then(|n| luma.checked_add(n))
            .ok_or_else(|| invalid("FFV1 storage overflow"))?
    } else {
        luma
    };
    let maximum = (1u32 << depth) - 1;
    if data.len() != expected
        || (bytes == 2
            && data
                .as_chunks::<2>()
                .0
                .iter()
                .any(|b| u32::from(u16::from_le_bytes([b[0], b[1]])) > maximum))
    {
        return Err(invalid("invalid FFV1 sample storage"));
    }
    if let Some(alpha) = alpha {
        validate_alpha(alpha, luma, depth)?;
    }
    let mut coder = RangeWriter::new();
    coder.bit(&mut 128, true)?; // keyframe has its own model
    let mut header = [128; 32];
    for value in [1, 1, 0, i32::from(depth)] {
        coder.integer(&mut header, value, false)?;
    }
    coder.bit(&mut header[0], has_chroma)?;
    coder.integer(&mut header, sx.trailing_zeros() as i32, false)?;
    coder.integer(&mut header, sy.trailing_zeros() as i32, false)?;
    coder.bit(&mut header[0], alpha.is_some())?;
    // Five all-zero context quantizers, each represented by one run of 128.
    for _ in 0..5 {
        coder.integer(&mut [128; 32], 127, false)?;
    }
    let mut states = [[128; 32]; 3];
    let mut offset = 0;
    for plane in (0..if has_chroma { 3 } else { 1 }).chain(alpha.is_some().then_some(3)) {
        let (w, h, len) = if plane == 0 || plane == 3 {
            (width, height, luma)
        } else {
            (cw, ch, chroma)
        };
        let samples = if plane == 3 {
            alpha.unwrap()
        } else {
            &data[offset..offset + len]
        };
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
                coder.integer(
                    &mut states[if plane == 3 {
                        2
                    } else {
                        usize::from(plane != 0)
                    }],
                    residual,
                    true,
                )?;
            }
        }
        if plane != 3 {
            offset += len;
        }
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

/// Interleaved RGB input, source precision unchanged. RFC 9043 3.7.2/3.7.2.1.
pub fn encode_rgb(frame: &GeometryFrame, depth: u8) -> Result<Vec<u8>> {
    encode_rgb_planes(frame, depth, None)
}
pub fn encode_rgb_with_alpha(frame: &GeometryFrame, depth: u8, alpha: &[u8]) -> Result<Vec<u8>> {
    encode_rgb_planes(frame, depth, Some(alpha))
}
fn encode_rgb_planes(frame: &GeometryFrame, depth: u8, alpha: Option<&[u8]>) -> Result<Vec<u8>> {
    if frame.subsampling.is_some()
        || !(8..=16).contains(&depth)
        || frame.width == 0
        || frame.height == 0
    {
        return Err(invalid("invalid FFV1 RGB geometry or depth"));
    }
    let count = frame
        .width
        .checked_mul(frame.height)
        .ok_or_else(|| invalid("FFV1 RGB size overflow"))?;
    let bytes = if depth == 8 { 1 } else { 2 };
    if count.checked_mul(3 * bytes) != Some(frame.data.len()) {
        return Err(invalid("invalid FFV1 RGB storage"));
    }
    if let Some(alpha) = alpha {
        validate_alpha(alpha, count * bytes, depth)?;
    }
    let components = if alpha.is_some() { 4 } else { 3 };
    let bias = 1i32 << depth;
    let mask = (bias << 1) - 1;
    let mut planes: [Vec<i32>; 4] = std::array::from_fn(|_| Vec::new());
    for plane in planes.iter_mut().take(components) {
        plane
            .try_reserve_exact(count)
            .map_err(|_| invalid("FFV1 RGB allocation failed"))?;
    }
    for pixel in frame.data.chunks_exact(3 * bytes) {
        let read = |i| {
            if bytes == 1 {
                i32::from(pixel[i])
            } else {
                i32::from(u16::from_le_bytes([pixel[i * 2], pixel[i * 2 + 1]]))
            }
        };
        let [r, g, b] = [read(0), read(1), read(2)];
        if [r, g, b].iter().any(|&v| v >= bias) {
            return Err(invalid("FFV1 RGB sample exceeds depth"));
        }
        let exception = (9..=15).contains(&depth) && alpha.is_none();
        let base = if exception { b } else { g };
        let cb = if exception { g - base } else { b - base };
        let cr = r - base;
        for (plane, value) in planes
            .iter_mut()
            .zip([base + ((cb + cr) >> 2), cb + bias, cr + bias])
        {
            plane.push(value);
        }
    }
    if let Some(alpha) = alpha {
        for sample in alpha.chunks_exact(bytes) {
            planes[3].push(if bytes == 1 {
                i32::from(sample[0])
            } else {
                i32::from(u16::from_le_bytes([sample[0], sample[1]]))
            });
        }
    }
    let mut coder = RangeWriter::new();
    coder.bit(&mut 128, true)?;
    let mut header = [128; 32];
    for value in [1, 1, 1, i32::from(depth)] {
        coder.integer(&mut header, value, false)?;
    }
    coder.bit(&mut header[0], true)?;
    coder.integer(&mut header, 0, false)?;
    coder.integer(&mut header, 0, false)?;
    coder.bit(&mut header[0], alpha.is_some())?;
    for _ in 0..5 {
        coder.integer(&mut [128; 32], 127, false)?;
    }
    let mut models = [[128; 32]; 3];
    let read = |plane: &[i32], x: isize, y: isize| {
        if y < 0 || x < -1 {
            return 0;
        }
        if x == -1 {
            if y == 0 {
                0
            } else {
                plane[(y as usize - 1) * frame.width]
            }
        } else {
            plane[y as usize * frame.width + (x as usize).min(frame.width - 1)]
        }
    };
    for y in 0..frame.height {
        for (component, plane) in planes.iter().take(components).enumerate() {
            for x in 0..frame.width {
                let (xx, yy) = (x as isize, y as isize);
                let l = read(plane, xx - 1, yy);
                let t = read(plane, xx, yy - 1);
                let tl = read(plane, xx - 1, yy - 1);
                let prediction = (l + t - tl).clamp(l.min(t), l.max(t));
                let residual = ((plane[y * frame.width + x] - prediction + bias) & mask) - bias;
                coder.integer(
                    &mut models[if component == 3 {
                        2
                    } else {
                        usize::from(component != 0)
                    }],
                    residual,
                    true,
                )?;
            }
        }
    }
    coder.finish()
}

/// Stream supported Y4M transforms into independently decodable FFV1 packets.
/// The sink receives output geometry, packet bytes, source PTS and duration. It
/// owns muxing/publication; an error must discard any partially written output.
pub fn encode_y4m(
    source: impl std::io::BufRead,
    transform: &fvid_media_info::DecodeTransform,
    sink: impl FnMut(&crate::owned_y4m::Header, &[u8], u64, u64) -> Result<()>,
) -> Result<fvid_media_info::DecodeStats> {
    encode_y4m_limited(source, transform, None, sink)
}
pub(crate) fn encode_y4m_limited(
    source: impl std::io::BufRead,
    transform: &fvid_media_info::DecodeTransform,
    max_packets: Option<u64>,
    sink: impl FnMut(&crate::owned_y4m::Header, &[u8], u64, u64) -> Result<()>,
) -> Result<fvid_media_info::DecodeStats> {
    encode_y4m_counted(source, transform, max_packets, sink).map(|(stats, _)| stats)
}
pub(crate) fn encode_y4m_counted(
    source: impl std::io::BufRead,
    transform: &fvid_media_info::DecodeTransform,
    max_packets: Option<u64>,
    mut sink: impl FnMut(&crate::owned_y4m::Header, &[u8], u64, u64) -> Result<()>,
) -> Result<(fvid_media_info::DecodeStats, u64)> {
    crate::owned_y4m_decode::visit_reader_transformed_counted(
        source,
        transform,
        max_packets,
        &mut |header, data, pts, duration| {
            let (sx, sy) = header.format.subsampling();
            let mut buf = buffer(data.len()).map_err(String::from)?;
            buf.copy_from_slice(data);
            let frame = GeometryFrame {
                data: buf,
                width: header.width,
                height: header.height,
                subsampling: Some([sx, sy]),
            };
            let packet = encode(&frame, header.depth()).map_err(String::from)?;
            sink(header, &packet, pts, duration).map_err(String::from)
        },
    )
    .map_err(Error::from)
}
