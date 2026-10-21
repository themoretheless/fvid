use fvid::codec::{
    hevc_sps::{LayeredSps, LayeredSpsContext},
    hevc_vps::LayeredVps,
};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/hevc-3d-chroma-constraints")
            .join(name),
    )
    .unwrap()
}
#[test]
fn three_d_main_depth_requires_monochrome_before_picture_tools() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let stem = case["stem"].as_str().unwrap();
        let vps = LayeredVps::parse(&read(&format!("{stem}.vps")), 1 << 20).unwrap();
        let base = LayeredSps::parse(
            &read("base.sps"),
            &LayeredSpsContext {
                vps: &vps,
                layer_id: 0,
                output_layer_set: 0,
                active_sps: &[],
            },
            1 << 20,
        )
        .unwrap();
        let context = LayeredSpsContext {
            vps: &vps,
            layer_id: 3,
            output_layer_set: 1,
            active_sps: std::slice::from_ref(&base),
        };
        let movie = fvid::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(read(&format!("{stem}.mp4"))),
            Default::default(),
        )
        .unwrap();
        let config =
            fvid::codec::config::HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
        let configured = config
            .arrays
            .iter()
            .find(|a| a.nal_type == 33)
            .unwrap()
            .units
            .last()
            .unwrap();
        assert_eq!(*configured, read(&format!("{stem}.sps")));
        let result = LayeredSps::parse(configured, &context, 1 << 20);
        if case["valid"] == true {
            let parsed = result.unwrap();
            assert!(parsed.sps.depth_layer);
            assert_eq!(parsed.sps.chroma_format, 0);
            assert!(parsed.effective_profile.profile.unwrap().compatible_with(8));
        } else {
            let error = result.err().expect("3D Main depth chroma violates profile");
            assert!(matches!(error, fvid::Error::Invalid(_)), "{error}");
            assert!(
                error
                    .to_string()
                    .contains("3D Main depth layer must be monochrome"),
                "{error}"
            );
        }
    }
}
