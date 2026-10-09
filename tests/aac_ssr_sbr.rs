use std::path::Path;
fn cases() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/aac-ssr-sbr.json")).unwrap()
}
fn video(case: &serde_json::Value) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap()),
    )
    .unwrap()
}
#[test]
fn ssr_sbr_transition_fixture_reproduces_exact_gap_with_valid_core_control() {
    for case in cases()["cases"].as_array().unwrap() {
        let mut pcm = Vec::new();
        let result = fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm);
        if case["name"] == "core-control" {
            result.unwrap();
            assert_eq!(pcm.len(), 6144 * 4);
            assert!(pcm.iter().all(|b| *b == 0));
        } else {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("AAC SSR SBR synthesis is not implemented"),
                "{}: {error}",
                case["name"]
            );
        }
    }
}
#[test]
#[ignore = "acceptance pending SSR core alignment to SBR synthesis and EOF drain"]
fn ssr_sbr_transition_video_acceptance() {
    for case in cases()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] != "core-control")
    {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm).unwrap();
        assert_eq!(pcm.len(), 12288 * 4);
        assert!(pcm
            .chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-6));
    }
}
