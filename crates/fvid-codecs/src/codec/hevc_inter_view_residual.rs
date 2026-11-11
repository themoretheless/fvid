//! Annex I inter-view residual prediction arithmetic (I-234..I-259).
//! Values stay at internal HEVC precision until the caller combines lists.
use super::super::hevc_plane::Plane;
use crate::{Result, invalid};

/// Two-tap interpolation at quarter/eighth-sample coordinates. `rect` is in
/// component samples and `fraction_bits` describes each component MV axis.
/// Reference edge extension is applied independently to all four taps.
pub fn bilinear(
    plane: &Plane,
    rect: [u32; 4],
    mv: [i64; 2],
    fraction_bits: [u8; 2],
    budget: usize,
) -> Result<Vec<i32>> {
    let [width, height] = plane.dimensions();
    let depth = plane.bit_depth();
    if width == 0
        || height == 0
        || !(8..=16).contains(&depth)
        || fraction_bits.iter().any(|b| !(2..=3).contains(b))
        || rect[2] == 0
        || rect[3] == 0
        || rect[0].checked_add(rect[2]).is_none()
        || rect[1].checked_add(rect[3]).is_none()
    {
        return Err(invalid(
            "invalid HEVC inter-view residual interpolation geometry",
        ));
    }
    let n = (rect[2] as usize)
        .checked_mul(rect[3] as usize)
        .ok_or_else(|| invalid("HEVC inter-view residual block size overflow"))?;
    if n.checked_mul(std::mem::size_of::<i32>())
        .is_none_or(|bytes| bytes > budget)
    {
        return Err(invalid(
            "HEVC inter-view residual interpolation exceeds budget",
        ));
    }
    let origin: [i128; 2] = std::array::from_fn(|axis| {
        i128::from(rect[axis]) + (i128::from(mv[axis]) >> fraction_bits[axis])
    });
    let phase: [i64; 2] = std::array::from_fn(|axis| {
        ((i128::from(mv[axis]) & ((1i128 << fraction_bits[axis]) - 1)) as i64)
            * (64 >> fraction_bits[axis])
    });
    let clip_x = |x: i128| x.clamp(0, width as i128 - 1) as usize;
    let clip_y = |y: i128| y.clamp(0, height as i128 - 1) as usize;
    let first = [clip_x(origin[0]), clip_y(origin[1])];
    let last = [
        clip_x(origin[0] + i128::from(rect[2]) - 1 + i128::from(phase[0] != 0)),
        clip_y(origin[1] + i128::from(rect[3]) - 1 + i128::from(phase[1] != 0)),
    ];
    if !plane.ready_rect([
        first[0],
        first[1],
        last[0] - first[0] + 1,
        last[1] - first[1] + 1,
    ]) {
        return Err(invalid(
            "HEVC inter-view residual reference samples are unavailable",
        ));
    }
    let sample = |x: i128, y: i128| i64::from(plane.samples()[clip_y(y) * width + clip_x(x)]);
    let shift1 = (depth - 8).min(4);
    let shift3 = 14u8.saturating_sub(depth).max(2);
    let mut output = Vec::new();
    output
        .try_reserve_exact(n)
        .map_err(|_| invalid("HEVC inter-view residual allocation failed"))?;
    for y in 0..rect[3] {
        for x in 0..rect[2] {
            let xx = origin[0] + i128::from(x);
            let yy = origin[1] + i128::from(y);
            let [fx, fy] = phase;
            let value = match (fx, fy) {
                (0, 0) => sample(xx, yy) << shift3,
                (_, 0) => ((64 - fx) * sample(xx, yy) + fx * sample(xx + 1, yy)) >> shift1,
                (0, _) => ((64 - fy) * sample(xx, yy) + fy * sample(xx, yy + 1)) >> shift1,
                _ => {
                    let top = ((64 - fx) * sample(xx, yy) + fx * sample(xx + 1, yy)) >> shift1;
                    let bottom =
                        ((64 - fx) * sample(xx, yy + 1) + fx * sample(xx + 1, yy + 1)) >> shift1;
                    ((64 - fy) * top + fy * bottom) >> 6
                }
            };
            output.push(value as i32);
        }
    }
    Ok(output)
}

