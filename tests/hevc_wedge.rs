use fvid::codec::hevc_wedge::pattern;

#[test]
fn wedge_diagonal_literal_and_invalid_input() {
    let mut output = [false; 16];
    pattern(4, [4, 0], [0, 4], &mut output).unwrap();
    assert_eq!(
        output,
        [
            true, true, true, false, true, true, false, false, true, false, false, false, false,
            false, false, false
        ]
    );
    let before = output;
    for (size, start, end) in [
        (3, [0, 0], [0, 0]),
        (4, [8, 0], [0, 4]),
        (4, [1, 1], [0, 4]),
        (4, [1, 0], [2, 3]),
    ] {
        assert!(pattern(size, start, end, &mut output).is_err());
        assert_eq!(output, before);
    }
}

#[test]
fn wedge_line_matches_independent_rational_rounding() {
    for size in [4usize, 8, 16] {
        let shift = usize::from(size < 16);
        let domain = size << shift;
        for orientation in [0, 4] {
            let step_start = if size == 16 { 2 } else { 1 };
            let step_end = if size == 16 && orientation == 0 { 2 } else { 1 };
            for m in (0..domain).step_by(step_start) {
                for n in (0..domain).step_by(step_end) {
                    let start = [m, 0];
                    let end = if orientation == 0 {
                        [0, n]
                    } else {
                        [n, domain - 1]
                    };
                    let mut expected = vec![false; size * size];
                    let steep = end[1] > m.abs_diff(end[0]);
                    let (mut a, mut b) = if steep {
                        ([start[1], start[0]], [end[1], end[0]])
                    } else {
                        (start, end)
                    };
                    if a[0] > b[0] {
                        std::mem::swap(&mut a, &mut b);
                    }
                    let dx = b[0] - a[0];
                    let dy = a[1].abs_diff(b[1]);
                    for x in a[0]..=b[0] {
                        // Closed-form nearest rounding rather than error accumulation.
                        let distance = if dx == 0 {
                            0
                        } else {
                            (2 * (x - a[0]) * dy + dx) / (2 * dx)
                        };
                        let y = if b[1] > a[1] {
                            a[1] + distance
                        } else {
                            a[1] - distance
                        };
                        let (px, py) = if steep {
                            (y >> shift, x >> shift)
                        } else {
                            (x >> shift, y >> shift)
                        };
                        expected[py * size + px] = true;
                    }
                    for y in 0..=(end[1] >> shift) {
                        let first = (0..size).find(|&x| expected[y * size + x]).unwrap_or(size);
                        expected[y * size..y * size + first].fill(true);
                    }
                    let mut output = vec![false; size * size];
                    pattern(size, start, end, &mut output).unwrap();
                    assert_eq!(output, expected, "size={size} start={start:?} end={end:?}");
                }
            }
        }
    }
}

#[test]
fn wedge_mask_drives_depth_dc_prediction() {
    use fvid::codec::{
        hevc_depth_prediction::{Partition, predict},
        hevc_depth_tables::{Layer, Tables},
    };
    let mut mask = [false; 16];
    pattern(4, [4, 0], [0, 4], &mut mask).unwrap();
    let tables = Tables {
        bit_depth: 8,
        layers: vec![Layer {
            id: 1,
            values: None,
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
    for i in 0..16 {
        assert_eq!(output[i], if mask[i] { 61 } else { 89 });
    }
}

#[test]
fn indexed_wedge_tables_match_generated_reference_and_budget() {
    use fvid::codec::hevc_wedge::Table;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let size = case["size"].as_u64().unwrap() as usize;
        let table = Table::new(size, 1 << 20).unwrap();
        assert_eq!(table.len(), case["count"].as_u64().unwrap() as usize);
        let expected = std::fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        assert_eq!(expected.len(), table.len() * size * size);
        let mut output = vec![false; size * size];
        for (index, reference) in expected.chunks_exact(size * size).enumerate() {
            table.get(index, &mut output).unwrap();
            assert!(
                output
                    .iter()
                    .zip(reference)
                    .all(|(&v, &r)| u8::from(v) == r)
            );
        }
        let before = output.clone();
        assert!(table.get(table.len(), &mut output).is_err());
        assert_eq!(output, before);
        assert!(Table::new(size, table.len() * 32 - 1).is_err());
        assert!(table.heap_bytes() <= 1 << 20);
    }
}

#[test]
fn arithmetic_wedge_indices_select_every_mask_and_refuse_unused_codes() {
    use fvid::codec::{
        hevc_cabac::{HevcCabac, SliceType},
        hevc_wedge::Table,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for size in [4, 8, 16, 32] {
        let table = Table::new(size, 1 << 20).unwrap();
        let reference = std::fs::read(root.join(format!("{size}.bin"))).unwrap();
        let bytes = std::fs::read(root.join(format!("{size}-indices.cabac"))).unwrap();
        let mut bins = HevcCabac::new(&bytes, 0, SliceType::I, false, 26).unwrap();
        let mut output = vec![false; size * size];
        for (index, expected) in reference.chunks_exact(size * size).enumerate() {
            assert_eq!(table.read_pattern(&mut bins, &mut output).unwrap(), index);
            assert!(output.iter().zip(expected).all(|(&v, &e)| u8::from(v) == e));
        }
        assert!(bins.terminate().unwrap());
        let invalid = std::fs::read(root.join(format!("{size}-invalid.cabac"))).unwrap();
        let mut bins = HevcCabac::new(&invalid, 0, SliceType::I, false, 26).unwrap();
        let before = output.clone();
        assert_eq!(
            table
                .read_pattern(&mut bins, &mut output)
                .unwrap_err()
                .to_string(),
            "HEVC wedge index exceeds table"
        );
        assert_eq!(output, before);
    }
}
