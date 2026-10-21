use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/sami-break-case")
}
#[test]
fn sami_break_names_preserve_lines_in_every_authored_case() {
    let expected = fvid::subtitles::load(&root().join("control.srt")).unwrap();
    for name in ["upper", "mixed", "attributes", "lower-control"] {
        let actual = fvid::subtitles::load(&root().join(format!("{name}.smi"))).unwrap();
        assert_eq!(
            (actual[0].start, actual[0].end),
            (expected[0].start, expected[0].end)
        );
        assert_eq!(actual, expected, "{name}");
    }
}
#[test]
fn sami_break_case_matches_independent_subrip_lines() {
    let video = std::fs::read(root().join("video.y4m")).unwrap();
    let stats = fvid_media::owned_y4m_decode::decode_reader(std::io::Cursor::new(video)).unwrap();
    assert_eq!((stats.video_frames, stats.width, stats.height), (5, 2, 2));
    let expected = fvid::subtitles::load(&root().join("control.srt"));
    for name in ["upper", "mixed", "attributes", "lower-control"] {
        assert_eq!(
            fvid::subtitles::load(&root().join(format!("{name}.smi"))),
            expected,
            "{name}"
        );
    }
}
