use std::path::Path;
#[test]
fn library_mp4_probe_matches_frontend_container_metadata() {
    for name in [
        "video.mp4",
        "audio.mp4",
        "playback-errors/alac-resample-window.m4a",
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
        assert_eq!(
            serde_json::to_value(fvid_media::probe(&source).unwrap()).unwrap(),
            serde_json::to_value(&own).unwrap()
        );
        if name == "short/avc-bframes.mp4" {
            assert_eq!(own.streams[0].average_frame_rate, [12, 1]);
        }
        for stream in own.streams.iter().filter(|s| s.media_type == "audio") {
            assert_eq!(stream.average_frame_rate, [0, 1]);
        }
        if name == "playback-errors/alac-resample-window.m4a" {
            assert_eq!(own.streams[0].bit_rate, Some(792000));
        }
        if let Some(duration) = own.duration_us.filter(|&d| d > 0) {
            let bytes = std::fs::metadata(&source).unwrap().len();
            assert_eq!(
                own.bit_rate,
                i64::try_from(u128::from(bytes) * 8 * 1_000_000 / duration as u128).ok()
            );
        }
        let hint = fvid_media::owned_probe::probe_as(&source, Some("mp4")).unwrap();
        let direct = fvid_media::owned_mp4_probe::probe_mp4(&source).unwrap();
        assert_eq!(
            serde_json::to_value(hint).unwrap(),
            serde_json::to_value(direct).unwrap()
        );
    }
}
