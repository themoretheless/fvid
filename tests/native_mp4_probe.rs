use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn owned_probe_describes_avc_hevc_aac_and_fragmented_tracks() {
    for (name, codec) in [
        ("avc/hlg-vui-only.mp4", "h264"),
        ("hevc/main10-ipb.mp4", "hevc"),
        ("audio/aac-native-edit.m4a", "aac"),
        ("video.mp4", "h264"),
    ] {
        let path = fixture(name);
        let info = fvid::native_probe::probe(&path).unwrap();
        assert_eq!(info.streams[0].codec, codec);
        assert!(info.duration_us.unwrap() > 0);
        assert!(info.streams[0].duration.unwrap() > 0);
        assert_eq!(info.streams[0].time_base[0], 1);
        assert!(info.streams[0].extradata_bytes > 0);
        assert_eq!(info.streams[0].pixel_format, -1);
        if codec == "h264" {
            assert!(info.streams[0].profile.is_some());
            assert!(info.streams[0].level.unwrap() > 0);
        } else {
            assert!(info.streams[0].profile.is_none());
        }
        assert_eq!(
            fvid::native_probe::probe_as(&path, Some("mov")).unwrap().streams,
            info.streams
        );
        if codec == "hevc" {
            assert_eq!((info.streams[0].width, info.streams[0].height), (128, 128));
        }
    }
    let info = fvid::native_probe::probe(&fixture("audio/two-audio.mp4")).unwrap();
    assert_eq!(info.streams.len(), 3);
    assert_eq!(
        info.streams.iter().map(|s| s.index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        info.streams
            .iter()
            .filter(|s| s.media_type == "audio")
            .count(),
        2
    );
}
#[test]
fn metadata_chapters_and_cli_are_exposed_without_decoding() {
    let path = fixture("chapters/chapters.mp4");
    let info = fvid::native_probe::probe(&path).unwrap();
    // Independently checked with ffprobe on the checked-in fixture.
    assert_eq!(info.chapters.iter().map(|c| (c.start, c.end, c.metadata["title"].as_str())).collect::<Vec<_>>(),
        [(0, 1_000_000, "Opening"), (1_000_000, 3_000_000, "Глава 2"), (3_000_000, 4_000_000, "End")]);
    for c in &info.chapters {
        assert!(c.start <= c.end);
        assert!(c.metadata.contains_key("title"));
    }
    for pair in info.chapters.windows(2) {
        assert_eq!(pair[0].end, pair[1].start);
    }
    let info = fvid::native_probe::probe(&fixture("tags/tags.mp4")).unwrap();
    for (key, value) in [("title", "T"), ("artist", "A"), ("album", "B"), ("date", "2026"), ("comment", "C"), ("genre", "G")] {
        assert_eq!(info.metadata.get(key).map(String::as_str), Some(value));
    }
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "probe"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert!(!json["chapters"].as_array().unwrap().is_empty());
}
