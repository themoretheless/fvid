use std::path::Path;
#[test]
fn library_mp4_probe_matches_frontend_container_metadata() {
    for name in [
        "video.mp4",
        "audio.mp4",
        "fragmented/video.mp4",
        "fragmented/audio.mp4",
        "hevc/hdr10.mp4",
        "hevc/main10-ipb.mp4",
        "short/avc-bframes.mp4",
        "tracks/named.mp4",
        "tags/tags.mp4",
        "alac/stereo-24.m4a",
        "playback-errors/aac-gap-repeat.m4a",
    ] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let front = fvid::native_probe::probe(&source).unwrap();
        let own = fvid_media::owned_probe::probe(&source).unwrap();
        assert_eq!(
            serde_json::to_value(&own).unwrap(),
            serde_json::to_value(front).unwrap(),
            "{name}"
        );
        #[cfg(not(feature = "media"))]
        assert_eq!(
            serde_json::to_value(fvid_media::probe(&source).unwrap()).unwrap(),
            serde_json::to_value(&own).unwrap()
        );
        let hint = fvid_media::owned_probe::probe_as(&source, Some("mp4")).unwrap();
        let direct = fvid_media::owned_mp4_probe::probe_mp4(&source).unwrap();
        assert_eq!(
            serde_json::to_value(hint).unwrap(),
            serde_json::to_value(direct).unwrap()
        );
    }
}
