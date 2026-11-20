use fvid_media::owned_aac::{
    aac_ld_mps_entropy::Kind,
    aac_ld_mps_prepare::dequantize,
    aac_ld_mps_tree_matrix::Band,
    aac_ld_mps_ttt_tree::{Auxiliary, Ttt, build},
    aac_sbr_qmf::Complex,
};
use serde_json::Value;
fn fixtures() -> (Value, Vec<u8>) {
    let r = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-ttt-tree");
    (
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap(),
        std::fs::read(r.join("expected.bin")).unwrap(),
    )
}
fn parameters(c: &Value) -> (Vec<Band>, Ttt) {
    let p = c["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Band {
            cld_db: dequantize(Kind::Cld, p["cld"].as_i64().unwrap() as i16).unwrap(),
            coherence: dequantize(Kind::Icc, p["icc"].as_i64().unwrap() as i16).unwrap(),
            residual: p["residual"].as_bool().unwrap(),
        })
        .collect();
    let mode = c["mode"].as_u64().unwrap() as u8;
    let kind = if mode < 2 { Kind::Cpc } else { Kind::Cld };
    (
        p,
        Ttt {
            mode,
            parameters: [
                dequantize(kind, c["first"].as_i64().unwrap() as i16).unwrap(),
                dequantize(kind, c["second"].as_i64().unwrap() as i16).unwrap(),
            ],
            coherence: dequantize(Kind::Icc, c["icc"].as_i64().unwrap() as i16).unwrap(),
            residual: c["ttt_residual"].as_bool().unwrap(),
            inversion: c["inversion"].as_bool().unwrap(),
        },
    )
}
#[test]
fn full_ttt_tree_matrices_match_unmodified_original_iso_routines() {
    let (m, blob) = fixtures();
    assert_eq!(m["cases"].as_array().unwrap().len(), 6912);
    let mut maximum = 0.0f64;
    let mut scalar_count = 0;
    for c in m["cases"].as_array().unwrap() {
        let (p, t) = parameters(c);
        let a = build(
            c["tree"].as_u64().unwrap() as u8,
            &p,
            t,
            c["lfe_active"].as_bool().unwrap(),
        )
        .unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let outputs = c["outputs"].as_u64().unwrap() as usize;
        assert_eq!(a.m1.len(), channels);
        assert!(a.m1.iter().all(|r| r.len() == 3));
        assert_eq!(a.dry.len(), outputs);
        assert_eq!(a.wet.len(), outputs);
        assert!(a.dry.iter().chain(&a.wet).all(|r| r.len() == channels));
        let got =
            a.m1.iter()
                .flatten()
                .flat_map(|z| [z.re, z.im])
                .chain(a.dry.iter().flatten().copied())
                .chain(a.wet.iter().flatten().copied());
        let mut offset = c["offset"].as_u64().unwrap() as usize;
        for v in got {
            let reference = f64::from_le_bytes(blob[offset..offset + 8].try_into().unwrap());
            offset += 8;
            scalar_count += 1;
            let error = (v - reference).abs();
            maximum = maximum.max(error);
            assert!(
                error < 5e-6,
                "{v} vs {reference}, offset {offset}, case {c}"
            );
        }
    }
    assert_eq!(scalar_count * 8, blob.len());
    println!("TTT tree original C max coefficient error: {maximum}; scalars: {scalar_count}");
}
#[test]
fn mode_gating_residual_columns_and_lfe_cutoff_remain_explicit() {
    let (m, _) = fixtures();
    for c in m["cases"].as_array().unwrap() {
        let (p, t) = parameters(c);
        let tree = c["tree"].as_u64().unwrap() as u8;
        let active = c["lfe_active"].as_bool().unwrap();
        let a = build(tree, &p, t, active).unwrap();
        assert_eq!(a.auxiliaries[2], Auxiliary::Ttt);
        if t.mode != 0 {
            assert!(a.m1[5].iter().all(|z| *z == Complex::default()));
        }
        if t.mode != 0 || t.residual {
            assert!(a.wet.iter().all(|r| r[5] == 0.0));
        }
        if tree == 2 && t.mode == 0 && !t.residual {
            for (dry, wet) in a.dry.iter().zip(&a.wet) {
                assert_eq!(dry[5], wet[5]);
            }
        } else {
            assert!(a.dry.iter().all(|r| r[5] == 0.0));
        }
        for (j, aux) in a.auxiliaries.iter().enumerate() {
            if let Auxiliary::Ott(b) = *aux {
                let col = j + 3;
                if p[b].residual {
                    assert!(a.wet.iter().all(|r| r[col] == 0.0));
                } else {
                    assert!(a.dry.iter().all(|r| r[col] == 0.0));
                }
            }
        }
        if !active {
            assert!(a.dry.last().unwrap().iter().all(|&v| v == 0.0));
            assert!(a.wet.last().unwrap().iter().all(|&v| v == 0.0));
        }
        if matches!(t.mode, 3 | 5) {
            let other = build(
                tree,
                &p,
                Ttt {
                    inversion: !t.inversion,
                    ..t
                },
                active,
            )
            .unwrap();
            assert_eq!(a, other);
        }
    }
}
#[test]
fn invalid_counts_modes_and_singular_prediction_are_refused() {
    let p = Band {
        cld_db: 0.0,
        coherence: 1.0,
        residual: false,
    };
    let t = Ttt {
        mode: 0,
        parameters: [1.0, 1.0],
        coherence: 1.0,
        residual: false,
        inversion: false,
    };
    assert!(build(1, &[p; 3], t, true).is_err());
    assert!(build(3, &[p; 3], t, true).is_err());
    assert!(build(2, &[p; 3], Ttt { mode: 6, ..t }, true).is_err());
    assert!(
        build(
            2,
            &[p; 3],
            Ttt {
                coherence: 0.0,
                ..t
            },
            true
        )
        .unwrap_err()
        .to_string()
        .contains("singular")
    );
    assert!(
        build(
            2,
            &[p; 3],
            Ttt {
                coherence: 0.0,
                residual: true,
                ..t
            },
            true
        )
        .is_ok()
    );
    let mut bad = [p; 3];
    bad[0].residual = true;
    assert!(
        build(2, &bad, t, true)
            .unwrap_err()
            .to_string()
            .contains("LFE residual")
    );
    bad = [p; 3];
    bad[2].cld_db = f64::NAN;
    assert!(build(2, &bad, t, true).is_err());
}

