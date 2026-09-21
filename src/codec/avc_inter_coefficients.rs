//! Progressive 4:2:0 CAVLC inter coefficient parsing.
use super::{
    avc_macroblock::luma_block_xy, avc_transform::inverse_scan_4x4,
    avc_transform8::inverse_scan_8x8, bits::BitReader, cavlc::read_residual,
};
use crate::{Result, invalid};
#[derive(Clone, Copy, Default)]
pub struct CoefficientNeighbours {
    pub luma_left: [Option<u8>; 4],
    pub luma_top: [Option<u8>; 4],
    pub chroma_left: [[Option<u8>; 2]; 2],
    pub chroma_top: [[Option<u8>; 2]; 2],
}
pub struct InterCoefficients {
    pub luma4: [[i32; 16]; 16],
    pub luma8: [[i32; 64]; 4],
    pub chroma_dc: [[i32; 4]; 2],
    pub chroma_ac: [[[i32; 16]; 4]; 2],
    /// Raster counts for propagation into the next macroblocks' nC context.
    pub luma_counts: [u8; 16],
    pub chroma_counts: [[u8; 4]; 2],
}
fn context(left: Option<u8>, top: Option<u8>) -> i8 {
    match (left, top) {
        (Some(a), Some(b)) => ((a + b + 1) / 2) as i8,
        (Some(a), None) | (None, Some(a)) => a as i8,
        _ => 0,
    }
}
/// Both bit cursor and caller-owned neighbour state remain unchanged on error.
/// Returned coefficients are raster ordered; only the selected luma size is used.
pub fn read_inter_coefficients(
    bits: &mut BitReader<'_>,
    pattern: u8,
    transform8: bool,
    neighbours: CoefficientNeighbours,
) -> Result<InterCoefficients> {
    if pattern > 47
        || neighbours
            .luma_left
            .iter()
            .chain(&neighbours.luma_top)
            .chain(neighbours.chroma_left.iter().flatten())
            .chain(neighbours.chroma_top.iter().flatten())
            .any(|n| n.is_some_and(|v| v > 16))
    {
        return Err(invalid("invalid AVC coefficient context"));
    }
    let mut b = bits.clone();
    let mut out = InterCoefficients {
        luma4: [[0; 16]; 16],
        luma8: [[0; 64]; 4],
        chroma_dc: [[0; 4]; 2],
        chroma_ac: [[[0; 16]; 4]; 2],
        luma_counts: [0; 16],
        chroma_counts: [[0; 4]; 2],
    };
    for block in 0..16 {
        let (x, y) = luma_block_xy(block)?;
        if pattern & (1 << (block / 4)) == 0 {
            continue;
        }
        let left = if x == 0 {
            neighbours.luma_left[y]
        } else {
            Some(out.luma_counts[y * 4 + x - 1])
        };
        let top = if y == 0 {
            neighbours.luma_top[x]
        } else {
            Some(out.luma_counts[(y - 1) * 4 + x])
        };
        let residual = read_residual(&mut b, context(left, top), 16)?;
        out.luma_counts[y * 4 + x] = residual.total_coefficients;
        if transform8 {
            for i in 0..16 {
                out.luma8[block / 4][4 * i + block % 4] = residual.coefficients[i];
            }
        } else {
            out.luma4[y * 4 + x] = inverse_scan_4x4(&residual.coefficients, false);
        }
    }
    if transform8 {
        for block in &mut out.luma8 {
            *block = inverse_scan_8x8(block, false);
        }
    }
    if pattern >> 4 != 0 {
        for component in 0..2 {
            out.chroma_dc[component]
                .copy_from_slice(&read_residual(&mut b, -1, 4)?.coefficients[..4]);
        }
    }
    if pattern >> 4 == 2 {
        for component in 0..2 {
            for block in 0..4 {
                let (x, y) = (block % 2, block / 2);
                let left = if x == 0 {
                    neighbours.chroma_left[component][y]
                } else {
                    Some(out.chroma_counts[component][block - 1])
                };
                let top = if y == 0 {
                    neighbours.chroma_top[component][x]
                } else {
                    Some(out.chroma_counts[component][block - 2])
                };
                let residual = read_residual(&mut b, context(left, top), 15)?;
                out.chroma_counts[component][block] = residual.total_coefficients;
                let mut levels = [0; 16];
                levels[1..].copy_from_slice(&residual.coefficients[..15]);
                out.chroma_ac[component][block] = inverse_scan_4x4(&levels, false);
            }
        }
    }
    *bits = b;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coded_zero_blocks_and_truncation() {
        for transform8 in [false, true] {
            let mut b = BitReader::new(&[0xff, 0xff]);
            let c =
                read_inter_coefficients(&mut b, 15, transform8, CoefficientNeighbours::default())
                    .unwrap();
            assert_eq!(b.position(), 16);
            assert_eq!(c.luma_counts, [0; 16]);
            assert_eq!(c.luma4, [[0; 16]; 16]);
            assert_eq!(c.luma8, [[0; 64]; 4]);
            let mut b = BitReader::new(&[0xff]);
            assert!(
                read_inter_coefficients(&mut b, 15, transform8, CoefficientNeighbours::default())
                    .is_err()
            );
            assert_eq!(b.position(), 0);
        }
        let mut b = BitReader::new(&[]);
        read_inter_coefficients(&mut b, 0, false, CoefficientNeighbours::default()).unwrap();
        assert_eq!(b.position(), 0);
    }
}

#[cfg(test)]
mod nonzero_tests {
    use super::*;
    #[test]
    fn one_dc_coefficient_updates_context_and_raster_output() {
        // First 4x4: coeff_token(1,1)=01, sign=0, total_zeros=0 -> 1;
        // remaining three coded 4x4 blocks: coeff_token(0,0)=1 each.
        let mut b = BitReader::new(&[0x5e]);
        let c =
            read_inter_coefficients(&mut b, 1, false, CoefficientNeighbours::default()).unwrap();
        assert_eq!(b.position(), 7);
        assert_eq!(c.luma_counts[0], 1);
        assert_eq!(c.luma_counts.iter().sum::<u8>(), 1);
        assert_eq!(c.luma4[0][0], 1);
    }
}
