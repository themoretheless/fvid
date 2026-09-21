//! Persistent motion metadata for co-located B-picture prediction.
//! Unlike the working field, references use stable decoded-picture identities.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceMotion {
    pub picture_id: u64,
    /// Index in the reference list of the picture that supplied this motion.
    pub reference_index: u8,
    pub vector: [i16; 2],
}
pub struct ReferenceMotionField {
    width: usize,
    height: usize,
    cells: Vec<[Option<ReferenceMotion>; 2]>,
}
impl ReferenceMotionField {
    pub fn storage_bytes(width: usize, height: usize) -> Result<usize> {
        if width == 0 || height == 0 || width % 16 != 0 || height % 16 != 0 {
            return Err(invalid("invalid reference motion geometry"));
        }
        (width / 4)
            .checked_mul(height / 4)
            .and_then(|n| n.checked_mul(std::mem::size_of::<[Option<ReferenceMotion>; 2]>()))
            .ok_or_else(|| invalid("reference motion storage overflow"))
    }
    pub(super) fn new(
        width: usize,
        height: usize,
        cells: Vec<[Option<ReferenceMotion>; 2]>,
    ) -> Result<Self> {
        Self::storage_bytes(width, height)?;
        if cells.len() != width / 4 * (height / 4) {
            return Err(invalid("reference motion cell count mismatch"));
        }
        Ok(Self {
            width,
            height,
            cells,
        })
    }
    /// Select the stored 4x4 cell containing this coded-luma position.
    pub fn at(&self, position: [usize; 2]) -> Result<[Option<ReferenceMotion>; 2]> {
        if position[0] >= self.width || position[1] >= self.height {
            return Err(invalid("co-located motion position out of range"));
        }
        Ok(self.cells[(position[1] / 4) * (self.width / 4) + position[0] / 4])
    }
    /// H.264's co-located list selection prefers L0 and falls back to L1.
    /// None represents an intra-coded cell, not a missing decoded cell.
    pub fn colocated(&self, position: [usize; 2]) -> Result<Option<ReferenceMotion>> {
        let lists = self.at(position)?;
        Ok(lists[0].or(lists[1]))
    }
    pub fn dimensions(&self) -> [usize; 2] {
        [self.width, self.height]
    }
}
/// Temporal direct maps the co-located reference by identity, never by its old
/// list index. List modification may have changed that index in the B picture.
pub fn map_reference(motion: ReferenceMotion, list0: &[u64]) -> Result<u8> {
    if list0.is_empty() || list0.len() > 32 {
        return Err(invalid("invalid direct reference list"));
    }
    list0
        .iter()
        .position(|&id| id == motion.picture_id)
        .map(|i| i as u8)
        .ok_or_else(|| invalid("co-located reference is absent from list0"))
}

#[cfg(test)]
mod tests {
    use super::super::{avc_motion_field::MotionField, avc_mv::Neighbour};
    use super::*;
    #[test]
    fn snapshots_keep_reference_identity_across_list_reordering() {
        let mut working = MotionField::new(16, 16, 65536).unwrap();
        working
            .store(
                [0, 0],
                [8, 16],
                2,
                [
                    Neighbour::Inter {
                        reference: 1,
                        vector: [12, -4],
                    },
                    Neighbour::NoPrediction,
                ],
            )
            .unwrap();
        working
            .store(
                [8, 0],
                [8, 16],
                2,
                [
                    Neighbour::NoPrediction,
                    Neighbour::Inter {
                        reference: 0,
                        vector: [-8, 2],
                    },
                ],
            )
            .unwrap();
        let stored = working.snapshot([&[100, 200], &[300]], 65536).unwrap();
        let left = stored.colocated([3, 15]).unwrap().unwrap();
        assert_eq!(
            left,
            ReferenceMotion {
                picture_id: 200,
                reference_index: 1,
                vector: [12, -4]
            }
        );
        assert_eq!(map_reference(left, &[200, 100]).unwrap(), 0);
        assert_eq!(map_reference(left, &[100, 200, 200]).unwrap(), 1);
        let right = stored.colocated([8, 0]).unwrap().unwrap();
        assert_eq!(right.picture_id, 300);
        assert_eq!(right.vector, [-8, 2]);
        assert!(map_reference(right, &[100, 200]).is_err());
        assert!(stored.at([16, 0]).is_err());
        assert!(working.snapshot([&[100], &[300]], 65536).is_err());
        assert!(working.snapshot([&[100, 200], &[300]], 1).is_err());
    }
    #[test]
    fn incomplete_and_mixed_slice_fields_are_not_silently_snapshotted() {
        let mut working = MotionField::new(16, 16, 65536).unwrap();
        assert!(working.snapshot([&[], &[]], 65536).is_err());
        working
            .store([0, 0], [8, 16], 0, [Neighbour::NoPrediction; 2])
            .unwrap();
        working
            .store([8, 0], [8, 16], 1, [Neighbour::NoPrediction; 2])
            .unwrap();
        assert!(working.snapshot([&[], &[]], 65536).is_err());
        let mut intra = MotionField::new(16, 16, 65536).unwrap();
        intra
            .store([0, 0], [16, 16], 0, [Neighbour::NoPrediction; 2])
            .unwrap();
        assert_eq!(
            intra
                .snapshot([&[], &[]], 65536)
                .unwrap()
                .colocated([0, 0])
                .unwrap(),
            None
        );
    }
}
