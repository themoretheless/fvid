use fvid::playback_webm::WebmVideoReader;
use std::{fs, io::Cursor, path::Path};
#[test]
fn audio_defaults_do_not_change_native_video_pixels_clocks_seek_or_eof() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-audio-defaults");
    for name in [
        "control",
        "missing-rate",
        "missing-channels",
        "missing-both",
        "empty-rate",
        "empty-channels",
        "empty-both",
        "empty-output-rate",
    ] {
        let data = fs::read(root.join(format!("{name}.mkv"))).unwrap();
        let mut video = WebmVideoReader::open_software(Cursor::new(data), 32 << 20).unwrap();
        for _ in 0..2 {
            for i in 0..3u128 {
                assert!(video.read_frame().unwrap());
                assert_eq!(video.dimensions(), [16, 16]);
                assert!(video.rgb().iter().all(|&v| v == 130));
                let clock = video.frame_interval().unwrap();
                assert_eq!((clock.0, clock.1), (i * 20_000_000, (i + 1) * 20_000_000));
            }
            assert!(!video.read_frame().unwrap());
            video.rewind();
        }
        video.seek_to_frame(40_000_000).unwrap();
        assert!(video.rgb().iter().all(|&v| v == 130));
        assert!(!video.read_frame().unwrap());
    }
}
