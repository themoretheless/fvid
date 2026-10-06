//! Progressive and explicit MBAFF B-direct derivation from retained motion.
use super::{
    avc_mv::{Neighbour, Neighbours, spatial_direct, spatial_direct_for_field, temporal_direct},
    avc_poc::FieldOrder,
    avc_reference_motion::{ColocatedScale, ReferenceMotionField, map_reference_mbaff},
    avc_references::FrameReference,
};
use crate::{Result, invalid};

/// MBAFF frame references retain both field POCs separately from list ordering.
/// This context never substitutes a frame's minimum for a selected field POC.
pub struct MbaffDirectPrediction<'a> {
    pub spatial: bool,
    pub inference8: bool,
    pub current_order: FieldOrder,
    pub list0: &'a [FrameReference],
    pub list1: &'a [FrameReference],
    pub list0_orders: &'a [FieldOrder],
    pub list1_orders: &'a [FieldOrder],
    pub colocated: Option<&'a ReferenceMotionField>,
}
fn frame_order(order: FieldOrder) -> Result<[i32; 2]> {
    match (order.top, order.bottom) {
        (Some(top), Some(bottom)) => Ok([top, bottom]),
        _ => Err(invalid("MBAFF direct requires both field POCs")),
    }
}
impl MbaffDirectPrediction<'_> {
    pub fn derive(
        &self,
        address: usize,
        local: [usize; 2],
        field: bool,
        neighbours: [Neighbours; 2],
    ) -> Result<[Neighbour; 2]> {
        if local.iter().any(|&v| v >= 16 || v % 4 != 0) {
            return Err(invalid("invalid MBAFF direct local position"));
        }
        let current = frame_order(self.current_order)?;
        for (list, orders) in [
            (self.list0, self.list0_orders),
            (self.list1, self.list1_orders),
        ] {
            if list.is_empty() || list.len() > 32 || orders.len() != list.len() {
                return Err(invalid("invalid MBAFF direct reference metadata"));
            }
            for (reference, order) in list.iter().zip(orders) {
                let [top, bottom] = frame_order(*order)?;
                if reference.poc != top.min(bottom) {
                    return Err(invalid("MBAFF direct reference POC mismatch"));
                }
            }
        }
        let selected = if self.inference8 {
            local.map(|v| if v < 8 { 0 } else { 12 })
        } else {
            local
        };
        let col = self
            .colocated
            .map(|motion| {
                motion.colocated_mbaff(
                    address,
                    selected,
                    field,
                    current[0].min(current[1]),
                    frame_order(self.list1_orders[0])?,
                )
            })
            .transpose()?;
        let raw = col.and_then(|c| c.motion);
        let scale = col.map_or(ColocatedScale::Same, |c| c.scale);
        let source_field = match scale {
            ColocatedScale::Same => field,
            ColocatedScale::FrameToField => false,
            ColocatedScale::FieldToFrame => true,
        };
        let result = if self.spatial {
            spatial_direct_for_field(
                neighbours,
                raw.map(|m| (m.reference_index, m.vector)),
                self.list1[0].long_term_index.is_some(),
                field,
                source_field,
            )?
        } else {
            let mut ids = [0u64; 32];
            for (target, reference) in ids.iter_mut().zip(self.list0) {
                *target = reference.id;
            }
            let index = raw
                .map(|motion| {
                    map_reference_mbaff(motion, &ids[..self.list0.len()], scale, address % 2 != 0)
                })
                .transpose()?
                .unwrap_or(0);
            let frame_index = usize::from(index) / if field { 2 } else { 1 };
            let bottom = address % 2;
            let (poc, poc0, poc1) = if field {
                (
                    current[bottom],
                    frame_order(self.list0_orders[frame_index])?[bottom ^ (usize::from(index) % 2)],
                    frame_order(self.list1_orders[0])?[bottom],
                )
            } else {
                (
                    self.current_order.picture(),
                    self.list0[frame_index].poc,
                    self.list1[0].poc,
                )
            };
            let vector = scale.temporal_vector(raw.map_or([0; 2], |m| m.vector))?;
            let vectors = temporal_direct(
                vector,
                poc.into(),
                poc0.into(),
                poc1.into(),
                self.list0[frame_index].long_term_index.is_some(),
            )?;
            [
                Neighbour::Inter {
                    reference: index,
                    vector: vectors[0],
                },
                Neighbour::Inter {
                    reference: 0,
                    vector: vectors[1],
                },
            ]
        };
        for (list, entries) in [self.list0, self.list1].into_iter().enumerate() {
            if matches!(result[list], Neighbour::Inter { reference, .. } if usize::from(reference) >= entries.len() * if field { 2 } else { 1 })
            {
                return Err(invalid(
                    "derived MBAFF direct reference exceeds active list",
                ));
            }
        }
        Ok(result)
    }
}

