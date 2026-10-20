use fvid::playback_webm::WebmVideoReader;
use std::{fs, io::Cursor, path::Path};

#[test]
fn track_clocks_drive_decoded_frames_seek_rewind_and_end_of_stream() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-clock");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let data = fs::read(root.join(file)).unwrap();
        let mut playback = WebmVideoReader::open_software(Cursor::new(data), 32 << 20).unwrap();
        let duration = u128::from(case["duration_ns"].as_u64().unwrap());
        for _ in 0..2 {
            for index in 0..3 {
                assert!(playback.read_frame().unwrap(), "{file} frame {index}");
                let pts = u128::from(case["pts_ns"][index].as_u64().unwrap());
                let size: [usize; 2] =
                    serde_json::from_value(case["sizes"][index].clone()).unwrap();
                assert_eq!(playback.dimensions(), size, "{file} frame {index}");
                let interval = playback.frame_interval().unwrap();
                assert_eq!(
                    (interval.0, interval.1),
                    (pts, pts + duration),
                    "{file} frame {index}"
                );
                assert!(playback.rgb().iter().all(|&sample| sample == 130));
            }
            assert!(!playback.read_frame().unwrap());
            playback.rewind();
        }
        for index in [2, 0, 1] {
            let pts = case["pts_ns"][index].as_u64().unwrap();
            playback.seek_to_frame(i64::try_from(pts).unwrap()).unwrap();
            let size: [usize; 2] = serde_json::from_value(case["sizes"][index].clone()).unwrap();
            assert_eq!(playback.dimensions(), size);
            let interval = playback.frame_interval().unwrap();
            assert_eq!(
                (interval.0, interval.1),
                (u128::from(pts), u128::from(pts) + duration)
            );
            assert!(playback.rgb().iter().all(|&sample| sample == 130));
            if index == 2 {
                assert!(!playback.read_frame().unwrap());
            }
        }
    }
}
