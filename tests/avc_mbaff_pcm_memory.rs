use fvid::playback_native::{NativeReader, RawFrame};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-mbaff-pcm-memory")
}
#[test]
fn independent_pcm_size_series_plays_exact_components_seek_rewind_and_eof() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let side = case["size"].as_u64().unwrap() as usize;
        let mut reader = NativeReader::software(
            Cursor::new(fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            64 << 20,
        )
        .unwrap();
        let verify = |raw: &RawFrame, index: usize| {
            let RawFrame::Avc { picture, .. } = raw else {
                panic!("mixed-depth PCM was narrowed")
            };
            assert_eq!((picture.coded_width, picture.coded_height), (side, side));
            assert_eq!(picture.component_depths(), [8, 10, 10]);
            for (plane, name) in [(&picture.y, "y"), (&picture.cb, "cb"), (&picture.cr, "cr")] {
                let bytes =
                    fs::read(root().join(case["frames"][index][name].as_str().unwrap())).unwrap();
                let expected: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect();
                assert_eq!(plane, &expected, "{} {name}", case["file"]);
            }
        };
        for _ in 0..2 {
            for i in 0..2 {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                verify(&raw, i);
                assert_eq!(
                    reader.frame_interval().unwrap(),
                    (i as u128, i as u128 + 1, 50)
                );
                assert_eq!(raw.into_rgb(64 << 20).unwrap().len(), side * side * 3);
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for i in [1, 0, 1] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(i as u64 * 20))
                .unwrap()
                .unwrap();
            verify(&raw, i);
            assert_eq!(
                reader.frame_interval().unwrap(),
                (i as u128, i as u128 + 1, 50)
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
