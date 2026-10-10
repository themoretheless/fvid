//! Join parsed partitions, derived motion and reference pictures into a predictor.
use super::{
    avc_compensation::{ComponentWeight, MacroblockPrediction, Prediction420, Reference420},
    avc_inter::{Partition, Prediction},
    avc_mv::Neighbour,
};
use crate::{Result, invalid};

/// Resolve explicit frame-indexed weights or implicit selected-field POCs.
/// The result is partition/list/component aligned for MBAFF compensation.
pub fn mbaff_weights(
    address: usize,
    field: bool,
    header: &super::avc_slice::SliceHeader,
    pps: &super::avc::Pps,
    motion: &[[Neighbour; 2]],
    direct: Option<&super::avc_direct::MbaffDirectPrediction<'_>>,
) -> Result<Option<Vec<[[ComponentWeight; 3]; 2]>>> {
    use super::avc_slice::SliceType;
    if motion.is_empty()
        || motion.len() > 16
        || !matches!(header.slice_type, SliceType::P | SliceType::Sp | SliceType::B)
        || pps.weighted_bipred > 2
    {
        return Err(invalid("invalid MBAFF weight inputs"));
    }
    let explicit = if header.slice_type == SliceType::B {
        pps.weighted_bipred == 1
    } else {
        pps.weighted_pred
    };
    let implicit = header.slice_type == SliceType::B && pps.weighted_bipred == 2;
    if !explicit && !implicit {
        return Ok(None);
    }
    let mut weights = Vec::with_capacity(motion.len());
    for vectors in motion {
        let mut weight = [[ComponentWeight::default(); 3]; 2];
        if vectors.iter().any(|n| matches!(n, Neighbour::Unavailable) || matches!(n, Neighbour::Inter { reference, .. } if *reference > if field {63} else {31})) {
            return Err(invalid("invalid MBAFF weighted motion"));
        }
        if explicit {
            let table = header
                .weights
                .as_ref()
                .ok_or_else(|| invalid("missing MBAFF explicit weight table"))?;
            for list in 0..2 {
                if let Neighbour::Inter { reference, .. } = vectors[list] {
                    let index = usize::from(reference) / if field { 2 } else { 1 };
                    let entry = [&table.l0, &table.l1][list]
                        .get(index)
                        .ok_or_else(|| invalid("missing MBAFF reference weight"))?;
                    weight[list] = [
                        ComponentWeight {
                            weight: entry.luma.0,
                            offset: entry.luma.1,
                            denominator: table.luma_denom,
                        },
                        ComponentWeight {
                            weight: entry.chroma[0].0,
                            offset: entry.chroma[0].1,
                            denominator: table.chroma_denom,
                        },
                        ComponentWeight {
                            weight: entry.chroma[1].0,
                            offset: entry.chroma[1].1,
                            denominator: table.chroma_denom,
                        },
                    ];
                }
            }
        } else if let [
            Neighbour::Inter { reference: a, .. },
            Neighbour::Inter { reference: b, .. },
        ] = *vectors
        {
            let context = direct.ok_or_else(|| invalid("missing MBAFF implicit weight context"))?;
            let mut pocs = [0i32; 2];
            let mut long_term = false;
            for (list, reference) in [a, b].into_iter().enumerate() {
                let index = usize::from(reference) / if field { 2 } else { 1 };
                let entry = [context.list0, context.list1][list]
                    .get(index)
                    .ok_or_else(|| invalid("missing MBAFF implicit reference"))?;
                let orders = [context.list0_orders, context.list1_orders][list];
                if orders.len() != [context.list0, context.list1][list].len() {
                    return Err(invalid("misaligned MBAFF implicit field orders"));
                }
                let order = orders[index];
                let (Some(top), Some(bottom)) = (order.top, order.bottom) else {
                    return Err(invalid("MBAFF implicit weights require both field POCs"));
                };
                if top.min(bottom) != entry.poc {
                    return Err(invalid("MBAFF implicit reference POC mismatch"));
                }
                pocs[list] = if field {
                    if (address % 2) ^ (usize::from(reference) % 2) != 0 {
                        bottom
                    } else {
                        top
                    }
                } else {
                    entry.poc
                };
                long_term |= entry.long_term_index.is_some();
            }
            let (Some(top), Some(bottom)) =
                (context.current_order.top, context.current_order.bottom)
            else {
                return Err(invalid("MBAFF implicit weights require current field POCs"));
            };
            let current = if field {
                if address % 2 == 0 { top } else { bottom }
            } else {
                top.min(bottom)
            };
            let values = super::avc_mv::implicit_weights(
                current.into(),
                pocs[0].into(),
                pocs[1].into(),
                long_term,
            );
            for list in 0..2 {
                weight[list] = [ComponentWeight {
                    weight: values[list],
                    offset: 0,
                    denominator: 5,
                }; 3];
            }
        }
        weights.push(weight);
    }
    Ok(Some(weights))
}

