/// Interleaved RGB input, source precision unchanged. RFC 9043 3.7.2/3.7.2.1.
pub fn encode_rgb(frame: &GeometryFrame, depth: u8) -> Result<Vec<u8>> {
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
    let bias = 1i32 << depth;
    let mask = (bias << 1) - 1;
    let mut planes: [Vec<i32>; 3] = std::array::from_fn(|_| Vec::new());
    for plane in &mut planes {
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
        let exception = (9..=15).contains(&depth);
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
    let mut coder = RangeWriter::new();
    coder.bit(&mut 128, true)?;
    let mut header = [128; 32];
    for value in [1, 1, 1, i32::from(depth)] {
        coder.integer(&mut header, value, false)?;
    }
    coder.bit(&mut header[0], true)?;
    coder.integer(&mut header, 0, false)?;
    coder.integer(&mut header, 0, false)?;
    coder.bit(&mut header[0], false)?;
    for _ in 0..5 {
        coder.integer(&mut [128; 32], 127, false)?;
    }
    let mut models = [[128; 32]; 2];
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
        for (component, plane) in planes.iter().enumerate() {
            for x in 0..frame.width {
                let (xx, yy) = (x as isize, y as isize);
                let l = read(plane, xx - 1, yy);
                let t = read(plane, xx, yy - 1);
                let tl = read(plane, xx - 1, yy - 1);
                let prediction = (l + t - tl).clamp(l.min(t), l.max(t));
                let residual = ((plane[y * frame.width + x] - prediction + bias) & mask) - bias;
                coder.integer(&mut models[usize::from(component != 0)], residual, true)?;
            }
        }
    }
    coder.finish()
}
