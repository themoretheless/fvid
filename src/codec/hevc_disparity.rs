//! Owned unclipped DepthToDisparityB/F arithmetic, equations I-37..I-43.
use crate::{Result, invalid};

#[path = "hevc_disparity_samples.rs"]
pub mod samples;
#[path = "hevc_disparity_derivation.rs"]
pub mod derivation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disparity {
    /// I-40 / B: the depth argument belongs to the referenced view.
    pub backward: i64,
    /// I-43 / F: the depth argument belongs to the current view.
    pub forward: i64,
}
/// Precomputed camera transform. No table allocation is required for sampling.
/// Consumers apply their prediction mode's clipping after this normative result.
#[derive(Clone, Copy, Debug)]
pub struct CameraTransform {
    scale: i64,
    offset: i64,
    inverse_scale: i64,
    inverse_offset: i64,
    rounding: i64,
    shift: u8,
    count: u32,
}
impl CameraTransform {
    /// Values are the four coded signed parameters, including inverse-plus-forward
    /// terms. Inverse subtraction is widened before evaluation, never in i32.
    pub fn new(values: [i32; 4], precision: u8, bit_depth: u8) -> Result<Self> {
        if precision > 5 {
            return Err(invalid("HEVC disparity camera precision exceeds range"));
        }
        if !(8..=16).contains(&bit_depth) {
            return Err(invalid("HEVC disparity bit depth exceeds range"));
        }
        let [scale, offset, sum_scale, sum_offset] = values.map(i64::from);
        let shift = bit_depth - 1 + precision;
        let unit = 1i64 << bit_depth;
        Ok(Self {
            scale,
            offset: offset * unit,
            inverse_scale: sum_scale - scale,
            inverse_offset: (sum_offset - offset) * unit,
            rounding: 1i64 << (shift - 1),
            shift,
            count: 1u32 << bit_depth,
        })
    }
    pub fn sample_count(self) -> u32 {
        self.count
    }
    fn evaluate(self, depth: u32) -> Disparity {
        // Under the admitted 8..16 bit / i32 parameter domains, inverse
        // differences, shifted offsets and products are bounded below 2^50.
        let depth = i64::from(depth);
        Disparity {
            backward: (self.scale * depth + self.offset + self.rounding) >> self.shift,
            forward: (self.inverse_scale * depth + self.inverse_offset + self.rounding)
                >> self.shift,
        }
    }
    /// Evaluate both symbolic tables at one numeric index. A prediction caller
    /// must choose the correct view's depth samples for B versus F.
    pub fn sample(self, depth: u32) -> Result<Disparity> {
        if depth >= self.count {
            return Err(invalid("HEVC disparity depth sample exceeds bit depth"));
        }
        Ok(self.evaluate(depth))
    }
    /// Ownership bound includes the Vec header and all retained samples.
    pub fn required_table_bytes(self) -> Result<usize> {
        (self.count as usize)
            .checked_mul(std::mem::size_of::<Disparity>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Vec<Disparity>>()))
            .ok_or_else(|| invalid("HEVC disparity table storage overflow"))
    }
    pub fn table(self, budget: usize) -> Result<Vec<Disparity>> {
        if self.required_table_bytes()? > budget {
            return Err(invalid("HEVC disparity table exceeds memory budget"));
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(self.count as usize)
            .map_err(|_| invalid("HEVC disparity table allocation failed"))?;
        let retained = values
            .capacity()
            .checked_mul(std::mem::size_of::<Disparity>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Vec<Disparity>>()))
            .ok_or_else(|| invalid("HEVC disparity table storage overflow"))?;
        if retained > budget {
            return Err(invalid("HEVC disparity table exceeds memory budget"));
        }
        for depth in 0..self.count {
            values.push(self.evaluate(depth));
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_ties_inverse_differences_and_unclipped_outputs() {
        let positive = CameraTransform::new([1, 0, 0, 0], 0, 8).unwrap();
        assert_eq!(
            positive.sample(64).unwrap(),
            Disparity {
                backward: 1,
                forward: 0
            }
        );
        assert_eq!(
            positive.sample(255).unwrap(),
            Disparity {
                backward: 2,
                forward: -2
            }
        );
        let negative = CameraTransform::new([-1, 0, 0, 0], 0, 8).unwrap();
        assert_eq!(
            negative.sample(64).unwrap(),
            Disparity {
                backward: 0,
                forward: 1
            }
        );
        let wide = CameraTransform::new([i32::MAX, -i32::MAX, -i32::MAX, i32::MAX], 0, 8).unwrap();
        assert_eq!(
            wide.sample(0).unwrap(),
            Disparity {
                backward: -4294967294,
                forward: 8589934588
            }
        );
        assert_eq!(
            wide.sample(255).unwrap(),
            Disparity {
                backward: -16777216,
                forward: 33554432
            }
        );
    }
    #[test]
    fn invalid_inputs_and_exact_table_ownership_budget() {
        assert!(CameraTransform::new([0; 4], 6, 8).is_err());
        for depth in [0, 7, 17, 255] {
            assert!(CameraTransform::new([0; 4], 0, depth).is_err());
        }
        let t = CameraTransform::new([7, -3, -2, 11], 5, 8).unwrap();
        assert!(t.sample(256).is_err());
        assert!(t.sample(u32::MAX).is_err());
        let needed = t.required_table_bytes().unwrap();
        assert!(t.table(needed - 1).is_err());
        assert!(t.table(0).is_err());
        let table = t.table(needed).unwrap();
        assert_eq!(table.len(), 256);
        for (d, &v) in table.iter().enumerate() {
            assert_eq!(v, t.sample(d as u32).unwrap());
        }
        let t = CameraTransform::new([i32::MIN, i32::MAX, i32::MAX, i32::MIN], 5, 16).unwrap();
        assert_eq!(t.sample_count(), 65536);
        assert!(t.sample(65536).is_err());
        assert!(t.sample(65535).is_ok());
    }
}

/// Reference metadata for Annex I.8.3.1 candidate-picture selection.
/// List slots are retained: repeated references must not be deduplicated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidatePicture {
    pub view_index: u16,
    pub temporal_id: u8,
    pub irap: bool,
    pub poc: i32,
}

