//! Owned three-plane permutation/duplication; bytes retain source precision.
#[derive(Clone, Copy, Debug)]
pub struct ShufflePlanes {
    pub mapping: [usize; 3],
}
impl ShufflePlanes {
    pub fn parse(args: &str) -> Result<Self, String> {
        let mut maps = [0usize, 1, 2, 3];
        let mut positional = 0;
        if !args.is_empty() {
            for entry in args.split(':') {
                let (index, value) = if let Some((key, value)) = entry.split_once('=') {
                    let index = ["map0", "map1", "map2", "map3"]
                        .iter()
                        .position(|&name| name == key)
                        .ok_or("unknown shuffleplanes option")?;
                    (index, value)
                } else {
                    let index = positional;
                    positional += 1;
                    (index, entry)
                };
                if index >= 4 {
                    return Err("too many shuffleplanes options".into());
                }
                let value = value
                    .parse::<usize>()
                    .map_err(|_| "invalid shuffleplanes index")?;
                if value > 3 {
                    return Err("shuffleplanes index must be 0..=3".into());
                }
                maps[index] = value;
            }
        }
        if maps[..3].contains(&3) {
            return Err("shuffleplanes references an absent alpha plane".into());
        }
        Ok(Self {
            mapping: [maps[0], maps[1], maps[2]],
        })
    }
    /// Permute planar storage. Duplication is allowed; source/destination plane
    /// sizes must agree. Errors leave the destination unchanged.
    pub fn apply_planar(self, data: &mut Vec<u8>, sizes: [usize; 3]) -> Result<(), String> {
        let total = sizes
            .iter()
            .try_fold(0usize, |sum, &n| sum.checked_add(n))
            .ok_or("shuffleplanes size overflow")?;
        if sizes.contains(&0)
            || total != data.len()
            || self
                .mapping
                .iter()
                .enumerate()
                .any(|(to, &from)| from >= 3 || sizes[to] != sizes[from])
        {
            return Err("shuffleplanes incompatible plane geometry".into());
        }
        let offsets = [0, sizes[0], sizes[0] + sizes[1]];
        let mut output = Vec::new();
        output
            .try_reserve_exact(total)
            .map_err(|_| "shuffleplanes allocation failed")?;
        for &from in &self.mapping {
            output.extend_from_slice(&data[offsets[from]..offsets[from] + sizes[from]]);
        }
        *data = output;
        Ok(())
    }
    /// RGB24 is interpreted through the planar GBR convention, then repacked.
    pub fn apply_rgb24(self, data: &mut [u8]) -> Result<(), String> {
        if data.len() % 3 != 0 || self.mapping.iter().any(|&v| v >= 3) {
            return Err("invalid shuffleplanes RGB storage".into());
        }
        for pixel in data.chunks_exact_mut(3) {
            let gbr = [pixel[1], pixel[2], pixel[0]];
            pixel.copy_from_slice(&[
                gbr[self.mapping[2]],
                gbr[self.mapping[0]],
                gbr[self.mapping[1]],
            ]);
        }
        Ok(())
    }
}
impl ShufflePlanes {
    /// Cross-size plane mappings promote YUV to 4:4:4 with point sampling.
    /// Existing chroma samples are repeated; their sample precision is unchanged.
    pub fn apply_yuv(
        self,
        data: &mut Vec<u8>,
        width: usize,
        height: usize,
        sampling: [usize; 2],
        depth: u8,
    ) -> Result<[usize; 2], String> {
        let [sx, sy] = sampling;
        if width == 0 || height == 0 || sx == 0 || sy == 0 || !(8..=16).contains(&depth) {
            return Err("invalid shuffleplanes YUV geometry".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let size = width
            .checked_mul(height)
            .and_then(|v| v.checked_mul(bytes))
            .ok_or("shuffleplanes plane size overflow")?;
        let cw = width.div_ceil(sx);
        let ch = height.div_ceil(sy);
        let chroma = cw
            .checked_mul(ch)
            .and_then(|v| v.checked_mul(bytes))
            .ok_or("shuffleplanes plane size overflow")?;
        let sizes = [size, chroma, chroma];
        let total = size
            .checked_add(chroma.checked_mul(2).ok_or("shuffleplanes size overflow")?)
            .ok_or("shuffleplanes size overflow")?;
        if total != data.len() || self.mapping.iter().any(|&i| i >= 3) {
            return Err("invalid shuffleplanes plane storage".into());
        }
        if self
            .mapping
            .iter()
            .enumerate()
            .all(|(to, &from)| sizes[to] == sizes[from])
        {
            self.apply_planar(data, sizes)?;
            return Ok(sampling);
        }
        let mut expanded = Vec::new();
        expanded
            .try_reserve_exact(size.checked_mul(3).ok_or("shuffleplanes size overflow")?)
            .map_err(|_| "shuffleplanes allocation failed")?;
        expanded.extend_from_slice(&data[..size]);
        for plane in 0..2 {
            let offset = size + plane * chroma;
            for y in 0..height {
                for x in 0..width {
                    let at = offset + ((y / sy) * cw + x / sx) * bytes;
                    expanded.extend_from_slice(&data[at..at + bytes]);
                }
            }
        }
        self.apply_planar(&mut expanded, [size; 3])?;
        *data = expanded;
        Ok([1, 1])
    }
}
