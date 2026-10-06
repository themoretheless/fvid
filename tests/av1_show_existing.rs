use std::io::Cursor;
#[test]
fn owned_show_existing_metadata_replays_in_webm_with_original_clock() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for metadata in ["plain", "id", "time", "time-id", "equal", "equal-id"] {
        for kind in ["key", "intra"] {
            for slot in [0, 7] {
                let name = format!("av1-show-existing-{metadata}-{kind}-slot{slot}.webm");
                let bytes = std::fs::read(root.join(&name)).unwrap();
                let mut reader =
                    fvid::playback_webm::WebmVideoReader::open(Cursor::new(&bytes), 8 << 20)
                        .unwrap();
                let shown = if kind == "key" { 1 } else { 2 };
                for _ in 0..2 {
                    for index in 0..shown {
                        let planes = reader
                            .read_frame_planes()
                            .unwrap_or_else(|e| panic!("{name}: {e}"))
                            .unwrap();
                        assert_eq!((planes.width, planes.height), (32, 32));
                        assert!(
                            planes
                                .y
                                .iter()
                                .chain(&planes.cb)
                                .chain(&planes.cr)
                                .all(|&v| v == 128)
                        );
                        let start = index * 20_000_000;
                        assert_eq!(
                            reader.frame_interval(),
                            Some((start, start + 20_000_000, 1_000_000_000)),
                            "{name}"
                        );
                    }
                    assert!(reader.read_frame_planes().unwrap().is_none());
                    reader.rewind();
                }
                assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
                assert!(reader.read_frame_planes().unwrap().is_some());
            }
        }
    }
}
