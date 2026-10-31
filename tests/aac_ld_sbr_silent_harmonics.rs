use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-sbr-silent-harmonics")
}
fn manifest() -> Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn pcm(case: &Value) -> Vec<u8> {
    let mut output = Vec::new();
    let info =
        fvid::native_media::decode_mp4_aac_pcm(&read(case["video"].as_str().unwrap()), &mut output)
            .unwrap();
    assert_eq!(u64::from(info.sample_rate), case["rate"].as_u64().unwrap());
    assert_eq!(info.channels, 1);
    assert_eq!(info.sample_frames, case["samples"].as_u64().unwrap());
    output
}
#[test]
fn silent_core_harmonics_match_independent_complex_pcm_in_both_containers() {
    let m = manifest();
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 24);
    for c in cases {
        let actual = pcm(c);
        let gold = read(c["golden"].as_str().unwrap());
        assert_eq!(actual.len(), gold.len());
        let error = actual
            .chunks_exact(4)
            .zip(gold.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0.0f32, f32::max);
        assert!(error < 1e-7, "{} independent PCM {error}", c["name"]);
        assert!(
            actual
                .chunks_exact(4)
                .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 1e-5)
        );
        let mut matroska = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(c["file"].as_str().unwrap()),
            &mut matroska,
            None,
        )
        .unwrap();
        assert_eq!(matroska, actual, "{} Matroska", c["name"]);
    }
}
#[test]
fn fine_envelope_steps_scale_harmonics_by_eight_without_clipping_or_phase_changes() {
    let m = manifest();
    let cases = m["cases"].as_array().unwrap();
    for low in cases.iter().filter(|c| c["envelope_level"] == 24) {
        let mut previous = pcm(low);
        for level in [36, 48] {
            let c = cases
                .iter()
                .find(|c| {
                    c["n"] == low["n"]
                        && c["bands"] == low["bands"]
                        && c["noise_level"] == low["noise_level"]
                        && c["envelope_level"] == level
                })
                .unwrap();
            let actual = pcm(c);
            assert_eq!(actual.len(), previous.len());
            for (a, b) in actual.chunks_exact(4).zip(previous.chunks_exact(4)) {
                let a = f32::from_le_bytes(a.try_into().unwrap());
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!(a.is_finite() && a.abs() < 1.0);
                assert_eq!(a, b * 8.0, "{} scalefactor amplitude/phase", c["name"]);
            }
            previous = actual;
        }
        assert!(
            previous
                .chunks_exact(4)
                .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 0.1)
        );
    }
}
