const CORPUS: &str = "aac-ld-mps-3d";
const CONTROLS: usize = 5;
const PROGRAMMES: usize = 540;
include!("support/aac_ld_mps_short_pcm.rs");

#[test]
fn signalled_3d_matrices_match_c_basis_and_high_precision_polar_reference() {
    use fvid_media::owned_aac::aac_ld_mps_assemble;
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(root().join("oracle.json")).unwrap()).unwrap();
    let golden = std::fs::read(root().join("expected.bin")).unwrap();
    let mut count = 0;
    let mut maximum = 0.0f64;
    let manifest = m();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 216);
    for control in oracle["cases"].as_array().unwrap() {
        let case = cases.iter().find(|c| c["name"] == control["name"]).unwrap();
        assert_eq!(case["name"], control["name"]);
        let cfg = Config::parse(&hex(&case["ssc"])).unwrap();
        assert!(cfg.arbitrary_downmix);
        let mut decoder = aac_ld_mps_assemble::Decoder::new(&cfg).unwrap();
        for (wire, expected) in case["reference"]
            .as_array()
            .unwrap()
            .iter()
            .zip(control["frames"].as_array().unwrap())
        {
            let packet = hex(wire);
            let saved = decoder.clone();
            let mut short = BitReader::new(&packet[..packet.len() - 3]);
            assert!(decoder.decode(&mut short).is_err());
            assert_eq!(short.position(), 0);
            assert_eq!(decoder, saved);
            let actual = decoder.decode(&mut BitReader::new(&packet)).unwrap();
            for (i, matrix) in [&actual.m1, &actual.dry, &actual.wet]
                .into_iter()
                .enumerate()
            {
                let mut at = n(&expected["offsets"][i]);
                assert_eq!(
                    matrix.endpoints.len(),
                    expected["slots"].as_array().unwrap().len()
                );
                for endpoint in &matrix.endpoints {
                    assert_eq!(endpoint.len(), n(&expected["widths"][i]));
                    for &value in endpoint {
                        let want =
                            f64::from(f32::from_le_bytes(golden[at..at + 4].try_into().unwrap()));
                        assert!(value.is_finite());
                        let error = (value - want).abs();
                        assert!(
                            error < 3e-4,
                            "{} plane{i} scalar{}: {value} != {want}",
                            case["name"],
                            (at - n(&expected["offsets"][i])) / 4
                        );
                        maximum = maximum.max(error);
                        count += 1;
                        at += 4;
                    }
                }
            }
        }
    }
    assert_eq!(count * 4, golden.len());
    eprintln!(
        "3D-stereo C-basis/Decimal-polar C/Decimal oracle: {count} matrix endpoints;max error {maximum}"
    );
}

