use fvid::playback_webm::WebmVideoReader;
use std::{fs, io::Cursor, path::PathBuf, time::Duration};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-reordered-tail")
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap()
}

fn open(file: &str) -> WebmVideoReader<Cursor<Vec<u8>>> {
    WebmVideoReader::open_software(Cursor::new(fs::read(root().join(file)).unwrap()), 32 << 20)
        .unwrap()
}

#[test]
fn avc_and_hevc_previous_explicit_duration_defines_unknown_tail_and_cache() {
    let manifest = manifest();
    let cases = manifest["progressive"].as_array().unwrap();
    assert_eq!(cases.len(), 4);
    for case in cases {
        let expected_ns = case["expected_ns"].as_u64().unwrap();
        let size: [usize; 2] = serde_json::from_value(case["size"].clone()).unwrap();
        let intervals = case["intervals"].as_array().unwrap();
        for key in ["inferred", "declared"] {
            let file = case[key].as_str().unwrap();
            let mut reader = open(file);
            assert_eq!(
                reader.duration(),
                Some(Duration::from_nanos(expected_ns)),
                "{file} duration"
            );
            let before = reader.cache_packets();
            assert_eq!(before.len(), intervals.len(), "{file} cache count");
            for (packet, interval) in before.iter().zip(intervals) {
                assert_eq!(
                    (packet.2.as_nanos(), packet.3.as_nanos()),
                    (
                        u128::from(interval[0].as_u64().unwrap()),
                        u128::from(interval[1].as_u64().unwrap())
                    ),
                    "{file} cache interval"
                );
            }
            for _ in 0..2 {
                for interval in intervals {
                    assert!(reader.read_frame().unwrap());
                    assert_eq!(reader.dimensions(), size);
                    let actual = reader.frame_interval().unwrap();
                    assert_eq!(
                        (actual.0, actual.1),
                        (
                            u128::from(interval[0].as_u64().unwrap()),
                            u128::from(interval[1].as_u64().unwrap())
                        ),
                        "{file} playback interval"
                    );
                }
                assert!(!reader.read_frame().unwrap());
                assert_eq!(reader.cache_packets(), before, "{file} post-playback cache");
                reader.rewind();
            }
            let last = intervals.last().unwrap();
            reader.seek_to_frame(last[0].as_i64().unwrap()).unwrap();
            assert_eq!(
                reader.frame_interval().unwrap().1,
                u128::from(last[1].as_u64().unwrap()),
                "{file} seek tail"
            );
            assert!(!reader.read_frame().unwrap());
        }
    }
}

#[test]
fn untimed_paff_with_unequal_field_cadence_uses_declared_tail_and_matches_transcode() {
    let manifest = manifest();
    let cases = manifest["paff"].as_array().unwrap();
    assert_eq!(cases.len(), 4);
    for (case_idx, case) in cases.iter().enumerate() {
        let file = case["file"].as_str().unwrap();
        let expected_ns = case["expected_ns"].as_u64().unwrap();
        let intervals = case["intervals"].as_array().unwrap();
        let mut reader = open(file);
        assert_eq!(
            reader.duration(),
            Some(Duration::from_nanos(expected_ns)),
            "{file} duration"
        );
        for _ in 0..2 {
            for interval in intervals {
                assert!(reader.read_frame().unwrap());
                assert_eq!(reader.dimensions(), [32, 32]);
                let actual = reader.frame_interval().unwrap();
                assert_eq!(
                    (actual.0, actual.1),
                    (
                        u128::from(interval[0].as_u64().unwrap()),
                        u128::from(interval[1].as_u64().unwrap())
                    ),
                    "{file} playback interval"
                );
            }
            assert!(!reader.read_frame().unwrap());
            reader.rewind();
        }
        let output = std::env::temp_dir().join(format!(
            "fvid-reordered-tail-paff-{}-{case_idx}.mkv",
            std::process::id()
        ));
        fvid_media::transcode_lossless(
            &root().join(file),
            &output,
            fvid_media::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let mut exported = WebmVideoReader::open_software(
            Cursor::new(fs::read(&output).unwrap()),
            32 << 20,
        )
        .unwrap();
        for interval in intervals {
            assert!(exported.read_frame().unwrap());
            let actual = exported.frame_interval().unwrap();
            assert_eq!(
                (actual.0, actual.1),
                (
                    u128::from(interval[0].as_u64().unwrap()),
                    u128::from(interval[1].as_u64().unwrap())
                ),
                "{file} exported interval"
            );
        }
        assert!(!exported.read_frame().unwrap());
        let _ = fs::remove_file(output);
    }
}
