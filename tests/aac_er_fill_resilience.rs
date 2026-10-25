//! Valid ER fill is acceptance; a nonzero EXT_FILL_DATA nibble is refusal.
#[test]
fn hcr_and_rvlc_fill_videos_preserve_pcm_and_refuse_the_specific_invalid_nibble() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let folder = root.join("aac-er-fill-resilience");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 32);
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let control = std::fs::read(root.join(c["control"].as_str().unwrap())).unwrap();
        let fill = std::fs::read(folder.join(c["fill"].as_str().unwrap())).unwrap();
        let malformed = std::fs::read(folder.join(c["malformed"].as_str().unwrap())).unwrap();
        let mut expected = Vec::new();
        let mut actual = Vec::new();
        let mut owned = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&control, &mut expected).unwrap();
        fvid::native_media::decode_mp4_aac_pcm(&fill, &mut actual)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&fill),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(actual, expected, "{name} valid fill");
        assert_eq!(owned, expected, "{name} backend valid fill");
        let error = fvid::native_media::decode_mp4_aac_pcm(&malformed, &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{name}: {error}"
        );
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&malformed),
            &mut Vec::new(),
            None,
            &Default::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{name} backend: {error}"
        );
    }
}
