//! Progressive B-direct derivation using retained reference-picture motion.
use super::{
    avc_mv::{Neighbour, Neighbours, spatial_direct, temporal_direct},
    avc_reference_motion::ReferenceMotionField,
    avc_references::FrameReference,
};
use crate::{Result, invalid};

pub struct DirectPrediction<'a> {
    pub spatial: bool,
    pub inference8: bool,
    pub current_poc: i32,
    pub list0: &'a [FrameReference],
    pub list1: &'a [FrameReference],
    /// Motion of list1[0]; None means an entirely intra-coded reference picture.
    pub colocated: Option<&'a ReferenceMotionField>,
}
impl DirectPrediction<'_> {
    pub fn derive(
        &self,
        position: [usize; 2],
        neighbours: [Neighbours; 2],
    ) -> Result<[Neighbour; 2]> {
        if self.list0.is_empty()
            || self.list1.is_empty()
            || self.list0.len() > 32
            || self.list1.len() > 32
            || position.iter().any(|n| n % 4 != 0)
        {
            return Err(invalid("invalid progressive direct prediction inputs"));
        }
        // With inference enabled, block indices 0,5,10,15 select the outer
        // corner of each 8x8 group, not that group's top-left 4x4 cell.
        let selected = if self.inference8 {
            position.map(|p| p / 16 * 16 + if p % 16 < 8 { 0 } else { 12 })
        } else {
            position
        };
        let col = self
            .colocated
            .map(|field| field.colocated(selected))
            .transpose()?
            .flatten();
        let result = if self.spatial {
            spatial_direct(
                neighbours,
                col.map(|m| (m.reference_index, m.vector)),
                self.list1[0].long_term_index.is_some(),
            )?
        } else {
            let index = match col {
                None => 0,
                Some(motion) => self
                    .list0
                    .iter()
                    .position(|r| r.id == motion.picture_id)
                    .ok_or_else(|| invalid("co-located reference is absent from list0"))?,
            };
            let vectors = temporal_direct(
                col.map_or([0; 2], |m| m.vector),
                self.current_poc.into(),
                self.list0[index].poc.into(),
                self.list1[0].poc.into(),
                self.list0[index].long_term_index.is_some(),
            )?;
            [
                Neighbour::Inter {
                    reference: index as u8,
                    vector: vectors[0],
                },
                Neighbour::Inter {
                    reference: 0,
                    vector: vectors[1],
                },
            ]
        };
        for (list, entries) in [self.list0, self.list1].into_iter().enumerate() {
            if matches!(result[list], Neighbour::Inter { reference, .. } if reference as usize >= entries.len())
            {
                return Err(invalid("derived direct reference exceeds active list"));
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        avc_inter::{MacroblockType, macroblock_type},
        avc_motion_field::MotionField,
        avc_slice::SliceType,
    };
    use super::*;
    fn reference(id: u64, poc: i32) -> FrameReference {
        FrameReference {
            id,
            frame_num: 0,
            poc,
            long_term_index: None,
        }
    }
    #[test]
    fn temporal_direct_corner_selection_identity_mapping_and_atomic_rollback() {
        let mut previous = MotionField::new(16, 16, 65536).unwrap();
        for y in 0..4 {
            for x in 0..4 {
                previous
                    .store(
                        [x * 4, y * 4],
                        [4, 4],
                        0,
                        [
                            Neighbour::Inter {
                                reference: 0,
                                vector: [(4 * (y * 4 + x)) as i16, -8],
                            },
                            Neighbour::NoPrediction,
                        ],
                    )
                    .unwrap();
            }
        }
        let stored = previous.snapshot([&[42], &[]], 65536).unwrap();
        let list0 = [reference(99, -4), reference(42, 0)];
        let list1 = [reference(70, 8)];
        let mut context = DirectPrediction {
            spatial: false,
            inference8: true,
            current_poc: 2,
            list0: &list0,
            list1: &list1,
            colocated: Some(&stored),
        };
        let MacroblockType::Inter { mut partitions, .. } =
            macroblock_type(SliceType::B, 0).unwrap()
        else {
            panic!()
        };
        let mut field = MotionField::new(16, 16, 65536).unwrap();
        // A late malformed partition must undo all previously published vectors.
        let saved = partitions[15];
        partitions[15].origin = [16, 12];
        assert!(
            field
                .decode_macroblock_with_direct([0, 0], 0, &partitions, Some(&context))
                .is_err()
        );
        assert!(field.snapshot([&[99, 42], &[70]], 65536).is_err());
        partitions[15] = saved;
        let vectors = field
            .decode_macroblock_with_direct([0, 0], 0, &partitions, Some(&context))
            .unwrap();
        for (p, actual) in partitions.iter().zip(vectors) {
            let x = if p.origin[0] < 8 { 0 } else { 3 };
            let y = if p.origin[1] < 8 { 0 } else { 3 };
            let k = y * 4 + x;
            assert_eq!(
                actual,
                [
                    Neighbour::Inter {
                        reference: 1,
                        vector: [k, -2]
                    },
                    Neighbour::Inter {
                        reference: 0,
                        vector: [-3 * k, 6]
                    }
                ]
            );
        }
        let frozen = field.snapshot([&[99, 42], &[70]], 65536).unwrap();
        assert_eq!(frozen.at([15, 15]).unwrap()[0].unwrap().picture_id, 42);
        context.inference8 = false;
        let empty = MotionField::new(16, 16, 65536).unwrap();
        let neighbours = [empty.neighbours([0, 0], [16, 16], 0, 0).unwrap(); 2];
        assert_eq!(
            context.derive([8, 8], neighbours).unwrap()[0],
            Neighbour::Inter {
                reference: 1,
                vector: [10, -2]
            }
        );
        context.list0 = &list0[..1];
        assert!(context.derive([0, 0], neighbours).is_err());
        context.colocated = None;
        assert_eq!(
            context.derive([0, 0], neighbours).unwrap(),
            [Neighbour::Inter {
                reference: 0,
                vector: [0; 2]
            }; 2]
        );
    }
}
