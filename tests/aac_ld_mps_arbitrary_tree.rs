use fvid_media::owned_aac::{aac_ld_mps_arbitrary_tree::Decoder, aac_sbr_qmf::Complex};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-arbitrary-tree")
}
fn json(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn floats(b: &[u8]) -> Vec<f64> {
    b.chunks_exact(4)
        .map(|v| f64::from(f32::from_le_bytes(v.try_into().unwrap())))
        .collect()
}
#[test]
fn post_shape_routing_matches_unmodified_iso_c_with_history_reset_and_rollback() {
    let m = json("component.json");
    let oracle = json("component-oracle.json");
    let input = std::fs::read(root().join("component-input.f32le")).unwrap();
    let expected = std::fs::read(root().join("component-expected.f32le")).unwrap();
    let mut count = 0;
    let mut maximum = 0.0f64;
    assert_eq!(m["cases"].as_array().unwrap().len(), 42);
    for (case, reference) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        let nodes: Vec<Vec<bool>> = serde_json::from_value(case["nodes"].clone()).unwrap();
        let cutoffs: Vec<u8> = serde_json::from_value(case["cutoffs"].clone()).unwrap();
        let positions: Vec<u8> = serde_json::from_value(case["positions"].clone()).unwrap();
        let map: Vec<u8> = serde_json::from_value(case["map"].clone()).unwrap();
        let fixed: Vec<f64> = serde_json::from_value(case["fixed"].clone()).unwrap();
        let mut d = Decoder::new(&nodes, &cutoffs, &positions, n(&case["bands"]) as u8).unwrap();
        let initial = d.clone();
        for (block, gold) in case["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(reference["blocks"].as_array().unwrap())
        {
            let slots = n(&block["slots"]);
            let channels = n(&case["inputs"]);
            let bank = n(&case["bank"]);
            let at = n(&block["offset"]);
            let values = floats(&input[at..at + slots * channels * bank * 8]);
            let mut source = vec![vec![vec![Complex::default(); bank]; channels]; slots];
            for (t, row) in source.iter_mut().enumerate() {
                for (c, bins) in row.iter_mut().enumerate() {
                    for (q, z) in bins.iter_mut().enumerate() {
                        let i = ((t * channels + c) * bank + q) * 2;
                        *z = Complex {
                            re: values[i],
                            im: values[i + 1],
                        };
                    }
                }
            }
            let endpoints: Vec<u8> = serde_json::from_value(block["endpoints"].clone()).unwrap();
            let cld: Vec<Vec<Vec<f64>>> = serde_json::from_value(block["cld"].clone()).unwrap();
            let checkpoint = d.clone();
            let mut bad = source.clone();
            bad[slots - 1][channels - 1][bank - 1].re = f64::NAN;
            assert!(d.process(&bad, &endpoints, &cld, &map, &fixed).is_err());
            assert_eq!(d, checkpoint);
            let got = d.process(&source, &endpoints, &cld, &map, &fixed).unwrap();
            let mut replay = checkpoint;
            assert_eq!(
                got,
                replay
                    .process(&source, &endpoints, &cld, &map, &fixed)
                    .unwrap()
            );
            assert_eq!(d, replay);
            let at = n(&gold["offset"]);
            let want = floats(&expected[at..at + slots * n(&case["outputs"]) * bank * 8]);
            for (a, b) in got
                .iter()
                .flatten()
                .flatten()
                .flat_map(|z| [z.re, z.im])
                .zip(want)
            {
                let error = (a - b).abs();
                maximum = maximum.max(error);
                assert!(
                    error <= 2e-7 * (1.0 + b.abs()),
                    "tree{} bank{bank} band{}: {a} != {b}",
                    case["tree"],
                    case["bands"]
                );
                count += 1;
            }
        }
        d.reset();
        assert_eq!(d, initial);
    }
    eprintln!("Arbitrary-tree original C: {count} scalar outputs, max error {maximum}");
}
#[test]
fn extra_ott_movie_has_complete_valid_parameters_but_is_still_a_playback_refusal() {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::{Config, ExtensionValue},
        aac_ld_mps_frame,
        bits::BitReader,
    };
    let m = json("manifest.json");
    let c = &m["errors"][0];
    let hex = |v: &Value| {
        let s = v.as_str().unwrap();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>()
    };
    let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
    cfg.validate_low_delay_profile().unwrap();
    let ExtensionValue::ArbitraryTree {
        nodes,
        ott,
        positions,
    } = &cfg.extensions[0].value
    else {
        panic!("missing additional OTT")
    };
    assert_eq!(nodes, &[vec![true, false, false], vec![false]]);
    assert_eq!(ott.len(), 1);
    assert_eq!(positions, &[0, 1, 2]);
    let mut syntax = aac_ld_mps_frame::Decoder::new(&cfg).unwrap();
    for wire in c["reference"].as_array().unwrap() {
        let data = hex(wire);
        let mut b = BitReader::new(&data);
        let f = syntax.decode(&mut b).unwrap();
        assert_eq!(b.remaining(), 0);
        assert_eq!(f.extensions[0].parameters.len(), 1);
        assert_eq!(f.extensions[0].parameters[0].values.len(), 2);
    }
    for mkv in [false, true] {
        let mut out = Vec::new();
        let bytes =
            std::fs::read(root().join(c[if mkv { "file" } else { "video" }].as_str().unwrap()))
                .unwrap();
        let error = if mkv {
            fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                std::io::Cursor::new(bytes),
                &mut out,
                None,
                &Default::default(),
            )
            .unwrap_err()
            .to_string()
        } else {
            fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut out)
                .unwrap_err()
                .to_string()
        };
        assert!(error.contains(c["error"].as_str().unwrap()), "{error}");
        assert!(out.is_empty());
    }
}
#[test]
fn malformed_topology_and_unqualified_domains_do_not_construct() {
    assert!(Decoder::new(&[vec![true, false]], &[4], &[0, 1], 4).is_err());
    assert!(Decoder::new(&[vec![false, false]], &[], &[0], 4).is_err());
    assert!(Decoder::new(&[vec![true, false, false]], &[4], &[0, 0], 4).is_err());
    assert!(Decoder::new(&[vec![true, false, false]], &[3], &[0, 1], 4)
        .unwrap_err()
        .to_string()
        .contains("partial-band"));
    assert!(Decoder::new(&[vec![false]], &[], &[2], 4)
        .unwrap_err()
        .to_string()
        .contains("sparse"));
}