/// Add the reference-view residual before ordinary weighted sample prediction.
/// Index 1 uses the full difference; index 2 arithmetic-shifts it by one.
/// Chroma of PBs no wider than eight luma samples receives no residual (I-235).
/// Invalid inputs leave the destination unchanged, without a scratch allocation.
pub fn add_residual(
    prediction: &mut [i32],
    current_reference: &[i32],
    temporal_reference: &[i32],
    weight_index: u8,
    component: usize,
    luma_width: u32,
) -> Result<()> {
    if !(1..=2).contains(&weight_index)
        || component > 2
        || luma_width == 0
        || prediction.is_empty()
        || prediction.len() != current_reference.len()
        || prediction.len() != temporal_reference.len()
    {
        return Err(invalid("invalid HEVC inter-view residual addition binding"));
    }
    if component != 0 && luma_width <= 8 {
        return Ok(());
    }
    let shift = weight_index - 1;
    let combined = |p: i32, a: i32, b: i32| i64::from(p) + ((i64::from(a) - i64::from(b)) >> shift);
    if prediction
        .iter()
        .zip(current_reference)
        .zip(temporal_reference)
        .any(|((&p, &a), &b)| i32::try_from(combined(p, a, b)).is_err())
    {
        return Err(invalid("HEVC inter-view residual addition overflow"));
    }
    for ((p, &a), &b) in prediction
        .iter_mut()
        .zip(current_reference)
        .zip(temporal_reference)
    {
        *p = combined(*p, a, b) as i32;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plane(depth: u8, values: &[u16]) -> Plane {
        let mut p = Plane::new(2, 2, depth, 1 << 20).unwrap();
        p.reconstruct_inter([0, 0, 2, 2], values).unwrap();
        p
    }
    #[test]
    fn every_quarter_and_eighth_phase_preserves_internal_precision() {
        let p = plane(8, &[0, 64, 128, 192]);
        for bits in [2u8, 3] {
            for y in 0..1i64 << bits {
                for x in 0..1i64 << bits {
                    let actual = bilinear(&p, [0, 0, 1, 1], [x, y], [bits; 2], 4).unwrap();
                    let expected = ((64 * x + 128 * y) * 64 / (1i64 << bits)) as i32;
                    assert_eq!(actual, [expected]);
                }
            }
        }
    }
    #[test]
    fn high_depth_rounding_and_integer_precision_follow_two_tap_rules() {
        let p = plane(12, &[1, 2, 3, 5]);
        assert_eq!(bilinear(&p, [0, 0, 1, 1], [1, 3], [2; 2], 4).unwrap(), [11]);
        assert_eq!(bilinear(&p, [0, 0, 1, 1], [1, 0], [2; 2], 4).unwrap(), [5]);
        assert_eq!(bilinear(&p, [0, 0, 1, 1], [0, 3], [2; 2], 4).unwrap(), [10]);
        let p = plane(10, &[1, 2, 3, 5]);
        assert_eq!(bilinear(&p, [0, 0, 1, 1], [1, 3], [2; 2], 4).unwrap(), [47]);
        let p = plane(8, &[0, 64, 128, 192]);
        assert_eq!(
            bilinear(&p, [0, 0, 1, 1], [1, 1], [3, 2], 4).unwrap(),
            [2560]
        );
        assert_eq!(
            bilinear(&p, [0, 0, 1, 1], [1, 1], [2, 3], 4).unwrap(),
            [2048]
        );
        let p = plane(16, &[65535; 4]);
        assert_eq!(
            bilinear(&p, [0, 0, 2, 2], [3, 7], [3; 2], 16).unwrap(),
            [262140; 4]
        );
    }
    #[test]
    fn wide_signed_vectors_extend_edges_without_overflow() {
        let p = plane(8, &[0, 64, 128, 192]);
        assert_eq!(
            bilinear(&p, [0, 0, 2, 2], [i64::MIN, i64::MAX], [2; 2], 16).unwrap(),
            [8192; 4]
        );
        assert!(bilinear(&p, [0, 0, 2, 2], [0; 2], [2; 2], 15).is_err());
        assert!(bilinear(&p, [0, 0, 1, 1], [0; 2], [1; 2], 4).is_err());
        let absent = Plane::new(2, 2, 8, 1 << 20).unwrap();
        assert!(bilinear(&absent, [0, 0, 1, 1], [0; 2], [2; 2], 4).is_err());
    }
    #[test]
    fn signed_residual_weights_do_not_clip_to_sample_range() {
        let mut p = [0, 100, 255 * 64];
        add_residual(&mut p, &[-1, 2, 100], &[0, 5, 0], 2, 0, 16).unwrap();
        assert_eq!(p, [-1, 98, 255 * 64 + 50]);
        let mut p = [10, 20];
        add_residual(&mut p, &[40, -40], &[0; 2], 1, 0, 16).unwrap();
        assert_eq!(p, [50, -20]);
    }
    #[test]
    fn small_chroma_gate_and_failure_atomicity_are_preserved() {
        let mut p = [10, 20];
        add_residual(&mut p, &[40, -40], &[0; 2], 1, 1, 8).unwrap();
        assert_eq!(p, [10, 20]);
        let mut p = [10, i32::MAX];
        let before = p;
        assert!(add_residual(&mut p, &[1, 1], &[0; 2], 1, 0, 16).is_err());
        assert_eq!(p, before);
        assert!(add_residual(&mut p, &[0], &[0; 2], 1, 0, 16).is_err());
        assert_eq!(p, before);
    }
}

/// CU extension presence (I.7.3.8.5.2). `reference_available` is the derived
/// RpRefPicAvailFlag, including disparity availability, not just a DPB count.
#[derive(Clone, Copy, Debug)]
pub struct CuSyntax {
    pub inter: bool,
    pub full_partition: bool,
    pub enabled: bool,
    pub reference_available: bool,
    /// None means unavailable after slice/tile/z-scan restrictions. Above
    /// syntax does not participate in Table I.18's first-bin context.
    pub left_weight: Option<u8>,
}
pub fn read_weight(
    bins: &mut impl super::super::hevc_residual::ResidualBins,
    cu: CuSyntax,
) -> Result<u8> {
    if cu.left_weight.is_some_and(|v| v > 2) {
        return Err(invalid("invalid HEVC left inter-view residual weight"));
    }
    if !cu.inter || !cu.full_partition || !cu.enabled || !cu.reference_available {
        return Ok(0);
    }
    let context = usize::from(cu.left_weight.is_some_and(|v| v != 0));
    use super::super::hevc_cabac::Syntax;
    if !bins.decision(Syntax::InterViewResidualWeight, context)? {
        return Ok(0);
    }
    Ok(1 + u8::from(bins.decision(Syntax::InterViewResidualWeight, 2)?))
}

#[cfg(test)]
mod syntax_tests {
    use super::super::super::{hevc_cabac::Syntax, hevc_residual::ResidualBins};
    use super::*;
    struct Script(std::collections::VecDeque<(usize, bool)>);
    impl ResidualBins for Script {
        fn decision(&mut self, syntax: Syntax, index: usize) -> Result<bool> {
            assert!(matches!(syntax, Syntax::InterViewResidualWeight));
            let (expected, value) = self.0.pop_front().expect("unexpected residual-weight bin");
            assert_eq!(index, expected);
            Ok(value)
        }
        fn bypass(&mut self) -> Result<bool> {
            panic!("residual weight bins are context-coded")
        }
    }
    fn cu() -> CuSyntax {
        CuSyntax {
            inter: true,
            full_partition: true,
            enabled: true,
            reference_available: true,
            left_weight: None,
        }
    }
    #[test]
    fn all_weight_values_use_left_context_then_context_two() {
        for left in [None, Some(0), Some(1), Some(2)] {
            for weight in 0..=2 {
                let mut expected = vec![(usize::from(left.is_some_and(|v| v != 0)), weight != 0)];
                if weight != 0 {
                    expected.push((2, weight == 2));
                }
                let mut bins = Script(expected.into());
                assert_eq!(
                    read_weight(
                        &mut bins,
                        CuSyntax {
                            left_weight: left,
                            ..cu()
                        }
                    )
                    .unwrap(),
                    weight
                );
                assert!(bins.0.is_empty());
            }
        }
    }
    #[test]
    fn inferred_zero_and_invalid_bindings_consume_no_bins() {
        for disabled in [
            CuSyntax {
                inter: false,
                ..cu()
            },
            CuSyntax {
                full_partition: false,
                ..cu()
            },
            CuSyntax {
                enabled: false,
                ..cu()
            },
            CuSyntax {
                reference_available: false,
                ..cu()
            },
        ] {
            let mut bins = Script(Default::default());
            assert_eq!(read_weight(&mut bins, disabled).unwrap(), 0);
        }
        let mut bins = Script(Default::default());
        assert!(
            read_weight(
                &mut bins,
                CuSyntax {
                    left_weight: Some(3),
                    ..cu()
                }
            )
            .is_err()
        );
    }
}

/// I-58..I-60: first nearest non-current-POC entry, with strict 32767 bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TemporalTarget {
    pub reference: u8,
    pub poc: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewAvailability {
    view: u16,
    lists: [bool; 2],
}
/// Fixed-capacity metadata; flags are keyed by explicit view identities rather
/// than assuming sparse VPS view indices fit a machine-word bit position.
#[derive(Clone, Debug)]
pub struct ReferenceTargets {
    pub temporal: [Option<TemporalTarget>; 2],
    syntax_available: bool,
    views: [ViewAvailability; 64],
    count: usize,
}
impl ReferenceTargets {
    pub fn syntax_available(&self) -> bool {
        self.syntax_available
    }
    pub fn available_for(&self, list: usize, view: u16) -> Result<bool> {
        if list > 1 {
            return Err(invalid("invalid HEVC residual prediction list"));
        }
        Ok(self.views[..self.count]
            .iter()
            .any(|entry| entry.view == view && entry.lists[list]))
    }
}

/// Derive RpRefIdxLX, RefRpRefAvailFlagLX and RpRefPicAvailFlag (I.8.3.5).
/// The callback must query the named current-AU view picture's retained *current*
/// short/long-term RPS sets, including used references omitted from its active
/// lists. Merely finding the POC somewhere in the DPB is not sufficient.
pub fn reference_targets(
    lists: [&[super::texture::Identity]; 2],
    bidirectional: bool,
    current_poc: i32,
    disparity_available: bool,
    active_reference_views: &[u16],
    mut in_view_current_rps: impl FnMut(u16, i32) -> Result<bool>,
) -> Result<ReferenceTargets> {
    if lists
        .iter()
        .take(if bidirectional { 2 } else { 1 })
        .any(|list| list.len() > 15)
        || active_reference_views.len() > 64
        || active_reference_views
            .iter()
            .enumerate()
            .any(|(i, view)| active_reference_views[..i].contains(view))
    {
        return Err(invalid(
            "invalid HEVC residual prediction reference metadata",
        ));
    }
    let mut result = ReferenceTargets {
        temporal: [None; 2],
        syntax_available: false,
        views: [ViewAvailability {
            view: 0,
            lists: [false; 2],
        }; 64],
        count: active_reference_views.len(),
    };
    for list in 0..if bidirectional { 2 } else { 1 } {
        let mut minimum = 32767i64;
        for (index, picture) in lists[list].iter().enumerate() {
            let difference = (i64::from(current_poc) - i64::from(picture.poc)).abs();
            if difference != 0 && difference < minimum {
                minimum = difference;
                result.temporal[list] = Some(TemporalTarget {
                    reference: index as u8,
                    poc: picture.poc,
                });
            }
        }
    }
    result.syntax_available = disparity_available && result.temporal.iter().any(Option::is_some);
    for (index, &view) in active_reference_views.iter().enumerate() {
        result.views[index].view = view;
        if !disparity_available {
            continue;
        }
        for list in 0..2 {
            if let Some(target) = result.temporal[list] {
                result.views[index].lists[list] = in_view_current_rps(view, target.poc)?;
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod reference_tests {
    use super::super::texture::Identity;
    use super::*;
    fn id(poc: i32, view: u16) -> Identity {
        Identity { poc, view }
    }
    #[test]
    fn closest_temporal_poc_keeps_first_tie_without_a_view_filter() {
        let l0 = [id(10, 0), id(6, 1), id(12, 2), id(8, 3), id(9, 255)];
        let l1 = [id(10, 4), id(12, 0), id(8, 0)];
        let result = reference_targets([&l0, &l1], true, 10, true, &[0, 255], |view, poc| {
            Ok((view, poc) == (255, 9))
        })
        .unwrap();
        assert_eq!(
            result.temporal,
            [
                Some(TemporalTarget {
                    reference: 4,
                    poc: 9
                }),
                Some(TemporalTarget {
                    reference: 1,
                    poc: 12
                })
            ]
        );
        assert!(result.syntax_available());
        assert!(result.available_for(0, 255).unwrap());
        assert!(!result.available_for(1, 255).unwrap());
    }
    #[test]
    fn syntax_availability_does_not_require_a_matching_source_rps() {
        let list = [id(7, 0)];
        let result =
            reference_targets([&list, &[]], false, 10, true, &[1], |_, _| Ok(false)).unwrap();
        assert!(result.syntax_available());
        assert!(!result.available_for(0, 1).unwrap());
        assert!(result.temporal[1].is_none());
        let no_disparity = reference_targets([&list, &[]], false, 10, false, &[1], |_, _| {
            panic!("unavailable disparity must not query RPS")
        })
        .unwrap();
        assert!(!no_disparity.syntax_available());
        assert_eq!(no_disparity.temporal[0], result.temporal[0]);
    }
    #[test]
    fn nearest_poc_uses_strict_normative_distance_bound_and_wide_differences() {
        let list = [id(i32::MIN, 0), id(32767, 0), id(32766, 0)];
        let result =
            reference_targets([&list, &[]], false, 0, true, &[], |_, _| unreachable!()).unwrap();
        assert_eq!(
            result.temporal[0],
            Some(TemporalTarget {
                reference: 2,
                poc: 32766
            })
        );
        let too_far = reference_targets(
            [&list[..2], &[]],
            false,
            0,
            true,
            &[],
            |_, _| unreachable!(),
        )
        .unwrap();
        assert!(!too_far.syntax_available());
        assert!(too_far.temporal[0].is_none());
    }
    #[test]
    fn each_list_queries_its_selected_poc_in_the_named_view_current_rps() {
        let l0 = [id(4, 0)];
        let l1 = [id(9, 0)];
        let mut calls = Vec::new();
        let result = reference_targets([&l0, &l1], true, 8, true, &[2, 200], |view, poc| {
            calls.push((view, poc));
            Ok((view, poc) == (2, 4) || (view, poc) == (200, 9))
        })
        .unwrap();
        assert_eq!(calls, [(2, 4), (2, 9), (200, 4), (200, 9)]);
        assert!(result.available_for(0, 2).unwrap());
        assert!(result.available_for(1, 200).unwrap());
        assert!(!result.available_for(1, 2).unwrap());
        assert!(!result.available_for(0, 200).unwrap());
        assert!(!result.available_for(0, 3).unwrap());
        assert!(result.available_for(2, 2).is_err());
    }
    #[test]
    fn malformed_metadata_and_rps_lookup_failures_are_not_silently_unavailable() {
        let list = [id(4, 0); 16];
        assert!(reference_targets([&list, &[]], false, 8, true, &[], |_, _| Ok(false)).is_err());
        assert!(
            reference_targets([&list[..1], &[]], false, 8, true, &[2, 2], |_, _| Ok(false))
                .is_err()
        );
        assert!(
            reference_targets([&list[..1], &[]], false, 8, true, &[2], |_, _| Err(
                invalid("bad bound RPS")
            ))
            .is_err()
        );
    }
}

/// Residual-specific temporal scaling I-260..I-264. Unlike the generic MVP
/// identity shortcut, even equal POC distances must apply signed-16 clipping.
pub fn scale_motion(
    mv: [i64; 2],
    current_poc: i32,
    reference_poc: i32,
    target_poc: i32,
) -> Result<[i64; 2]> {
    let td = (i64::from(current_poc) - i64::from(reference_poc)).clamp(-128, 127);
    let tb = (i64::from(current_poc) - i64::from(target_poc)).clamp(-128, 127);
    if td == 0 {
        return Err(invalid(
            "HEVC residual prediction motion has zero temporal distance",
        ));
    }
    let tx = (16384 + (td.abs() >> 1)) / td;
    let factor = ((tb * tx + 32) >> 6).clamp(-4096, 4095);
    Ok(mv.map(|value| {
        let product = i128::from(value) * i128::from(factor);
        (product.signum() * ((product.abs() + 127) >> 8)).clamp(-32768, 32767) as i64
    }))
}

#[cfg(test)]
mod scaling_tests {
    use super::*;
    #[test]
    fn equal_distances_still_clip_camera_width_motion() {
        assert_eq!(
            scale_motion([65536, -65536], 10, 9, 9).unwrap(),
            [32767, -32768]
        );
        assert_eq!(
            scale_motion([i64::MAX, i64::MIN], 10, 9, 9).unwrap(),
            [32767, -32768]
        );
    }
    #[test]
    fn half_scale_signed_ties_and_opposite_poc_direction() {
        assert_eq!(scale_motion([3, -3], 10, 8, 9).unwrap(), [1, -1]);
        assert_eq!(scale_motion([3, -3], 10, 8, 11).unwrap(), [-1, 1]);
        assert_eq!(scale_motion([1, -1], 10, 8, 9).unwrap(), [0, 0]);
    }
    #[test]
    fn poc_subtraction_is_wide_and_zero_distance_is_an_error() {
        assert_eq!(
            scale_motion([40000, -40000], i32::MAX, i32::MIN, i32::MIN + 1).unwrap(),
            [32767, -32768]
        );
        assert!(scale_motion([0, 0], 10, 10, 9).is_err());
    }
}

/// One source slice's exact current temporal RPS, pinned to its activation/AU.
/// Only identities and kinds are retained: this snapshot cannot retain a chain
/// of decoded pictures through recursive reference-picture ownership.
#[derive(Clone)]
pub struct CurrentRps {
    current: super::super::hevc_inter_layer::PictureStatus,
    slice_address: u32,
    binding: std::sync::Arc<super::super::hevc_decoder::LayeredBinding>,
    used: Vec<super::super::hevc_reference_list::LayeredReference>,
}
impl CurrentRps {
    pub fn bind(
        source: &super::super::hevc_decoder::LayeredAccessPicture,
        slice: usize,
        access_unit: u64,
        temporal: &[super::Reference],
        budget: usize,
    ) -> Result<Self> {
        use super::super::hevc_reference_list::{LayeredReference, PictureIdentity, ReferenceKind};
        use std::mem::size_of;
        let header = source
            .slices
            .get(slice)
            .ok_or_else(|| invalid("HEVC residual RPS slice is absent"))?;
        source.binding.context().validate_poc_header(header)?;
        let current = super::super::hevc_inter_layer::PictureStatus {
            identity: PictureIdentity {
                layer_id: source.binding.sps.layer_id,
                poc: source.poc,
            },
            access_unit,
            nal: header.nal,
            discardable: header.layered.as_ref().is_some_and(|s| s.discardable),
        };
        current.validate()?;
        let entries = header
            .short_term
            .len()
            .saturating_add(header.long_term.len());
        let needed = entries
            .saturating_mul(4)
            .saturating_add(60)
            .saturating_mul(
                size_of::<super::Reference>()
                    + size_of::<LayeredReference>()
                    + size_of::<(i32, bool)>(),
            )
            .saturating_add(temporal.len().saturating_mul(4 * size_of::<i32>()))
            .saturating_add(size_of::<Self>());
        if needed > budget {
            return Err(invalid("HEVC residual current RPS exceeds metadata budget"));
        }
        for (i, reference) in temporal.iter().enumerate() {
            if reference.layer_id > 62
                || temporal[..i]
                    .iter()
                    .any(|r| r.identity() == reference.identity())
            {
                return Err(invalid(
                    "invalid or duplicate HEVC residual temporal identity",
                ));
            }
        }
        let (_, groups) = super::super::hevc_decoder::temporal_reference_groups(
            header,
            source.poc,
            temporal,
            source.binding.sps.sps.poc_bits,
        )?;
        let count = groups.iter().map(Vec::len).sum();
        let mut used = Vec::new();
        used.try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate HEVC residual RPS identities"))?;
        for (group, references) in groups.into_iter().enumerate() {
            for reference in references {
                let picture = reference.picture.as_deref().ok_or_else(|| {
                    invalid("HEVC residual used RPS picture is not reconstructed")
                })?;
                if picture.nal_header().layer_id != reference.layer_id
                    || !picture.nal_header().is_vcl()
                {
                    return Err(invalid(
                        "HEVC residual RPS picture identity disagrees with decoded source",
                    ));
                }
                let identity = reference.identity();
                if used
                    .iter()
                    .any(|entry: &LayeredReference| entry.picture == identity)
                {
                    return Err(invalid("HEVC residual current RPS repeats an identity"));
                }
                used.push(LayeredReference {
                    picture: identity,
                    kind: if group == 2 {
                        ReferenceKind::LongTerm
                    } else {
                        ReferenceKind::ShortTerm
                    },
                });
            }
        }
        Ok(Self {
            current,
            slice_address: header.address,
            binding: source.binding.clone(),
            used,
        })
    }
    pub fn current(&self) -> super::super::hevc_inter_layer::PictureStatus {
        self.current
    }
    pub fn slice_address(&self) -> u32 {
        self.slice_address
    }
    pub fn entries(&self) -> &[super::super::hevc_reference_list::LayeredReference] {
        &self.used
    }
    pub fn binding(&self) -> &std::sync::Arc<super::super::hevc_decoder::LayeredBinding> {
        &self.binding
    }
    pub fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.used.capacity()
                * std::mem::size_of::<super::super::hevc_reference_list::LayeredReference>()
    }
    pub fn resolve(
        &self,
        identity: super::super::hevc_reference_list::PictureIdentity,
        temporal: &[super::Reference],
    ) -> Result<Option<std::sync::Arc<super::super::hevc_picture::Picture>>> {
        let Some(entry) = self.used.iter().find(|entry| entry.picture == identity) else {
            return Ok(None);
        };
        let mut matches = temporal
            .iter()
            .filter(|reference| reference.identity() == identity);
        let reference = matches.next().ok_or_else(|| {
            invalid("HEVC residual current RPS picture is missing from bound DPB")
        })?;
        if matches.next().is_some() || reference.long_term != entry.long_term() {
            return Err(invalid(
                "HEVC residual current RPS picture classification disagrees with DPB",
            ));
        }
        let picture = reference
            .picture
            .as_ref()
            .ok_or_else(|| invalid("HEVC residual current RPS samples are unavailable"))?;
        if picture.nal_header().layer_id != identity.layer_id || !picture.nal_header().is_vcl() {
            return Err(invalid(
                "HEVC residual resolved RPS picture identity disagrees",
            ));
        }
        Ok(Some(picture.clone()))
    }
}

/// Current-AU texture pictures paired with their picture-level used RPS.
/// Borrows AU/DPB ownership; retaining this adapter cannot create a picture chain.
pub struct BoundSources<'a> {
    current: super::super::hevc_inter_layer::PictureStatus,
    components: &'a super::super::hevc_picture::ViewComponentPool,
    snapshots: &'a [CurrentRps],
    temporal: &'a [super::Reference],
}
impl<'a> BoundSources<'a> {
    /// The picture RPS is derived at its first independent slice (address zero).
    /// The AU owner supplies exactly one immutable snapshot per texture source.
    pub fn bind(
        components: &'a super::super::hevc_picture::ViewComponentPool,
        context: &super::super::hevc_slice::LayeredSliceContext<'_>,
        header: &super::super::hevc_slice::SliceHeader,
        current: super::super::hevc_inter_layer::PictureStatus,
        snapshots: &'a [CurrentRps],
        temporal: &'a [super::Reference],
    ) -> Result<Self> {
        components.validate_for(context, header, current)?;
        if components.depth() || context.sps.sps.depth_layer || snapshots.len() != components.len()
        {
            return Err(invalid(
                "HEVC residual sources require matching texture RPS snapshots",
            ));
        }
        for view in components.views() {
            let layer = components.layer(view).unwrap();
            let mut matches = snapshots
                .iter()
                .filter(|s| s.current.identity.layer_id == layer);
            let snapshot = matches
                .next()
                .ok_or_else(|| invalid("HEVC residual source RPS is missing"))?;
            if matches.next().is_some() || snapshot.slice_address != 0 {
                return Err(invalid(
                    "HEVC residual source requires one picture-level RPS",
                ));
            }
            let picture = components.get(view).unwrap();
            // Revalidate the snapshot's pinned source activation against the
            // actual current-AU picture, rather than trusting a layer number.
            super::super::hevc_picture::select_current_view_component(
                context,
                header,
                current,
                &snapshot.binding,
                view,
                false,
                &[(snapshot.current, picture.clone())],
            )?;
            for entry in &snapshot.used {
                snapshot
                    .resolve(entry.picture, temporal)?
                    .ok_or_else(|| invalid("HEVC residual source RPS member is unavailable"))?;
            }
        }
        Ok(Self {
            current,
            components,
            snapshots,
            temporal,
        })
    }
    pub fn validate_for(
        &self,
        context: &super::super::hevc_slice::LayeredSliceContext<'_>,
        header: &super::super::hevc_slice::SliceHeader,
        poc: i32,
    ) -> Result<()> {
        if self.current.identity.poc != poc {
            return Err(invalid(
                "HEVC residual sources reconstruction POC disagrees",
            ));
        }
        self.components.validate_for(context, header, self.current)
    }
    pub fn current_view(
        &self,
        view: u16,
    ) -> Result<std::sync::Arc<super::super::hevc_picture::Picture>> {
        self.components
            .get(view)
            .cloned()
            .ok_or_else(|| invalid("HEVC residual current view is unavailable"))
    }
    pub fn temporal_view(
        &self,
        view: u16,
        poc: i32,
    ) -> Result<Option<std::sync::Arc<super::super::hevc_picture::Picture>>> {
        let layer = self
            .components
            .layer(view)
            .ok_or_else(|| invalid("HEVC residual temporal view is unavailable"))?;
        let snapshot = self
            .snapshots
            .iter()
            .find(|s| s.current.identity.layer_id == layer)
            .ok_or_else(|| invalid("HEVC residual source RPS is missing"))?;
        snapshot.resolve(
            super::super::hevc_reference_list::PictureIdentity {
                layer_id: layer,
                poc,
            },
            self.temporal,
        )
    }
}

/// I-265..I-268: align the displaced PB centre before reference edge clipping.
pub fn reference_location(
    dimensions: [u32; 2],
    rect: [u32; 4],
    disparity: [i64; 2],
) -> Result<[u32; 2]> {
    if dimensions.contains(&0)
        || rect[2] == 0
        || rect[3] == 0
        || rect[0].checked_add(rect[2]).is_none()
        || rect[1].checked_add(rect[3]).is_none()
    {
        return Err(invalid("invalid HEVC residual reference block geometry"));
    }
    Ok(std::array::from_fn(|axis| {
        let full = i128::from(rect[axis])
            + i128::from(rect[axis + 2] >> 1)
            + ((i128::from(disparity[axis]) + 2) >> 2);
        ((full >> 3) << 3).clamp(0, i128::from(dimensions[axis]) - 1) as u32
    }))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceBlockMotion {
    pub list: usize,
    pub mv: [i64; 2],
    pub point: [u32; 2],
    pub source_reference: super::super::hevc_reference_list::PictureIdentity,
}
/// I.8.5.3.3.3.5: select the first eligible temporal motion, L0 before L1.
/// Saved picture lists are canonical across slices and may exceed an active
/// slice's 15-entry limit. Resolve the exact saved index rather than truncating.
/// None means no reference-block motion; the outer residual process owns its
/// specified zero-vector fallback and final reference-view availability gate.
pub fn reference_block_motion(
    picture: &super::super::hevc_picture::Picture,
    source_poc: i32,
    current_poc: i32,
    rect: [u32; 4],
    disparity: [i64; 2],
    targets: &ReferenceTargets,
) -> Result<Option<ReferenceBlockMotion>> {
    let point = reference_location(picture.dimensions, rect, disparity)?;
    let motion = picture.prediction_motion_at(point)?;
    for list in 0..2 {
        let (Some(vector), Some(target)) = (motion[list], targets.temporal[list]) else {
            continue;
        };
        if picture.reference_pocs[list].len() != picture.reference_layers[list].len()
            || picture.reference_pocs[list].len() != picture.reference_long_term[list].len()
        {
            return Err(invalid("HEVC residual saved reference identities disagree"));
        }
        let index = vector.reference as usize;
        let reference_poc = *picture.reference_pocs[list]
            .get(index)
            .ok_or_else(|| invalid("HEVC residual motion reference outside saved map"))?;
        let layer = picture.reference_layers[list][index];
        if layer > 62 {
            return Err(invalid(
                "HEVC residual saved reference names an invalid layer",
            ));
        }
        if reference_poc == source_poc {
            continue;
        }
        return Ok(Some(ReferenceBlockMotion {
            list,
            point,
            mv: scale_motion(vector.mv, current_poc, reference_poc, target.poc)?,
            source_reference: super::super::hevc_reference_list::PictureIdentity {
                layer_id: layer,
                poc: reference_poc,
            },
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod reference_location_tests {
    use super::*;
    #[test]
    fn signed_rounding_alignment_and_padding_precede_motion_lookup() {
        assert_eq!(
            reference_location([64, 64], [0, 0, 8, 8], [13, 0]).unwrap(),
            [0, 0]
        );
        assert_eq!(
            reference_location([64, 64], [0, 0, 8, 8], [14, 0]).unwrap(),
            [8, 0]
        );
        assert_eq!(
            reference_location([64, 64], [0, 0, 8, 8], [i64::MAX, i64::MIN]).unwrap(),
            [63, 0]
        );
        assert!(reference_location([0, 64], [0, 0, 8, 8], [0, 0]).is_err());
        assert!(reference_location([64, 64], [u32::MAX, 0, 8, 8], [0, 0]).is_err());
    }
}

/// Inputs after current-slice reference lists/disparity/temporal targets bind.
pub struct PredictionInputs<'a> {
    pub lists: &'a [Vec<super::Reference>; 2],
    pub motion: super::Motion,
    pub targets: &'a ReferenceTargets,
    pub current_poc: i32,
    pub rect: [u32; 4],
    pub disparity: super::disparity::derivation::Vector,
}
struct Contribution {
    current: std::sync::Arc<super::super::hevc_picture::Picture>,
    reference: std::sync::Arc<super::super::hevc_picture::Picture>,
    current_mv: [i64; 2],
    reference_mv: [i64; 2],
}
/// One list's bilinear base and optional reference-view difference. Images are
/// shared native owners; no recursive reference-picture graph is constructed.
pub struct PreparedPrediction {
    rect: [u32; 4],
    base: std::sync::Arc<super::super::hevc_picture::Picture>,
    base_identity: super::super::hevc_reference_list::PictureIdentity,
    mv: [i64; 2],
    contribution: Option<Contribution>,
}
impl PreparedPrediction {
    pub fn base_identity(&self) -> super::super::hevc_reference_list::PictureIdentity {
        self.base_identity
    }
    pub fn motion(&self) -> [i64; 2] {
        self.mv
    }
    pub fn has_residual(&self) -> bool {
        self.contribution.is_some()
    }
    pub fn residual_motion(&self) -> Option<[[i64; 2]; 2]> {
        self.contribution
            .as_ref()
            .map(|c| [c.current_mv, c.reference_mv])
    }
    /// Return internal-precision samples, before ordinary list combination.
    pub fn predict(
        &self,
        rect: [u32; 4],
        component: usize,
        chroma_format: u8,
        weight: u8,
        budget: usize,
    ) -> Result<Vec<i32>> {
        if rect != self.rect
            || component > 2
            || chroma_format > 3
            || chroma_format == 0 && component != 0
            || !(1..=2).contains(&weight)
        {
            return Err(invalid("invalid HEVC residual prepared prediction format"));
        }
        let shifts = super::component_shifts(component, chroma_format);
        let block: [u32; 4] = std::array::from_fn(|i| rect[i] >> shifts[i % 2]);
        if (0..2).any(|axis| {
            rect[axis] % (1 << shifts[axis]) != 0 || rect[axis + 2] % (1 << shifts[axis]) != 0
        }) {
            return Err(invalid("unaligned HEVC residual component block"));
        }
        let active = self
            .contribution
            .as_ref()
            .filter(|_| component == 0 || rect[2] > 8);
        let bytes = (block[2] as usize)
            .checked_mul(block[3] as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| invalid("HEVC residual prediction size overflow"))?;
        if bytes
            .checked_mul(if active.is_some() { 3 } else { 1 })
            .is_none_or(|n| n > budget)
        {
            return Err(invalid("HEVC residual prepared prediction exceeds budget"));
        }
        let expected = self.base.dimensions.map(|v| v as usize);
        let dims = [expected[0] >> shifts[0], expected[1] >> shifts[1]];
        if self.base.planes[component].dimensions() != dims {
            return Err(invalid("HEVC residual base plane geometry disagrees"));
        }
        let bits = shifts.map(|s| 2 + s as u8);
        let mut base = bilinear(&self.base.planes[component], block, self.mv, bits, bytes)?;
        if let Some(extra) = active {
            for picture in [&extra.current, &extra.reference] {
                if picture.dimensions != self.base.dimensions
                    || picture.depth != self.base.depth
                    || picture.planes[component].dimensions() != dims
                {
                    return Err(invalid(
                        "HEVC residual picture geometry or precision disagrees",
                    ));
                }
            }
            let current = bilinear(
                &extra.current.planes[component],
                block,
                extra.current_mv,
                bits,
                bytes,
            )?;
            let reference = bilinear(
                &extra.reference.planes[component],
                block,
                extra.reference_mv,
                bits,
                bytes,
            )?;
            add_residual(&mut base, &current, &reference, weight, component, rect[2])?;
        }
        Ok(base)
    }
}
/// Finish residual-enabled list prediction using the original slice-list weight
/// indices, even when temporal target selection replaced a base reference.
/// All intermediate samples remain signed/internal precision until this step.
pub fn predict_prepared(
    plans: [Option<(&PreparedPrediction, u8)>; 2],
    component: usize,
    chroma_format: u8,
    residual_weight: u8,
    weights: Option<&super::super::hevc_slice::Weights>,
    budget: usize,
) -> Result<Vec<u16>> {
    if component > 2
        || !(1..=2).contains(&residual_weight)
        || weights.is_some_and(|w| w.denominators.iter().any(|&d| d > 7))
    {
        return Err(invalid("invalid HEVC residual output format or weights"));
    }
    let first = plans
        .iter()
        .flatten()
        .next()
        .ok_or_else(|| invalid("HEVC residual prediction has no active list"))?
        .0;
    let depth = first.base.depth[usize::from(component != 0)];
    if !(8..=16).contains(&depth) {
        return Err(invalid("invalid HEVC residual output precision"));
    }
    let precision = 14u8.saturating_sub(depth).max(2)
        + weights.map_or(0, |w| w.denominators[usize::from(component != 0)]);
    let mut factors = [(1i64, 0i64); 2];
    for (list, entry) in plans.iter().enumerate() {
        let Some((plan, index)) = entry else { continue };
        if plan.rect != first.rect
            || plan.base.depth != first.base.depth
            || plan.base.dimensions != first.base.dimensions
        {
            return Err(invalid("HEVC residual list representations disagree"));
        }
        if let Some(weights) = weights {
            let w = weights.lists[list]
                .get(*index as usize)
                .ok_or_else(|| invalid("HEVC residual weight index out of range"))?;
            factors[list] = (
                i64::from(w.values[component]),
                i64::from(w.offsets[component])
                    << if weights.high_precision_offsets {
                        0
                    } else {
                        depth - 8
                    },
            );
        }
    }
    let mut pixels: [Option<Vec<i32>>; 2] = [None, None];
    let mut held = 0usize;
    for (list, entry) in plans.iter().enumerate() {
        if let Some((plan, _)) = entry {
            let samples = plan.predict(
                first.rect,
                component,
                chroma_format,
                residual_weight,
                budget
                    .checked_sub(held)
                    .ok_or_else(|| invalid("HEVC residual output exceeds budget"))?,
            )?;
            held = held
                .checked_add(
                    samples
                        .len()
                        .checked_mul(4)
                        .ok_or_else(|| invalid("HEVC residual output storage overflow"))?,
                )
                .ok_or_else(|| invalid("HEVC residual output storage overflow"))?;
            pixels[list] = Some(samples);
        }
    }
    let count = pixels.iter().flatten().count();
    let length = pixels.iter().flatten().next().unwrap().len();
    let bytes = length
        .checked_mul(2)
        .and_then(|n| n.checked_add(held))
        .ok_or_else(|| invalid("HEVC residual output storage overflow"))?;
    if bytes > budget || pixels.iter().flatten().any(|p| p.len() != length) {
        return Err(invalid(
            "HEVC residual output exceeds budget or list sizes disagree",
        ));
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| invalid("cannot allocate HEVC residual output"))?;
    let maximum = (1i64 << depth) - 1;
    for i in 0..length {
        let mut sum = 0i64;
        let mut offsets = 0i64;
        for list in 0..2 {
            if let Some(samples) = &pixels[list] {
                sum += i64::from(samples[i]) * factors[list].0;
                offsets += factors[list].1;
            }
        }
        let value = if count == 1 {
            ((sum + (1i64 << (precision - 1))) >> precision) + offsets
        } else {
            (sum + ((offsets + 1) << precision)) >> (precision + 1)
        };
        output.push(value.clamp(0, maximum) as u16);
    }
    Ok(output)
}
/// Owned command data for direct or queued CU reconstruction.
pub struct Prediction {
    pub plans: [Option<PreparedPrediction>; 2],
    pub references: [u8; 2],
    pub weight: u8,
}
impl Prediction {
    pub fn predict(
        &self,
        component: usize,
        chroma_format: u8,
        weights: Option<&super::super::hevc_slice::Weights>,
        budget: usize,
    ) -> Result<Vec<u16>> {
        predict_prepared(
            std::array::from_fn(|list| {
                self.plans[list]
                    .as_ref()
                    .map(|plan| (plan, self.references[list]))
            }),
            component,
            chroma_format,
            self.weight,
            weights,
            budget,
        )
    }
}
fn completed_reference(
    reference: &super::Reference,
) -> Result<std::sync::Arc<super::super::hevc_picture::Picture>> {
    let picture = reference
        .picture
        .as_ref()
        .ok_or_else(|| invalid("HEVC residual reference is not a completed picture"))?;
    if picture.nal_header().layer_id != reference.layer_id || !picture.nal_header().is_vcl() {
        return Err(invalid(
            "HEVC residual reference picture identity disagrees",
        ));
    }
    Ok(picture.clone())
}
fn target_reference<'a>(
    inputs: &PredictionInputs<'a>,
    list: usize,
) -> Result<&'a super::Reference> {
    let target = inputs.targets.temporal[list]
        .ok_or_else(|| invalid("HEVC residual temporal target is unavailable"))?;
    let reference = inputs.lists[list]
        .get(target.reference as usize)
        .ok_or_else(|| invalid("HEVC residual temporal target index is outside list"))?;
    if reference.poc != target.poc || reference.poc == inputs.current_poc {
        return Err(invalid("HEVC residual temporal target identity disagrees"));
    }
    Ok(reference)
}
fn sum_motion(a: [i64; 2], b: [i64; 2]) -> Result<[i64; 2]> {
    Ok([
        a[0].checked_add(b[0])
            .ok_or_else(|| invalid("HEVC residual horizontal motion overflow"))?,
        a[1].checked_add(b[1])
            .ok_or_else(|| invalid("HEVC residual vertical motion overflow"))?,
    ])
}
/// I.8.5.3.3.3.1 list selection, including mixed-list, reference-block and zero
/// motion fallback paths. Providers must enforce bound view-component/current
/// AU ownership and exact CurrentRps membership (not arbitrary DPB presence).
pub fn prepare_prediction(
    inputs: &PredictionInputs<'_>,
    list: usize,
    mut view_of_layer: impl FnMut(u8) -> Result<u16>,
    mut current_view: impl FnMut(u16) -> Result<std::sync::Arc<super::super::hevc_picture::Picture>>,
    mut temporal_view: impl FnMut(
        u16,
        i32,
    )
        -> Result<Option<std::sync::Arc<super::super::hevc_picture::Picture>>>,
) -> Result<PreparedPrediction> {
    if list > 1 {
        return Err(invalid("invalid HEVC residual active list"));
    }
    let vector =
        inputs.motion[list].ok_or_else(|| invalid("HEVC residual prediction list is inactive"))?;
    let original = inputs.lists[list]
        .get(vector.reference as usize)
        .ok_or_else(|| invalid("HEVC residual source index is outside list"))?;
    if original.poc != inputs.current_poc {
        let target = target_reference(inputs, list)?;
        let mv = scale_motion(vector.mv, inputs.current_poc, original.poc, target.poc)?;
        let mut prediction = PreparedPrediction {
            rect: inputs.rect,
            base: completed_reference(target)?,
            base_identity: target.identity(),
            mv,
            contribution: None,
        };
        let view = inputs.disparity.reference_view;
        if inputs.targets.available_for(list, view)? {
            let current = current_view(view)?;
            let reference = temporal_view(view, target.poc)?
                .ok_or_else(|| invalid("HEVC residual available view RPS picture is missing"))?;
            prediction.contribution = Some(Contribution {
                current,
                reference,
                current_mv: inputs.disparity.mv,
                reference_mv: sum_motion(mv, inputs.disparity.mv)?,
            });
        }
        return Ok(prediction);
    }
    let mut prediction = PreparedPrediction {
        rect: inputs.rect,
        base: completed_reference(original)?,
        base_identity: original.identity(),
        mv: vector.mv,
        contribution: None,
    };
    let other = 1 - list;
    let other_reference = inputs.motion[other]
        .map(|v| {
            inputs.lists[other]
                .get(v.reference as usize)
                .ok_or_else(|| invalid("HEVC residual other-list source index is outside list"))
        })
        .transpose()?;
    let selected = if let Some(reference) = other_reference.filter(|r| r.poc != inputs.current_poc)
    {
        let target = target_reference(inputs, other)?;
        Some((
            other,
            scale_motion(
                inputs.motion[other].unwrap().mv,
                inputs.current_poc,
                reference.poc,
                target.poc,
            )?,
        ))
    } else {
        let lookup = if other_reference.is_some() { 0 } else { list };
        let selected = inputs.motion[lookup]
            .ok_or_else(|| invalid("HEVC residual lookup list is inactive"))?;
        let reference = &inputs.lists[lookup][selected.reference as usize];
        let picture = completed_reference(reference)?;
        match reference_block_motion(
            &picture,
            reference.poc,
            inputs.current_poc,
            inputs.rect,
            selected.mv,
            inputs.targets,
        )? {
            Some(motion) => Some((motion.list, motion.mv)),
            None if inputs.targets.temporal[lookup].is_some() => Some((lookup, [0, 0])),
            None => None,
        }
    };
    if let Some((selected, mv)) = selected {
        let view = view_of_layer(original.layer_id)?;
        if inputs.targets.available_for(selected, view)? {
            let target = target_reference(inputs, selected)?;
            let current = completed_reference(target)?;
            let reference = temporal_view(view, target.poc)?.ok_or_else(|| {
                invalid("HEVC residual available inter-view RPS picture is missing")
            })?;
            prediction.contribution = Some(Contribution {
                current,
                reference,
                current_mv: mv,
                reference_mv: sum_motion(vector.mv, mv)?,
            });
        }
    }
    Ok(prediction)
}
