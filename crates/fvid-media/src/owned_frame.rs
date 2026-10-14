//! Shared decoded geometry frame storage.
#[derive(Debug)]
pub struct GeometryFrame {
    pub width: usize,
    pub height: usize,
    /// Horizontal/vertical luma samples per chroma sample; None for RGB.
    /// A quarter-turn swaps the axes (4:2:2 becomes 4:4:0).
    pub subsampling: Option<[usize; 2]>,
    /// Packed RGB or packed Y, Cb, Cr. Sample depth is unchanged;
    /// chroma axes follow `subsampling`.
    pub data: Vec<u8>,
}

pub(crate) fn buffer(size: usize) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    data.try_reserve_exact(size).map_err(|e| e.to_string())?;
    data.resize(size, 0);
    Ok(data)
}

/// Pack identity-matrix codec planes (G, B, R) as depth-preserving RGB.
pub fn pack_gbr(
    size: [u32; 2],
    depth: u8,
    planes: [(usize, usize, &[u16]); 3],
) -> Result<GeometryFrame, String> {
    if !matches!(depth, 8 | 10 | 12 | 16) {
        return Err("invalid RGB sample depth".into());
    }
    let [width, height] = size.map(|n| n as usize);
    let bytes = if depth == 8 { 1 } else { 2 };
    let length = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(3 * bytes))
        .ok_or("RGB frame size overflow")?;
    if width == 0 || height == 0 {
        return Err("empty RGB frame".into());
    }
    for &(w, h, data) in &planes {
        if w < width || h < height || w.checked_mul(h) != Some(data.len()) {
            return Err("invalid RGB plane storage".into());
        }
    }
    let mut data = buffer(length)?;
    let mut cursor = 0;
    for y in 0..height {
        for x in 0..width {
            for channel in [2, 0, 1] {
                let (stride, _, samples) = planes[channel];
                let value = samples[y * stride + x];
                if u32::from(value) >= 1u32 << depth {
                    return Err("RGB sample exceeds coded depth".into());
                }
                if bytes == 1 {
                    data[cursor] = value as u8;
                } else {
                    data[cursor..cursor + 2].copy_from_slice(&value.to_le_bytes());
                }
                cursor += bytes;
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
