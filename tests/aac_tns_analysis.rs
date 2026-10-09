use fvid_media::owned_aac::aac_tns::{TnsData, TnsFilter};
use serde_json::Value;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn doubles(raw: &[u8]) -> Vec<f64> {
    raw.chunks_exact(8)
        .map(|s| f64::from_le_bytes(s.try_into().unwrap()))
        .collect()
}
#[test]
fn tns_analysis_matches_direct_fir_and_reverses_synthesis() {
    let m: Value = serde_json::from_slice(&bytes("aac-tns-analysis.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 96);
    let raw = bytes("aac-tns-analysis-input.f64le");
    let gold = bytes("aac-tns-analysis-reference.f64le");
    for c in m["cases"].as_array().unwrap() {
        let n = c["n"].as_u64().unwrap() as usize;
        let at = c["input_offset"].as_u64().unwrap() as usize;
        let input = doubles(&raw[at..at + n * 8]);
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        let expected = doubles(&gold[at..at + n * 8]);
        let offsets: Vec<_> = c["offsets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as usize)
            .collect();
        let limit = c["limit"].as_u64().unwrap() as usize;
        let data = TnsData {
            windows: c["windows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| {
                    w.as_array()
                        .unwrap()
                        .iter()
                        .map(|f| TnsFilter {
                            length: f["length"].as_u64().unwrap() as usize,
                            reverse: f["reverse"].as_bool().unwrap(),
                            lpc: f["lpc"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|x| x.as_f64().unwrap())
                                .collect(),
                        })
                        .collect()
                })
                .collect(),
        };
        let address = input.as_ptr();
        let capacity = input.capacity();
        let encoded = data.analyze_owned(input.clone(), &offsets, limit).unwrap();
        for (i, (a, b)) in encoded.iter().zip(&expected).enumerate() {
            assert!((a - b).abs() < 1e-12, "n={n} sample={i}: {a} vs {b}");
        }
        let restored = data
            .filter(
                &encoded.iter().map(|x| *x as f32).collect::<Vec<_>>(),
                &offsets,
                limit,
            )
            .unwrap();
        for (a, b) in restored.iter().zip(&input) {
            assert!((f64::from(*a) - b).abs() < 2e-6);
        }
        let owned = data.analyze_owned(input, &offsets, limit).unwrap();
        assert_eq!(owned.as_ptr(), address);
        assert_eq!(owned.capacity(), capacity);
        assert_eq!(owned, encoded);
    }
}
#[test]
fn malformed_tns_analysis_refuses_geometry_coefficients_and_overflow() {
    let mk = |lpc| TnsData {
        windows: vec![vec![TnsFilter {
            length: 2,
            reverse: false,
            lpc,
        }]],
    };
    for data in [mk(vec![f64::NAN]), mk(vec![0.; 21])] {
        assert!(
            data.analyze_owned(vec![0.; 1024], &[0, 4, 1024], 2)
                .unwrap_err()
                .to_string()
                .contains("predictor")
        );
    }
    let data = mk(vec![1.]);
    let mut input = vec![0.; 1024];
    input[0] = f64::INFINITY;
    assert!(data.analyze_owned(input, &[0, 4, 1024], 2).is_err());
    assert!(
        data.analyze_owned(vec![f64::MAX; 1024], &[0, 4, 1024], 2)
            .unwrap_err()
            .to_string()
            .contains("overflow")
    );
    for (n, offsets, limit) in [
        (512, vec![0, 4, 512], 2),
        (1024, vec![1, 4, 1024], 2),
        (1024, vec![0, 4, 4, 1024], 2),
        (1024, vec![0, 4, 1024], 3),
    ] {
        assert!(data.analyze_owned(vec![0.; n], &offsets, limit).is_err());
    }
    assert!(
        TnsData { windows: vec![] }
            .analyze_owned(vec![0.; 1024], &[0, 1024], 1)
            .is_err()
    );
}
