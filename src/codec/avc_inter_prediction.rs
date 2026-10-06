//! Join parsed partitions, derived motion and reference pictures into a predictor.
use super::{
    avc_compensation::{ComponentWeight, MacroblockPrediction, Prediction420, Reference420},
    avc_inter::{Partition, Prediction},
    avc_mv::Neighbour,
};
use crate::{Result, invalid};

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
