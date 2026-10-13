fn decode_rgb(
    r: &mut Range<'_>,
    state: &mut State,
    width: usize,
    height: usize,
    budget: usize,
) -> Result<GeometryFrame> {
    let count = width
        .checked_mul(height)
        .ok_or_else(|| invalid("FFV1 RGB size overflow"))?;
    let bytes = if state.depth == 8 { 1 } else { 2 };
    let length = count
        .checked_mul(3 * bytes)
        .ok_or_else(|| invalid("FFV1 RGB size overflow"))?;
    let storage = count
        .checked_mul(12)
        .and_then(|n| n.checked_add(length))
        .and_then(|n| n.checked_add(state.models[0].len() * 64 + std::mem::size_of::<State>()))
        .ok_or_else(|| invalid("FFV1 RGB storage overflow"))?;
    if storage > budget {
        return Err(invalid("FFV1 RGB frame exceeds memory budget"));
    }
    let mut planes: [Vec<i32>; 3] = std::array::from_fn(|_| Vec::new());
    for p in &mut planes {
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
    for y in 0..height {
        for (component, p) in planes.iter_mut().enumerate() {
            for x in 0..width {
                let (xx, yy) = (x as isize, y as isize);
                let l = read(p, xx - 1, yy);
                let t = read(p, xx, yy - 1);
                let tl = read(p, xx - 1, yy - 1);
                let tr = read(p, xx + 1, yy - 1);
                let ll = read(p, xx - 2, yy);
                let tt = read(p, xx, yy - 2);
                let context: i32 = state
                    .tables
                    .iter()
                    .zip([l - tl, tl - t, t - tr, ll - l, tt - t])
                    .map(|(table, d)| table[(d & 255) as usize])
                    .sum();
                let probabilities = state.models[usize::from(component != 0)]
                    .get_mut(context.unsigned_abs() as usize)
                    .ok_or_else(|| invalid("invalid FFV1 RGB context"))?;
                let mut residual = r.integer(probabilities, true)?;
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
    for i in 0..count {
        let cb = planes[1][i] - bias;
        let cr = planes[2][i] - bias;
        let base = planes[0][i] - ((cb + cr) >> 2);
        let rgb = if (9..=15).contains(&state.depth) {
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
    }
    Ok(GeometryFrame {
        width,
        height,
        subsampling: None,
        data,
    })
}
