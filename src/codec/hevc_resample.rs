//! H.8.1.4.2 inter-layer sample resampling (not motion/mode resampling).
use super::{
    hevc_plane::Plane,
    hevc_pps::{ReferenceGeometry, ReferenceLocation},
    hevc_sps::Sps,
};
use crate::{Result, invalid};
const LUMA: [[i16; 8]; 16] = [
    [0, 0, 0, 64, 0, 0, 0, 0],
    [0, 1, -3, 63, 4, -2, 1, 0],
    [-1, 2, -5, 62, 8, -3, 1, 0],
    [-1, 3, -8, 60, 13, -4, 1, 0],
    [-1, 4, -10, 58, 17, -5, 1, 0],
    [-1, 4, -11, 52, 26, -8, 3, -1],
    [-1, 3, -9, 47, 31, -10, 4, -1],
    [-1, 4, -11, 45, 34, -10, 4, -1],
    [-1, 4, -11, 40, 40, -11, 4, -1],
    [-1, 4, -10, 34, 45, -11, 4, -1],
    [-1, 4, -10, 31, 47, -9, 3, -1],
    [-1, 3, -8, 26, 52, -11, 4, -1],
    [0, 1, -5, 17, 58, -10, 4, -1],
    [0, 1, -4, 13, 60, -8, 3, -1],
    [0, 1, -3, 8, 62, -5, 2, -1],
    [0, 1, -2, 4, 63, -3, 1, 0],
];
const CHROMA: [[i16; 4]; 16] = [
    [0, 64, 0, 0],
    [-2, 62, 4, 0],
    [-2, 58, 10, -2],
    [-4, 56, 14, -2],
    [-4, 54, 16, -2],
    [-6, 52, 20, -2],
    [-6, 46, 28, -4],
    [-4, 42, 30, -4],
    [-4, 36, 36, -4],
    [-4, 30, 42, -4],
    [-4, 28, 46, -6],
    [-2, 20, 52, -6],
    [-2, 16, 54, -4],
    [-2, 14, 56, -4],
    [-2, 10, 58, -2],
    [0, 4, 62, -2],
];
#[derive(Clone, Debug)]
pub struct Resampling {
    current: [[usize; 2]; 3],
    reference: [[usize; 2]; 3],
    current_depth: [u8; 2],
    reference_depth: [u8; 2],
    scales: [[i64; 2]; 2],
    current_offsets: [[i64; 2]; 2],
    reference_offsets: [[i64; 2]; 2],
    phases: [[i64; 2]; 2],
    geometry: ReferenceGeometry,
    identity: bool,
}
fn subsampling(s: &Sps) -> Result<[u64; 2]> {
    match s.chroma_format {
        1 if !s.separate_colour_plane => Ok([2, 2]),
        2 if !s.separate_colour_plane => Ok([2, 1]),
        3 => Ok([1, 1]),
        _ => Err(crate::unsupported(
            "HEVC inter-layer resampling requires three sample planes",
        )),
    }
}
impl Resampling {
    /// Resolve direct-reference offsets/phases and validate H.8.1.4.1 constraints.
    /// If colour mapping is enabled, reference SPS depths must describe its output.
    pub fn new(current: &Sps, reference: &Sps, location: &ReferenceLocation) -> Result<Self> {
        Self::with_reference_depth(current, reference, reference.depth, location)
    }
    pub(crate) fn with_reference_depth(
        current: &Sps,
        reference: &Sps,
        reference_depth: [u8; 2],
        location: &ReferenceLocation,
    ) -> Result<Self> {
        if reference_depth.iter().any(|d| !(8..=16).contains(d)) {
            return Err(invalid("HEVC resampling reference depth is invalid"));
        }
        let cs = subsampling(current)?;
        let rs = subsampling(reference)?;
        for (s, sub) in [(current, cs), (reference, rs)] {
            if s.dimensions.iter().any(|&d| d == 0)
                || (0..2).any(|a| u64::from(s.dimensions[a]) % sub[a] != 0)
                || s.depth.iter().any(|d| !(8..=16).contains(d))
            {
                return Err(invalid("HEVC resampling geometry or depth is invalid"));
            }
        }
        if (0..2).any(|c| reference_depth[c] > current.depth[c]) {
            return Err(invalid("HEVC resampling cannot reduce reference depth"));
        }
        if location
            .phases
            .is_some_and(|p| p[0] > 31 || p[1] > 31 || p[2] > 63 || p[3] > 63)
        {
            return Err(invalid(
                "HEVC resampling direct-reference phase exceeds range",
            ));
        }
        if location
            .scaled_offsets
            .into_iter()
            .chain(location.region_offsets)
            .flatten()
            .any(|v| !(-16384..=16383).contains(&v))
        {
            return Err(invalid("HEVC resampling reference offset exceeds range"));
        }
        let geometry = location.geometry(current, reference, true)?;
        if (0..2).any(|a| geometry.scaled_size[a] < geometry.reference_size[a]) {
            return Err(invalid(
                "HEVC inter-layer resampling cannot downscale luma regions",
            ));
        }
        let mut scales = [[0i64; 2]; 2];
        let mut current_offsets = [[0i64; 2]; 2];
        let mut reference_offsets = [[0i64; 2]; 2];
        let phases = [
            [geometry.phases[0], geometry.phases[1]],
            [geometry.phases[2], geometry.phases[3]],
        ];
        for c in 0..2 {
            for a in 0..2 {
                let csub = if c == 0 { 1 } else { cs[a] };
                let rsub = if c == 0 { 1 } else { rs[a] };
                let width = geometry.scaled_size[a] / csub;
                let ref_width = geometry.reference_size[a] / rsub;
                if width == 0 || ref_width == 0 {
                    return Err(invalid("HEVC resampling empty component region"));
                }
                if width == ref_width && phases[c][a] != 0 {
                    return Err(invalid("HEVC equal resampling regions require zero phase"));
                }
                scales[c][a] = (((ref_width << 16) + (width >> 1)) / width) as i64;
                current_offsets[c][a] = i64::from(geometry.scaled_offsets[a]) / csub as i64;
                reference_offsets[c][a] =
                    (i64::from(geometry.reference_offsets[a]) / rsub as i64) * 16;
            }
        }
        let dimensions = |s: &Sps, sub: [u64; 2]| {
            let full = s.dimensions.map(|v| v as usize);
            let chroma = [full[0] / sub[0] as usize, full[1] / sub[1] as usize];
            [full, chroma, chroma]
        };
        let identity = current.dimensions == reference.dimensions
            && current.depth == reference_depth
            && cs == rs
            && geometry.scaled_offsets == geometry.reference_offsets
            && geometry.phases == [0; 4];
        Ok(Self {
            identity,
            current: dimensions(current, cs),
            reference: dimensions(reference, rs),
            current_depth: current.depth,
            reference_depth,
            scales,
            current_offsets,
            reference_offsets,
            phases,
            geometry,
        })
    }
    pub(crate) fn is_identity(&self) -> bool {
        self.identity
    }
    pub fn geometry(&self) -> &ReferenceGeometry {
        &self.geometry
    }
    /// Reference coordinate in sixteenth-sample units, before edge extension.
    pub fn coordinate(&self, chroma: bool, position: [usize; 2]) -> Result<[i64; 2]> {
        let c = usize::from(chroma);
        if (0..2).any(|a| position[a] >= self.current[c][a]) {
            return Err(invalid("HEVC resampling position outside current picture"));
        }
        Ok(std::array::from_fn(|a| {
            let scale = self.scales[c][a];
            let add = -((scale * self.phases[c][a] + 8) >> 4);
            (((position[a] as i64 - self.current_offsets[c][a]) * scale + add + 2048) >> 12)
                + self.reference_offsets[c][a]
        }))
    }
    /// Map complete uncropped raster sample planes. The budget covers output
    /// planes, row scratch and allocation metadata; input storage stays owned.
    pub fn picture(&self, input: [&[u16]; 3], budget: usize) -> Result<[Plane; 3]> {
        let mut count = 0usize;
        for c in 0..3 {
            let [w, h] = self.reference[c];
            if w.checked_mul(h) != Some(input[c].len())
                || input[c]
                    .iter()
                    .any(|&v| u32::from(v) >= 1u32 << self.reference_depth[usize::from(c != 0)])
            {
                return Err(invalid("HEVC resampling input plane mismatch"));
            }
            let [w, h] = self.current[c];
            count = count
                .checked_add(
                    w.checked_mul(h)
                        .ok_or_else(|| invalid("HEVC resampling picture size overflow"))?,
                )
                .ok_or_else(|| invalid("HEVC resampling picture size overflow"))?;
        }
        let overhead = 3
            * (std::mem::size_of::<Plane>()
                + 2 * (std::mem::size_of::<Vec<u16>>() + 2 * std::mem::size_of::<usize>()));
        let required = count
            .checked_mul(3)
            .and_then(|n| {
                self.current[0][0]
                    .checked_mul(2)
                    .and_then(|row| n.checked_add(row))
            })
            .and_then(|n| n.checked_add(overhead))
            .ok_or_else(|| invalid("HEVC resampling picture size overflow"))?;
        if required > budget {
            return Err(invalid("HEVC resampling picture exceeds memory budget"));
        }
        let make = |c: usize| {
            Plane::new(
                self.current[c][0],
                self.current[c][1],
                self.current_depth[usize::from(c != 0)],
                budget,
            )
        };
        let mut output = [make(0)?, make(1)?, make(2)?];
        for c in 0..3 {
            let chroma = c != 0;
            let [w, h] = self.current[c];
            let [rw, rh] = self.reference[c];
            let mut row = vec![0u16; w];
            for y in 0..h {
                if self.identity {
                    // H.8.1.4.1: preserve all source bits rather than quantizing
                    // an already identical >14-bit reference through the FIR.
                    row.copy_from_slice(&input[c][y * w..(y + 1) * w]);
                } else {
                    for (x, value) in row.iter_mut().enumerate() {
                        let [rx, ry] = self.coordinate(chroma, [x, y])?;
                        let ix = rx >> 4;
                        let iy = ry >> 4;
                        let px = (rx & 15) as usize;
                        let py = (ry & 15) as usize;
                        let (fx, fy, origin): (&[i16], &[i16], i64) = if chroma {
                            (&CHROMA[px], &CHROMA[py], 1)
                        } else {
                            (&LUMA[px], &LUMA[py], 3)
                        };
                        let mut total = 0i64;
                        for (j, &f) in fy.iter().enumerate() {
                            let sy = (iy + j as i64 - origin).clamp(0, rh as i64 - 1) as usize;
                            let horizontal: i64 = fx
                                .iter()
                                .enumerate()
                                .map(|(i, &g)| {
                                    let sx =
                                        (ix + i as i64 - origin).clamp(0, rw as i64 - 1) as usize;
                                    i64::from(g) * i64::from(input[c][sy * rw + sx])
                                })
                                .sum();
                            total += i64::from(f)
                                * (horizontal >> (self.reference_depth[usize::from(chroma)] - 8));
                        }
                        let depth = self.current_depth[usize::from(chroma)];
                        let shift = 20 - depth;
                        *value = ((total + (1i64 << (shift - 1))) >> shift)
                            .clamp(0, (1i64 << depth) - 1) as u16;
                    }
                }
                output[c].reconstruct_inter([0, y, w, 1], &row)?;
            }
        }
        Ok(output)
    }
}