/// One active separate-field reference, with its selected-parity order.
#[derive(Clone, Copy)]
pub struct FieldDirectReference {
    pub id: u64,
    pub bottom: bool,
    pub poc: i32,
    pub long_term: bool,
}
/// Direct prediction in compact separate-field coordinates and vector units.
/// A full-frame co-located source retains its raw vector units until temporal direct.
pub struct FieldDirectPrediction<'a> {
    pub spatial: bool,
    pub inference8: bool,
    pub current_poc: i32,
    pub lists: [&'a [FieldDirectReference]; 2],
    pub current_bottom: bool,
    /// The selected reference field originated in a decoded full frame.
    pub colocated_is_frame: bool,
    pub colocated: Option<&'a ReferenceMotionField>,
}
impl FieldDirectPrediction<'_> {
    pub fn derive(
        &self,
        position: [usize; 2],
        neighbours: [Neighbours; 2],
    ) -> Result<[Neighbour; 2]> {
        if self.lists.iter().any(|l| l.is_empty() || l.len() > 32)
            || position.iter().any(|p| p % 4 != 0)
        {
            return Err(invalid("invalid separate-field direct context"));
        }
        let selected = if self.inference8 {
            position.map(|p| p / 16 * 16 + if p % 16 < 8 { 0 } else { 12 })
        } else {
            position
        };
        let located = self
            .colocated
            .map(|m| {
                if self.colocated_is_frame {
                    m.colocated_for_field(selected, self.current_bottom)
                } else {
                    m.colocated(selected).map(|motion| {
                        super::avc_reference_motion::MbaffColocated {
                            motion,
                            scale: super::avc_reference_motion::ColocatedScale::Same,
                        }
                    })
                }
            })
            .transpose()?;
        let col = located.and_then(|v| v.motion);
        let scale = located.map_or(super::avc_reference_motion::ColocatedScale::Same, |v| {
            v.scale
        });
        if col.is_some_and(|m| {
            (scale == super::avc_reference_motion::ColocatedScale::Same
                && m.reference_bottom_field.is_none())
                || m.reference_index > 31
        }) {
            return Err(invalid(
                "separate-field direct requires field co-located metadata",
            ));
        }
        let result = if self.spatial {
            spatial_direct_for_field(
                neighbours,
                col.map(|m| (m.reference_index, m.vector)),
                self.lists[1][0].long_term,
                true,
                true,
            )?
        } else {
            let index = match col {
                None => 0,
                Some(m) => self.lists[0]
                    .iter()
                    .position(|r| {
                        r.id == m.picture_id
                            && Some(r.bottom)
                                == if scale
                                    == super::avc_reference_motion::ColocatedScale::FrameToField
                                {
                                    Some(self.current_bottom)
                                } else {
                                    m.reference_bottom_field
                                }
                    })
                    .ok_or_else(|| invalid("co-located field reference is absent from list0"))?,
            };
            let vectors = temporal_direct(
                scale.temporal_vector(col.map_or([0; 2], |m| m.vector))?,
                self.current_poc.into(),
                self.lists[0][index].poc.into(),
                self.lists[1][0].poc.into(),
                self.lists[0][index].long_term,
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
        for list in 0..2 {
            if matches!(result[list], Neighbour::Inter { reference, .. } if usize::from(reference) >= self.lists[list].len())
            {
                return Err(invalid(
                    "derived direct field reference exceeds active list",
                ));
            }
        }
        Ok(result)
    }
}

pub struct DirectPrediction<'a> {
    pub spatial: bool,
    pub inference8: bool,
    pub current_poc: i32,
    pub list0: &'a [FrameReference],
    pub list1: &'a [FrameReference],
    /// Motion of list1[0]; None means an entirely intra-coded reference picture.
    pub colocated_field_pocs: [i32; 2],
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
        let located = self
            .colocated
            .map(|field| {
                field.colocated_for_frame(selected, self.current_poc, self.colocated_field_pocs)
            })
            .transpose()?;
        let col = located.and_then(|c| c.motion);
        let scale = located.map_or(ColocatedScale::Same, |c| c.scale);
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
                scale.temporal_vector(col.map_or([0; 2], |m| m.vector))?,
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
    fn joined_field_frame_direct_selects_poc_and_scales_temporal_only() {
        use super::super::avc_reference_motion::ReferenceMotion;
        let make = |bottom, vector| {
            ReferenceMotionField::new(
                16,
                16,
                vec![
                    [
                        Some(ReferenceMotion {
                            picture_id: 7,
                            reference_index: 0,
                            reference_bottom_field: Some(bottom),
                            vector
                        }),
                        None
                    ];
                    16
                ],
            )
            .unwrap()
        };
        let top = make(false, [8, -4]);
        let bottom = make(true, [16, 2]);
        let joined =
            ReferenceMotionField::weave_fields(Some(&top), Some(&bottom), 16, 32, 65536).unwrap();
        let l0 = [reference(99, -4), reference(7, 0)];
        let l1 = [reference(9, 8)];
        let empty = Neighbours {
            left: Neighbour::Unavailable,
            top: Neighbour::Unavailable,
            top_right: Neighbour::Unavailable,
            top_left: Neighbour::Unavailable,
        };
        let mut context = DirectPrediction {
            spatial: false,
            inference8: true,
            current_poc: 4,
            list0: &l0,
            list1: &l1,
            colocated_field_pocs: [0, 8],
            colocated: Some(&joined),
        };
        // Equal distances select bottom; the stable reference is L0[1], and
        // its field vertical vector doubles before temporal POC weighting.
        assert_eq!(
            context.derive([8, 24], [empty; 2]).unwrap(),
            [
                Neighbour::Inter {
                    reference: 1,
                    vector: [8, 2]
                },
                Neighbour::Inter {
                    reference: 0,
                    vector: [-8, -2]
                }
            ]
        );
        context.current_poc = 3;
        assert_eq!(
            context.derive([0, 0], [empty; 2]).unwrap(),
            [
                Neighbour::Inter {
                    reference: 1,
                    vector: [3, -3]
                },
                Neighbour::Inter {
                    reference: 0,
                    vector: [-5, 5]
                }
            ]
        );
        let raw = make(false, [0, 1]);
        let joined_raw =
            ReferenceMotionField::weave_fields(Some(&raw), Some(&raw), 16, 32, 65536).unwrap();
        context.spatial = true;
        context.colocated = Some(&joined_raw);
        let neighbour = Neighbours {
            left: Neighbour::Inter {
                reference: 0,
                vector: [12, 8],
            },
            ..empty
        };
        // Eager field-to-frame doubling would turn [0,1] into [0,2] and
        // incorrectly suppress spatial colZeroFlag.
        assert_eq!(
            context.derive([0, 0], [neighbour; 2]).unwrap(),
            [
                Neighbour::Inter {
                    reference: 0,
                    vector: [0, 0]
                },
                Neighbour::Inter {
                    reference: 0,
                    vector: [0, 0]
                }
            ]
        );
    }
    #[test]
    fn migrated_frame_direct_scales_temporal_only_and_maps_current_parity() {
        use super::super::avc_reference_motion::ReferenceMotion;
        let l0 = [
            FieldDirectReference {
                id: 7,
                bottom: false,
                poc: 0,
                long_term: false,
            },
            FieldDirectReference {
                id: 7,
                bottom: true,
                poc: 0,
                long_term: false,
            },
        ];
        let l1 = [FieldDirectReference {
            id: 9,
            bottom: false,
            poc: 8,
            long_term: false,
        }];
        let empty = Neighbours {
            left: Neighbour::Unavailable,
            top: Neighbour::Unavailable,
            top_right: Neighbour::Unavailable,
            top_left: Neighbour::Unavailable,
        };
        for bottom in [false, true] {
            let saved = ReferenceMotionField::new(
                16,
                32,
                vec![
                    [
                        Some(ReferenceMotion {
                            picture_id: 7,
                            reference_index: 0,
                            reference_bottom_field: None,
                            vector: [8, -4]
                        }),
                        None
                    ];
                    32
                ],
            )
            .unwrap();
            let mut context = FieldDirectPrediction {
                current_bottom: bottom,
                colocated_is_frame: true,
                spatial: false,
                inference8: true,
                current_poc: 4,
                lists: [&l0, &l1],
                colocated: Some(&saved),
            };
            assert_eq!(
                context.derive([8, 8], [empty; 2]).unwrap(),
                [
                    Neighbour::Inter {
                        reference: u8::from(bottom),
                        vector: [4, -1]
                    },
                    Neighbour::Inter {
                        reference: 0,
                        vector: [-4, 1]
                    }
                ]
            );
            let raw = ReferenceMotionField::new(
                16,
                32,
                vec![
                    [
                        Some(ReferenceMotion {
                            picture_id: 7,
                            reference_index: 0,
                            reference_bottom_field: None,
                            vector: [0, 2]
                        }),
                        None
                    ];
                    32
                ],
            )
            .unwrap();
            context.spatial = true;
            context.colocated = Some(&raw);
            let neighbour = Neighbours {
                left: Neighbour::Inter {
                    reference: 0,
                    vector: [12, 8],
                },
                ..empty
            };
            assert_eq!(
                context.derive([0, 0], [neighbour; 2]).unwrap(),
                [
                    Neighbour::Inter {
                        reference: 0,
                        vector: [12, 8]
                    },
                    Neighbour::Inter {
                        reference: 0,
                        vector: [12, 8]
                    }
                ]
            );
        }
    }
    #[test]
    fn separate_field_temporal_mapping_uses_identity_parity_and_selected_poc() {
        use super::super::avc_reference_motion::ReferenceMotion;
        let l0 = [
            FieldDirectReference {
                id: 7,
                bottom: false,
                poc: 1,
                long_term: false,
            },
            FieldDirectReference {
                id: 7,
                bottom: true,
                poc: 0,
                long_term: false,
            },
        ];
        let l1 = [FieldDirectReference {
            id: 9,
            bottom: true,
            poc: 8,
            long_term: false,
        }];
        let mut cells = vec![[None; 2]; 16];
        // L1-only co-located prediction must be used when L0 is absent.
        cells[15][1] = Some(ReferenceMotion {
            picture_id: 7,
            reference_index: 1,
            reference_bottom_field: Some(true),
            vector: [8, -4],
        });
        let saved = ReferenceMotionField::new(16, 16, cells).unwrap();
        let neighbours = Neighbours {
            left: Neighbour::Unavailable,
            top: Neighbour::Unavailable,
            top_right: Neighbour::Unavailable,
            top_left: Neighbour::Unavailable,
        };
        let mut context = FieldDirectPrediction {
            current_bottom: true,
            colocated_is_frame: false,
            spatial: false,
            inference8: true,
            current_poc: 4,
            lists: [&l0, &l1],
            colocated: Some(&saved),
        };
        assert_eq!(
            context.derive([8, 8], [neighbours; 2]).unwrap(),
            [
                Neighbour::Inter {
                    reference: 1,
                    vector: [4, -2]
                },
                Neighbour::Inter {
                    reference: 0,
                    vector: [-4, 2]
                }
            ]
        );
        context.inference8 = false;
        assert_eq!(
            context.derive([8, 8], [neighbours; 2]).unwrap(),
            [
                Neighbour::Inter {
                    reference: 0,
                    vector: [0, 0]
                },
                Neighbour::Inter {
                    reference: 0,
                    vector: [0, 0]
                }
            ]
        );
        context.inference8 = true;
        let missing = [l0[0]];
        context.lists[0] = &missing;
        assert!(context.derive([8, 8], [neighbours; 2]).is_err());
    }
    #[test]
    fn mbaff_direct_uses_selected_field_pocs_and_reference_parity() {
        use super::super::avc_reference_motion::ReferenceMotion;
        let current = FieldOrder {
            top: Some(2),
            bottom: Some(8),
        };
        let orders0 = [FieldOrder {
            top: Some(0),
            bottom: Some(4),
        }];
        let orders1 = [FieldOrder {
            top: Some(8),
            bottom: Some(12),
        }];
        let list0 = [reference(42, 0)];
        let list1 = [reference(70, 8)];
        let absent = Neighbours {
            left: Neighbour::Unavailable,
            top: Neighbour::Unavailable,
            top_right: Neighbour::Unavailable,
            top_left: Neighbour::Unavailable,
        };
        for source_field in [false, true] {
            let stored = ReferenceMotionField::new_mbaff(
                16,
                32,
                vec![
                    [
                        Some(ReferenceMotion {
                            picture_id: 42,
                            reference_index: 0,
                            reference_bottom_field: source_field.then_some(false),
                            vector: [8, 4]
                        }),
                        None
                    ];
                    32
                ],
                vec![source_field],
            )
            .unwrap();
            let mut context = MbaffDirectPrediction {
                spatial: false,
                inference8: true,
                current_order: current,
                list0: &list0,
                list1: &list1,
                list0_orders: &orders0,
                list1_orders: &orders1,
                colocated: Some(&stored),
            };
            let expected = if source_field {
                [([2, 1], [-6, -3], 0), ([5, 3], [-3, -1], 1)]
            } else {
                [([2, 1], [-6, -1], 0), ([4, 1], [-4, -1], 0)]
            };
            for (address, (first, second, index)) in expected.into_iter().enumerate() {
                assert_eq!(
                    context.derive(address, [0, 0], true, [absent; 2]).unwrap(),
                    [
                        Neighbour::Inter {
                            reference: index,
                            vector: first
                        },
                        Neighbour::Inter {
                            reference: 0,
                            vector: second
                        }
                    ]
                );
            }
            let (first, second) = if source_field {
                ([2, 2], [-6, -6])
            } else {
                ([2, 1], [-6, -3])
            };
            assert_eq!(
                context.derive(0, [0, 0], false, [absent; 2]).unwrap(),
                [
                    Neighbour::Inter {
                        reference: 0,
                        vector: first
                    },
                    Neighbour::Inter {
                        reference: 0,
                        vector: second
                    }
                ]
            );
            context.colocated = None;
            assert_eq!(
                context.derive(1, [0, 0], true, [absent; 2]).unwrap(),
                [Neighbour::Inter {
                    reference: 0,
                    vector: [0, 0]
                }; 2]
            );
            context.spatial = true;
            assert_eq!(
                context.derive(1, [0, 0], true, [absent; 2]).unwrap(),
                [Neighbour::Inter {
                    reference: 0,
                    vector: [0, 0]
                }; 2]
            );
            context.list0_orders = &[];
            assert!(context.derive(0, [0, 0], true, [absent; 2]).is_err());
        }
        assert!(
            frame_order(FieldOrder {
                top: Some(0),
                bottom: None
            })
            .is_err()
        );
    }
    #[test]
    fn mbaff_direct_corner_inference_and_expanded_index_63_are_checked() {
        use super::super::avc_reference_motion::ReferenceMotion;
        let current = FieldOrder {
            top: Some(2),
            bottom: Some(8),
        };
        let orders0 = [FieldOrder {
            top: Some(0),
            bottom: Some(4),
        }; 32];
        let orders1 = [FieldOrder {
            top: Some(8),
            bottom: Some(12),
        }];
        let list0: Vec<_> = (100..132).map(|id| reference(id, 0)).collect();
        let list1 = [reference(70, 8)];
        let absent = Neighbours {
            left: Neighbour::Unavailable,
            top: Neighbour::Unavailable,
            top_right: Neighbour::Unavailable,
            top_left: Neighbour::Unavailable,
        };
        let cells = (0..32)
            .map(|cell| {
                [
                    Some(ReferenceMotion {
                        picture_id: 100,
                        reference_index: 0,
                        reference_bottom_field: Some(false),
                        vector: [(cell % 16 * 4) as i16, 0],
                    }),
                    None,
                ]
            })
            .collect();
        let stored = ReferenceMotionField::new_mbaff(16, 32, cells, vec![true]).unwrap();
        let mut context = MbaffDirectPrediction {
            spatial: false,
            inference8: true,
            current_order: current,
            list0: &list0,
            list1: &list1,
            list0_orders: &orders0,
            list1_orders: &orders1,
            colocated: Some(&stored),
        };
        for (local, expected) in [([4, 4], 0), ([8, 0], 3), ([0, 8], 12), ([8, 8], 15)] {
            assert_eq!(
                context.derive(0, local, true, [absent; 2]).unwrap()[0],
                Neighbour::Inter {
                    reference: 0,
                    vector: [expected, 0]
                }
            );
        }
        context.inference8 = false;
        assert_eq!(
            context.derive(0, [4, 4], true, [absent; 2]).unwrap()[0],
            Neighbour::Inter {
                reference: 0,
                vector: [5, 0]
            }
        );
        for (address, local) in [(2, [0, 0]), (0, [1, 0]), (0, [0, 16])] {
            assert!(context.derive(address, local, true, [absent; 2]).is_err());
        }
        let last = ReferenceMotionField::new_mbaff(
            16,
            32,
            vec![
                [
                    Some(ReferenceMotion {
                        picture_id: 131,
                        reference_index: 63,
                        reference_bottom_field: Some(true),
                        vector: [8, 4]
                    }),
                    None
                ];
                32
            ],
            vec![true],
        )
        .unwrap();
        context.colocated = Some(&last);
        assert_eq!(
            context.derive(0, [0, 0], true, [absent; 2]).unwrap(),
            [
                Neighbour::Inter {
                    reference: 63,
                    vector: [-4, -2]
                },
                Neighbour::Inter {
                    reference: 0,
                    vector: [-12, -6]
                }
            ]
        );
        context.list0 = &list0[..31];
        context.list0_orders = &orders0[..31];
        assert!(context.derive(0, [0, 0], true, [absent; 2]).is_err());
        context.list0 = &list0;
        context.list0_orders = &orders0;
        let bad_orders = [FieldOrder {
            top: Some(1),
            bottom: Some(4),
        }; 32];
        context.list0_orders = &bad_orders;
        assert!(context.derive(0, [0, 0], true, [absent; 2]).is_err());
        let partial = [FieldOrder {
            top: Some(0),
            bottom: None,
        }; 32];
        context.list0_orders = &partial;
        assert!(context.derive(0, [0, 0], true, [absent; 2]).is_err());
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
            colocated_field_pocs: [0, 0],
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
