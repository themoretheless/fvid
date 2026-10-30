use fvid_media::owned_aac::{
    aac_ld_mps_entropy::Kind,
    aac_ld_mps_ott::Ott,
    aac_ld_mps_prepare::dequantize,
    aac_ld_mps_tree_matrix::{Band, build},
};
use serde_json::Value;
fn fixtures() -> (Value, Vec<u8>) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-tree-matrix");
    (
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap(),
        std::fs::read(root.join("expected.bin")).unwrap(),
    )
}
fn parameters(c: &Value) -> Vec<Band> {
    c["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Band {
            cld_db: dequantize(Kind::Cld, p["cld"].as_i64().unwrap() as i16).unwrap(),
            coherence: dequantize(Kind::Icc, p["icc"].as_i64().unwrap() as i16).unwrap(),
            residual: p["residual"].as_bool().unwrap(),
        })
        .collect()
}
#[test]
fn ott_forest_matrices_match_frozen_original_tree_routines() {
    let (m, blob) = fixtures();
    assert_eq!(m["cases"].as_array().unwrap().len(), 1184);
    assert_eq!(blob.len(), 715264);
    let mut maximum = 0.0f64;
    let mut stereo_differences = 0;
    for c in m["cases"].as_array().unwrap() {
        let tree = c["tree"].as_u64().unwrap() as u8;
        let p = parameters(c);
        let matrices = build(tree, &p, c["lfe_active"].as_bool().unwrap()).unwrap();
        let inputs = c["inputs"].as_u64().unwrap() as usize;
        let outputs = c["outputs"].as_u64().unwrap() as usize;
        let columns = c["columns"].as_u64().unwrap() as usize;
        assert_eq!(matrices.m1.len(), columns);
        assert!(matrices.m1.iter().all(|r| r.len() == inputs));
        assert_eq!(matrices.dry.len(), outputs);
        assert_eq!(matrices.wet.len(), outputs);
        assert!(
            matrices
                .dry
                .iter()
                .chain(&matrices.wet)
                .all(|r| r.len() == columns)
        );
        let mut offset = c["offset"].as_u64().unwrap() as usize;
        for (plane, rows) in [&matrices.m1, &matrices.dry, &matrices.wet]
            .into_iter()
            .enumerate()
        {
            for (r, row) in rows.iter().enumerate() {
                for (col, &actual) in row.iter().enumerate() {
                    let reference =
                        f64::from_le_bytes(blob[offset..offset + 8].try_into().unwrap());
                    offset += 8;
                    if tree == 7 && plane == 1 && col == 1 {
                        // Original 212 writes H12/H22 into the residual plane instead of
                        // H12_res/H22_res. Preserve its raw output as evidence, while checking
                        // the owned residual injection against the covariance kernel.
                        let h = Ott::physical(p[0].cld_db, p[0].coherence, p[0].residual).unwrap();
                        assert!((reference - h.rows[r][1]).abs() < 2e-6);
                        assert_eq!(actual, h.rows[r][2]);
                        stereo_differences += 1;
                    } else {
                        let error = (actual - reference).abs();
                        maximum = maximum.max(error);
                        assert!(
                            error < 2e-6,
                            "tree {tree} plane {plane} row {r} col {col}: {actual} vs {reference}; case {c}"
                        );
                    }
                }
            }
        }
    }
    assert_eq!(stereo_differences, 64);
    println!("maximum original C coefficient error: {maximum}");
}
#[test]
fn passthrough_lfe_cutoff_and_residual_basis_have_explicit_routing() {
    let (m, _) = fixtures();
    for c in m["cases"].as_array().unwrap() {
        let tree = c["tree"].as_u64().unwrap() as u8;
        let p = parameters(c);
        let a = build(tree, &p, c["lfe_active"].as_bool().unwrap()).unwrap();
        let inputs = c["inputs"].as_u64().unwrap() as usize;
        for input in 0..inputs {
            for col in 0..inputs {
                assert_eq!(a.m1[input][col], if input == col { 1.0 } else { 0.0 });
            }
        }
        for (j, &box_index) in a.auxiliary_boxes.iter().enumerate() {
            let col = inputs + j;
            if p[box_index].residual {
                assert!(a.wet.iter().all(|r| r[col] == 0.0));
            } else {
                assert!(a.dry.iter().all(|r| r[col] == 0.0));
            }
        }
        if matches!(tree, 0 | 1) && !c["lfe_active"].as_bool().unwrap() {
            let row = if tree == 0 { 3 } else { 5 };
            assert!(a.dry[row].iter().all(|&v| v == 0.0));
            assert!(a.wet[row].iter().all(|&v| v == 0.0));
        }
        if matches!(tree, 5 | 6) {
            for (row, input) in [(6, 2), (7, 3)] {
                assert_eq!(a.dry[row][input], 1.0);
                assert!(a.wet[row].iter().all(|&v| v == 0.0));
            }
        }
    }
}
#[test]
fn invalid_physical_boxes_and_unimplemented_ttt_are_explicit() {
    let valid = Band {
        cld_db: 0.0,
        coherence: 0.5,
        residual: false,
    };
    for tree in 2..=4 {
        assert!(
            build(tree, &[], false)
                .unwrap_err()
                .to_string()
                .contains("TTT")
        );
    }
    assert!(build(8, &[], false).is_err());
    assert!(build(0, &[valid], false).is_err());
    assert!(
        build(
            7,
            &[Band {
                cld_db: f64::NAN,
                ..valid
            }],
            false
        )
        .is_err()
    );
    assert!(
        build(
            7,
            &[Band {
                coherence: 1.1,
                ..valid
            }],
            false
        )
        .is_err()
    );
}

#[test]
fn stereo_residual_has_signed_unit_injection_instead_of_reference_zero_column() {
    // Equal powers and zero coherence give direct coefficients 1/2.
    // Residual R must produce [X/2+R, X/2-R], not two identical outputs.
    let h = build(
        7,
        &[Band {
            cld_db: 0.0,
            coherence: 0.0,
            residual: true,
        }],
        false,
    )
    .unwrap();
    assert_eq!(h.dry, vec![vec![0.5, 1.0], vec![0.5, -1.0]]);
    assert_eq!(h.dry[0][0] * 2.0 + h.dry[0][1] * 0.25, 1.25);
    assert_eq!(h.dry[1][0] * 2.0 + h.dry[1][1] * 0.25, 0.75);
    assert!(h.wet.iter().all(|r| r[1] == 0.0));
    let p = Band {
        cld_db: 0.0,
        coherence: 0.0,
        residual: false,
    };
    let mut boxes = [p; 5];
    boxes[4].residual = true;
    assert!(
        build(0, &boxes, true)
            .unwrap_err()
            .to_string()
            .contains("LFE residual")
    );
}
