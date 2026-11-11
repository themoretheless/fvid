//! Native Annex I illumination parameter derivation (I-216..I-231).
//! Callers supply available pre-deblocking CU boundary samples in above/left order.
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameters {
    /// Q5 gain; chroma always uses 32.
    pub weight: i32,
    pub offset: i32,
}

/// Derive parameters without allocation. Every other boundary sample is used;
/// the complete available boundary arrays must be supplied, not pre-subsampled.
pub fn derive(
    current: &[u16],
    reference: &[u16],
    depth: u8,
    component: usize,
) -> Result<Parameters> {
    if !(8..=16).contains(&depth)
        || component > 2
        || current.len() != reference.len()
        || !(4..=128).contains(&current.len())
        || !current.len().is_power_of_two()
    {
        return Err(invalid("invalid HEVC illumination boundary geometry"));
    }
    let max = (1u32 << depth) - 1;
    if current.iter().chain(reference).any(|&v| u32::from(v) > max) {
        return Err(invalid(
            "HEVC illumination boundary sample exceeds bit depth",
        ));
    }
    let shift = (current.len() / 2).ilog2();
    let round = 1i128 << (shift - 1);
    let precision = depth.saturating_sub(12);
    let (mut sum_ref, mut sum_cur, mut square, mut product) = (0i128, 0i128, 0i128, 0i128);
    for (&c, &r) in current.iter().zip(reference).step_by(2) {
        let (c, r) = (i128::from(c), i128::from(r));
        sum_ref += r;
        sum_cur += c;
        square += (r * r) >> precision;
        product += (r * c) >> precision;
    }
    let weight = if component != 0 {
        32
    } else {
        let denominator = ((square + (square >> 7)) << shift) - ((sum_ref * sum_ref) >> precision);
        if denominator < 0 {
            return Err(invalid("negative HEVC illumination regression denominator"));
        }
        let numerator = (((product + (square >> 7)) << shift) - ((sum_ref * sum_cur) >> precision))
            .clamp(0, 2 * denominator);
        let denominator_shift = if denominator == 0 {
            0
        } else {
            (denominator.ilog2() as i32 - 5).max(0) as u32
        };
        let numerator_shift = denominator_shift.saturating_sub(12);
        let scaled_denominator = denominator >> denominator_shift;
        // Table I.3 is exactly the nearest integer to 32768/d, with entry 0=0.
        let reciprocal = if scaled_denominator == 0 {
            0
        } else {
            (32768 + scaled_denominator / 2) / scaled_denominator
        };
        (((numerator >> numerator_shift) * reciprocal)
            >> (denominator_shift - numerator_shift + 10)) as i32
    };
    let offset = (sum_cur - ((i128::from(weight) * sum_ref) >> 5) + round) >> shift;
    Ok(Parameters {
        weight,
        offset: offset as i32,
    })
}

