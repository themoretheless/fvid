const CORPUS: &str = "aac-ld-mps-arbdmx-ttt";
const CONTROLS: usize = 5;
const PROGRAMMES: usize = 270;
include!("support/aac_ld_mps_short_pcm.rs");

#[test]
fn stereo_gain_matrices_match_original_ttt_reference_before_temporal_interpolation() {
    use fvid_media::owned_aac::aac_ld_mps_assemble;
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(root().join("oracle.json")).unwrap()).unwrap();
    let golden = std::fs::read(root().join("expected.bin")).unwrap();
    let mut count = 0;
    let mut maximum = 0.0f64;
    let manifest = m();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 108);
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
                        assert!(error < 1e-5, "{} plane{i}: {value} != {want}", case["name"]);
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
        "Stereo arbitrary-downmix C/Decimal oracle: {count} matrix endpoints;max error {maximum}"
    );
}

#[test]
fn ttt_gain_order_is_noncommutative_and_keeps_m2_unchanged() {
    use fvid_media::owned_aac::{
        aac_ld_mps_tree_matrix::Band,
        aac_ld_mps_ttt_tree::{self, Ttt},
    };
    let mut difference = 0.0f64;
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
                let ordinary = aac_ld_mps_ttt_tree::build(tree, &bands, ttt, true).unwrap();
                let compensated =
                    aac_ld_mps_ttt_tree::build_with_downmix(tree, &bands, ttt, true, [0.5, 2.])
                        .unwrap();
                assert_eq!(ordinary.dry, compensated.dry);
                assert_eq!(ordinary.wet, compensated.wet);
                assert_eq!(
                    ordinary,
                    aac_ld_mps_ttt_tree::build_with_downmix(tree, &bands, ttt, true, [1., 1.])
                        .unwrap()
                );
                let mut gap = 0.0f64;
                for (normal, gained) in ordinary.m1.iter().zip(&compensated.m1) {
                    for (col, (&a, &b)) in normal.iter().zip(gained).enumerate() {
                        let factor = if col == 0 {
                            0.5
                        } else if col == 1 {
                            2.
                        } else {
                            1.
                        };
                        gap = gap
                            .max((a.re * factor - b.re).abs())
                            .max((a.im * factor - b.im).abs());
                    }
                }
                if !inversion || matches!(mode, 3 | 5) {
                    assert!(gap < 1e-12);
                } else {
                    difference = difference.max(gap);
                }
                for gain in [[-1., 1.], [f64::NAN, 1.], [1., f64::INFINITY]] {
                    assert!(
                        aac_ld_mps_ttt_tree::build_with_downmix(tree, &bands, ttt, true, gain)
                            .is_err()
                    );
                }
            }
        }
    }
    assert!(
        difference > 1e-3,
        "inversion must distinguish pre-gain from post-column scaling"
    );
    eprintln!("Incorrect post-inversion column scaling negative-control gap: {difference}");
}

#[test]
fn downmix_trees_without_qualified_gain_composition_remain_explicit() {
    use fvid_media::owned_aac::aac_ld_mps_assemble;
    let parent = root().join("../aac-ld-mps-assemble");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(parent.join("manifest.json")).unwrap()).unwrap();
    let configs = std::fs::read(parent.join("configs.bin")).unwrap();
    for tree in 5..=7 {
        let case = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| n(&c["tree"]) == tree && n(&c["variant"]) == 1)
            .unwrap();
        let at = n(&case["config_offset"]);
        let mut cfg = Config::parse(&configs[at..at + n(&case["config_bytes"])]).unwrap();
        cfg.arbitrary_downmix = true;
        assert!(
            aac_ld_mps_assemble::Decoder::new(&cfg)
                .unwrap_err()
                .to_string()
                .contains("arbitrary-downmix")
        );
    }
}
