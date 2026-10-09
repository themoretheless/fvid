use std::path::Path;
fn cases() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ssr-sbr-cce.json"
    ))
    .unwrap()
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
fn ssr_sbr_independent_cce_fixture_has_accepted_nonzero_core_control() {
    let manifest = cases();
    let mut pcm = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&video(&manifest["cases"][0]), &mut pcm).unwrap();
    let reference =
        include_bytes!("fixtures/playback-errors/aac-ssr-sbr-active-core-reference.f32le");
    assert_eq!(pcm, reference);
    assert!(
        pcm.chunks_exact(4)
            .any(|s| f32::from_le_bytes(s.try_into().unwrap()) != 0.0)
    );
}
#[test]
fn ssr_sbr_independent_cce_reproduces_per_source_synthesis_refusal() {
    let manifest = cases();
    let mut pcm = Vec::new();
    let error = fvid::native_media::decode_mp4_aac_pcm(&video(&manifest["cases"][1]), &mut pcm)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("SSR SBR independent coupling requires per-source aligned synthesis"),
        "{error}"
    );
    assert!(pcm.is_empty());
}
#[test]
#[ignore = "requires per-source aligned SSR/SBR CCE synthesis"]
fn ssr_sbr_independent_cce_pcm_acceptance() {
    let manifest = cases();
    let mut pcm = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&video(&manifest["cases"][1]), &mut pcm).unwrap();
    let reference = include_bytes!("fixtures/playback-errors/aac-ssr-sbr-active-reference.f64le");
    assert_eq!(pcm.len() * 2, reference.len());
    for (i, (sample, gold)) in pcm
        .chunks_exact(4)
        .zip(reference.chunks_exact(8))
        .enumerate()
    {
        let sample = f32::from_le_bytes(sample.try_into().unwrap()) as f64;
        let gold = f64::from_le_bytes(gold.try_into().unwrap());
        assert!(
            (sample - gold).abs() < 1e-9,
            "sample {i}: {sample} vs {gold}"
        );
    }
}
