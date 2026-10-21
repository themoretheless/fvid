use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/sami-body-boundaries")
}
#[test]
fn sami_body_closing_text_in_values_comments_and_headers_is_ignored() {
    let expected = fvid::subtitles::load(&root().join("control.srt")).unwrap();
    for name in [
        "quoted-body-close",
        "comment-body-close",
        "title-raw-close",
        "style-raw-close",
        "title-control",
        "comment-control",
    ] {
        let actual = fvid::subtitles::load(&root().join(format!("{name}.smi")));
        assert_eq!(actual.as_ref(), Some(&expected), "{name}");
    }
}
#[test]
fn sami_body_looking_text_preserves_independent_caption() {
    let video = std::fs::read(root().join("video.y4m")).unwrap();
    let stats = fvid_media::owned_y4m_decode::decode_reader(std::io::Cursor::new(video)).unwrap();
    assert_eq!((stats.video_frames, stats.width, stats.height), (5, 2, 2));
    let expected = fvid::subtitles::load(&root().join("control.srt"));
    assert!(expected.is_some());
    for name in [
        "quoted-body-close",
        "comment-body-close",
        "title-raw-close",
        "style-raw-close",
        "title-control",
        "comment-control",
    ] {
        assert_eq!(
            fvid::subtitles::load(&root().join(format!("{name}.smi"))),
            expected,
            "{name}"
        );
    }
}