/// Return at most two (list, slot) pairs in normative derivation order.
/// `lists` contains only active entries; P slices use L0 exclusively.
pub fn candidate_pictures(
    lists: [&[CandidatePicture]; 2],
    b_slice: bool,
    temporal_mvp: bool,
    collocated_from_l0: bool,
    collocated_index: usize,
    current_view: u16,
    current_poc: i32,
) -> Result<Vec<(usize, usize)>> {
    if !temporal_mvp {
        return Ok(Vec::new());
    }
    let first = usize::from(!collocated_from_l0);
    if (!b_slice && first != 0) || lists[first].get(collocated_index).is_none() {
        return Err(invalid("HEVC disparity collocated reference is missing"));
    }
    for list in lists.iter().take(if b_slice { 2 } else { 1 }) {
        if list.iter().any(|picture| picture.temporal_id > 6) {
            return Err(invalid("HEVC disparity temporal ID exceeds range"));
        }
    }
    let order = [first, 1 - first];
    let eligible = |list: usize, slot: usize, picture: &CandidatePicture| {
        picture.view_index == current_view && (list != first || slot != collocated_index)
    };
    let mut result = vec![(first, collocated_index)];
    let mut minimum_temporal_id = 7;
    for &list in order.iter().take(if b_slice { 2 } else { 1 }) {
        for (slot, picture) in lists[list].iter().enumerate() {
            if eligible(list, slot, picture) {
                if picture.irap {
                    result.push((list, slot));
                    return Ok(result);
                }
                minimum_temporal_id = minimum_temporal_id.min(picture.temporal_id);
            }
        }
    }
    // Strict comparison preserves the first candidate on a tie, and 255 is
    // the normative sentinel rather than an unbounded nearest-picture search.
    let mut minimum_distance = 255i64;
    let mut selected = None;
    for &list in order.iter().take(if b_slice { 2 } else { 1 }) {
        for (slot, picture) in lists[list].iter().enumerate() {
            let distance = (i64::from(current_poc) - i64::from(picture.poc)).abs();
            if eligible(list, slot, picture)
                && picture.temporal_id == minimum_temporal_id
                && distance < minimum_distance
            {
                minimum_distance = distance;
                selected = Some((list, slot));
            }
        }
    }
    result.extend(selected);
    Ok(result)
}

