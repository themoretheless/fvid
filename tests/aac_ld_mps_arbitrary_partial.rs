const PROGRAMMES: usize = 80;
const CORPUS: &str = "aac-ld-mps-arbitrary-partial";
include!("support/aac_ld_mps_arbitrary_playback.rs");

#[test]
fn partial_acceptance_covers_every_mapped_bin_and_uncovered_cases_are_explicit_refusals() {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::ExtensionValue, aac_ld_mps_frame, aac_ld_mps_prepare, AacDecodeOptions,
        NativeAacDecoder,
    };
    let m = manifest();
    assert_eq!(m["controls"].as_array().unwrap().len(), 64);
    assert_eq!(m["errors"].as_array().unwrap().len(), 68);
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["controls"].as_array().unwrap())
        .chain(m["errors"].as_array().unwrap())
    {
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        cfg.validate_low_delay_profile().unwrap();
        let ExtensionValue::ArbitraryTree { ott, .. } = &cfg.extensions[0].value else {
            panic!("missing arbitrary tree")
        };
        assert!(ott[0].lfe);
        assert_eq!(usize::from(ott[0].bands), n(&c["cutoff"]));
        let map = aac_ld_mps_prepare::qmf_map(cfg.parameter_bands, n(&c["bank"])).unwrap();
        let covered = map.iter().all(|&b| ott.iter().all(|o| b < o.bands));
        assert_eq!(covered, c["acceptance"] == true, "{}", c["name"]);
        let mut syntax = aac_ld_mps_frame::Decoder::new(&cfg).unwrap();
        for v in c["reference"].as_array().unwrap() {
            let wire = hex(v);
            let mut b = BitReader::new(&wire);
            let f = syntax.decode(&mut b).unwrap();
            assert_eq!(b.remaining(), 0);
            assert_eq!(f.extensions[0].parameters.len(), ott.len());
            for (parameter, o) in f.extensions[0].parameters.iter().zip(ott) {
                for row in &parameter.values {
                    assert_eq!(row.len(), usize::from(o.bands));
                }
            }
        }
        if covered {
            continue;
        }
        for mode in [Mode::Real, Mode::Complex] {
            let e = NativeAacDecoder::new_with_options(
                &hex(&c["asc"]),
                AacDecodeOptions {
                    output_sample_rate: Some(n(&c["rate"]) as u32),
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .err()
            .expect("uncovered grids are not playback acceptance")
            .to_string();
            assert!(
                e.contains("partial-band arbitrary-tree splits require qualification"),
                "{e}"
            );
        }
        for mkv in [false, true] {
            let mut out = Vec::new();
            let e = if mkv {
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                    Cursor::new(read(c, "file")),
                    &mut out,
                    None,
                    &Default::default(),
                )
                .unwrap_err()
                .to_string()
            } else {
                fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut out)
                    .unwrap_err()
                    .to_string()
            };
            assert!(
                e.contains("partial-band arbitrary-tree splits require qualification"),
                "{e}"
            );
            assert!(out.is_empty());
        }
    }
}
#[test]
fn full_grid_controls_match_independent_analytic_composition() {
    for c in manifest()["controls"].as_array().unwrap() {
        let want = expected(c, Mode::Complex);
        for mkv in [false, true] {
            let mut out = Vec::new();
            if mkv {
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                    Cursor::new(read(c, "file")),
                    &mut out,
                    None,
                    &Default::default(),
                )
                .unwrap();
            } else {
                fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut out).unwrap();
            }
            near(&out, &want, &c["name"]);
        }
    }
}
#[test]
fn component_rejects_a_later_map_that_addresses_uncoded_bands_without_changing_history() {
    use fvid_media::owned_aac::{aac_ld_mps_arbitrary_tree::Decoder, aac_ld_mps_prepare};
    let mut d = Decoder::new_for_qmf(
        &[vec![true, false, false], vec![false]],
        &[20],
        &[0, 1, 2],
        23,
        32,
    )
    .unwrap();
    let input = vec![vec![vec![Complex { re: 0.01, im: 0.02 }; 32]; 2]; 3];
    let levels = vec![vec![vec![0.0; 20]]];
    let map = aac_ld_mps_prepare::qmf_map(23, 32).unwrap();
    d.process(&input, &[2], &levels, &map, &[1.0, 1.0]).unwrap();
    let old = d.clone();
    let mut bad = map;
    bad[31] = 20;
    assert!(d
        .process(&input, &[2], &levels, &bad, &[1.0, 1.0])
        .unwrap_err()
        .to_string()
        .contains("require qualification"));
    assert_eq!(d, old);
}

