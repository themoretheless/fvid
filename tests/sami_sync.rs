use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/sami-sync")
}
#[test]
fn sami_sync_start_loads_each_authored_caption_sequence() {
    for name in [
        "clear",
        "replace",
        "quoted",
        "lines",
        "language-gap",
        "invalid-clock",
    ] {
        let text = std::fs::read_to_string(root().join(format!("{name}.smi"))).unwrap();
        let control = std::fs::read_to_string(root().join(format!("{name}.srt"))).unwrap();
        assert_eq!(
            fvid::subtitles::parse_sami(&text),
            fvid::subtitles::parse_srt(&control),
            "{name}"
        );
        assert!(fvid::subtitles::load(&root().join(format!("{name}.srt"))).is_some());
    }
}
#[test]
fn sami_sync_start_matches_literal_subrip_clocks_and_clear_events() {
    let video = std::fs::read(root().join("video.y4m")).unwrap();
    let stats = fvid_media::owned_y4m_decode::decode_reader(std::io::Cursor::new(video)).unwrap();
    assert_eq!((stats.video_frames, stats.width, stats.height), (5, 2, 2));
    for name in [
        "clear",
        "replace",
        "quoted",
        "lines",
        "language-gap",
        "invalid-clock",
    ] {
        assert_eq!(
            fvid::subtitles::load(&root().join(format!("{name}.smi"))),
            fvid::subtitles::load(&root().join(format!("{name}.srt"))),
            "{name}"
        );
    }
}
