use fvid_media::owned_aac::{
    aac_ld_mps_matrix_apply::apply,
    aac_ld_mps_matrix_time::{Layout, MatrixFrame},
    aac_sbr_qmf::Complex,
};
use serde_json::Value;
fn fixture() -> (Value, Vec<u8>) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-matrix-apply");
    (
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap(),
        std::fs::read(root.join("expected.bin")).unwrap(),
    )
}
fn stimuli(c: &Value) -> (Layout, MatrixFrame, Vec<Vec<Vec<Complex>>>) {
    let rows = c["rows"].as_u64().unwrap() as usize;
    let columns = c["columns"].as_u64().unwrap() as usize;
    let planes = c["planes"].as_u64().unwrap() as usize;
    let bands = c["bands"].as_u64().unwrap() as usize;
    let bins = c["bins"].as_u64().unwrap() as usize;
    let mut slots = Vec::new();
    for t in 0..2 {
        let mut values = Vec::new();
        for b in 0..bands {
            for p in 0..planes {
                for r in 0..rows {
                    for col in 0..columns {
                        values.push(
                            ((t * 17 + b * 13 + p * 11 + r * 7 + col * 3) % 29) as f64 / 32.0
                                - 14.0 / 32.0,
                        );
                    }
                }
            }
        }
        slots.push(values);
    }
    let input = (0..2)
        .map(|t| {
            (0..columns)
                .map(|col| {
                    (0..bins)
                        .map(|q| Complex {
                            re: ((t * 7 + col * 5 + q * 3) % 23) as f64 / 16.0 - 11.0 / 16.0,
                            im: ((t * 11 + col * 3 + q * 7) % 19) as f64 / 16.0 - 9.0 / 16.0,
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    (
        Layout {
            rows: rows as u8,
            columns: columns as u8,
            planes: planes as u8,
        },
        MatrixFrame {
            endpoints: vec![],
            slots,
        },
        input,
    )
}
#[test]
fn exact_integer_oracle_all_resolutions_and_matrix_dimensions() {
    let (manifest, bytes) = fixture();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 63);
    for c in cases {
        let (layout, frame, input) = stimuli(c);
        let bands = c["bands"].as_u64().unwrap() as u8;
        let bins = c["bins"].as_u64().unwrap() as usize;
        let real = c["real"].as_u64().unwrap() as u8;
        let imag = c["imag"].as_u64().map(|p| p as u8);
        let out = apply(&frame, layout, bands, bins, real, imag, &input).unwrap();
        let mut offset = c["offset"].as_u64().unwrap() as usize;
        for value in out.iter().flatten().flatten() {
            for actual in [value.re, value.im] {
                let expected = i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
                    as f64
                    / 512.0;
                assert_eq!(actual, expected, "case {c}, offset {offset}");
                offset += 8;
            }
        }
        // Slot-local application agrees with a whole frame, and never modifies
        // caller-owned input/coefficient histories.
        for t in 0..2 {
            let part = MatrixFrame {
                slots: vec![frame.slots[t].clone()],
                endpoints: vec![],
            };
            assert_eq!(
                apply(&part, layout, bands, bins, real, imag, &input[t..t + 1]).unwrap()[0],
                out[t]
            );
        }
    }
}
#[test]
fn invalid_geometry_nonfinite_and_overflow_are_refused() {
    let (m, _) = fixture();
    let c = m["cases"].as_array().unwrap().last().unwrap();
    let (layout, frame, input) = stimuli(c);
    let bands = 23;
    let bins = 128;
    let run = |f: &MatrixFrame, l: Layout, x: &[Vec<Vec<Complex>>], r, i| {
        apply(f, l, bands, bins, r, i, x)
    };
    assert!(run(&frame, layout, &input, 7, None).is_err());
    assert!(run(&frame, layout, &input, 5, Some(5)).is_err());
    let mut bad = frame.clone();
    bad.slots[1].pop();
    assert!(run(&bad, layout, &input, 5, Some(6)).is_err());
    let mut bad = frame.clone();
    bad.slots[1][0] = f64::NAN;
    assert!(run(&bad, layout, &input, 5, Some(6)).is_err());
    let mut x = input.clone();
    x[1][12][127].im = f64::INFINITY;
    assert!(run(&frame, layout, &x, 5, Some(6)).is_err());
    let mut x = input.clone();
    x[1].pop();
    assert!(run(&frame, layout, &x, 5, Some(6)).is_err());
    let mut f = frame.clone();
    f.slots[1].fill(f64::MAX);
    let mut x = input.clone();
    x[1].iter_mut()
        .for_each(|v| v.fill(Complex { re: 2.0, im: 0.0 }));
    assert!(run(&f, layout, &x, 5, None).is_err());
    let empty = MatrixFrame {
        slots: vec![],
        endpoints: vec![],
    };
    assert!(run(&empty, layout, &[], 5, Some(6)).unwrap().is_empty());
    assert!(
        run(
            &frame,
            Layout {
                columns: 0,
                ..layout
            },
            &input,
            5,
            None
        )
        .is_err()
    );
    assert!(apply(&frame, layout, 6, bins, 5, None, &input).is_err());
    assert!(apply(&frame, layout, bands, 63, 5, None, &input).is_err());
}

#[test]
fn interpolated_m1_then_m2_preserves_complex_rotation_and_channel_order() {
    use fvid_media::owned_aac::{
        aac_ld_mps_matrix_time::MatrixTime, aac_ld_mps_prepare::SmoothControl,
    };
    let layout = Layout {
        planes: 2,
        rows: 2,
        columns: 2,
    };
    let mut time = MatrixTime::new(4, layout).unwrap();
    // M1 = [[i,1],[0,-i]], M2 = [[0,1],[1,0]]. M2*M1 is
    // [[0,-i],[i,1]], so stage order is observable.
    let raw = (0..4)
        .flat_map(|_| [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0])
        .collect();
    let frame = time
        .process(
            2,
            &[1],
            &[SmoothControl {
                time: 0,
                mask: vec![false; 4],
            }],
            &[raw],
            0,
        )
        .unwrap();
    let input = vec![
        vec![
            vec![Complex { re: 2.0, im: 3.0 }; 32],
            vec![Complex { re: 5.0, im: 7.0 }; 32]
        ];
        2
    ];
    let first = apply(&frame, layout, 4, 32, 0, Some(1), &input).unwrap();
    let swap = MatrixFrame {
        endpoints: vec![],
        slots: vec![(0..4).flat_map(|_| [0.0, 1.0, 1.0, 0.0]).collect(); 2],
    };
    let second = apply(
        &swap,
        Layout {
            planes: 1,
            ..layout
        },
        4,
        32,
        0,
        None,
        &first,
    )
    .unwrap();
    for (t, scale) in [0.5, 1.0].iter().enumerate() {
        for q in 0..32 {
            assert_eq!(
                second[t][0][q],
                Complex {
                    re: 7.0 * scale,
                    im: -5.0 * scale
                }
            );
            assert_eq!(
                second[t][1][q],
                Complex {
                    re: 2.0 * scale,
                    im: 9.0 * scale
                }
            );
        }
    }
    time.reset();
    let raw = (0..4)
        .flat_map(|_| [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0])
        .collect();
    assert_eq!(
        time.process(
            2,
            &[1],
            &[SmoothControl {
                time: 0,
                mask: vec![false; 4]
            }],
            &[raw],
            0
        )
        .unwrap(),
        frame
    );
}
