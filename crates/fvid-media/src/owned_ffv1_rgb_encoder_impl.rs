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
