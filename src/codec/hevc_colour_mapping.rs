//! Annex F colour-mapping syntax/LUT and the scalar H.8.1.4.4 mapping kernel.
//! Picture sampling/interpolation is the caller's separate responsibility.
use super::super::bits::BitReader;
use crate::{Result, invalid};
type Cell = [[i16; 3]; 4];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColourMappingTable {
    pub reference_layers: Vec<u8>,
    pub input_depth: [u8; 2],
    pub output_depth: [u8; 2],
    pub octant_depth: u8,
    pub y_partition_log2: u8,
    pub residual_quant_bits: u8,
    pub residual_low_bits: u8,
    pub thresholds: [u32; 2],
    dimensions: [usize; 3],
    coefficients: Vec<Cell>,
}
impl ColourMappingTable {
    pub(super) fn read(
        b: &mut BitReader<'_>,
        current_depth: [u8; 2],
        direct: u64,
        reference_depth: impl Fn(u8) -> Result<[u8; 2]>,
        budget: usize,
    ) -> Result<Self> {
        let n = super::ue(b, 61)? as usize + 1;
        let mut refs = Vec::with_capacity(n);
        for _ in 0..n {
            let id = b.read(6)? as u8;
            if direct & (1u64 << id) == 0 {
                return Err(invalid(
                    "HEVC colour mapping requires a direct reference layer",
                ));
            }
            refs.push(id);
        }
        let octant = b.read(2)? as u8;
        let y_part = b.read(2)? as u8;
        if octant > 1 || octant + y_part > 3 {
            return Err(crate::unsupported(
                "reserved HEVC colour mapping partition depth",
            ));
        }
        let input = [super::ue(b, 8)? as u8 + 8, super::ue(b, 8)? as u8 + 8];
        let output = [super::ue(b, 8)? as u8 + 8, super::ue(b, 8)? as u8 + 8];
        if (0..2).any(|c| output[c] < input[c] || output[c] > current_depth[c]) {
            return Err(invalid("HEVC colour mapping output depth contradicts SPS"));
        }
        for &id in &refs {
            if reference_depth(id)? != input {
                return Err(invalid(
                    "HEVC colour mapping input depth contradicts reference SPS",
                ));
            }
        }
        let quant = b.read(2)? as u8;
        let delta = b.read(2)? as u8 + 1;
        let low = (10i32 + i32::from(input[0])
            - i32::from(output[0])
            - i32::from(quant)
            - i32::from(delta))
        .max(0) as u8;
        let half = 1i32 << (input[1] - 1);
        let mut thresholds = [half as u32; 2];
        if octant == 1 {
            for t in &mut thresholds {
                *t = (super::se(b, -half, half - 1)? + half) as u32;
            }
        }
        let ny = 1usize << (octant + y_part);
        let nc = 1usize << octant;
        let count = ny * nc * nc;
        // Scratch residuals have a fixed normative maximum of 32 cells.
        if std::mem::size_of::<Self>() + n + count * std::mem::size_of::<Cell>() + 32 * 4 * 3 * 8
            > budget
        {
            return Err(invalid(
                "HEVC colour mapping metadata exceeds memory budget",
            ));
        }
        let mut residual = [[[0i64; 3]; 4]; 32];
        fn octants(
            b: &mut BitReader<'_>,
            residual: &mut [[[i64; 3]; 4]; 32],
            depth: u8,
            maximum: u8,
            part: usize,
            pos: [usize; 3],
            length: usize,
            nc: usize,
            low: u8,
            quant: u8,
        ) -> Result<()> {
            if depth < maximum && b.bit()? {
                let half = length / 2;
                for y in 0..2 {
                    for cb in 0..2 {
                        for cr in 0..2 {
                            octants(
                                b,
                                residual,
                                depth + 1,
                                maximum,
                                part,
                                [
                                    pos[0] + part * y * half,
                                    pos[1] + cb * half,
                                    pos[2] + cr * half,
                                ],
                                half,
                                nc,
                                low,
                                quant,
                            )?;
                        }
                    }
                }
            } else {
                for i in 0..part {
                    let y = pos[0] + (i << (maximum - depth));
                    let index = (y * nc + pos[1]) * nc + pos[2];
                    for j in 0..4 {
                        if b.bit()? {
                            for c in 0..3 {
                                let q = u64::from(b.unsigned_golomb()?);
                                let r = u64::from(b.read(low)?);
                                let magnitude = ((q << low) + r) << quant;
                                let negative = magnitude != 0 && b.bit()?;
                                residual[index][j][c] = if negative {
                                    -(magnitude as i64)
                                } else {
                                    magnitude as i64
                                };
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        octants(
            b,
            &mut residual,
            0,
            octant,
            1usize << y_part,
            [0; 3],
            nc,
            nc,
            low,
            quant,
        )?;
        let mut coefficients = vec![[[0i16; 3]; 4]; count];
        for y in 0..ny {
            for cb in 0..nc {
                for cr in 0..nc {
                    let index = (y * nc + cb) * nc + cr;
                    for j in 0..4 {
                        for c in 0..3 {
                            let predicted = if y == 0 {
                                if j == c { 1024 } else { 0 }
                            } else {
                                i64::from(coefficients[index - nc * nc][j][c])
                            };
                            let value = predicted + residual[index][j][c];
                            if !(-2048..=2047).contains(&value) {
                                return Err(invalid(
                                    "HEVC colour mapping coefficient exceeds signed twelve bits",
                                ));
                            }
                            coefficients[index][j][c] = value as i16;
                        }
                    }
                }
            }
        }
        Ok(Self {
            reference_layers: refs,
            input_depth: input,
            output_depth: output,
            octant_depth: octant,
            y_partition_log2: y_part,
            residual_quant_bits: quant,
            residual_low_bits: low,
            thresholds,
            dimensions: [ny, nc, nc],
            coefficients,
        })
    }
    pub fn dimensions(&self) -> [usize; 3] {
        self.dimensions
    }
    pub fn coefficients(&self, index: [usize; 3]) -> Result<&Cell> {
        if (0..3).any(|i| index[i] >= self.dimensions[i]) {
            return Err(invalid("HEVC colour mapping LUT index out of range"));
        }
        self.coefficients
            .get((index[0] * self.dimensions[1] + index[1]) * self.dimensions[2] + index[2])
            .ok_or_else(|| invalid("HEVC colour mapping LUT is incomplete"))
    }
    /// Map an already spatially sampled/interpolated Y/Cb/Cr triplet.
    /// Luma and chroma picture interpolation from H.8.1.4.4 is not done here.
    pub fn map_sample(&self, component: usize, samples: [u16; 3]) -> Result<u16> {
        if component > 2
            || self.octant_depth > 1
            || self.y_partition_log2 > 3
            || self.octant_depth + self.y_partition_log2 > 3
            || self
                .input_depth
                .iter()
                .chain(&self.output_depth)
                .any(|&d| !(8..=16).contains(&d))
            || (0..2).any(|c| self.output_depth[c] < self.input_depth[c])
        {
            return Err(invalid("invalid HEVC colour mapping sample parameters"));
        }
        if (0..3).any(|c| u32::from(samples[c]) >= 1u32 << self.input_depth[usize::from(c != 0)]) {
            return Err(invalid("HEVC colour mapping sample exceeds input depth"));
        }
        let y = usize::from(samples[0])
            >> (self.input_depth[0] - self.octant_depth - self.y_partition_log2);
        let chroma = |c: usize| {
            if self.octant_depth == 1 {
                usize::from(u32::from(samples[c]) >= self.thresholds[c - 1])
            } else {
                usize::from(samples[c]) >> self.input_depth[1]
            }
        };
        let cell = self.coefficients([y, chroma(1), chroma(2)])?;
        let shift = 10 + self.input_depth[0] - self.output_depth[0];
        let sum: i64 = (0..3)
            .map(|c| i64::from(cell[c][component]) * i64::from(samples[c]))
            .sum();
        let value = ((sum + (1i64 << (shift - 1))) >> shift) + i64::from(cell[3][component]);
        Ok(value.clamp(
            0,
            (1i64 << self.output_depth[usize::from(component != 0)]) - 1,
        ) as u16)
    }
}
