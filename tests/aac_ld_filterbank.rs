use fvid_media::owned_aac::aac_ld_synthesis::{LdSynthesis, LdWindowShape};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("aac-ld-filterbank.json")).unwrap()
}
fn shape(x: &serde_json::Value) -> LdWindowShape {
    if x.as_u64().unwrap() == 0 {
        LdWindowShape::Sine
    } else {
        LdWindowShape::LowOverlap
    }
}
#[test]
fn ld_480_512_sine_low_overlap_switches_match_scalar_pcm_and_checkpoints() {
    let gold = bytes("aac-ld-filterbank-reference.f32le");
    for c in manifest()["cases"].as_array().unwrap() {
        let n = c["n"].as_u64().unwrap() as usize;
        let mut bank = LdSynthesis::new(n).unwrap();
        let retained = bank.retained_bytes().unwrap();
        let mut first = vec![];
        for pass in 0..2 {
            bank.reset();
            let mut all = vec![];
            for row in c["frames"].as_array().unwrap() {
                let mut spectrum = vec![0.; n];
                for (out, v) in spectrum.iter_mut().zip(row["spectrum"].as_array().unwrap()) {
                    *out = v.as_f64().unwrap() as f32;
                }
                let saved = bank.checkpoint();
                let mut actual = vec![0.; n];
                bank.synthesize_pcm(&spectrum, shape(&row["shape"]), &mut actual)
                    .unwrap();
                bank.restore(&saved).unwrap();
                let mut replay = vec![0.; n];
                bank.synthesize_pcm(&spectrum, shape(&row["shape"]), &mut replay)
                    .unwrap();
                assert_eq!(actual, replay);
                let at = row["reference_offset"].as_u64().unwrap() as usize;
                for (i, (x, y)) in actual
                    .iter()
                    .zip(gold[at..at + n * 4].chunks_exact(4))
                    .enumerate()
                {
                    let expected = f32::from_le_bytes(y.try_into().unwrap()) as f64;
                    assert!(
                        (x - expected).abs() < 1e-10,
                        "n={n} i={i}: {x} vs {expected}"
                    );
                }
                all.extend(actual);
                assert_eq!(bank.retained_bytes().unwrap(), retained);
            }
            if pass == 0 {
                first = all;
            } else {
                assert_eq!(all, first);
            }
        }
    }
}
#[test]
fn ld_forward_analysis_matches_all_four_window_pairs_without_advancing_history() {
    let gold = bytes("aac-ld-filterbank-analysis.f64le");
    for c in manifest()["cases"].as_array().unwrap() {
        let n = c["n"].as_u64().unwrap() as usize;
        let mut bank = LdSynthesis::new(n).unwrap();
        let mut out = vec![0.; n];
        bank.synthesize_raw(&vec![1.; n], LdWindowShape::LowOverlap, &mut out)
            .unwrap();
        let mut control = bank.clone();
        for row in c["analysis"].as_array().unwrap() {
            let values: Vec<_> = (0..2 * n)
                .map(|i| (0.013 * (i + 1) as f64).sin() + 0.4 * (0.021 * i as f64).cos())
                .collect();
            bank.analyze(
                &values,
                shape(&row["previous"]),
                shape(&row["current"]),
                &mut out,
            )
            .unwrap();
            let at = row["reference_offset"].as_u64().unwrap() as usize;
            for (i, (x, y)) in out
                .iter()
                .zip(gold[at..at + n * 8].chunks_exact(8))
                .enumerate()
            {
                let expected = f64::from_le_bytes(y.try_into().unwrap());
                assert!(
                    (x - expected).abs() < 1e-9,
                    "n={n} bin={i}: {x} vs {expected}"
                );
            }
        }
        out.fill(77.);
        assert!(
            bank.analyze(
                &vec![f64::NAN; 2 * n],
                LdWindowShape::Sine,
                LdWindowShape::Sine,
                &mut out
            )
            .is_err()
        );
        assert_eq!(out, vec![77.; n]);
        let mut expected = vec![0.; n];
        bank.synthesize_raw(&vec![0.; n], LdWindowShape::Sine, &mut out)
            .unwrap();
        control
            .synthesize_raw(&vec![0.; n], LdWindowShape::Sine, &mut expected)
            .unwrap();
        assert_eq!(out, expected);
    }
}
#[test]
fn aot23_public_gap_videos_refuse_profile_until_ld_packet_integration() {
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut out = vec![];
        let e = fvid::native_media::decode_mp4_aac_pcm(&data, &mut out).unwrap_err();
        assert!(
            e.to_string().contains(
                "only AAC Main, LC, SSR, LTP, ER-LC and ER-LTP core configurations are implemented"
            ),
            "{e}"
        );
    }
}
