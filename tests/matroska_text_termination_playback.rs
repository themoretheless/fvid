use fvid::playback_webm::WebmVideoReader;
use std::{fs, io::Cursor, path::Path};
#[test]
fn nul_terminated_text_preserves_native_playback_seek_rewind_and_eof() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-text-termination");
    for name in ["control.mkv", "doctype-padding.mkv", "doctype-suffix.mkv", "codec-suffix.mkv", "codec-invalid-suffix.mkv", "name-suffix.mkv", "name-invalid-suffix.mkv", "metadata-suffix.mkv"] {
        let mut video = WebmVideoReader::open_software(Cursor::new(fs::read(root.join(name)).unwrap()),32 << 20).unwrap();
        for _ in 0..2 {
            for (index, tick) in [0u128,10,50].into_iter().enumerate() {
                assert!(video.read_frame().unwrap(),"{name}");
                assert_eq!(video.dimensions(),[[16,16],[32,16],[16,32]][index]);
                let interval = video.frame_interval().unwrap();
                assert_eq!((interval.0, interval.1),(tick*1_000_000,(tick+5)*1_000_000));
                assert!(video.rgb().iter().all(|&v| v == 130));
            }
            assert!(!video.read_frame().unwrap());
            video.rewind();
        }
        video.seek_to_frame(50_000_000).unwrap();
        assert_eq!(video.dimensions(),[16,32]);
        assert!(video.rgb().iter().all(|&v| v == 130));
        assert!(!video.read_frame().unwrap());
    }
}
