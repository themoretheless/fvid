use std::path::PathBuf;
use std::time::Duration;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/sami-sidecar-extension")
}
fn cases() -> [(&'static str, &'static str); 4] {
    [
        ("lower", "video.sami"),
        ("upper", "video.SAMI"),
        ("loose", "video.en.sami"),
        ("smi-control", "video.smi"),
    ]
}
#[test]
fn sami_sidecars_load_explicitly_and_are_discovered() {
    for (name, subtitle) in cases() {
        let directory = root().join(name);
        let path = directory.join(subtitle);
        let cues = fvid::subtitles::load(&path).unwrap();
        assert_eq!(
            (cues[0].start, cues[0].end),
            (Duration::from_secs(1), Duration::from_secs(2))
        );
        assert_eq!(cues[0].text, "Строка α");
        let found = fvid::subtitles::sidecars(&directory.join("video.y4m"));
        assert_eq!(found, vec![path], "{name}: {found:?}");
    }
}
#[test]
fn sami_sidecars_are_found_beside_authored_video() {
    for (name, subtitle) in cases() {
        let directory = root().join(name);
        let video = std::fs::read(directory.join("video.y4m")).unwrap();
        let stats =
            fvid_media::owned_y4m_decode::decode_reader(std::io::Cursor::new(video)).unwrap();
        assert_eq!((stats.video_frames, stats.width, stats.height), (5, 2, 2));
        assert_eq!(
            fvid::subtitles::sidecars(&directory.join("video.y4m")),
            vec![directory.join(subtitle)],
            "{name}"
        );
    }
}
