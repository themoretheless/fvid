use fvid_media::owned_aac::{
    aac_eld_cldfb::{Analysis, Synthesis},
    aac_sbr_qmf::Complex,
};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(CORPUS)
            .join(name),
    )
    .unwrap()
}
fn doubles(b: &[u8]) -> Vec<f64> {
    assert_eq!(b.len() % 8, 0);
    b.chunks_exact(8)
        .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
#[test]
fn centered_cldfb_matches_independent_oracle_across_frame_boundaries() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let b = case["bands"].as_u64().unwrap() as usize;
        let raw = doubles(&read(&format!("{name}.qmf.f64le")));
        if case["kind"] == "analysis" {
            let expected: Vec<[Complex; 32]> = raw
                .chunks_exact(64)
                .map(|r| {
                    std::array::from_fn(|k| Complex {
                        re: r[2 * k],
                        im: r[2 * k + 1],
                    })
                })
                .collect();
            let pcm: Vec<f32> = read(&format!("{name}.f32le"))
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let mut state = Analysis::new_downscaled(32 / b).unwrap();
            assert_eq!(state.bands(), b);
            let actual: Vec<_> = pcm
                .chunks(15 * b)
                .flat_map(|chunk| state.process(chunk).unwrap())
                .collect();
            assert_eq!(actual.len(), expected.len());
            for (a, e) in actual.iter().zip(expected) {
                for k in 0..32 {
                    assert!(
                        (a[k].re - e[k].re).abs() < 2e-11 && (a[k].im - e[k].im).abs() < 2e-11,
                        "{name} bin{k}"
                    );
                }
            }
            state.reset();
            assert_eq!(actual, state.process(&pcm).unwrap());
            let saved = state.clone();
            assert!(state.process(&[f32::NAN; 32]).is_err());
            assert_eq!(saved, state);
            assert!(state.process(&vec![0.; b + 1]).is_err());
            assert_eq!(saved, state);
        } else {
            let rows: Vec<[Complex; 64]> = raw
                .chunks_exact(128)
                .map(|r| {
                    std::array::from_fn(|k| Complex {
                        re: r[2 * k],
                        im: r[2 * k + 1],
                    })
                })
                .collect();
            let expected = doubles(&read(&format!("{name}.f64le")));
            let original = if b <= 8 { 32 } else { 64 };
            let mut state = Synthesis::new_downscaled(original, original / b).unwrap();
            assert_eq!(state.bands(), b);
            let actual: Vec<_> = rows
                .chunks(15)
                .flat_map(|chunk| state.process(chunk).unwrap())
                .collect();
            assert_eq!(actual.len(), expected.len());
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, e)| (a - e).abs() < 2e-11),
                "{name}"
            );
            state.reset();
            assert_eq!(actual, state.process(&rows).unwrap());
            let saved = state.clone();
            let mut bad = rows.clone();
            bad[30][63].im = f64::NAN;
            assert!(state.process(&bad).is_err());
            assert_eq!(saved, state);
            match state.process(
                &[[Complex {
                    re: f64::MAX,
                    im: f64::MAX,
                }; 64]; 15],
            ) {
                Ok(pcm) => assert!(pcm.iter().all(|v| v.is_finite())),
                Err(_) => assert_eq!(saved, state),
            }
        }
    }
    for f in [0, 3, 16] {
        assert!(Analysis::new_downscaled(f).is_err());
        assert!(Synthesis::new_downscaled(64, f).is_err());
    }
    assert!(Synthesis::new_downscaled(48, 2).is_err());
}
