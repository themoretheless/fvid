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
    let mut scratch = [0i32; super::avc_motion::SCRATCH];
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
                let picture = references[list]
                    .get(usize::from(reference))
                    .ok_or_else(|| invalid("AVC prediction reference is missing"))?;
                picture.predict_into(
                    [x, y],
                    vector.map(i32::from),
                    partition.size.map(usize::from),
                    &mut buffers[list],
                    &mut scratch,
                )?;
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
