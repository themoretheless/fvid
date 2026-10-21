use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/sami-attributes")
}
fn cases() -> Vec<serde_json::Value> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    manifest["cases"].as_array().unwrap().clone()
}
#[test]
fn sami_ignores_attribute_looking_text_inside_other_values() {
    for case in cases() {
        let actual = std::fs::read_to_string(root().join(case["file"].as_str().unwrap())).unwrap();
        let control =
            std::fs::read_to_string(root().join(case["control"].as_str().unwrap())).unwrap();
        assert_eq!(
            fvid::subtitles::parse_sami(&actual) == fvid::subtitles::parse_sami(&control),
            true,
            "{}",
            case["name"]
        );
    }
}
#[test]
fn sami_attributes_match_independent_caption_clocks_and_untimed_skip_controls() {
    let video = std::fs::read(root().join("video.y4m")).unwrap();
    let stats = fvid_media::owned_y4m_decode::decode_reader(std::io::Cursor::new(video)).unwrap();
    assert_eq!((stats.video_frames, stats.width, stats.height), (5, 2, 2));
    for case in cases() {
        let actual = root().join(case["file"].as_str().unwrap());
        let control = root().join(case["control"].as_str().unwrap());
        let expected = fvid::subtitles::load(&control);
        if case["kind"] == "skip-untimed" {
            assert!(expected.is_none());
        } else {
            let cues = expected.as_ref().unwrap();
            assert_eq!(cues[0].start, std::time::Duration::from_secs(1));
            assert_eq!(cues[0].end, std::time::Duration::from_secs(2));
        }
        assert_eq!(fvid::subtitles::load(&actual), expected, "{}", case["name"]);
    }
}
