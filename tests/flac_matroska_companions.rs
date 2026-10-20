#![cfg(feature = "player")]
use std::{io::Cursor, path::Path};
#[test]
fn authored_flac_regression_video_companions_really_decode() {
    for corpus in ["flac-matroska", "flac-layout"] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(corpus);
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
        for case in manifest["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let source = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
            let mut reader =
                fvid::playback_native::NativeReader::software(Cursor::new(source), 32 << 20)
                    .unwrap();
            assert!(reader.read_frame_raw().unwrap().is_some(), "{name}");
            assert!(reader.read_frame_raw().unwrap().is_none(), "{name}");
        }
    }
}
