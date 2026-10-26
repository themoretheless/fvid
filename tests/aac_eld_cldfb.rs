use fvid_media::owned_aac::{
    aac_eld_cldfb::{Analysis, Synthesis},
    aac_sbr_qmf::Complex,
};
fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-cldfb")
}
fn floats(bytes: &[u8]) -> Vec<f64> {
    bytes
        .chunks_exact(8)
        .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
#[test]
fn cldfb_matches_independent_direct_convolution_and_modulated_superposition() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let analysis = c["kind"] == "analysis";
        let raw = std::fs::read(root().join(format!("{name}.qmf.f64le"))).unwrap();
        let width = if analysis { 32 } else { 64 };
        let mut rows: Vec<[Complex; 64]> = raw
            .chunks_exact(width * 16)
            .map(|b| {
                std::array::from_fn(|k| {
                    if k < width {
                        Complex {
                            re: f64::from_le_bytes(b[k * 16..k * 16 + 8].try_into().unwrap()),
                            im: f64::from_le_bytes(b[k * 16 + 8..k * 16 + 16].try_into().unwrap()),
                        }
                    } else {
                        Complex::default()
                    }
                })
            })
            .collect();
        if analysis {
            let pcm: Vec<f32> = std::fs::read(root().join(format!("{name}.f32le")))
                .unwrap()
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let mut state = Analysis::default();
            let mut actual = Vec::new();
            for chunk in pcm.chunks(480) {
                actual.extend(state.process(chunk).unwrap());
            }
            for (a, b) in actual.iter().zip(&rows) {
                for k in 0..32 {
                    assert!(
                        (a[k].re - b[k].re).abs() < 2e-11 && (a[k].im - b[k].im).abs() < 2e-11,
                        "{name} analysis bin {k}"
                    );
                }
            }
            state.reset();
            assert_eq!(actual, state.process(&pcm).unwrap());
            rows = actual
                .iter()
                .map(|r| std::array::from_fn(|k| if k < 32 { r[k] } else { Complex::default() }))
                .collect();
        }
        for bands in [32, 64] {
            let expected =
                floats(&std::fs::read(root().join(format!("{name}.pcm{bands}.f64le"))).unwrap());
            let mut state = Synthesis::new(bands).unwrap();
            let mut actual = Vec::new();
            for chunk in rows.chunks(15) {
                actual.extend(state.process(chunk).unwrap());
            }
            assert_eq!(actual.len(), expected.len());
            assert!(
                actual
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| (a - b).abs() < 2e-11),
                "{name} synthesis {bands}"
            );
            state.reset();
            assert_eq!(actual, state.process(&rows).unwrap());
        }
    }
}
#[test]
fn cldfb_bad_input_preserves_history_and_mode_is_explicit() {
    assert!(Synthesis::new(48).is_err());
    let mut a = Analysis::default();
    a.process(&vec![0.25; 512]).unwrap();
    let saved = a.clone();
    assert!(a.process(&vec![0.25; 481]).is_err());
    assert_eq!(a, saved);
    let mut bad = vec![0.25; 480];
    bad[479] = f32::NAN;
    assert!(a.process(&bad).is_err());
    assert_eq!(a, saved);
    for n in [32, 64] {
        let mut s = Synthesis::new(n).unwrap();
        s.process(&vec![[Complex { re: 0.25, im: 0.5 }; 64]; 16])
            .unwrap();
        let saved = s.clone();
        let mut bad = vec![[Complex::default(); 64]; 15];
        bad[14][31].re = f64::NAN;
        assert!(s.process(&bad).is_err());
        assert_eq!(s, saved);
        assert!(
            s.process(&vec![
                [Complex {
                    re: f64::MAX,
                    im: f64::MAX
                }; 64];
                15
            ])
            .is_err()
        );
        assert_eq!(s, saved);
    }
}