#[test]
fn balanced_split_has_analytic_energy_and_unity_initialized_gain_ramps() {
    let mut d = Decoder::new(
        &[vec![true, true, false, false, true, false, false]],
        &[4, 4, 4],
        &[3, 1, 0, 2],
        4,
    )
    .unwrap();
    let input = vec![vec![vec![Complex { re: 1.0, im: 0.0 }; 4]]; 5];
    let levels = vec![vec![vec![0.0; 4]; 3]];
    let first = d
        .process(&input, &[4], &levels, &[0, 1, 2, 3], &[1.0])
        .unwrap();
    for (t, row) in first.iter().enumerate() {
        let expected = 1.0 - 0.5 * (t + 1) as f64 / 5.0;
        for z in row.iter().flatten() {
            assert!((z.re - expected).abs() < 1e-14);
            assert_eq!(z.im, 0.0);
        }
    }
    let steady = d
        .process(&input, &[4], &levels, &[0, 1, 2, 3], &[1.0])
        .unwrap();
    for row in steady {
        for q in 0..4 {
            let energy = row.iter().map(|c| c[q].re * c[q].re).sum::<f64>();
            assert!((energy - 1.0).abs() < 1e-14);
        }
    }
    d.reset();
    assert_eq!(
        d.process(&input, &[4], &levels, &[0, 1, 2, 3], &[1.0])
            .unwrap(),
        first
    );
}
