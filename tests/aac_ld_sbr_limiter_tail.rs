use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-sbr-limiter-tail")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn check(mode_zero: bool) {
    let manifest: Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        if (c["limiter_mode"].as_u64().unwrap() == 0) != mode_zero {
            continue;
        }
        let mut actual = Vec::new();
        let info = fvid::native_media::decode_mp4_aac_pcm(
            &read(c["video"].as_str().unwrap()),
            &mut actual,
        )
        .unwrap();
        assert_eq!(info.sample_frames, c["samples"].as_u64().unwrap());
        let gold = read(c["golden"].as_str().unwrap());
        assert_eq!(actual.len(), gold.len());
        let error = actual
            .chunks_exact(4)
            .zip(gold.chunks_exact(4))
            .map(|(a, b)| {
                let a = f32::from_le_bytes(a.try_into().unwrap());
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!(a.is_finite());
                (a - b).abs()
            })
            .fold(0.0f32, f32::max);
        assert!(
            error < 1e-7,
            "{}: discarded-tail limiter PCM error {error}",
            c["name"]
        );
        let mut mkv = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(c["file"].as_str().unwrap()),
            &mut mkv,
            None,
        )
        .unwrap();
        assert_eq!(actual, mkv, "{} transport", c["name"]);
    }
}
#[test]
fn mode_zero_control_matches_independent_whole_range_pcm() {
    check(true);
}
#[test]
fn discarded_tail_is_not_an_internal_limiter_boundary() {
    check(false);
}
