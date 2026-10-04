//! Causal spatial/temporal denoising on owned integer planar samples.
use crate::owned_frame::{GeometryFrame, buffer};
use std::cell::RefCell;
type Result<T> = std::result::Result<T, String>;
#[derive(Debug)]
pub struct HqDn3d {
    strength: [f64; 4],
    enable: crate::owned_timeline::Timeline,
    state: RefCell<Option<State>>,
}
#[derive(Debug)]
struct State {
    shape: (usize, usize, [usize; 2], u8),
    last: u64,
    previous: Vec<u16>,
    tables: [Vec<i16>; 4],
}
impl HqDn3d {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid hqdn3d options".into());
        }
        let mut strength = [0.; 4];
        let mut position = 0;
        let mut enable = crate::owned_timeline::Timeline::default();
        let names = ["luma_spatial", "chroma_spatial", "luma_tmp", "chroma_tmp"];
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, text) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *names.get(position).ok_or("too many hqdn3d options")?;
                position += 1;
                (key, entry)
            };
            if key.trim() == "enable" {
                enable =
                    crate::owned_timeline::Timeline::grayworld(&format!("enable={}", text.trim()))?;
            } else {
                let index = names
                    .iter()
                    .position(|&v| v == key.trim())
                    .ok_or("unknown hqdn3d option")?;
                let value = crate::owned_expression::constant(text.trim())?;
                if !value.is_finite() || value < 0. {
                    return Err("hqdn3d strength must be finite and nonnegative".into());
                }
                strength[index] = value;
            }
        }
        if strength[0] == 0. {
            strength[0] = 4.;
        }
        if strength[1] == 0. {
            strength[1] = 3. * strength[0] / 4.;
        }
        if strength[2] == 0. {
            strength[2] = 6. * strength[0] / 4.;
        }
        if strength[3] == 0. {
            strength[3] = strength[2] * strength[1] / strength[0];
        }
        if strength.iter().any(|v| !v.is_finite()) {
            return Err("hqdn3d derived strength overflow".into());
        }
        Ok(Self {
            strength,
            enable,
            state: RefCell::new(None),
        })
    }
    pub fn apply(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        if !matches!(depth, 8 | 9 | 10 | 12 | 14 | 16) || frame.width == 0 || frame.height == 0 {
            return Err("invalid hqdn3d geometry or depth".into());
        }
        let sub = frame.subsampling.ok_or("hqdn3d requires planar YUV")?;
        if sub.contains(&0) {
            return Err("invalid hqdn3d subsampling".into());
        }
        let sizes = [
            (frame.width, frame.height),
            (frame.width.div_ceil(sub[0]), frame.height.div_ceil(sub[1])),
            (frame.width.div_ceil(sub[0]), frame.height.div_ceil(sub[1])),
        ];
        let samples = sizes.iter().try_fold(0usize, |sum, &(w, h)| {
            sum.checked_add(w.checked_mul(h).ok_or("hqdn3d geometry overflow")?)
                .ok_or("hqdn3d storage overflow")
        })?;
        let bytes = if depth == 8 { 1 } else { 2 };
        if samples.checked_mul(bytes) != Some(frame.data.len()) {
            return Err("hqdn3d storage mismatch".into());
        }
        let shift = 16 - depth;
        let midpoint = ((1u32 << shift) - 1) / 2;
        let load = |index: usize| -> u16 {
            let sample = if bytes == 1 {
                u16::from(frame.data[index])
            } else {
                u16::from_le_bytes([frame.data[index * 2], frame.data[index * 2 + 1]])
            };
            ((u32::from(sample) << shift) + midpoint) as u16
        };
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) >= 1u32 << depth)
        {
            return Err("hqdn3d sample exceeds depth".into());
        }
        let enabled = self.enable.enabled(n, t, frame.width, frame.height)?;
        let shape = (frame.width, frame.height, sub, depth);
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| "hqdn3d state is already borrowed")?;
        if let Some(old) = state.as_ref().filter(|_| n != 0) {
            if old.shape != shape {
                return Err("hqdn3d stream geometry changed".into());
            }
            if n <= old.last {
                return Err("hqdn3d frame clock must advance or rewind to zero".into());
            }
        }
        // Complete all fallible allocations before updating persistent state.
        let mut output = buffer(frame.data.len())?;
        let mut line = Vec::<u16>::new();
        line.try_reserve_exact(frame.width)
            .map_err(|_| "hqdn3d line allocation failed")?;
        line.resize(frame.width, 0);
        if n == 0 || state.is_none() {
            let mut previous = Vec::new();
            previous
                .try_reserve_exact(samples)
                .map_err(|_| "hqdn3d history allocation failed")?;
            previous.extend((0..samples).map(load));
            let mut tables: [Vec<i16>; 4] = Default::default();
            for (table, &strength) in tables.iter_mut().zip(&self.strength) {
                *table = coefficients(strength, depth)?;
            }
            *state = Some(State {
                shape,
                last: n,
                previous,
                tables,
            });
        }
        let state = state.as_mut().unwrap();
        let mut offset = 0;
        for (plane, (w, h)) in sizes.into_iter().enumerate() {
            let spatial = &state.tables[usize::from(plane != 0)];
            let temporal = &state.tables[2 + usize::from(plane != 0)];
            for y in 0..h {
                let mut horizontal = i32::from(load(offset + y * w));
                for x in 0..w {
                    let index = offset + y * w + x;
                    if y == 0 || x != 0 {
                        horizontal = smooth(horizontal, i32::from(load(index)), spatial, depth);
                    }
                    let spatial_value = if y == 0 {
                        horizontal
                    } else {
                        smooth(i32::from(line[x]), horizontal, spatial, depth)
                    };
                    // Preserve signed intermediate corrections; clamp only stored history/output.
                    line[x] = spatial_value.clamp(0, 65535) as u16;
                    let value = smooth(
                        i32::from(state.previous[index]),
                        spatial_value,
                        temporal,
                        depth,
                    );
                    state.previous[index] = value.clamp(0, 65535) as u16;
                    let value = ((value.max(0) as u32) >> shift).min((1u32 << depth) - 1) as u16;
                    if bytes == 1 {
                        output[index] = value as u8;
                    } else {
                        output[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
            offset += w * h;
        }
        state.last = n;
        if enabled {
            frame.data = output;
        }
        Ok(())
    }
}
fn coefficients(strength: f64, depth: u8) -> Result<Vec<i16>> {
    let bits = if depth == 16 { 8 } else { 4 };
    let radius = 256i32 << bits;
    let mut table = Vec::new();
    table
        .try_reserve_exact((2 * radius) as usize)
        .map_err(|_| "hqdn3d table allocation failed")?;
    let exponent = 0.25f64.ln() / (1. - strength.min(252.) / 255. - 0.00001).ln();
    for bin in -radius..radius {
        let delta = f64::from(bin * (1 << (9 - bits)) + (1 << (8 - bits)) - 1) / 512.;
        let weight = (1. - delta.abs() / 255.).max(0.).powf(exponent);
        table.push((weight * 256. * delta).round_ties_even() as i16);
    }
    table[0] = i16::from(strength != 0.);
    Ok(table)
}
fn smooth(previous: i32, current: i32, table: &[i16], depth: u8) -> i32 {
    let bits = if depth == 16 { 8 } else { 4 };
    let radius = 256 << bits;
    let bin = ((previous - current) >> (8 - bits)).clamp(-radius, radius - 1);
    current + i32::from(table[(radius + bin) as usize])
}
