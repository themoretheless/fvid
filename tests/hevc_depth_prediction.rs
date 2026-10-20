use fvid::codec::{
    hevc_depth_prediction::{Partition, contour_pattern, predict},
    hevc_depth_tables::{Layer, Tables},
};

#[test]
fn partition_edges_offsets_and_lookup_match_literal_reference() {
    let top = [20, 30, 40, 50, 60, 70, 80, 90];
    let left = [100, 110, 120, 130, 140, 150, 160, 170];
    for depth in [8, 10, 12, 16] {
        for lookup in [None, Some(vec![0, 60, 90, 130, 170, 255])] {
            let tables = Tables {
                bit_depth: depth,
                layers: vec![Layer {
                    id: 1,
                    values: lookup.clone(),
                }],
            };
            for (vertical, horizontal, lt, br) in [
                (false, false, 60u16, 170u16),
                (true, true, 60, 90),
                (true, false, 110, 50),
                (false, true, 30, 130),
            ] {
                for first in [false, true] {
                    let mut pattern = [first; 16];
                    pattern[3] = first ^ vertical;
                    pattern[12] = first ^ horizontal;
                    pattern[5] = !first;
                    for offsets in [[-1, 1], [-1000, 1000], [i64::MIN, i64::MAX]] {
                        let mut expected = [0i128; 16];
                        for (i, &segment) in pattern.iter().enumerate() {
                            let predictor = if segment == first { lt } else { br };
                            let offset = i128::from(offsets[usize::from(segment)]);
                            expected[i] = if let Some(values) = &lookup {
                                // Exhaustive scalar search with upper midpoint ties.
                                let index = (0..values.len())
                                    .min_by_key(|&j| {
                                        (values[j].abs_diff(predictor), std::cmp::Reverse(j))
                                    })
                                    .unwrap();
                                let shifted = (index as i128 + offset)
                                    .clamp(0, (1i128 << depth) - 1)
                                    as usize;
                                i128::from(values.get(shifted).copied().unwrap_or(0))
                            } else {
                                i128::from(predictor) + offset
                            };
                        }
                        let mut output = [0; 16];
                        predict(
                            &tables,
                            1,
                            Partition {
                                size: 4,
                                top: &top,
                                left: &left,
                                pattern: &pattern,
                                offsets,
                            },
                            &mut output,
                        )
                        .unwrap();
                        assert_eq!(output, expected);
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_partition_preserves_output_and_neighbours() {
    let mut tables = Tables {
        bit_depth: 8,
        layers: vec![Layer {
            id: 1,
            values: None,
        }],
    };
    let mut output = [777; 16];
    let top = [0; 8];
    let left = [0; 8];
    let pattern = [false; 16];
    for (size, top_slice, layer) in [
        (3, top.as_slice(), 1),
        (4, &top[..7], 1),
        (4, top.as_slice(), 2),
    ] {
        assert!(
            predict(
                &tables,
                layer,
                Partition {
                    size,
                    top: top_slice,
                    left: &left,
                    pattern: &pattern,
                    offsets: [0; 2]
                },
                &mut output
            )
            .is_err()
        );
        assert_eq!(output, [777; 16]);
    }
    tables.layers[0].values = Some(vec![]);
    assert!(
        predict(
            &tables,
            1,
            Partition {
                size: 4,
                top: &top,
                left: &left,
                pattern: &pattern,
                offsets: [0; 2]
            },
            &mut output
        )
        .is_err()
    );
    assert_eq!(output, [777; 16]);
}

#[test]
fn equal_edge_flags_select_stronger_boundary_with_left_tie() {
    let tables = Tables {
        bit_depth: 8,
        layers: vec![Layer {
            id: 1,
            values: None,
        }],
    };
    let mut pattern = [false; 16];
    pattern[5] = true;
    for (top_end, left_end, expected) in [(200, 170, 200), (90, 200, 200), (90, 170, 170)] {
        let mut top = [20, 30, 40, 50, 60, 70, 80, 90];
        top[7] = top_end;
        let mut left = [100, 110, 120, 130, 140, 150, 160, 170];
        left[7] = left_end;
        let mut output = [0; 16];
        predict(
            &tables,
            1,
            Partition {
                size: 4,
                top: &top,
                left: &left,
                pattern: &pattern,
                offsets: [0; 2],
            },
            &mut output,
        )
        .unwrap();
        assert_eq!(output[0], 60);
        assert_eq!(output[5], expected);
    }
}

#[test]
fn contour_uses_texture_corners_strict_threshold_and_strided_origin() {
    for depth in [8, 10, 12, 16] {
        for size in [4, 8, 16, 32, 64] {
            let stride = size + 5;
            let limit = (1u32 << depth) - 1;
            let mut texture = vec![0; stride * (size + 3)];
            for y in 0..size {
                for x in 0..size {
                    texture[(y + 1) * stride + x + 2] = ((x + 3 * y) as u32 % (limit + 1)) as u16;
                }
            }
            let corners = [
                stride + 2,
                stride + size + 1,
                size * stride + 2,
                size * stride + size + 1,
            ];
            for (i, &at) in corners.iter().enumerate() {
                texture[at] = [0, 1, 2, 4][i];
            }
            // Sum=7: threshold=1, whereas rounded mean would incorrectly be 2.
            let before = texture.clone();
            let mut pattern = vec![false; size * size];
            contour_pattern(&texture, stride, [2, 1], size, depth, &mut pattern).unwrap();
            for y in 0..size {
                for x in 0..size {
                    assert_eq!(pattern[y * size + x], texture[(y + 1) * stride + x + 2] > 1);
                }
            }
            assert_eq!(texture, before);
        }
    }
    let mut output = [true; 16];
    for (stride, origin, depth) in [
        (3, [0, 0], 8),
        (4, [0, usize::MAX], 8),
        (4, [0, 1], 8),
        (4, [0, 0], 7),
    ] {
        assert!(contour_pattern(&[0; 16], stride, origin, 4, depth, &mut output).is_err());
        assert_eq!(output, [true; 16]);
    }
    let mut source = [0; 16];
    source[6] = 256;
    assert!(contour_pattern(&source, 4, [0, 0], 4, 8, &mut output).is_err());
    assert_eq!(output, [true; 16]);
}

#[test]
fn contour_mask_drives_two_segment_dc_prediction() {
    let texture: Vec<u16> = (0..16).collect();
    let mut mask = [false; 16];
    contour_pattern(&texture, 4, [0, 0], 4, 8, &mut mask).unwrap();
    for values in [None, Some(vec![0, 30, 60, 130, 255])] {
        let tables = Tables {
            bit_depth: 8,
            layers: vec![Layer {
                id: 1,
                values: values.clone(),
            }],
        };
        let mut output = [0; 16];
        predict(
            &tables,
            1,
            Partition {
                size: 4,
                top: &[20, 30, 40, 50, 60, 70, 80, 90],
                left: &[100, 110, 120, 130, 140, 150, 160, 170],
                pattern: &mask,
                offsets: [-1, 1],
            },
            &mut output,
        )
        .unwrap();
        assert_eq!(&output[..8], &[if values.is_some() { 0 } else { 29 }; 8]);
        assert_eq!(&output[8..], &[if values.is_some() { 255 } else { 131 }; 8]);
    }
}