#[test]
fn constructed_m1_and_m2_feed_the_existing_complex_qmf_executor() {
    use fvid_media::owned_aac::{
        aac_ld_mps_matrix_apply::apply,
        aac_ld_mps_matrix_time::{Layout, MatrixFrame},
    };
    let p = Band {
        cld_db: 0.0,
        coherence: 1.0,
        residual: false,
    };
    let t = Ttt {
        mode: 0,
        parameters: [1.0, 1.0],
        coherence: 1.0,
        residual: false,
        inversion: false,
    };
    for tree in 2..=4 {
        let a = build(tree, &vec![p; if tree == 2 { 3 } else { 5 }], t, false).unwrap();
        let channels = a.m1.len();
        let plane: Vec<f64> = [false, true]
            .into_iter()
            .flat_map(|imag| {
                a.m1.iter()
                    .flatten()
                    .map(move |z| if imag { z.im } else { z.re })
            })
            .collect();
        let frame = MatrixFrame {
            endpoints: vec![],
            slots: vec![plane.repeat(4)],
        };
        let input = vec![vec![
            vec![Complex { re: 2.0, im: 3.0 }; 32],
            vec![Complex { re: 5.0, im: 7.0 }; 32],
            vec![Complex::default(); 32],
        ]];
        let virtuals = apply(
            &frame,
            Layout {
                planes: 2,
                rows: channels as u8,
                columns: 3,
            },
            4,
            32,
            0,
            Some(1),
            &input,
        )
        .unwrap();
        // Direct W columns use the first three virtual signals. Auxiliary dry
        // columns remain zero outside residual bands; wet direct columns are zero.
        let mut dry = vec![vec![vec![Complex::default(); 32]; channels]];
        dry[0][..3].clone_from_slice(&virtuals[0][..3]);
        let frame = MatrixFrame {
            endpoints: vec![],
            slots: vec![
                a.dry
                    .iter()
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>()
                    .repeat(4),
            ],
        };
        let output = apply(
            &frame,
            Layout {
                planes: 1,
                rows: a.dry.len() as u8,
                columns: channels as u8,
            },
            4,
            32,
            0,
            None,
            &dry,
        )
        .unwrap();
        let half = 0.5f64.sqrt();
        let gains = match tree {
            2 => vec![
                (0, half),
                (0, half),
                (1, half),
                (1, half),
                (2, 0.0),
                (2, 0.0),
            ],
            3 => vec![
                (0, 0.5),
                (0, 0.5),
                (0, half),
                (1, 0.5),
                (1, 0.5),
                (1, half),
                (2, 0.0),
                (2, 0.0),
            ],
            _ => vec![
                (0, half),
                (0, 0.5),
                (0, 0.5),
                (1, half),
                (1, 0.5),
                (1, 0.5),
                (2, 0.0),
                (2, 0.0),
            ],
        };
        for (row, (source, gain)) in gains.into_iter().enumerate() {
            for q in 0..32 {
                let actual = output[0][row][q];
                let expected = input[0][source][q];
                assert!((actual.re - expected.re * gain).abs() < 2e-14);
                assert!((actual.im - expected.im * gain).abs() < 2e-14);
            }
        }
    }
}
