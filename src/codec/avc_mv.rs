//! Progressive AVC motion-vector prediction (8.4.1) and implicit B weights (8.4.3).
//! The slice reader supplies neighbours with slice/decoding-order availability applied.
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Neighbour {
    Unavailable,
    /// Available intra partition, or an inter partition that does not use this list.
    NoPrediction,
    Inter {
        reference: u8,
        vector: [i16; 2],
    },
}
impl Neighbour {
    fn reference(self) -> Option<u8> {
        match self {
            Self::Inter { reference, .. } => Some(reference),
            _ => None,
        }
    }
    fn vector(self) -> [i16; 2] {
        match self {
            Self::Inter { vector, .. } => vector,
            _ => [0; 2],
        }
    }
}
/// Convert a spatial neighbour to the current macroblock's frame/field units
/// (H.264 8.4.1.3.2). Field references index twice as many entries. Division of
/// negative vertical vectors truncates toward zero, as specified for `/`.
/// Conversion rejects values outside this decoder's signed-16-bit storage.
pub fn normalize_neighbour(
    value: Neighbour,
    current_field: bool,
    neighbour_field: bool,
) -> Result<Neighbour> {
    let Neighbour::Inter {
        mut reference,
        mut vector,
    } = value
    else {
        return Ok(value);
    };
    if reference > if neighbour_field { 63 } else { 31 } {
        return Err(invalid("motion reference exceeds frame/field list range"));
    }
    match (current_field, neighbour_field) {
        (true, false) => {
            vector[1] /= 2;
            reference *= 2;
        }
        (false, true) => {
            vector[1] = vector[1]
                .checked_mul(2)
                .ok_or_else(|| invalid("normalized motion vector exceeds signed 16-bit range"))?;
            reference /= 2;
        }
        _ => {}
    }
    Ok(Neighbour::Inter { reference, vector })
}
#[derive(Clone, Copy, Debug)]
pub struct Neighbours {
    pub left: Neighbour,
    pub top: Neighbour,
    pub top_right: Neighbour,
    pub top_left: Neighbour,
}
#[derive(Clone, Copy, Debug)]
pub enum Partition {
    Median,
    Top16x8,
    Bottom16x8,
    Left8x16,
    Right8x16,
}
fn candidates(n: Neighbours) -> Result<[Neighbour; 3]> {
    candidates_with_limit(n, 31)
}
fn candidates_with_limit(n: Neighbours, maximum_reference: u8) -> Result<[Neighbour; 3]> {
    for value in [n.left, n.top, n.top_right, n.top_left] {
        if value.reference().is_some_and(|r| r > maximum_reference) {
            return Err(invalid(
                "motion reference index exceeds predictor list range",
            ));
        }
    }
    Ok([
        n.left,
        n.top,
        if n.top_right == Neighbour::Unavailable {
            n.top_left
        } else {
            n.top_right
        },
    ])
}
fn median(a: i16, b: i16, c: i16) -> i16 {
    a.max(b).min(a.min(b).max(c))
}
pub fn predict(reference: u8, partition: Partition, neighbours: Neighbours) -> Result<[i16; 2]> {
    predict_with_limit(reference, partition, neighbours, 31)
}
/// Predict from neighbours already normalized to the current block's units.
/// Field blocks may index 0..63, while frame blocks retain 0..31. Reference
/// list construction and neighbour frame/field conversion are separate steps.
pub fn predict_for_field(
    reference: u8,
    partition: Partition,
    neighbours: Neighbours,
    field: bool,
) -> Result<[i16; 2]> {
    predict_with_limit(
        reference,
        partition,
        neighbours,
        if field { 63 } else { 31 },
    )
}
fn predict_with_limit(
    reference: u8,
    partition: Partition,
    neighbours: Neighbours,
    maximum_reference: u8,
) -> Result<[i16; 2]> {
    if reference > maximum_reference {
        return Err(invalid(
            "motion reference index exceeds predictor list range",
        ));
    }
    let [a, mut b, mut c] = candidates_with_limit(neighbours, maximum_reference)?;
    let preferred = match partition {
        Partition::Top16x8 => Some(b),
        Partition::Bottom16x8 | Partition::Left8x16 => Some(a),
        Partition::Right8x16 => Some(c),
        Partition::Median => None,
    };
    if let Some(value) = preferred.filter(|v| v.reference() == Some(reference)) {
        return Ok(value.vector());
    }
    if b == Neighbour::Unavailable && c == Neighbour::Unavailable && a != Neighbour::Unavailable {
        b = a;
        c = a;
    }
    let matches = [a, b, c].map(|v| v.reference() == Some(reference));
    if matches.iter().filter(|&&v| v).count() == 1 {
        return Ok([a, b, c][matches.iter().position(|&v| v).unwrap()].vector());
    }
    let (a, b, c) = (a.vector(), b.vector(), c.vector());
    Ok([median(a[0], b[0], c[0]), median(a[1], b[1], c[1])])
}
pub fn p_skip(neighbours: Neighbours) -> Result<[i16; 2]> {
    p_skip_for_field(neighbours, false)
}
/// P-skip from neighbours normalized into the current frame/field units.
pub fn p_skip_for_field(neighbours: Neighbours, field: bool) -> Result<[i16; 2]> {
    candidates_with_limit(neighbours, if field { 63 } else { 31 })?;
    for value in [neighbours.left, neighbours.top] {
        if value == Neighbour::Unavailable
            || value
                == (Neighbour::Inter {
                    reference: 0,
                    vector: [0; 2],
                })
        {
            return Ok([0; 2]);
        }
    }
    predict_for_field(0, Partition::Median, neighbours, field)
}
/// Spatial direct prediction for one progressive co-located subpartition.
/// Neighbours must be taken at macroblock partition 0 for both lists, even when
/// deriving a later direct subpartition. `None` denotes an intra co-located block.
pub fn spatial_direct(
    neighbours: [Neighbours; 2],
    colocated: Option<(u8, [i16; 2])>,
    colocated_picture_long_term: bool,
) -> Result<[Neighbour; 2]> {
    if colocated.is_some_and(|(reference, _)| reference > 31) {
        return Err(invalid("co-located reference index exceeds 31"));
    }
    let mut references = [None; 2];
    for list in 0..2 {
        references[list] = candidates(neighbours[list])?
            .into_iter()
            .filter_map(Neighbour::reference)
            .min();
    }
    let direct_zero = references == [None; 2];
    if direct_zero {
        references = [Some(0); 2];
    }
    let col_zero = !colocated_picture_long_term
        && colocated.is_some_and(|(reference, vector)| {
            reference == 0 && vector.into_iter().all(|v| (-1..=1).contains(&v))
        });
    let mut result = [Neighbour::NoPrediction; 2];
    for list in 0..2 {
        if let Some(reference) = references[list] {
            let vector = if direct_zero || (reference == 0 && col_zero) {
                [0; 2]
            } else {
                predict(reference, Partition::Median, neighbours[list])?
            };
            result[list] = Neighbour::Inter { reference, vector };
        }
    }
    Ok(result)
}
/// Wide addition rejects out-of-range motion rather than silently wrapping.
pub fn add_difference(predictor: [i16; 2], difference: [i32; 2]) -> Result<[i16; 2]> {
    let mut out = [0; 2];
    for i in 0..2 {
        out[i] = i16::try_from(i64::from(predictor[i]) + i64::from(difference[i]))
            .map_err(|_| invalid("motion vector exceeds signed 16-bit range"))?;
    }
    Ok(out)
}
fn distance_scale(current: i64, reference0: i64, reference1: i64) -> Option<i32> {
    let td = (i128::from(reference1) - i128::from(reference0)).clamp(-128, 127) as i32;
    if td == 0 {
        return None;
    }
    let tb = (i128::from(current) - i128::from(reference0)).clamp(-128, 127) as i32;
    let tx = (16384 + (td / 2).abs()) / td;
    Some(((tb * tx + 32) >> 6).clamp(-1024, 1023))
}
/// Weights to pass to avc_motion::bipred with offsets=[0,0], denominator=5.
pub fn implicit_weights(
    current: i64,
    reference0: i64,
    reference1: i64,
    either_long_term: bool,
) -> [i16; 2] {
    if either_long_term {
        return [32; 2];
    }
    match distance_scale(current, reference0, reference1).map(|scale| scale >> 2) {
        Some(weight) if (-64..=128).contains(&weight) => [(64 - weight) as i16, weight as i16],
        _ => [32; 2],
    }
}
/// Temporal direct vectors after co-located-to-list0 mapping and field normalization.
/// The caller retains reference identities; this function derives only the two vectors.
pub fn temporal_direct(
    colocated: [i16; 2],
    current: i64,
    reference0: i64,
    reference1: i64,
    reference0_long_term: bool,
) -> Result<[[i16; 2]; 2]> {
    let scale = if reference0_long_term {
        None
    } else {
        distance_scale(current, reference0, reference1)
    };
    let Some(scale) = scale else {
        return Ok([colocated, [0; 2]]);
    };
    let mut out = [[0; 2]; 2];
    for i in 0..2 {
        let first = (scale * i32::from(colocated[i]) + 128) >> 8;
        out[0][i] = i16::try_from(first)
            .map_err(|_| invalid("temporal direct vector exceeds signed 16-bit range"))?;
        out[1][i] = i16::try_from(first - i32::from(colocated[i]))
            .map_err(|_| invalid("temporal direct vector exceeds signed 16-bit range"))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_skip_accepts_expanded_indices_and_checks_zero_after_normalization() {
        let mut n = Neighbours {
            left: Neighbour::Inter {
                reference: 63,
                vector: [7, -3],
            },
            top: Neighbour::Inter {
                reference: 0,
                vector: [8, 4],
            },
            top_right: Neighbour::NoPrediction,
            top_left: Neighbour::Unavailable,
        };
        assert_eq!(p_skip_for_field(n, true).unwrap(), [8, 4]);
        assert!(p_skip(n).is_err());
        n.left = normalize_neighbour(
            Neighbour::Inter {
                reference: 0,
                vector: [0, 1],
            },
            true,
            false,
        )
        .unwrap();
        assert_eq!(p_skip_for_field(n, true).unwrap(), [0; 2]);
        n.left = Neighbour::Unavailable;
        assert_eq!(p_skip_for_field(n, true).unwrap(), [0; 2]);
        n.top = Neighbour::Inter {
            reference: 64,
            vector: [0; 2],
        };
        assert!(p_skip_for_field(n, true).is_err());
    }
    #[test]
    fn field_predictor_uses_normalized_references_and_partition_preferences() {
        let left = normalize_neighbour(
            Neighbour::Inter {
                reference: 31,
                vector: [7, -7],
            },
            true,
            false,
        )
        .unwrap();
        let neighbours = Neighbours {
            left,
            top: Neighbour::Inter {
                reference: 63,
                vector: [11, 4],
            },
            top_right: Neighbour::Unavailable,
            top_left: Neighbour::Inter {
                reference: 63,
                vector: [15, 6],
            },
        };
        assert_eq!(
            predict_for_field(62, Partition::Median, neighbours, true).unwrap(),
            [7, -3]
        );
        assert_eq!(
            predict_for_field(63, Partition::Top16x8, neighbours, true).unwrap(),
            [11, 4]
        );
        assert_eq!(
            predict_for_field(63, Partition::Right8x16, neighbours, true).unwrap(),
            [15, 6]
        );
        assert_eq!(
            predict_for_field(63, Partition::Median, neighbours, true).unwrap(),
            [11, 4]
        );
        assert!(predict(62, Partition::Median, neighbours).is_err());
        assert!(predict_for_field(62, Partition::Median, neighbours, false).is_err());
        assert!(predict_for_field(64, Partition::Median, neighbours, true).is_err());
        let invalid = Neighbours {
            top: Neighbour::Inter {
                reference: 64,
                vector: [0; 2],
            },
            ..neighbours
        };
        assert!(predict_for_field(62, Partition::Median, invalid, true).is_err());
        let ordinary = Neighbours {
            left: Neighbour::Inter {
                reference: 0,
                vector: [7, -3],
            },
            top: Neighbour::Unavailable,
            top_right: Neighbour::Unavailable,
            top_left: Neighbour::Unavailable,
        };
        assert_eq!(
            predict_for_field(0, Partition::Median, ordinary, false).unwrap(),
            predict(0, Partition::Median, ordinary).unwrap()
        );
    }
    #[test]
    fn mixed_frame_field_normalization_keeps_sign_and_reference_units() {
        let inter = |reference, y| Neighbour::Inter {
            reference,
            vector: [17, y],
        };
        for (input, expected) in [(-7, -3), (-3, -1), (-1, 0), (0, 0), (1, 0), (3, 1), (7, 3)] {
            assert_eq!(
                normalize_neighbour(inter(31, input), true, false).unwrap(),
                inter(62, expected)
            );
        }
        for reference in 0..64 {
            assert_eq!(
                normalize_neighbour(inter(reference, -7), false, true).unwrap(),
                inter(reference / 2, -14)
            );
            assert_eq!(
                normalize_neighbour(inter(reference, -7), true, true).unwrap(),
                inter(reference, -7)
            );
        }
        for value in [Neighbour::Unavailable, Neighbour::NoPrediction] {
            for current in [false, true] {
                for neighbour in [false, true] {
                    assert_eq!(
                        normalize_neighbour(value, current, neighbour).unwrap(),
                        value
                    );
                }
            }
        }
        assert_eq!(
            normalize_neighbour(inter(0, -16384), false, true).unwrap(),
            inter(0, i16::MIN)
        );
        assert_eq!(
            normalize_neighbour(inter(0, 16383), false, true).unwrap(),
            inter(0, 32766)
        );
        assert!(normalize_neighbour(inter(0, 16384), false, true).is_err());
        assert!(normalize_neighbour(inter(0, -16385), false, true).is_err());
        assert!(normalize_neighbour(inter(32, 0), true, false).is_err());
        assert!(normalize_neighbour(inter(64, 0), false, true).is_err());
    }
    fn inter(reference: u8, vector: [i16; 2]) -> Neighbour {
        Neighbour::Inter { reference, vector }
    }
    fn neighbours() -> Neighbours {
        Neighbours {
            left: inter(0, [12, -8]),
            top: inter(0, [4, 20]),
            top_right: inter(0, [8, 4]),
            top_left: inter(1, [30, -30]),
        }
    }
    #[test]
    fn median_unique_reference_and_partition_direction() {
        let n = neighbours();
        assert_eq!(predict(0, Partition::Median, n).unwrap(), [8, 4]);
        assert_eq!(predict(0, Partition::Top16x8, n).unwrap(), [4, 20]);
        assert_eq!(predict(0, Partition::Bottom16x8, n).unwrap(), [12, -8]);
        assert_eq!(predict(0, Partition::Left8x16, n).unwrap(), [12, -8]);
        assert_eq!(predict(0, Partition::Right8x16, n).unwrap(), [8, 4]);
        assert_eq!(
            predict(
                1,
                Partition::Median,
                Neighbours {
                    top_right: Neighbour::Unavailable,
                    ..n
                }
            )
            .unwrap(),
            [30, -30]
        );
    }
    #[test]
    fn availability_is_distinct_from_no_prediction() {
        let n = Neighbours {
            left: inter(0, [12, -8]),
            top: Neighbour::Unavailable,
            top_right: Neighbour::Unavailable,
            top_left: Neighbour::Unavailable,
        };
        assert_eq!(predict(1, Partition::Median, n).unwrap(), [12, -8]);
        assert_eq!(
            predict(
                1,
                Partition::Median,
                Neighbours {
                    top: Neighbour::NoPrediction,
                    ..n
                }
            )
            .unwrap(),
            [0, 0]
        );
        // An available C using another list must not be replaced by D.
        assert_eq!(
            predict(
                1,
                Partition::Median,
                Neighbours {
                    top_right: Neighbour::NoPrediction,
                    top_left: inter(1, [99, 99]),
                    ..n
                }
            )
            .unwrap(),
            [0, 0]
        );
        assert_eq!(p_skip(n).unwrap(), [0, 0]);
        assert_eq!(p_skip(neighbours()).unwrap(), [8, 4]);
        assert_eq!(
            p_skip(Neighbours {
                left: inter(0, [0, 0]),
                ..neighbours()
            })
            .unwrap(),
            [0, 0]
        );
    }
    #[test]
    fn spatial_direct_reference_selection_and_colocated_zero() {
        let n = neighbours();
        let absent = Neighbours {
            left: Neighbour::Unavailable,
            top: Neighbour::Unavailable,
            top_right: Neighbour::Unavailable,
            top_left: Neighbour::Unavailable,
        };
        assert_eq!(
            spatial_direct([absent; 2], None, false).unwrap(),
            [inter(0, [0; 2]); 2]
        );
        assert_eq!(
            spatial_direct([n, absent], None, false).unwrap(),
            [inter(0, [8, 4]), Neighbour::NoPrediction]
        );
        for vector in [[-1, 1], [0, 0], [1, -1]] {
            assert_eq!(
                spatial_direct([n; 2], Some((0, vector)), false).unwrap(),
                [inter(0, [0; 2]); 2]
            );
        }
        for (colocated, long_term) in [
            (Some((0, [2, 0])), false),
            (Some((1, [0, 0])), false),
            (Some((0, [0, 0])), true),
            (None, false),
        ] {
            assert_eq!(
                spatial_direct([n; 2], colocated, long_term).unwrap(),
                [inter(0, [8, 4]); 2]
            );
        }
        let fallback = Neighbours {
            left: inter(3, [1, 2]),
            top: inter(2, [3, 4]),
            top_right: Neighbour::Unavailable,
            top_left: inter(1, [5, 6]),
        };
        assert_eq!(
            spatial_direct([fallback, n], Some((0, [0, 0])), false).unwrap(),
            [inter(1, [5, 6]), inter(0, [0; 2])]
        );
        assert!(spatial_direct([n; 2], Some((32, [0; 2])), false).is_err());
        assert!(
            spatial_direct(
                [Neighbours {
                    left: inter(32, [0; 2]),
                    ..n
                }; 2],
                None,
                false
            )
            .is_err()
        );
    }
    #[test]
    fn temporal_scaling_long_term_and_implicit_weights() {
        assert_eq!(implicit_weights(2, 0, 8, false), [48, 16]);
        assert_eq!(implicit_weights(2, 8, 0, false), [16, 48]);
        assert_eq!(implicit_weights(2, 0, 8, true), [32, 32]);
        assert_eq!(implicit_weights(2, 8, 8, false), [32, 32]);
        assert_eq!(implicit_weights(100, 0, 1, false), [32, 32]);
        assert_eq!(
            temporal_direct([16, -8], 2, 0, 8, false).unwrap(),
            [[4, -2], [-12, 6]]
        );
        assert_eq!(
            temporal_direct([16, -8], 2, 0, 8, true).unwrap(),
            [[16, -8], [0, 0]]
        );
        assert_eq!(
            temporal_direct([16, -8], 2, 8, 8, false).unwrap(),
            [[16, -8], [0, 0]]
        );
        for current in [i64::MIN, 0, i64::MAX] {
            let weights = implicit_weights(current, i64::MIN, i64::MAX, false);
            assert_eq!(weights[0] + weights[1], 64);
        }
    }
    #[test]
    fn checked_motion_difference_and_extremes() {
        assert_eq!(add_difference([100, -100], [-10, 10]).unwrap(), [90, -90]);
        assert!(add_difference([i16::MAX, 0], [1, 0]).is_err());
        assert!(add_difference([0, 0], [i32::MIN, i32::MAX]).is_err());
        assert!(predict(32, Partition::Median, neighbours()).is_err());
        assert!(temporal_direct([i16::MAX, 0], 127, 0, 1, false).is_err());
        assert_eq!(median(i16::MIN, i16::MAX, 0), 0);
    }
}
