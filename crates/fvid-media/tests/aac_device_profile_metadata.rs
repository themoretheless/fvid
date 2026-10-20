use std::path::Path;
#[test]
fn every_saved_er_sbr_indexed_layout_negotiates_its_independent_channel_mask() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("aac-er-multi-sbr.json")).unwrap())
            .unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 72);
    for c in cases {
        let hex = c["asc"].as_str().unwrap();
        let asc: Vec<u8> = hex
            .as_bytes()
            .chunks_exact(2)
            .map(|bytes| u8::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap())
            .collect();
        let channels = c["channels"].as_u64().unwrap() as u16;
        let rate = c["container_rate"].as_u64().unwrap() as u32;
        let router =
            fvid_media::owned_aac::speaker_routing::stereo_router(&asc, rate, channels, false)
                .unwrap();
        assert_eq!(router.input_channels(), channels);
        assert_eq!(
            router.source_mask(),
            Some(c["channel_mask"].as_u64().unwrap() as u32),
            "{}",
            c["name"]
        );
    }
}