/// Annex I.8.3.2: lowest earlier view with a same-POC active reference.
/// This is view order, independent of reference-list order and temporal ID.
/// `None` represents DispAvailFlag = 0, including the base view.
pub fn default_reference_view(
    lists: [&[CandidatePicture]; 2],
    b_slice: bool,
    current_view: u16,
    current_poc: i32,
) -> Option<u16> {
    lists
        .iter()
        .take(if b_slice { 2 } else { 1 })
        .flat_map(|list| list.iter())
        .filter(|picture| picture.view_index < current_view && picture.poc == current_poc)
        .map(|picture| picture.view_index)
        .min()
}

#[cfg(test)]
mod candidate_tests {
    use super::*;
    fn picture(view_index: u16, temporal_id: u8, irap: bool, poc: i32) -> CandidatePicture {
        CandidatePicture {
            view_index,
            temporal_id,
            irap,
            poc,
        }
    }
    #[test]
    fn irap_precedes_temporal_and_distance_and_uses_collocated_list_first() {
        let l0 = [picture(0, 0, true, 0)];
        let l1 = [picture(1, 0, false, 10), picture(0, 6, true, -1000)];
        assert_eq!(
            candidate_pictures([&l0, &l1], true, true, false, 0, 0, 10).unwrap(),
            [(1, 0), (1, 1)]
        );
    }
    #[test]
    fn temporal_priority_strict_distance_and_duplicate_slots() {
        let l0 = [
            picture(0, 2, false, 0),
            picture(0, 1, false, -5),
            picture(0, 0, false, -255),
        ];
        assert_eq!(
            candidate_pictures([&l0, &[]], false, true, true, 0, 0, 0).unwrap(),
            [(0, 0)]
        );
        let l0 = [
            picture(0, 2, false, 0),
            picture(0, 1, false, -5),
            picture(0, 1, false, 5),
        ];
        assert_eq!(
            candidate_pictures([&l0, &l0], true, true, true, 0, 0, 0).unwrap(),
            [(0, 0), (0, 1)]
        );
        // The same picture in the opposite list is eligible, including slot 0.
        assert_eq!(
            candidate_pictures([&l0[..1], &l0[..1]], true, true, true, 0, 0, 0).unwrap(),
            [(0, 0), (1, 0)]
        );
    }
    #[test]
    fn default_view_matches_ordered_normative_search() {
        let choices = [
            picture(0, 0, false, -1),
            picture(0, 1, false, 0),
            picture(1, 2, true, 0),
            picture(2, 0, false, 0),
            picture(3, 0, false, 0),
            picture(1, 0, false, 1),
        ];
        for a in choices {
            for b in choices {
                for c in choices {
                    for current in 0..=4 {
                        for b_slice in [false, true] {
                            let l0 = [a, b];
                            let l1 = [c];
                            let lists: [&[CandidatePicture]; 2] = [&l0, &l1];
                            let mut expected = None;
                            'views: for view in 0..current {
                                for list in lists.iter().take(if b_slice { 2 } else { 1 }) {
                                    for p in *list {
                                        if p.view_index == view && p.poc == 0 {
                                            expected = Some(view);
                                            break 'views;
                                        }
                                    }
                                }
                            }
                            assert_eq!(
                                default_reference_view(lists, b_slice, current, 0),
                                expected
                            );
                        }
                    }
                }
            }
        }
        let l0 = [picture(2, 0, false, 0)];
        let l1 = [picture(0, 6, false, 0)];
        assert_eq!(default_reference_view([&l0, &l1], false, 3, 0), Some(2));
        assert_eq!(default_reference_view([&l0, &l1], true, 3, 0), Some(0));
    }
    #[test]
    fn disabled_missing_other_views_and_extreme_poc() {
        assert!(
            candidate_pictures([&[], &[]], false, false, true, 99, 0, 0)
                .unwrap()
                .is_empty()
        );
        assert!(candidate_pictures([&[], &[]], false, true, true, 0, 0, 0).is_err());
        let l0 = [
            picture(1, 0, false, 0),
            picture(2, 0, true, 0),
            picture(0, 0, false, i32::MIN),
        ];
        assert_eq!(
            candidate_pictures([&l0, &[]], false, true, true, 0, 0, i32::MAX).unwrap(),
            [(0, 0)]
        );
        let invalid = [picture(0, 7, false, 0)];
        assert!(candidate_pictures([&invalid, &[]], false, true, true, 0, 0, 0).is_err());
    }
}