impl Parameters {
    /// I-201: clip the gained sample before adding the signed offset. The caller
    /// combines active lists and applies the final reconstruction range rule.
    pub fn compensate(self, sample: u16, depth: u8) -> Result<i32> {
        if !(8..=16).contains(&depth)
            || u32::from(sample) >= 1u32 << depth
            || !(0..=64).contains(&self.weight)
        {
            return Err(invalid("invalid HEVC illumination compensation sample"));
        }
        let gained =
            ((i64::from(sample) * i64::from(self.weight)) >> 5).clamp(0, (1i64 << depth) - 1);
        i32::try_from(gained + i64::from(self.offset))
            .map_err(|_| invalid("HEVC illumination compensated sample overflow"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chroma_uses_even_boundary_samples_and_signed_rounded_offset() {
        let reference = [10, 255, 20, 255, 30, 255, 40, 255];
        let current = [17, 0, 27, 0, 37, 0, 47, 0];
        for component in [1, 2] {
            assert_eq!(
                derive(&current, &reference, 8, component).unwrap(),
                Parameters {
                    weight: 32,
                    offset: 7
                }
            );
            assert_eq!(
                derive(&reference, &current, 8, component).unwrap(),
                Parameters {
                    weight: 32,
                    offset: -7
                }
            );
        }
    }
    #[test]
    fn luma_identity_constant_zero_and_gain_clipping() {
        let ramp: Vec<u16> = (0..64).map(|v| v * 3).collect();
        assert_eq!(
            derive(&ramp, &ramp, 8, 0).unwrap(),
            Parameters {
                weight: 32,
                offset: 0
            }
        );
        assert_eq!(
            derive(&[77; 32], &[0; 32], 8, 0).unwrap(),
            Parameters {
                weight: 0,
                offset: 77
            }
        );
        let high: Vec<u16> = ramp.iter().map(|v| (v * 2).min(255)).collect();
        let p = derive(&high, &ramp, 8, 0).unwrap();
        assert!((32..=64).contains(&p.weight));
        assert_eq!(
            Parameters {
                weight: 64,
                offset: -7
            }
            .compensate(200, 8)
            .unwrap(),
            248
        );
        assert_eq!(
            Parameters {
                weight: 32,
                offset: -7
            }
            .compensate(0, 8)
            .unwrap(),
            -7
        );
    }
    #[test]
    fn wide_boundary_accumulators_preserve_high_depth_identity() {
        for depth in 8..=16 {
            let max = (1u32 << depth) - 1;
            let values: Vec<u16> = (0..128)
                .map(|v| ((v * 499 + max / 3) & max) as u16)
                .collect();
            assert_eq!(
                derive(&values, &values, depth, 0).unwrap(),
                Parameters {
                    weight: 32,
                    offset: 0
                }
            );
        }
    }
    #[test]
    fn malformed_boundaries_are_rejected() {
        assert!(derive(&[], &[], 8, 0).is_err());
        assert!(derive(&[0; 6], &[0; 6], 8, 0).is_err());
        assert!(derive(&[256; 8], &[0; 8], 8, 0).is_err());
        assert!(derive(&[0; 8], &[0; 4], 8, 0).is_err());
        assert!(derive(&[0; 8], &[0; 8], 8, 3).is_err());
    }
}

/// Geometry and availability belong to the current CU, not an individual PB.
/// `available` is [above, left], already restricted by slice/tile/z-scan rules.
#[derive(Clone, Copy, Debug)]
pub struct Boundary {
    pub luma_origin: [u32; 2],
    pub log2_cu: u8,
    pub component: usize,
    pub subsampling: [u8; 2],
    pub available: [bool; 2],
    pub mv: [i64; 2],
    pub different_view: bool,
}

/// I-204..I-215, with edge extension on the reference only. The current plane
/// must still contain pre-deblocking reconstruction. No sample arrays escape.
pub fn derive_boundary(
    current: &super::super::hevc_plane::Plane,
    reference: &super::super::hevc_plane::Plane,
    boundary: Boundary,
) -> Result<Option<Parameters>> {
    let Boundary {
        luma_origin,
        log2_cu,
        component,
        subsampling,
        available,
        mv,
        different_view,
    } = boundary;
    if component > 2
        || !(3..=6).contains(&log2_cu)
        || !matches!(subsampling, [1, 1] | [2, 1] | [2, 2])
        || component == 0 && subsampling != [1, 1]
        || current.bit_depth() != reference.bit_depth()
    {
        return Err(invalid("invalid HEVC illumination CU binding"));
    }
    let [sw, sh] = subsampling.map(usize::from);
    let [lx, ly] = luma_origin.map(|v| v as usize);
    if lx % sw != 0 || ly % sh != 0 {
        return Err(invalid("unaligned HEVC illumination component origin"));
    }
    let (x, y, w, h) = (
        lx / sw,
        ly / sh,
        (1usize << log2_cu) / sw,
        (1usize << log2_cu) / sh,
    );
    let [cw, ch] = current.dimensions();
    let [rw, rh] = reference.dimensions();
    if x.checked_add(w).is_none_or(|v| v > cw)
        || y.checked_add(h).is_none_or(|v| v > ch)
        || rw == 0
        || rh == 0
        || available[0] && y == 0
        || available[1] && x == 0
    {
        return Err(invalid("HEVC illumination boundary outside picture"));
    }
    if !different_view || available == [false, false] {
        return Ok(None);
    }
    let shift = if component == 0 { 2 } else { 3 };
    let rounding = 1i128 << (shift - 1);
    let rx = x as i128 + ((i128::from(mv[0]) + rounding) >> shift);
    let ry = y as i128 + ((i128::from(mv[1]) + rounding) >> shift);
    let mut cur = [0u16; 128];
    let mut refs = [0u16; 128];
    let mut count = 0;
    for (column, enabled) in available.into_iter().enumerate() {
        if !enabled {
            continue;
        }
        let vertical = column == 1;
        let rect = if vertical {
            [x - 1, y, 1, h]
        } else {
            [x, y - 1, w, 1]
        };
        if !current.ready_rect(rect) {
            return Err(invalid(
                "HEVC illumination current boundary is not reconstructed",
            ));
        }
        for i in 0..if vertical { h } else { w } {
            let (dx, dy) = if vertical {
                (-1, i as i128)
            } else {
                (i as i128, -1)
            };
            let xx = (rx + dx).clamp(0, rw as i128 - 1) as usize;
            let yy = (ry + dy).clamp(0, rh as i128 - 1) as usize;
            if !reference.ready_rect([xx, yy, 1, 1]) {
                return Err(invalid(
                    "HEVC illumination reference boundary is not reconstructed",
                ));
            }
            refs[count] = reference.samples()[yy * rw + xx];
            cur[count] =
                current.samples()[((y as i128 + dy) as usize) * cw + (x as i128 + dx) as usize];
            count += 1;
        }
    }
    derive(
        &cur[..count],
        &refs[..count],
        current.bit_depth(),
        component,
    )
    .map(Some)
}

#[cfg(test)]
mod boundary_tests {
    use super::super::super::hevc_plane::Plane;
    use super::*;
    fn plane(bias: u16) -> Plane {
        let mut p = Plane::new(16, 16, 8, 1 << 20).unwrap();
        let samples: Vec<u16> = (0..256).map(|i| (i % 16 + i / 16) as u16 + bias).collect();
        p.reconstruct_inter([0, 0, 16, 16], &samples).unwrap();
        p
    }
    fn boundary() -> Boundary {
        Boundary {
            luma_origin: [8, 8],
            log2_cu: 3,
            component: 1,
            subsampling: [1, 1],
            available: [true, true],
            mv: [0, 0],
            different_view: true,
        }
    }
    #[test]
    fn two_sided_chroma_boundary_and_temporal_disable() {
        let cur = plane(17);
        let reference = plane(10);
        assert_eq!(
            derive_boundary(&cur, &reference, boundary()).unwrap(),
            Some(Parameters {
                weight: 32,
                offset: 7
            })
        );
        for flags in [[true, false], [false, true]] {
            assert_eq!(
                derive_boundary(
                    &cur,
                    &reference,
                    Boundary {
                        available: flags,
                        ..boundary()
                    }
                )
                .unwrap(),
                Some(Parameters {
                    weight: 32,
                    offset: 7
                })
            );
        }
        assert_eq!(
            derive_boundary(
                &cur,
                &reference,
                Boundary {
                    different_view: false,
                    ..boundary()
                }
            )
            .unwrap(),
            None
        );
        assert_eq!(
            derive_boundary(
                &cur,
                &reference,
                Boundary {
                    available: [false, false],
                    ..boundary()
                }
            )
            .unwrap(),
            None
        );
    }
    #[test]
    fn wide_signed_displacement_extends_reference_edges_without_wrapping() {
        let cur = plane(17);
        let reference = plane(10);
        let high = derive_boundary(
            &cur,
            &reference,
            Boundary {
                mv: [i64::MAX; 2],
                ..boundary()
            },
        )
        .unwrap()
        .unwrap();
        let low = derive_boundary(
            &cur,
            &reference,
            Boundary {
                mv: [i64::MIN; 2],
                ..boundary()
            },
        )
        .unwrap()
        .unwrap();
        // Even-index boundary current mean is 35; reference corners are 40/10.
        assert_eq!(
            high,
            Parameters {
                weight: 32,
                offset: -5
            }
        );
        assert_eq!(
            low,
            Parameters {
                weight: 32,
                offset: 25
            }
        );
    }
    #[test]
    fn chroma_420_uses_component_coordinates_and_signed_eighth_rounding() {
        let cur = plane(17);
        let reference = plane(10);
        for (horizontal, offset) in [(3, 7), (4, 6), (-4, 7), (-5, 8)] {
            let config = Boundary {
                luma_origin: [16, 16],
                subsampling: [2, 2],
                mv: [horizontal, 0],
                ..boundary()
            };
            assert_eq!(
                derive_boundary(&cur, &reference, config).unwrap(),
                Some(Parameters { weight: 32, offset })
            );
        }
    }
    #[test]
    fn unreconstructed_and_impossible_boundaries_are_errors() {
        let cur = Plane::new(16, 16, 8, 1 << 20).unwrap();
        let reference = plane(10);
        assert!(derive_boundary(&cur, &reference, boundary()).is_err());
        assert!(
            derive_boundary(
                &reference,
                &reference,
                Boundary {
                    luma_origin: [0, 0],
                    ..boundary()
                }
            )
            .is_err()
        );
    }
}

/// I.7.3.8.5.2 and I-48/I-49; motion/view identities must already be resolved.
#[derive(Clone, Copy, Debug)]
pub struct CuSyntax {
    pub inter: bool,
    pub full_partition: bool,
    pub slice_enabled: bool,
    pub disabled_merge_zero: bool,
    /// `None` denotes explicit motion; merge indices include skip inference.
    pub merge_index: Option<u8>,
    /// Active-list reference view differs from the current view (explicit only).
    pub cross_view: [bool; 2],
    pub residual_prediction_weight: u8,
}
pub fn read_flag(
    bins: &mut impl super::super::hevc_residual::ResidualBins,
    cu: CuSyntax,
) -> Result<bool> {
    if cu.merge_index.is_some_and(|i| i > 5) || cu.residual_prediction_weight > 2 {
        return Err(invalid("invalid HEVC illumination CU syntax binding"));
    }
    let enabled = match cu.merge_index {
        Some(index) => index != 0 || !cu.disabled_merge_zero,
        None => cu.cross_view.into_iter().any(|v| v),
    };
    if cu.inter
        && cu.full_partition
        && cu.slice_enabled
        && enabled
        && cu.residual_prediction_weight == 0
    {
        bins.decision(super::super::hevc_cabac::Syntax::Illumination, 0)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod syntax_tests {
    use super::super::super::{hevc_cabac::Syntax, hevc_residual::ResidualBins};
    use super::*;
    struct Flag {
        calls: usize,
        value: bool,
    }
    impl ResidualBins for Flag {
        fn decision(&mut self, s: Syntax, increment: usize) -> Result<bool> {
            assert!(matches!(s, Syntax::Illumination));
            assert_eq!(increment, 0);
            self.calls += 1;
            Ok(self.value)
        }
        fn bypass(&mut self) -> Result<bool> {
            panic!("IC flag is context-coded")
        }
    }
    fn cu() -> CuSyntax {
        CuSyntax {
            inter: true,
            full_partition: true,
            slice_enabled: true,
            disabled_merge_zero: false,
            merge_index: Some(0),
            cross_view: [false; 2],
            residual_prediction_weight: 0,
        }
    }
    #[test]
    fn merge_flag_presence_obeys_zero_index_disable() {
        for index in 0..=5 {
            for disabled in [false, true] {
                for value in [false, true] {
                    let mut bins = Flag { calls: 0, value };
                    let present = index != 0 || !disabled;
                    assert_eq!(
                        read_flag(
                            &mut bins,
                            CuSyntax {
                                merge_index: Some(index),
                                disabled_merge_zero: disabled,
                                ..cu()
                            }
                        )
                        .unwrap(),
                        present && value
                    );
                    assert_eq!(bins.calls, usize::from(present));
                }
            }
        }
    }
    #[test]
    fn explicit_motion_uses_only_active_cross_view_references() {
        for refs in [[false, false], [true, false], [false, true], [true, true]] {
            let mut bins = Flag {
                calls: 0,
                value: true,
            };
            let present = refs[0] || refs[1];
            assert_eq!(
                read_flag(
                    &mut bins,
                    CuSyntax {
                        merge_index: None,
                        cross_view: refs,
                        ..cu()
                    }
                )
                .unwrap(),
                present
            );
            assert_eq!(bins.calls, usize::from(present));
        }
    }
    #[test]
    fn out_of_range_residual_weight_does_not_consume_ic_entropy() {
        let mut bins = Flag {
            calls: 0,
            value: true,
        };
        assert!(
            read_flag(
                &mut bins,
                CuSyntax {
                    residual_prediction_weight: 3,
                    ..cu()
                }
            )
            .is_err()
        );
        assert_eq!(bins.calls, 0);
    }
    #[test]
    fn inferred_zero_does_not_consume_entropy() {
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
                slice_enabled: false,
                ..cu()
            },
            CuSyntax {
                residual_prediction_weight: 1,
                ..cu()
            },
        ] {
            let mut bins = Flag {
                calls: 0,
                value: true,
            };
            assert!(!read_flag(&mut bins, disabled).unwrap());
            assert_eq!(bins.calls, 0);
        }
    }
}

/// CU binding carried with a reconstruction command; boundary pixels are read
/// when the command executes, after preceding PCM/intra/residual commands.
#[derive(Clone, Copy, Debug)]
pub struct Prediction {
    pub origin: [u32; 2],
    pub log2_cu: u8,
    pub available: [bool; 2],
    pub cross_view: [bool; 2],
}
pub(crate) fn predict(
    lists: &[Vec<super::Reference>; 2],
    motion: super::Motion,
    rect: [u32; 4],
    component: usize,
    depth: u8,
    scratch: &mut Vec<i32>,
    chroma_format: u8,
    current: &[super::super::hevc_plane::Plane; 3],
    depths: [u8; 2],
    plan: Prediction,
) -> Result<Vec<u16>> {
    // I.8.5.3.3.1 step 4: chroma of PBs no wider than eight luma
    // samples uses ordinary weighted prediction even when IlluCompFlag=1.
    // Preserve ordinary bipred precision instead of rounding each list here.
    if component != 0 && rect[2] <= 8 {
        return super::predict_with_current(
            lists,
            motion,
            rect,
            component,
            depth,
            None,
            scratch,
            chroma_format,
            Some((current, depths)),
        );
    }
    let shifts = super::component_shifts(component, chroma_format);
    let mut predictions: [Option<Vec<u16>>; 2] = [None, None];
    for list in 0..2 {
        let Some(vector) = motion[list] else { continue };
        let reference = lists[list]
            .get(vector.reference as usize)
            .ok_or_else(|| invalid("HEVC illumination reference index out of range"))?;
        let picture = reference.view(Some((current, depths)))?;
        let parameters = derive_boundary(
            &current[component],
            &picture.planes[component],
            Boundary {
                luma_origin: plan.origin,
                log2_cu: plan.log2_cu,
                component,
                subsampling: shifts.map(|s| 1u8 << s),
                available: plan.available,
                mv: vector.mv,
                different_view: plan.cross_view[list],
            },
        )?;
        let mut single = [None; 2];
        single[list] = Some(vector);
        let mut pixels = super::predict_with_current(
            lists,
            single,
            rect,
            component,
            depth,
            None,
            scratch,
            chroma_format,
            Some((current, depths)),
        )?;
        if let Some(parameters) = parameters {
            for sample in &mut pixels {
                *sample = parameters
                    .compensate(*sample, depth)?
                    .clamp(0, (1i32 << depth) - 1) as u16;
            }
        }
        predictions[list] = Some(pixels);
    }
    match predictions {
        [Some(mut a), Some(b)] => {
            if a.len() != b.len() {
                return Err(invalid("HEVC illumination prediction dimensions disagree"));
            }
            for (x, y) in a.iter_mut().zip(b) {
                *x = ((u32::from(*x) + u32::from(y) + 1) >> 1) as u16;
            }
            Ok(a)
        }
        [Some(a), None] | [None, Some(a)] => Ok(a),
        _ => Err(invalid(
            "HEVC illumination prediction has no active reference",
        )),
    }
}
