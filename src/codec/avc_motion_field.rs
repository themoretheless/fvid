//! Progressive 4x4 motion storage with slice and decoding-order availability.
use super::avc_mv::{Neighbour, Neighbours, Partition, add_difference, predict};
use crate::{Result, invalid};
#[derive(Clone, Copy)]
struct Cell {
    slice: u32,
    lists: [Neighbour; 2],
}
pub struct MotionField {
    width: usize,
    height: usize,
    cells: Vec<Option<Cell>>,
}
impl MotionField {
    /// Dimensions are coded luma samples. Budget covers the complete field.
    pub fn new(width: usize, height: usize, memory_limit: usize) -> Result<Self> {
        if width == 0 || height == 0 || width % 16 != 0 || height % 16 != 0 {
            return Err(invalid("invalid AVC motion-field geometry"));
        }
        let count = (width / 4)
            .checked_mul(height / 4)
            .ok_or_else(|| invalid("AVC motion-field size overflow"))?;
        if count
            .checked_mul(std::mem::size_of::<Option<Cell>>())
            .is_none_or(|n| n > memory_limit)
        {
            return Err(invalid("AVC motion field exceeds memory budget"));
        }
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate AVC motion field"))?;
        cells.resize(count, None);
        Ok(Self {
            width: width / 4,
            height: height / 4,
            cells,
        })
    }
    /// Freeze a complete picture's vectors for future co-located prediction.
    /// Slice-local indices are resolved through the supplied list identities.
    /// This entry point supports one reference-list mapping for the picture;
    /// callers must not use it for slices with different list modifications.
    pub fn snapshot(
        &self,
        references: [&[u64]; 2],
        memory_limit: usize,
    ) -> Result<super::avc_reference_motion::ReferenceMotionField> {
        use super::avc_reference_motion::{ReferenceMotion, ReferenceMotionField};
        if references.iter().any(|list| list.len() > 32) {
            return Err(invalid("invalid snapshot reference list"));
        }
        let bytes = ReferenceMotionField::storage_bytes(self.width * 4, self.height * 4)?;
        if bytes > memory_limit {
            return Err(invalid("reference motion snapshot exceeds budget"));
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(self.cells.len())
            .map_err(|_| invalid("reference motion allocation failed"))?;
        let mut slice = None;
        for cell in &self.cells {
            let cell = cell.ok_or_else(|| invalid("cannot snapshot incomplete motion field"))?;
            if slice.is_some_and(|previous| previous != cell.slice) {
                return Err(invalid("motion snapshot needs one slice reference mapping"));
            }
            slice = Some(cell.slice);
            let mut stored = [None; 2];
            for list in 0..2 {
                stored[list] = match cell.lists[list] {
                    Neighbour::NoPrediction => None,
                    Neighbour::Inter { reference, vector } => Some(ReferenceMotion {
                        picture_id: *references[list]
                            .get(reference as usize)
                            .ok_or_else(|| invalid("snapshot reference index out of range"))?,
                        reference_index: reference,
                        vector,
                    }),
                    Neighbour::Unavailable => {
                        return Err(invalid("cannot snapshot unavailable motion"));
                    }
                };
            }
            output.push(stored);
        }
        ReferenceMotionField::new(self.width * 4, self.height * 4, output)
    }
    fn region(&self, origin: [usize; 2], size: [usize; 2]) -> Result<([usize; 2], [usize; 2])> {
        if ![4, 8, 16].contains(&size[0])
            || ![4, 8, 16].contains(&size[1])
            || origin.iter().any(|n| n % 4 != 0)
        {
            return Err(invalid("invalid AVC motion partition"));
        }
        let at = [origin[0] / 4, origin[1] / 4];
        let extent = [size[0] / 4, size[1] / 4];
        if at[0] > self.width - extent[0]
            || at[1] > self.height - extent[1]
            || at[0] % 4 + extent[0] > 4
            || at[1] % 4 + extent[1] > 4
        {
            return Err(invalid("AVC motion partition crosses macroblock boundary"));
        }
        Ok((at, extent))
    }
    fn get(&self, x: Option<usize>, y: Option<usize>, slice: u32, list: usize) -> Neighbour {
        let (Some(x), Some(y)) = (x, y) else {
            return Neighbour::Unavailable;
        };
        if x >= self.width || y >= self.height {
            return Neighbour::Unavailable;
        }
        self.cells[y * self.width + x]
            .filter(|c| c.slice == slice)
            .map_or(Neighbour::Unavailable, |c| c.lists[list])
    }
    pub fn neighbours(
        &self,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        list: usize,
    ) -> Result<Neighbours> {
        if list > 1 {
            return Err(invalid("invalid AVC reference list"));
        }
        let ([x, y], [w, _]) = self.region(origin, size)?;
        Ok(Neighbours {
            left: self.get(x.checked_sub(1), Some(y), slice, list),
            top: self.get(Some(x), y.checked_sub(1), slice, list),
            top_right: self.get(Some(x + w), y.checked_sub(1), slice, list),
            top_left: self.get(x.checked_sub(1), y.checked_sub(1), slice, list),
        })
    }
    /// Publish a completely decoded partition. Available intra/unused-list cells
    /// use NoPrediction, never Unavailable. Duplicate writes are rejected.
    pub fn store(
        &mut self,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        lists: [Neighbour; 2],
    ) -> Result<()> {
        let ([x, y], [w, h]) = self.region(origin, size)?;
        if lists.iter().any(|n| {
            matches!(n, Neighbour::Unavailable)
                || matches!(n,Neighbour::Inter{reference,..} if *reference>31)
        }) {
            return Err(invalid("invalid decoded AVC motion value"));
        }
        for row in y..y + h {
            for col in x..x + w {
                if self.cells[row * self.width + col].is_some() {
                    return Err(invalid("AVC motion partition already decoded"));
                }
            }
        }
        for row in y..y + h {
            for col in x..x + w {
                self.cells[row * self.width + col] = Some(Cell { slice, lists });
            }
        }
        Ok(())
    }
    /// Derive both non-direct list vectors, then publish the partition atomically.
    /// P-skip/B-direct use their own derivation and call store with final vectors.
    pub fn decode(
        &mut self,
        origin: [usize; 2],
        size: [usize; 2],
        slice: u32,
        kind: Partition,
        references: [Option<u8>; 2],
        differences: [[i32; 2]; 2],
    ) -> Result<[Neighbour; 2]> {
        let mut lists = [Neighbour::NoPrediction; 2];
        for list in 0..2 {
            if let Some(reference) = references[list] {
                let prediction =
                    predict(reference, kind, self.neighbours(origin, size, slice, list)?)?;
                lists[list] = Neighbour::Inter {
                    reference,
                    vector: add_difference(prediction, differences[list])?,
                };
            }
        }
        self.store(origin, size, slice, lists)?;
        Ok(lists)
    }
    /// Decode an entire non-direct macroblock in bitstream partition order.
    /// Any error restores all sixteen cells, including earlier partitions.
    pub fn decode_macroblock(
        &mut self,
        origin: [usize; 2],
        slice: u32,
        parts: &[super::avc_inter::Partition],
    ) -> Result<Vec<[Neighbour; 2]>> {
        self.decode_macroblock_with_direct(origin, slice, parts, None)
    }
    /// Decode mixed explicit/direct partitions, rolling back the entire macroblock
    /// on failure. Direct predictors always use the original macroblock neighbours.
    pub fn decode_macroblock_with_direct(
        &mut self,
        origin: [usize; 2],
        slice: u32,
        parts: &[super::avc_inter::Partition],
        direct: Option<&super::avc_direct::DirectPrediction<'_>>,
    ) -> Result<Vec<[Neighbour; 2]>> {
        use super::avc_inter::Prediction;
        if origin.iter().any(|n| n % 16 != 0) || parts.is_empty() || parts.len() > 16 {
            return Err(invalid("invalid AVC inter macroblock partitions"));
        }
        let ([x, y], _) = self.region(origin, [16, 16])?;
        let previous: [Option<Cell>; 16] =
            std::array::from_fn(|i| self.cells[(y + i / 4) * self.width + x + i % 4]);
        if previous.iter().any(Option::is_some) {
            return Err(invalid("AVC macroblock already has motion data"));
        }
        let direct_neighbours = [
            self.neighbours(origin, [16, 16], slice, 0)?,
            self.neighbours(origin, [16, 16], slice, 1)?,
        ];
        let decoded = (|| {
            let mut result = Vec::with_capacity(parts.len());
            for p in parts {
                if p.origin.iter().any(|&n| n >= 16) {
                    return Err(invalid("AVC partition origin outside macroblock"));
                }
                if p.prediction == Prediction::Direct {
                    if p.size != [4, 4] || p.references != [None; 2] || p.differences != [[0; 2]; 2]
                    {
                        return Err(invalid("invalid direct partition syntax"));
                    }
                    let at = [
                        origin[0] + usize::from(p.origin[0]),
                        origin[1] + usize::from(p.origin[1]),
                    ];
                    let vectors = direct
                        .ok_or_else(|| invalid("missing B-direct context"))?
                        .derive(at, direct_neighbours)?;
                    self.store(at, [4, 4], slice, vectors)?;
                    result.push(vectors);
                    continue;
                }
                let expected = match p.prediction {
                    Prediction::L0 => [true, false],
                    Prediction::L1 => [false, true],
                    Prediction::Bi => [true, true],
                    Prediction::Direct => {
                        return Err(invalid("B-direct motion derivation is not connected"));
                    }
                };
                if [p.references[0].is_some(), p.references[1].is_some()] != expected {
                    return Err(invalid(
                        "AVC partition references disagree with prediction mode",
                    ));
                }
                let size = p.size.map(usize::from);
                let kind = match size {
                    [16, 8] => {
                        if p.origin[1] == 0 {
                            Partition::Top16x8
                        } else {
                            Partition::Bottom16x8
                        }
                    }
                    [8, 16] => {
                        if p.origin[0] == 0 {
                            Partition::Left8x16
                        } else {
                            Partition::Right8x16
                        }
                    }
                    _ => Partition::Median,
                };
                result.push(self.decode(
                    [
                        origin[0] + usize::from(p.origin[0]),
                        origin[1] + usize::from(p.origin[1]),
                    ],
                    size,
                    slice,
                    kind,
                    p.references,
                    p.differences,
                )?);
            }
            if (0..16).any(|i| self.cells[(y + i / 4) * self.width + x + i % 4].is_none()) {
                return Err(invalid("AVC motion partitions leave gaps"));
            }
            Ok(result)
        })();
        if decoded.is_err() {
            for (i, cell) in previous.into_iter().enumerate() {
                self.cells[(y + i / 4) * self.width + x + i % 4] = cell;
            }
        }
        decoded
    }
    pub fn decode_p_skip(&mut self, origin: [usize; 2], slice: u32) -> Result<[i16; 2]> {
        if origin.iter().any(|n| n % 16 != 0) {
            return Err(invalid("unaligned AVC P-skip macroblock"));
        }
        let vector = super::avc_mv::p_skip(self.neighbours(origin, [16, 16], slice, 0)?)?;
        self.store(
            origin,
            [16, 16],
            slice,
            [
                Neighbour::Inter {
                    reference: 0,
                    vector,
                },
                Neighbour::NoPrediction,
            ],
        )?;
        Ok(vector)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prediction_order_slice_isolation_and_atomic_failure() {
        let mut field = MotionField::new(32, 16, 8192).unwrap();
        let first = field
            .decode(
                [0, 0],
                [16, 16],
                1,
                Partition::Median,
                [Some(0), None],
                [[8, -4], [0, 0]],
            )
            .unwrap();
        let n = field.neighbours([16, 0], [16, 16], 1, 0).unwrap();
        assert_eq!(n.left, first[0]);
        assert_eq!(
            field.neighbours([16, 0], [16, 16], 2, 0).unwrap().left,
            Neighbour::Unavailable
        );
        assert!(
            field
                .decode(
                    [16, 0],
                    [16, 16],
                    1,
                    Partition::Median,
                    [Some(0), None],
                    [[i32::MAX, 0], [0, 0]]
                )
                .is_err()
        );
        let second = field
            .decode(
                [16, 0],
                [16, 16],
                1,
                Partition::Median,
                [Some(0), None],
                [[1, 2], [0, 0]],
            )
            .unwrap();
        assert_eq!(
            second[0],
            Neighbour::Inter {
                reference: 0,
                vector: [9, -2]
            }
        );
        assert!(field.store([16, 0], [16, 16], 1, first).is_err());
        assert!(MotionField::new(16, 16, 1).is_err());
        assert!(field.neighbours([usize::MAX, 0], [16, 16], 1, 0).is_err());
    }
}

#[cfg(test)]
mod macroblock_tests {
    use super::super::{avc_inter::read_prediction, avc_slice::SliceType, bits::BitReader};
    use super::*;
    #[test]
    fn entropy_partitions_connect_to_motion_and_rollback() {
        let mut field = MotionField::new(32, 16, 8192).unwrap();
        // P_8x8ref0, all four subtypes 8x8, all motion differences zero.
        let mut bits = BitReader::new(&[0xff, 0xf0]);
        let parts = read_prediction(&mut bits, SliceType::P, 4, [1, 0]).unwrap();
        let mut invalid = parts.clone();
        invalid[3].differences[0][0] = i32::MAX;
        assert!(field.decode_macroblock([0, 0], 0, &invalid).is_err());
        let decoded = field.decode_macroblock([0, 0], 0, &parts).unwrap();
        assert_eq!(decoded.len(), 4);
        assert!(decoded.iter().all(|p| p[0]
            == Neighbour::Inter {
                reference: 0,
                vector: [0, 0]
            }));
        assert_eq!(field.decode_p_skip([16, 0], 0).unwrap(), [0, 0]);
    }
}
