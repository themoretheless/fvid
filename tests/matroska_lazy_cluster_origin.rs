use fvid::playback_webm::WebmVideoReader;
use std::{fs, io::Cursor, path::PathBuf, time::Duration};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-lazy-cluster-origin")
}

fn cases() -> Vec<serde_json::Value> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    manifest["cases"].as_array().unwrap().clone()
}

fn open(file: &str) -> WebmVideoReader<Cursor<Vec<u8>>> {
    WebmVideoReader::open_software(Cursor::new(fs::read(root().join(file)).unwrap()), 64 << 20)
        .unwrap()
}

#[test]
fn multi_cluster_declared_duration_and_origin_resolve_before_first_frame_when_cluster_zero_has_no_visible_video() {
    let all = cases();
    assert_eq!(all.len(), 8);
    for case in all {
        let file = case["file"].as_str().unwrap();
        let expected_ns = case["expected_duration_ns"].as_u64().unwrap();
        let intervals = case["intervals"].as_array().unwrap();
        let sizes = case["sizes"].as_array().unwrap();
        let mut reader = open(file);
        assert_eq!(
            reader.duration(),
            Some(Duration::from_nanos(expected_ns)),
            "{file} pre-decode duration"
        );
        if let Some(cll) = case["initial_cll"].as_array() {
            let hdr = reader.bitstream_hdr();
            assert_eq!(
                (hdr.light.max_cll, hdr.light.max_fall),
                (
                    cll[0].as_f64().unwrap() as f32,
                    cll[1].as_f64().unwrap() as f32
                ),
                "{file} pre-decode AV1 in-band HDR"
            );
        }
        let before = reader.cache_packets();
        assert_eq!(before.len(), intervals.len(), "{file} pre-decode cache count");
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
            for (i, interval) in intervals.iter().enumerate() {
                assert!(reader.read_frame().unwrap());
                let size: [usize; 2] = serde_json::from_value(sizes[i].clone()).unwrap();
                assert_eq!(reader.dimensions(), size, "{file} frame {i} size");
                let actual = reader.frame_interval().unwrap();
                assert_eq!(
                    (actual.0, actual.1),
                    (
                        u128::from(interval[0].as_u64().unwrap()),
                        u128::from(interval[1].as_u64().unwrap())
                    ),
                    "{file} frame {i} interval"
                );
            }
            assert!(!reader.read_frame().unwrap());
            assert_eq!(
                reader.duration(),
                Some(Duration::from_nanos(expected_ns)),
                "{file} post-decode duration"
            );
            assert_eq!(reader.cache_packets(), before, "{file} post-decode cache");
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
