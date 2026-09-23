//! H.265 scaling_list_data and diagonal-scan expansion (7.4.5).
use super::bits::BitReader;
use crate::{Result, invalid};
const INTRA: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 16, 17, 16, 17, 18, 17, 18, 18, 17, 18, 21, 19, 20,
    21, 20, 19, 21, 24, 22, 22, 24, 24, 22, 22, 24, 25, 25, 27, 30, 27, 25, 25, 29, 31, 35, 35, 31,
    29, 36, 41, 44, 41, 36, 47, 54, 54, 47, 65, 70, 65, 88, 88, 115,
];
const INTER: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 18, 18, 18, 18, 18, 18, 20, 20, 20,
    20, 20, 20, 20, 24, 24, 24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 28, 28, 28, 28, 28,
    28, 33, 33, 33, 33, 33, 41, 41, 41, 41, 54, 54, 54, 71, 71, 91,
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Matrix {
    /// First 16 entries for sizeId=0; otherwise all 64. Diagonal scan order.
    pub coefficients: [u8; 64],
    pub dc: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScalingLists {
    matrices: [[Matrix; 6]; 4],
}
impl Default for ScalingLists {
    fn default() -> Self {
        Self {
            matrices: std::array::from_fn(|size| {
                std::array::from_fn(|id| Matrix {
                    coefficients: if size == 0 {
                        [16; 64]
                    } else if id < 3 {
                        INTRA
                    } else {
                        INTER
                    },
                    dc: 16,
                })
            }),
        }
    }
}
impl ScalingLists {
    pub fn flat() -> Self {
        Self {
            matrices: [[Matrix {
                coefficients: [16; 64],
                dc: 16,
            }; 6]; 4],
        }
    }
    pub fn read(b: &mut BitReader<'_>) -> Result<Self> {
        let mut output = Self::default();
        for size in 0..4 {
            let step = if size == 3 { 3 } else { 1 };
            for id in (0..6).step_by(step) {
                if !b.bit()? {
                    let delta = b.unsigned_golomb()? as usize;
                    if delta > id / step {
                        return Err(invalid(
                            "HEVC scaling-list predictor outside prior matrices",
                        ));
                    }
                    if delta != 0 {
                        output.matrices[size][id] = output.matrices[size][id - delta * step];
                    }
                } else {
                    let mut next = 8i32;
                    if size > 1 {
                        let dc = b.signed_golomb()?;
                        if !(-7..=247).contains(&dc) {
                            return Err(invalid("HEVC scaling-list DC out of range"));
                        }
                        next = dc + 8;
                        output.matrices[size][id].dc = next as u8;
                    }
                    for i in 0..if size == 0 { 16 } else { 64 } {
                        let delta = b.signed_golomb()?;
                        if !(-128..=127).contains(&delta) {
                            return Err(invalid("HEVC scaling-list delta out of range"));
                        }
                        next = (next + delta + 256) % 256;
                        if next == 0 {
                            return Err(invalid("zero HEVC scaling coefficient"));
                        }
                        output.matrices[size][id].coefficients[i] = next as u8;
                    }
                }
            }
        }
        // 4:4:4 32x32 chroma matrices inherit the corresponding 16x16 list.
        for id in [1, 2, 4, 5] {
            output.matrices[3][id] = output.matrices[2][id];
        }
        Ok(output)
    }
    pub fn matrix(&self, size: usize, id: usize) -> Result<&Matrix> {
        self.matrices
            .get(size)
            .and_then(|s| s.get(id))
            .ok_or_else(|| invalid("invalid HEVC scaling matrix index"))
    }
    pub fn factor(&self, size: usize, id: usize, x: usize, y: usize) -> Result<u8> {
        let matrix = self.matrix(size, id)?;
        let side = 1usize << (size + 2);
        if x >= side || y >= side {
            return Err(invalid("HEVC scaling coordinate outside matrix"));
        }
        if size > 1 && x == 0 && y == 0 {
            return Ok(matrix.dc);
        }
        let replication = if size > 1 { 1 << (size - 1) } else { 1 };
        let (x, y) = (x / replication, y / replication);
        let n = if size == 0 { 4 } else { 8 };
        let diagonal = x + y;
        let index = if diagonal < n {
            diagonal * (diagonal + 1) / 2 + x
        } else {
            let remaining = 2 * n - 1 - diagonal;
            n * n - remaining * (remaining + 1) / 2 + x - (diagonal + 1 - n)
        };
        Ok(matrix.coefficients[index])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_diagonal_index_matches_scan_at_every_coordinate() {
        let mut lists = ScalingLists::default();
        for matrix in lists.matrices.iter_mut().flatten() {
            matrix.coefficients = std::array::from_fn(|index| index as u8 + 1);
            matrix.dc = 97;
        }
        for size in 0..4 {
            let n = if size == 0 { 4 } else { 8 };
            let replication = if size > 1 { 1 << (size - 1) } else { 1 };
            for id in 0..6 {
                let matrix = lists.matrix(size, id).unwrap();
                let mut index = 0;
                for sum in 0..2 * n - 1 {
                    for x in 0..n {
                        if sum < x || sum - x >= n {
                            continue;
                        }
                        for dx in 0..replication {
                            for dy in 0..replication {
                                let xx = x * replication + dx;
                                let yy = (sum - x) * replication + dy;
                                let expected = if size > 1 && xx == 0 && yy == 0 {
                                    matrix.dc
                                } else {
                                    matrix.coefficients[index]
                                };
                                assert_eq!(lists.factor(size, id, xx, yy).unwrap(), expected);
                            }
                        }
                        index += 1;
                    }
                }
            }
        }
    }
    #[test]
    fn explicit_deltas_copy_dc_wraparound_and_chroma_inference() {
        let data = [
            143, 73, 36, 146, 73, 36, 145, 42, 170, 171, 12, 47, 255, 255, 255, 255, 255, 255, 255,
            234, 177, 229, 255, 255, 255, 255, 255, 255, 255, 254, 80,
        ];
        let mut bits = BitReader::new(&data);
        let lists = ScalingLists::read(&mut bits).unwrap();
        bits.finish_rbsp().unwrap();
        assert_eq!(
            &lists.matrix(0, 0).unwrap().coefficients[..16],
            &(1..=16).collect::<Vec<_>>()
        );
        assert_eq!(lists.matrix(0, 0).unwrap(), lists.matrix(0, 1).unwrap());
        assert_eq!(lists.factor(0, 0, 0, 1).unwrap(), 2);
        assert_eq!(lists.factor(0, 0, 1, 0).unwrap(), 3);
        assert_eq!(lists.factor(2, 1, 0, 0).unwrap(), 20);
        assert_eq!(lists.factor(2, 1, 1, 0).unwrap(), 21);
        assert_eq!(lists.factor(3, 1, 0, 0).unwrap(), 20);
        assert_eq!(lists.factor(3, 1, 31, 31).unwrap(), 21);
        assert_eq!(lists.factor(3, 3, 0, 0).unwrap(), 1);
        assert_eq!(lists.factor(3, 3, 31, 31).unwrap(), 255);
        for end in 0..data.len() - 1 {
            assert!(ScalingLists::read(&mut BitReader::new(&data[..end])).is_err());
        }
    }
    #[test]
    fn defaults_flat_and_diagonal_expansion() {
        // Twenty matrices, each pred_mode=0 and delta=0, then trailing bit.
        let data = [0x55, 0x55, 0x55, 0x55, 0x55, 0x80];
        let mut bits = BitReader::new(&data);
        let lists = ScalingLists::read(&mut bits).unwrap();
        assert_eq!(lists, ScalingLists::default());
        bits.finish_rbsp().unwrap();
        assert_eq!(lists.factor(1, 0, 7, 7).unwrap(), 115);
        assert_eq!(lists.factor(1, 3, 7, 7).unwrap(), 91);
        assert_eq!(lists.factor(3, 0, 31, 31).unwrap(), 115);
        assert_eq!(lists.factor(2, 0, 0, 0).unwrap(), 16);
        assert_eq!(ScalingLists::flat().factor(3, 0, 31, 31).unwrap(), 16);
        assert!(lists.factor(4, 0, 0, 0).is_err());
        assert!(lists.factor(1, 0, 8, 0).is_err());
        assert!(ScalingLists::read(&mut BitReader::new(&[0x20])).is_err());
    }
}
