//! Native subtitle parser admission, independently of the media command parser.
#[test]
fn native_subtitle_clocks_refuse_unaddressable_times_without_panicking() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-native-clocks");
    let video = std::fs::read(root.join("companion.y4m")).unwrap();
    let decoded = fvid_media::owned_y4m_decode::decode_reader(std::io::Cursor::new(video)).unwrap();
    assert_eq!(
        (decoded.video_frames, decoded.width, decoded.height),
        (2, 2, 2)
    );
    for index in 0..4 {
        let clock = format!("overflow-{index}.srt");
        let source = std::fs::read_to_string(root.join(&clock)).unwrap();
        let cues = fvid::subtitles::parse_srt(&source);
        assert_eq!(cues.len(), 1, "{clock}");
        assert_eq!(cues[0].start, std::time::Duration::from_millis(1500));
        assert_eq!(cues[0].end, std::time::Duration::from_millis(2250));
    }
}

#[test]
fn long_decimal_subtitle_fractions_keep_millisecond_truncation() {
    let text = "00:00:00.9999999999999999999 --> 00:00:01.0000000000000000001\nprecise fraction\n";
    for source in [text.to_owned(), format!("WEBVTT\n\n{text}")] {
        let cues = fvid::subtitles::parse(&source);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start, std::time::Duration::from_millis(999));
        assert_eq!(cues[0].end, std::time::Duration::from_secs(1));
    }
}
