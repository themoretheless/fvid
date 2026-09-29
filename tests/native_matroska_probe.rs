#![cfg(feature = "media")]
use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn webm_and_matroska_tracks_are_described_by_owned_index() {
    for (name, codec) in [
        ("vp9/adaptive.webm", "vp9"),
        ("av1/random-access.webm", "av1"),
        ("audio/aac-stereo.mka", "aac"),
        ("audio/aac-960-48000.mka", "aac"),
    ] {
        let source = fixture(name);
        let info = fvid::media::probe(&source).unwrap();
        assert_eq!(info.format, "matroska,webm");
        assert_eq!(info.streams[0].codec, codec);
        assert_eq!(info.streams[0].time_base, [1, 1_000_000_000]);
        assert!(info.streams[0].start.is_some());
        assert_eq!(info.streams[0].duration, None);
        assert_eq!(info.streams[0].pixel_format, -1);
        assert_eq!(
            fvid::media::probe_as(&source, Some("matroska"))
                .unwrap()
                .streams,
            info.streams
        );
        if codec == "aac" {
            assert_eq!(info.streams[0].sample_rate, 48000);
        }
    }
    // Checked independently against ffprobe; these are container facts, not
    // estimates from compressed size or guesses about the last packet's length.
    let info = fvid::media::probe(&fixture("audio/aac-stereo.mka")).unwrap();
    assert_eq!(info.duration_us, Some(1_021_000));
    // The fixture declares CodecDelay=21_333_333 ns and first block PTS=0.
    // Native probe includes the pre-roll origin, before decoded delay trimming.
    assert_eq!(info.start_us, Some(-21_333));
    assert_eq!(info.streams[0].start, Some(-21_333_333));
    assert_eq!(info.streams[0].channels, 2);
}
#[test]
fn chapters_tags_and_cli_are_preserved() {
    let source = fixture("chapters/chapters.mkv");
    let info = fvid::media::probe(&source).unwrap();
    assert_eq!(
        info.chapters
            .iter()
            .map(|c| (c.start, c.end, c.metadata["title"].as_str()))
            .collect::<Vec<_>>(),
        [
            (0, 1_000_000, "Opening"),
            (1_000_000, 3_000_000, "Глава 2"),
            (3_000_000, 4_000_000, "End")
        ]
    );
    let tags = fvid::media::probe(&fixture("tags/tags.mkv"))
        .unwrap()
        .metadata;
    for (key, value) in [
        ("title", "T"),
        ("artist", "A"),
        ("album", "B"),
        ("genre", "G"),
        ("comment", "C"),
        ("date", "2026"),
    ] {
        assert_eq!(tags.get(key).map(String::as_str), Some(value));
    }
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "probe"])
        .arg(source)
        .args(["--input-format", "webm"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(json["format"], "matroska,webm");
    assert_eq!(json["chapters"].as_array().unwrap().len(), 3);
    assert!(fvid::media::probe_as(&fixture("video.mp4"), Some("matroska")).is_err());
}
