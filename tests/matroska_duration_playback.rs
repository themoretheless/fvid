use fvid::playback_webm::WebmVideoReader;
use std::{fs, io::Cursor, path::Path, time::Duration};

#[test]
fn declared_durations_match_metadata_without_changing_decoded_frames_or_seeks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-duration");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let mut video = WebmVideoReader::open_software(
            Cursor::new(fs::read(root.join(file)).unwrap()),
            32 << 20,
        )
        .unwrap();
        let expected = Duration::from_nanos(case["expected_ns"].as_u64().unwrap());
        assert_eq!(video.duration(), Some(expected), "{file}");
        for _ in 0..2 {
            for (index, tick) in [0, 10, 50].into_iter().enumerate() {
                assert!(video.read_frame().unwrap());
                assert_eq!(video.dimensions(), [[16, 16], [32, 16], [16, 32]][index]);
                let start = tick * case["scale"].as_u64().unwrap();
                let end = start + 5 * case["scale"].as_u64().unwrap();
                let interval = video.frame_interval().unwrap();
                assert_eq!(
                    (interval.0, interval.1),
                    (u128::from(start), u128::from(end))
                );
                assert!(video.rgb().iter().all(|&sample| sample == 130));
            }
            assert!(!video.read_frame().unwrap());
            video.rewind();
        }
        video
            .seek_to_frame(50 * case["scale"].as_u64().unwrap() as i64)
            .unwrap();
        assert_eq!(video.dimensions(), [16, 32]);
        assert!(video.rgb().iter().all(|&sample| sample == 130));
        assert!(!video.read_frame().unwrap());
        assert_eq!(video.duration(), Some(expected));
    }
}