#[test]
fn covered_partial_components_match_unmodified_c_including_mixed_cutoffs_and_history() {
    use fvid_media::owned_aac::aac_ld_mps_arbitrary_tree::Decoder;
    let load = |name: &str| {
        serde_json::from_slice::<Value>(&std::fs::read(root().join(name)).unwrap()).unwrap()
    };
    let m = load("component.json");
    let oracle = load("component-oracle.json");
    let source = std::fs::read(root().join("component-input.f32le")).unwrap();
    let gold = std::fs::read(root().join("component-expected.f32le")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 20);
    assert_eq!(m["controls"].as_array().unwrap().len(), 12);
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 32);
    let mut maximum = 0.0f64;
    let mut count = 0;
    for (c, refcase) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["controls"].as_array().unwrap())
        .zip(oracle["cases"].as_array().unwrap())
    {
        assert_eq!(c["name"], refcase["name"]);
        let nodes: Vec<Vec<bool>> = serde_json::from_value(c["nodes"].clone()).unwrap();
        let cutoffs: Vec<u8> = serde_json::from_value(c["cutoffs"].clone()).unwrap();
        let positions: Vec<u8> = serde_json::from_value(c["positions"].clone()).unwrap();
        let map: Vec<u8> = serde_json::from_value(c["map"].clone()).unwrap();
        let mut d =
            Decoder::new_for_qmf(&nodes, &cutoffs, &positions, n(&c["bands"]) as u8, 32).unwrap();
        let initial = d.clone();
        for (b, r) in c["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(refcase["blocks"].as_array().unwrap())
        {
            let mut input = vec![vec![vec![Complex::default(); 32]; 2]; 8];
            let at = n(&b["offset"]);
            for t in 0..8 {
                for ch in 0..2 {
                    for q in 0..32 {
                        let p = at + ((t * 2 + ch) * 32 + q) * 8;
                        input[t][ch][q] = Complex {
                            re: f64::from(f32::from_le_bytes(source[p..p + 4].try_into().unwrap())),
                            im: f64::from(f32::from_le_bytes(
                                source[p + 4..p + 8].try_into().unwrap(),
                            )),
                        };
                    }
                }
            }
            let cld: Vec<Vec<Vec<f64>>> = serde_json::from_value(b["cld"].clone()).unwrap();
            let endpoints: Vec<u8> = serde_json::from_value(b["endpoints"].clone()).unwrap();
            let checkpoint = d.clone();
            let got = d
                .process(&input, &endpoints, &cld, &map, &[1.0, 1.0])
                .unwrap();
            let mut replay = checkpoint;
            assert_eq!(
                got,
                replay
                    .process(&input, &endpoints, &cld, &map, &[1.0, 1.0])
                    .unwrap()
            );
            assert_eq!(d, replay);
            let at = n(&r["offset"]);
            for (i, v) in got
                .iter()
                .flatten()
                .flatten()
                .flat_map(|z| [z.re, z.im])
                .enumerate()
            {
                let p = at + i * 4;
                let want = f64::from(f32::from_le_bytes(gold[p..p + 4].try_into().unwrap()));
                let error = (v - want).abs();
                maximum = maximum.max(error);
                assert!(error < 2e-8, "{}: {v} != {want}", c["name"]);
                count += 1;
            }
        }
        d.reset();
        assert_eq!(d, initial);
    }
    eprintln!("Covered partial-grid original C: {count} scalar outputs, max error {maximum}");
}
