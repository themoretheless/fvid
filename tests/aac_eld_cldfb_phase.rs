use fvid_media::owned_aac::aac_eld_cldfb::Analysis;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-cldfb-phase")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
#[test]
fn nominal_analysis_uses_the_same_canonical_phase_as_centered_factor_one() {
    let pcm: Vec<_> = (0..1280)
        .map(|i| ((i as f64 * 0.113).sin() * 0.7 + (i as f64 * 0.017).cos() * 0.2) as f32)
        .collect();
    let mut nominal = Analysis::default();
    let mut centered = Analysis::new_downscaled(1).unwrap();
    for input in pcm.chunks(480) {
        let a = nominal.process(input).unwrap();
        let b = centered.process(input).unwrap();
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(&b) {
            for k in 0..32 {
                assert!(
                    (a[k].re - b[k].re).abs() < 2e-11 && (a[k].im - b[k].im).abs() < 2e-11,
                    "canonical analysis bin{k}: {:?} vs {:?}",
                    a[k],
                    b[k]
                );
            }
        }
    }
}
#[test]
fn canonical_phase_movies_match_independent_direct_pcm_in_both_containers() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 8);
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mut pcm = Vec::new();
        let mut mkv = Vec::new();
        let stats =
            fvid::native_media::decode_mp4_aac_pcm(&read(c["video"].as_str().unwrap()), &mut pcm)
                .unwrap();
        assert_eq!(stats.sample_rate as u64, c["rate"].as_u64().unwrap());
        assert_eq!(u64::from(stats.channels), c["channels"].as_u64().unwrap());
        assert_eq!(stats.sample_frames, c["samples"].as_u64().unwrap());
        let gold = read(c["golden"].as_str().unwrap());
        assert_eq!(pcm.len(), gold.len());
        let mut worst = 0f32;
        for (a, b) in pcm.chunks_exact(4).zip(gold.chunks_exact(4)) {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(a.is_finite());
            worst = worst.max((a - b).abs());
        }
        assert!(worst < 1e-7, "{name}: canonical PCM error {worst}");
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(c["file"].as_str().unwrap()),
            &mut mkv,
            None,
        )
        .unwrap();
        assert_eq!(mkv, pcm, "{name} Matroska");
    }
}