/// `motion` contains final vectors (including externally derived direct vectors).
/// Optional weights are per partition, list and component; implicit B weights
/// must be derived from the two selected reference pictures before this call.
/// Origin is the macroblock's absolute luma position in the coded frame.
pub fn predict_macroblock(
    origin: [i32; 2],
    depth: u8,
    partitions: &[Partition],
    motion: &[[Neighbour; 2]],
    references: [&[&Reference420<'_>]; 2],
    weights: Option<&[[[ComponentWeight; 3]; 2]]>,
) -> Result<Prediction420> {
    predict_macroblock_impl(origin, depth, partitions, motion, references, weights, None)
}
/// References are the modified frame lists, not pre-expanded field lists.
/// Weights, if supplied, must already use the selected frame indices.
pub fn predict_macroblock_mbaff(
    address: usize,
    geometry: [usize; 2],
    field: bool,
    depth: u8,
    partitions: &[Partition],
    motion: &[[Neighbour; 2]],
    references: [&[&Reference420<'_>]; 2],
    weights: Option<&[[[ComponentWeight; 3]; 2]]>,
) -> Result<Prediction420> {
    if references.iter().any(|list| list.len() > 32)
        || motion.iter().flatten().any(|n| matches!(n, Neighbour::Inter { reference, .. } if *reference > if field {63} else {31})) {
        return Err(invalid("invalid MBAFF prediction reference list"));
    }
    let layout = super::avc_mbaff::layout(address, geometry[0], geometry[1], true, field, [1, 1])?;
    let origin = [
        layout.origin[0],
        layout.origin[1] / if field { 2 } else { 1 },
    ];
    let origin = [
        i32::try_from(origin[0]).map_err(|_| invalid("MBAFF prediction coordinate overflow"))?,
        i32::try_from(origin[1]).map_err(|_| invalid("MBAFF prediction coordinate overflow"))?,
    ];
    predict_macroblock_impl(
        origin,
        depth,
        partitions,
        motion,
        references,
        weights,
        field.then_some(address % 2 != 0),
    )
}
fn predict_macroblock_impl(
    origin: [i32; 2],
    depth: u8,
    partitions: &[Partition],
    motion: &[[Neighbour; 2]],
    references: [&[&Reference420<'_>]; 2],
    weights: Option<&[[[ComponentWeight; 3]; 2]]>,
    current_bottom: Option<bool>,
) -> Result<Prediction420> {
    if origin.iter().any(|&v| v < 0 || v % 16 != 0)
        || partitions.is_empty()
        || partitions.len() > 16
        || motion.len() != partitions.len()
        || weights.is_some_and(|w| w.len() != partitions.len())
    {
        return Err(invalid("invalid AVC macroblock prediction inputs"));
    }
    let mut assembled = MacroblockPrediction::new(depth)?;
    // One buffer per list, reused across partitions, so predictions are never
    // moved by value through the blend and weight steps.
    let mut buffers = [Prediction420::empty(depth), Prediction420::empty(depth)];
    let mut scratch = super::avc_motion::Scratch::new();
    for (index, (partition, vectors)) in partitions.iter().zip(motion).enumerate() {
        let actual = vectors.map(|v| matches!(v, Neighbour::Inter { .. }));
        let valid = match partition.prediction {
            Prediction::L0 => actual == [true, false],
            Prediction::L1 => actual == [false, true],
            Prediction::Bi => actual == [true, true],
            Prediction::Direct => actual[0] || actual[1],
        };
        if !valid || vectors.contains(&Neighbour::Unavailable) {
            return Err(invalid("AVC motion does not match partition prediction"));
        }
        let at = [
            origin[0].checked_add(i32::from(partition.origin[0])),
            origin[1].checked_add(i32::from(partition.origin[1])),
        ];
        let [Some(x), Some(y)] = at else {
            return Err(invalid("AVC prediction coordinate overflow"));
        };
        let w = weights.map_or([[ComponentWeight::default(); 3]; 2], |w| w[index]);
        let mut predicted = [false; 2];
        for list in 0..2 {
            if let Neighbour::Inter { reference, vector } = vectors[list] {
                if partition.prediction != Prediction::Direct
                    && partition.references[list] != Some(reference)
                {
                    return Err(invalid("AVC parsed and derived references differ"));
                }
                let index = if current_bottom.is_some() {
                    if reference > 63 || references[list].len() > 32 {
                        return Err(invalid("invalid MBAFF prediction reference list"));
                    }
                    usize::from(reference / 2)
                } else {
                    usize::from(reference)
                };
                let picture = references[list]
                    .get(index)
                    .ok_or_else(|| invalid("AVC prediction reference is missing"))?;
                if let Some(bottom) = current_bottom {
                    let reference_bottom = bottom ^ (reference % 2 != 0);
                    let view = picture.field_view(reference_bottom)?;
                    let luma_motion = vector.map(i32::from);
                    let mut chroma_motion = luma_motion;
                    chroma_motion[1] += match (reference_bottom, bottom) {
                        (false, true) => 2,
                        (true, false) => -2,
                        _ => 0,
                    };
                    view.predict_into_motion(
                        [x, y],
                        luma_motion,
                        chroma_motion,
                        partition.size.map(usize::from),
                        &mut buffers[list],
                        &mut scratch,
                    )?;
                } else {
                    picture.predict_into(
                        [x, y],
                        vector.map(i32::from),
                        partition.size.map(usize::from),
                        &mut buffers[list],
                        &mut scratch,
                    )?;
                }
                predicted[list] = true;
            }
        }
        let prediction = match predicted {
            [true, true] => {
                let (a, b) = buffers.split_at_mut(1);
                a[0].blend_in_place(&b[0], w)?;
                &buffers[0]
            }
            [true, false] => {
                buffers[0].weight_in_place(w[0])?;
                &buffers[0]
            }
            [false, true] => {
                buffers[1].weight_in_place(w[1])?;
                &buffers[1]
            }
            _ => return Err(invalid("AVC partition has no predictor")),
        };
        assembled.insert(partition.origin.map(usize::from), prediction)?;
    }
    assembled.finish()
}
#[cfg(test)]
mod tests {
    use super::super::{
        avc_inter::read_prediction, avc_motion_field::MotionField, avc_slice::SliceType,
        bits::BitReader,
    };
    use super::*;
    #[test]
    fn mbaff_b_weights_select_field_pocs_and_blend_every_component() {
        use super::super::{
            avc::{Pps, Sps},
            avc_direct::MbaffDirectPrediction,
            avc_poc::FieldOrder,
            avc_references::FrameReference,
            avc_slice::{SliceHeader, Weight, Weights},
        };
        fn hex(s: &str) -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect()
        }
        let sps = Sps::parse(&hex("674d400ada7a1000000300100000030320f1226a")).unwrap();
        let mut pps = Pps::parse(&hex("68ee06cb20"), &sps).unwrap();
        let mut header =
            SliceHeader::parse(&hex("419a23ff5d2e09a431c3d011f0"), &sps, &pps).unwrap();
        header.slice_type = SliceType::B;
        let l0 = [FrameReference {
            id: 42,
            frame_num: 0,
            poc: 0,
            long_term_index: None,
        }];
        let l1 = [FrameReference {
            id: 70,
            frame_num: 1,
            poc: 8,
            long_term_index: None,
        }];
        let orders0 = [FieldOrder {
            top: Some(0),
            bottom: Some(4),
        }];
        let orders1 = [FieldOrder {
            top: Some(8),
            bottom: Some(12),
        }];
        let mut context = MbaffDirectPrediction {
            spatial: true,
            inference8: true,
            current_order: FieldOrder {
                top: Some(2),
                bottom: Some(8),
            },
            list0: &l0,
            list1: &l1,
            list0_orders: &orders0,
            list1_orders: &orders1,
            colocated: None,
        };
        let make = |top, bottom, rows, width| {
            (0..rows)
                .flat_map(|row| vec![if row % 2 == 0 { top } else { bottom }; width])
                .collect::<Vec<u16>>()
        };
        let y0 = make(20, 100, 32, 16);
        let cb0 = make(40, 80, 16, 8);
        let cr0 = make(60, 100, 16, 8);
        let y1 = make(180, 220, 32, 16);
        let cb1 = make(140, 200, 16, 8);
        let cr1 = make(160, 240, 16, 8);
        let ref0 = Reference420::new([&y0, &cb0, &cr0], 16, 32, [16, 8, 8], 8).unwrap();
        let ref1 = Reference420::new([&y1, &cb1, &cr1], 16, 32, [16, 8, 8], 8).unwrap();
        let part = Partition {
            origin: [0, 0],
            size: [16, 16],
            prediction: Prediction::Bi,
            group: 0,
            references: [Some(0), Some(0)],
            differences: [[0; 2]; 2],
        };
        let vectors = [[Neighbour::Inter {
            reference: 0,
            vector: [0, 0],
        }; 2]];
        pps.weighted_bipred = 2;
        for (address, expected_weights, expected_pixels) in
            [(0, [48, 16], [60, 65, 85]), (1, [32, 32], [160, 140, 170])]
        {
            let weights = mbaff_weights(address, true, &header, &pps, &vectors, Some(&context))
                .unwrap()
                .unwrap();
            assert_eq!(weights[0].map(|list| list[0].weight), expected_weights);
            let prediction = predict_macroblock_mbaff(
                address,
                [1, 2],
                true,
                8,
                &[part],
                &vectors,
                [&[&ref0], &[&ref1]],
                Some(&weights),
            )
            .unwrap();
            for (plane, pixel) in [&prediction.y[..], &prediction.cb[..], &prediction.cr[..]]
                .into_iter()
                .zip(expected_pixels)
            {
                assert!(plane.iter().all(|&v| v == pixel));
            }
        }
        let opposite = [[Neighbour::Inter {
            reference: 1,
            vector: [0, 0],
        }; 2]];
        let weights = mbaff_weights(0, true, &header, &pps, &opposite, Some(&context))
            .unwrap()
            .unwrap();
        assert_eq!(weights[0].map(|list| list[0].weight), [80, -16]);
        let opposite_part = Partition {
            references: [Some(1), Some(1)],
            ..part
        };
        let predicted = predict_macroblock_mbaff(
            0,
            [1, 2],
            true,
            8,
            &[opposite_part],
            &opposite,
            [&[&ref0], &[&ref1]],
            Some(&weights),
        )
        .unwrap();
        assert!(predicted.y.iter().all(|&v| v == 70));
        assert!(predicted.cb.iter().all(|&v| v == 50));
        assert!(predicted.cr.iter().all(|&v| v == 65));
        assert!(mbaff_weights(0, true, &header, &pps, &vectors, None).is_err());
        context.list0_orders = &[];
        assert!(mbaff_weights(0, true, &header, &pps, &vectors, Some(&context)).is_err());
        context.list0_orders = &orders0;
        context.list0 = &[];
        let mut long_term = l0;
        long_term[0].long_term_index = Some(0);
        context.list0 = &long_term;
        assert_eq!(
            mbaff_weights(0, true, &header, &pps, &vectors, Some(&context))
                .unwrap()
                .unwrap()[0]
                .map(|list| list[0].weight),
            [32, 32]
        );
        // Explicit weights index frame entries even when ref_idx is a field.
        pps.weighted_bipred = 1;
        header.weights = Some(Weights {
            luma_denom: 1,
            chroma_denom: 1,
            l0: vec![
                Weight {
                    luma: (9, 9),
                    chroma: [(9, 9); 2],
                },
                Weight {
                    luma: (3, 1),
                    chroma: [(3, 1); 2],
                },
            ],
            l1: vec![
                Weight {
                    luma: (9, 9),
                    chroma: [(9, 9); 2],
                },
                Weight {
                    luma: (1, -3),
                    chroma: [(1, -3); 2],
                },
            ],
        });
        let expanded = [[Neighbour::Inter {
            reference: 2,
            vector: [0, 0],
        }; 2]];
        let weights = mbaff_weights(0, true, &header, &pps, &expanded, None)
            .unwrap()
            .unwrap();
        assert_eq!(weights[0].map(|list| list[0].weight), [3, 1]);
        let part = Partition {
            references: [Some(2), Some(2)],
            ..part
        };
        let prediction = predict_macroblock_mbaff(
            0,
            [1, 2],
            true,
            8,
            &[part],
            &expanded,
            [&[&ref0, &ref0], &[&ref1, &ref1]],
            Some(&weights),
        )
        .unwrap();
        assert!(prediction.y.iter().all(|&v| v == 59));
        assert!(prediction.cb.iter().all(|&v| v == 64));
        assert!(prediction.cr.iter().all(|&v| v == 84));
        for depth in [10, 12, 14] {
            let shift = depth - 8;
            let a =
                [&y0, &cb0, &cr0].map(|plane| plane.iter().map(|v| v << shift).collect::<Vec<_>>());
            let b =
                [&y1, &cb1, &cr1].map(|plane| plane.iter().map(|v| v << shift).collect::<Vec<_>>());
            let ra = Reference420::new([&a[0], &a[1], &a[2]], 16, 32, [16, 8, 8], depth).unwrap();
            let rb = Reference420::new([&b[0], &b[1], &b[2]], 16, 32, [16, 8, 8], depth).unwrap();
            let prediction = predict_macroblock_mbaff(
                0,
                [1, 2],
                true,
                depth,
                &[part],
                &expanded,
                [&[&ra, &ra], &[&rb, &rb]],
                Some(&weights),
            )
            .unwrap();
            assert!(prediction.y.iter().all(|&v| v == 59 << shift));
            assert!(prediction.cb.iter().all(|&v| v == 64 << shift));
            assert!(prediction.cr.iter().all(|&v| v == 84 << shift));
        }
        header.weights = None;
        assert!(mbaff_weights(0, true, &header, &pps, &expanded, None).is_err());
    }
    #[test]
    fn mbaff_compensation_selects_parity_and_adjusts_only_opposite_chroma() {
        let y: Vec<u16> = (0..64)
            .flat_map(|row| [if row % 2 == 0 { 20 } else { 200 }; 16])
            .collect();
        let cb: Vec<u16> = (0..32).flat_map(|row| [row * 8; 8]).collect();
        let cr: Vec<u16> = (0..32).flat_map(|row| [row * 6; 8]).collect();
        let reference = Reference420::new([&y, &cb, &cr], 16, 64, [16, 8, 8], 8).unwrap();
        for address in [2, 3] {
            for index in [0, 1] {
                let part = Partition {
                    origin: [0, 0],
                    size: [16, 16],
                    prediction: Prediction::L0,
                    group: 0,
                    references: [Some(index), None],
                    differences: [[0; 2]; 2],
                };
                let motion = [[
                    Neighbour::Inter {
                        reference: index,
                        vector: [0; 2],
                    },
                    Neighbour::NoPrediction,
                ]];
                let predicted = predict_macroblock_mbaff(
                    address,
                    [1, 4],
                    true,
                    8,
                    &[part],
                    &motion,
                    [&[&reference], &[]],
                    None,
                )
                .unwrap();
                let selected_bottom = (address % 2 != 0) ^ (index == 1);
                assert_eq!(predicted.y, [if selected_bottom { 200 } else { 20 }; 256]);
                let (expected_cb, expected_cr) = if index == 1 {
                    (132, 99)
                } else if address % 2 == 0 {
                    (128, 96)
                } else {
                    (136, 102)
                };
                assert_eq!(predicted.cb[0], expected_cb);
                assert_eq!(predicted.cr[0], expected_cr);
                assert!(
                    predict_macroblock_mbaff(
                        address,
                        [1, 4],
                        true,
                        8,
                        &[part],
                        &motion,
                        [&[], &[]],
                        None
                    )
                    .is_err()
                );
            }
        }
    }
    #[test]
    fn mbaff_frame_prediction_keeps_progressive_compensation() {
        let y: Vec<u16> = (0..1024).map(|i| (i % 255) as u16).collect();
        let c: Vec<u16> = (0..256).map(|i| (i % 255) as u16).collect();
        let reference = Reference420::new([&y, &c, &c], 16, 64, [16, 8, 8], 8).unwrap();
        let part = Partition {
            origin: [0, 0],
            size: [16, 16],
            prediction: Prediction::L0,
            group: 0,
            references: [Some(0), None],
            differences: [[0; 2]; 2],
        };
        let motion = [[
            Neighbour::Inter {
                reference: 0,
                vector: [1, -3],
            },
            Neighbour::NoPrediction,
        ]];
        let frame =
            predict_macroblock([0, 32], 8, &[part], &motion, [&[&reference], &[]], None).unwrap();
        let mbaff = predict_macroblock_mbaff(
            2,
            [1, 4],
            false,
            8,
            &[part],
            &motion,
            [&[&reference], &[]],
            None,
        )
        .unwrap();
        assert_eq!(mbaff.y, frame.y);
        assert_eq!(mbaff.cb, frame.cb);
        assert_eq!(mbaff.cr, frame.cr);
    }
    #[test]
    fn cavlc_to_motion_to_bipred_pixels() {
        let low = [20; 256];
        let high = [100; 256];
        let cb = [60; 64];
        let cr = [180; 64];
        let a = Reference420::new([&low, &cb, &cr], 16, 16, [16, 8, 8], 8).unwrap();
        let b = Reference420::new([&high, &cr, &cb], 16, 16, [16, 8, 8], 8).unwrap();
        // B_Bi_16x16: one reference in each list, four zero MVD components.
        let parts = read_prediction(&mut BitReader::new(&[0xf0]), SliceType::B, 3, [1, 1]).unwrap();
        let mut field = MotionField::new(16, 16, 8192).unwrap();
        let vectors = field.decode_macroblock([0, 0], 0, &parts).unwrap();
        let frame = predict_macroblock([0, 0], 8, &parts, &vectors, [&[&a], &[&b]], None).unwrap();
        assert_eq!(frame.y, [60; 256]);
        assert_eq!(frame.cb, [120; 64]);
        assert_eq!(frame.cr, [120; 64]);
        assert!(predict_macroblock([0, 0], 8, &parts, &vectors, [&[&a], &[]], None).is_err());
        assert!(predict_macroblock([0, 0], 10, &parts, &vectors, [&[&a], &[&b]], None).is_err());
    }
}
