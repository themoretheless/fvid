//! Persistent motion metadata for co-located B-picture prediction.
//! Unlike the working field, references use stable decoded-picture identities.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceMotion {
    pub picture_id: u64,
    /// Index in the reference list of the picture that supplied this motion.
    pub reference_index: u8,
    /// Selected reference field parity; None means a frame reference.
    pub reference_bottom_field: Option<bool>,
    pub vector: [i16; 2],
}
pub struct ReferenceMotionField {
    width: usize,
    height: usize,
    cells: Vec<[Option<ReferenceMotion>; 2]>,
    mbaff_fields: Option<Vec<bool>>,
}
impl ReferenceMotionField {
    pub fn storage_bytes(width: usize, height: usize) -> Result<usize> {
        if width == 0 || height == 0 || width % 16 != 0 || height % 16 != 0 {
            return Err(invalid("invalid reference motion geometry"));
        }
        (width / 4)
            .checked_mul(height / 4)
            .and_then(|n| n.checked_mul(std::mem::size_of::<[Option<ReferenceMotion>; 2]>()))
            .and_then(|n| {
                (width / 16)
                    .checked_mul(height / 32)
                    .and_then(|pairs| n.checked_add(pairs))
            })
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
            mbaff_fields: None,
        })
    }
    pub(super) fn new_mbaff(
        width: usize,
        height: usize,
        cells: Vec<[Option<ReferenceMotion>; 2]>,
        fields: Vec<bool>,
    ) -> Result<Self> {
        if height % 32 != 0 || fields.len() != width / 16 * (height / 32) {
            return Err(invalid("invalid MBAFF reference motion pair geometry"));
        }
        let mut result = Self::new(width, height, cells)?;
        result.mbaff_fields = Some(fields);
        Ok(result)
    }
    /// Resolve a physical frame sample into address-local MBAFF motion storage.
    /// Returns raw field/frame vector units and selected reference-field parity.
    /// Direct prediction must perform its specified co-located conversions.
    pub fn at_mbaff(&self, position: [usize; 2]) -> Result<([Option<ReferenceMotion>; 2], bool)> {
        let fields = self
            .mbaff_fields
            .as_ref()
            .ok_or_else(|| invalid("reference motion is not MBAFF"))?;
        let (address, local) = super::avc_mbaff::owner(
            position,
            self.width / 16,
            self.height / 16,
            true,
            [1, 1],
            |pair| fields.get(pair).copied(),
        )?
        .ok_or_else(|| invalid("MBAFF co-located position out of range"))?;
        Ok((
            self.cells[address * 16 + local[1] / 4 * 4 + local[0] / 4],
            fields[address / 2],
        ))
    }
    pub fn is_mbaff(&self) -> bool {
        self.mbaff_fields.is_some()
    }
    /// Select the stored 4x4 cell containing this coded-luma position.
    pub fn at(&self, position: [usize; 2]) -> Result<[Option<ReferenceMotion>; 2]> {
        if self.is_mbaff() {
            return Err(crate::unsupported(
                "MBAFF co-located motion requires pair-aware selection",
            ));
        }
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
    fn mbaff_snapshot_retains_address_mode_vectors_and_reference_field_parity() {
        for mixed in [false, true] {
            let mut working = MotionField::new(32, 32, 65536).unwrap();
            let ids: Vec<_> = (100..132).collect();
            for address in 0..4 {
                let field = address / 2 == 1 || !mixed;
                for cell in 0..16 {
                    working
                        .store_mbaff(
                            address,
                            [cell % 4 * 4, cell / 4 * 4],
                            [4, 4],
                            address as u32 / 2,
                            field,
                            [
                                Neighbour::Inter {
                                    reference: if field {
                                        if cell == 15 { 63 } else { cell as u8 }
                                    } else {
                                        (cell % 2) as u8
                                    },
                                    vector: [address as i16 * 100 + cell as i16, -(cell as i16)],
                                },
                                Neighbour::NoPrediction,
                            ],
                        )
                        .unwrap();
                }
            }
            let mappings = [
                (0, [ids.as_slice(), &[][..]]),
                (1, [ids.as_slice(), &[][..]]),
            ];
            let bytes = ReferenceMotionField::storage_bytes(32, 32).unwrap();
            let saved = working.snapshot_mbaff_slices(&mappings, bytes).unwrap();
            assert!(saved.is_mbaff());
            assert!(saved.at([0, 0]).is_err());
            assert!(saved.colocated([0, 0]).is_err());
            for y in 0..32 {
                for x in 0..32 {
                    let field = x >= 16 || !mixed;
                    let address = x / 16 * 2 + if field { y % 2 } else { y / 16 };
                    let local_y = if field { y / 2 } else { y % 16 };
                    let cell = local_y / 4 * 4 + x % 16 / 4;
                    let reference = if field {
                        if cell == 15 { 63 } else { cell as u8 }
                    } else {
                        (cell % 2) as u8
                    };
                    let (lists, actual_field) = saved.at_mbaff([x, y]).unwrap();
                    assert_eq!(actual_field, field);
                    let motion = lists[0].unwrap();
                    assert_eq!(
                        motion.picture_id,
                        ids[usize::from(reference) / if field { 2 } else { 1 }]
                    );
                    assert_eq!(motion.reference_index, reference);
                    assert_eq!(
                        motion.vector,
                        [address as i16 * 100 + cell as i16, -(cell as i16)]
                    );
                    assert_eq!(
                        motion.reference_bottom_field,
                        field.then_some((address % 2) ^ (usize::from(reference) % 2) != 0)
                    );
                    assert!(lists[1].is_none());
                }
            }
            assert!(saved.at_mbaff([32, 0]).is_err());
            assert!(saved.at_mbaff([0, 32]).is_err());
            assert!(working.snapshot_mbaff_slices(&mappings, bytes - 1).is_err());
            assert!(
                working
                    .snapshot_mbaff_slices(&mappings[..1], bytes)
                    .is_err()
            );
            assert!(
                working
                    .snapshot_mbaff_slices(&[mappings[0], mappings[0]], bytes)
                    .is_err()
            );
            assert!(
                working
                    .snapshot_mbaff_slices(&[(0, [&ids[..1], &[]]), (1, [&ids[..1], &[]])], bytes)
                    .is_err()
            );
        }
        let mut incomplete = MotionField::new(16, 32, 65536).unwrap();
        incomplete
            .store_mbaff(0, [0, 0], [16, 16], 0, true, [Neighbour::NoPrediction; 2])
            .unwrap();
        assert!(
            incomplete
                .snapshot_mbaff_slices(&[(0, [&[], &[]])], 65536)
                .is_err()
        );
    }
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
                reference_bottom_field: None,
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