fn hrtf(bands: usize) -> fvid_media::owned_aac::aac_ld_mps_config::Hrtf {
    use fvid_media::owned_aac::aac_ld_mps_config::{Hrtf, HrtfChannel};
    Hrtf {
        bands: bands as u8,
        asymmetric: true,
        channels: (0..5)
            .map(|ch| HrtfChannel {
                left: vec![[25, 9, 18, 21, 7][ch]; bands],
                right: vec![[9, 25, 19, 7, 21][ch]; bands],
                phase: Some(vec![7; bands]),
            })
            .collect(),
    }
}
#[test]
fn determinant_floor_has_independent_analytic_rank_deficient_controls() {
    use fvid_media::owned_aac::{aac_ld_mps_3d, aac_sbr_qmf::Complex};
    let z = |re| Complex { re, im: 0. };
    let root = 10.0f64.sqrt();
    for (input, expected) in [
        ([[1., 2.], [1., 2.]], [[20.1, -19.9], [-9.8, 10.2]]),
        ([[0.2, 0.], [0., 0.1]], [[root, 0.], [0., root]]),
        ([[2., 0.], [0., 0.01]], [[0.5, 0.], [0., 20.]]),
        ([[0., 0.], [0., 0.]], [[root, 0.], [0., root]]),
    ] {
        let result = aac_ld_mps_3d::invert(input.map(|r| r.map(z))).unwrap();
        assert!(result.regularized);
        for r in 0..2 {
            for c in 0..2 {
                assert!((result.inverse[r][c].re - expected[r][c]).abs() < 1e-10);
                assert!(result.inverse[r][c].im.abs() < 1e-10);
            }
        }
        for r in 0..2 {
            for c in 0..2 {
                let real: f64 = (0..2)
                    .map(|k| {
                        result.forward[r][k].re * result.inverse[k][c].re
                            - result.forward[r][k].im * result.inverse[k][c].im
                    })
                    .sum();
                let imag: f64 = (0..2)
                    .map(|k| {
                        result.forward[r][k].re * result.inverse[k][c].im
                            + result.forward[r][k].im * result.inverse[k][c].re
                    })
                    .sum();
                assert!((real - f64::from(r == c)).abs() < 1e-10);
                assert!(imag.abs() < 1e-10);
            }
        }
    }
    let input = [
        [Complex { re: 1., im: 0.2 }, Complex { re: 0.1, im: -0.3 }],
        [Complex { re: 0.2, im: 0.15 }, Complex { re: 0.7, im: -0.1 }],
    ];
    let ordinary = aac_ld_mps_3d::invert(input).unwrap();
    assert!(!ordinary.regularized);
    for r in 0..2 {
        for c in 0..2 {
            let real: f64 = (0..2)
                .map(|k| {
                    input[r][k].re * ordinary.inverse[k][c].re
                        - input[r][k].im * ordinary.inverse[k][c].im
                })
                .sum();
            let imag: f64 = (0..2)
                .map(|k| {
                    input[r][k].re * ordinary.inverse[k][c].im
                        + input[r][k].im * ordinary.inverse[k][c].re
                })
                .sum();
            assert!((real - f64::from(r == c)).abs() < 1e-12);
            assert!(imag.abs() < 1e-12);
        }
    }
    assert!(aac_ld_mps_3d::invert([[z(f64::NAN), z(0.)], [z(0.), z(1.)]]).is_err());
}
#[test]
fn signalled_3d_phase_cutoff_and_standard_inverse_priority_are_explicit() {
    use fvid_media::owned_aac::{
        aac_ld_mps_3d,
        aac_ld_mps_tree_matrix::Band,
        aac_ld_mps_ttt,
        aac_ld_mps_ttt_tree::{self, ThreeD, Ttt},
    };
    let h = hrtf(23);
    let mut zero = h.clone();
    for c in &mut zero.channels {
        c.phase = None;
    }
    let pre = aac_ld_mps_ttt::transform(2, -4., 6., 1., false).unwrap();
    assert_eq!(
        aac_ld_mps_3d::project(pre, [3., -4.], &h, 12, 1.).unwrap(),
        aac_ld_mps_3d::project(pre, [3., -4.], &zero, 12, 1.).unwrap()
    );
    assert_ne!(
        aac_ld_mps_3d::project(pre, [3., -4.], &h, 11, 1.).unwrap(),
        aac_ld_mps_3d::project(pre, [3., -4.], &zero, 11, 1.).unwrap()
    );
    let mut changed = 0.0f64;
    for tree in 2..=4 {
        let boxes = if tree == 2 { 3 } else { 5 };
        let bands: Vec<_> = [3., 6., -4., 8., 2.][..boxes]
            .iter()
            .map(|&cld_db| Band {
                cld_db,
                coherence: 0.5,
                residual: false,
            })
            .collect();
        for mode in 0..6 {
            for inversion in [false, true] {
                let ttt = Ttt {
                    mode,
                    parameters: if mode < 2 { [0.2, 0.1] } else { [-4., 6.] },
                    coherence: 0.7,
                    residual: false,
                    inversion,
                };
                let old = aac_ld_mps_ttt_tree::build(tree, &bands, ttt, true).unwrap();
                let new = aac_ld_mps_ttt_tree::build_with_spatial(
                    tree,
                    &bands,
                    ttt,
                    true,
                    [1., 1.],
                    Some(ThreeD {
                        hrtf: &h,
                        band: 0,
                        surround_gain: 1.,
                    }),
                )
                .unwrap();
                assert_eq!(old.dry, new.dry);
                assert_eq!(old.wet, new.wet);
                if inversion && matches!(mode, 0 | 1 | 2 | 4) {
                    assert_eq!(old, new);
                } else {
                    for (a, b) in old.m1.iter().flatten().zip(new.m1.iter().flatten()) {
                        changed = changed.max((a.re - b.re).abs()).max((a.im - b.im).abs());
                    }
                }
            }
        }
    }
    assert!(changed > 0.01);
    assert!(
        aac_ld_mps_3d::validate(&h, 9)
            .unwrap_err()
            .to_string()
            .contains("grid conversion")
    );
    let mut bad = h.clone();
    bad.channels[0].left[0] = 64;
    assert!(aac_ld_mps_3d::validate(&bad, 23).is_err());
}

#[test]
fn unequal_hrtf_grid_movie_reproduces_specific_mapping_refusal() {
    let manifest = m();
    let bad = &manifest["errors"][0];
    let config = Config::parse(&hex(&bad["ssc"])).unwrap();
    assert_eq!(config.parameter_bands, 23);
    assert_eq!(config.hrtf.as_ref().unwrap().bands, 9);
    let error = fvid::native_media::decode_mp4_aac_pcm(&read(&bad["video"]), &mut Vec::new())
        .unwrap_err()
        .to_string();
    assert!(error.contains(bad["error"].as_str().unwrap()), "{error}");
}
