use fvid_media::owned_aac::{aac_ld_mps_arbitrary_tree::Decoder, aac_sbr_qmf::Complex};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-arbitrary-defined")
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
#[test]
fn defined_history_and_precise_undefined_boundary_match_original_c() {
    let load = |name: &str| {
        serde_json::from_slice::<Value>(&std::fs::read(root().join(name)).unwrap()).unwrap()
    };
    let m = load("components.json");
    let oracle = load("oracle.json");
    let input = std::fs::read(root().join("input.f32le")).unwrap();
    let gold = std::fs::read(root().join("expected.f32le")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 696);
    let mut maximum = 0.0f64;
    let mut count = 0;
    let mut accepted = 0;
    let mut refused = 0;
    for (c, r) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        assert_eq!(c["name"], r["name"]);
        let nodes: Vec<Vec<bool>> = serde_json::from_value(c["nodes"].clone()).unwrap();
        let mut previous: Option<Decoder> = None;
        let mut at = n(&r["offset"]);
        for (i, f) in c["blocks"].as_array().unwrap().iter().enumerate() {
            let cutoffs: Vec<u8> = serde_json::from_value(f["cutoffs"].clone()).unwrap();
            let map: Vec<u8> = serde_json::from_value(f["map"].clone()).unwrap();
            let levels: Vec<Vec<Vec<f64>>> = serde_json::from_value(f["cld"].clone()).unwrap();
            let mut d =
                Decoder::new_for_qmf(&nodes, &cutoffs, &[0, 1, 2, 3], n(&f["bands"]) as u8, 32)
                    .unwrap();
            if let Some(old) = &previous {
                let saved = d.clone();
                let inherited = d.inherit_history(old);
                if r["first_error"].as_u64() == Some(i as u64) {
                    let error = inherited.unwrap_err().to_string();
                    assert!(
                        error.contains("undefined dormant gain history"),
                        "{}: {error}",
                        c["name"]
                    );
                    assert_eq!(d, saved);
                    refused += 1;
                    break;
                }
                inherited.unwrap_or_else(|e| panic!("{} block{i}: {e}", c["name"]));
            }
            let mut source = vec![vec![vec![Complex::default(); 32]; 2]; 3];
            let mut offset = n(&f["source_offset"]);
            for row in &mut source {
                for channel in row {
                    for z in channel {
                        z.re = f32::from_le_bytes(input[offset..offset + 4].try_into().unwrap())
                            as f64;
                        z.im = f32::from_le_bytes(input[offset + 4..offset + 8].try_into().unwrap())
                            as f64;
                        offset += 8;
                    }
                }
            }
            let out = d
                .process(&source, &[0, 2], &levels, &map, &[1., 1.])
                .unwrap();
            for row in out {
                for channel in row {
                    for z in channel {
                        for value in [z.re, z.im] {
                            let want =
                                f32::from_le_bytes(gold[at..at + 4].try_into().unwrap()) as f64;
                            at += 4;
                            count += 1;
                            let error = (value - want).abs();
                            maximum = maximum.max(error);
                            assert!(error < 2e-8, "{} block{i}: {error}", c["name"]);
                        }
                    }
                }
            }
            previous = Some(d);
            if i == 2 {
                accepted += 1;
                let mut d = previous.clone().unwrap();
                d.reset();
                let mut fresh =
                    Decoder::new_for_qmf(&nodes, &cutoffs, &[0, 1, 2, 3], n(&f["bands"]) as u8, 32)
                        .unwrap();
                fresh.reset();
                assert_eq!(d, fresh);
            }
        }
        assert_eq!(at, n(&r["offset"]) + n(&r["blocks"]) * 3 * 4 * 32 * 8);
    }
    assert_eq!((accepted, refused), (648, 48));
    assert_eq!(count * 4, gold.len());
    println!(
        "Defined mixed histories: {accepted} accepted, {refused} exact refusals, {count} scalars, max error {maximum}"
    );
}
