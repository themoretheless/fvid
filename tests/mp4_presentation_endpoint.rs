use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-presentation-endpoint")
        .join(n)
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&std::fs::read(path("manifest.json")).unwrap())
        .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn composition_endpoint_witnesses_keep_raw_media_clock_and_exact_decoded_frames() {
    for c in cases() {
        let data = std::fs::read(path(c["file"].as_str().unwrap())).unwrap();
        let root =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let owned =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        assert_eq!(root.tracks()[0].duration, 3);
        assert_eq!(owned.tracks()[0].duration, 3);
        assert!(root.tracks()[0].edits.is_empty());
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(&data),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for wanted in c["frames"].as_array().unwrap() {
            let f = video.read_frame().unwrap().unwrap();
            assert_eq!(f.presentation_time.ticks, wanted["pts"].as_i64().unwrap());
            assert_eq!(f.duration.ticks, wanted["duration"].as_i64().unwrap());
            let pixels: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(
                pixels,
                std::fs::read(path(&format!(
                    "frame-{}.yuv16",
                    wanted["sample"].as_u64().unwrap()
                )))
                .unwrap()
            );
        }
        assert!(video.read_frame().unwrap().is_none());
    }
}
#[cfg(feature = "player")]
#[test]
fn native_duration_and_seek_reach_the_actual_composition_endpoint() {
    use std::time::Duration;
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let data = std::fs::read(path(n)).unwrap();
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(data), 32 << 20).unwrap();
        let end = c["end"].as_u64().unwrap() * 20;
        let expected = Some(Duration::from_millis(end));
        if r.duration() != expected {
            failures.push(format!("{n}: {:?} != {expected:?}", r.duration()));
        }
        for frame in c["frames"].as_array().unwrap() {
            assert!(r.read_frame_raw().unwrap().is_some());
            let at = frame["pts"].as_u64().unwrap() as u128;
            assert_eq!(
                r.frame_interval(),
                Some((at, at + frame["duration"].as_u64().unwrap() as u128, 50))
            );
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        r.rewind().unwrap();
        assert_eq!(r.duration(), expected, "{n}: duration after rewind");
        assert!(r.read_frame_raw().unwrap().is_some());
        assert!(
            r.seek_raw(Duration::from_millis(end - 1))
                .unwrap()
                .is_some()
        );
        let last = c["frames"].as_array().unwrap().last().unwrap();
        assert_eq!(
            r.frame_interval(),
            Some((
                last["pts"].as_u64().unwrap() as u128,
                c["end"].as_u64().unwrap() as u128,
                50
            ))
        );
        assert!(r.read_frame_raw().unwrap().is_none());
        assert!(
            r.seek_raw(Duration::from_millis(end + 40))
                .unwrap()
                .is_some()
        );
        assert!(r.read_frame_raw().unwrap().is_none());
        assert_eq!(r.duration(), expected, "{n}: duration after EOF and seeks");
    }
    assert!(
        failures.is_empty(),
        "declared duration truncates composition timeline: {failures:?}"
    );
}
