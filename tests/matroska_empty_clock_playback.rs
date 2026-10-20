use fvid::playback_webm::WebmVideoReader;
use std::{fs, io::Cursor, path::Path};
fn check(name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-empty-clock");
    let mut video = WebmVideoReader::open_software(
        Cursor::new(fs::read(root.join(format!("{name}.mkv"))).unwrap()),
        32 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        for (i, t) in [0u128, 10, 50].into_iter().enumerate() {
            assert!(video.read_frame().unwrap());
            assert_eq!(video.dimensions(), [[16, 16], [32, 16], [16, 32]][i]);
            assert!(video.rgb().iter().all(|&v| v == 130));
            let clock = video.frame_interval().unwrap();
            assert_eq!((clock.0, clock.1), (t * 1_000_000, (t + 5) * 1_000_000));
        }
        assert!(!video.read_frame().unwrap());
        video.rewind();
    }
    video.seek_to_frame(50_000_000).unwrap();
    assert_eq!(video.dimensions(), [16, 32]);
    assert!(!video.read_frame().unwrap());
}
#[test]
fn explicit_clock_control_plays_correctly() {
    check("control");
}
#[test]
fn empty_segment_timestamp_scale_uses_one_millisecond() {
    check("segment");
}
#[test]
fn empty_track_timestamp_scale_uses_one() {
    check("track");
}

#[test]
fn both_clock_defaults_work_together() {
    check("both");
}
#[test]
fn explicit_zero_and_malformed_float_remain_invalid() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-empty-clock");
    for name in ["zero-segment", "invalid-track-width"] {
        assert!(
            WebmVideoReader::open_software(
                Cursor::new(fs::read(root.join(format!("{name}.mkv"))).unwrap()),
                32 << 20
            )
            .is_err()
        );
    }
}
