use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-negative-composition-endpoint")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn exact_signed_composition_packets_decode_without_changing_media_duration() {
    for c in cases() {
        let data = bytes(c["file"].as_str().unwrap());
        let shift = c["shift"].as_i64().unwrap();
        let a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        macro_rules! check_track {
            ($t:expr) => {{
                let t = $t;
                assert_eq!(
                    (t.duration, t.timescale),
                    (c["media_duration"].as_u64().unwrap(), 50)
                );
                assert!(t.edits.is_empty());
                for i in 0..3 {
                    let s = t.samples.get(i).unwrap();
                    assert_eq!((s.dts, s.pts, s.duration), (i as u64, i as i64 + shift, 1));
                }
            }};
        }
        check_track!(&a.tracks()[0]);
        check_track!(&b.tracks()[0]);
        let mut v = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(data),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let f = v.read_frame().unwrap().unwrap();
            assert_eq!(
                (f.presentation_time.ticks, f.duration.ticks),
                (i as i64 + shift, 1)
            );
            let y: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|n| n.to_le_bytes())
                .collect();
            assert_eq!(y, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(v.read_frame().unwrap().is_none());
    }
}
#[test]
fn negative_preroll_does_not_create_a_phantom_decode_duration_tail() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let end = c["end"].as_u64().unwrap() * 20;
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let expected = Some(Duration::from_millis(end));
        if r.duration() != expected {
            failures.push(format!("{n}: {:?},expected {expected:?}", r.duration()));
        }
        while r.read_frame_raw().unwrap().is_some() {}
        r.rewind().unwrap();
        assert!(
            r.seek_raw(Duration::from_millis(end - 1))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            r.frame_interval(),
            Some((
                (c["shift"].as_i64().unwrap() + 2) as u128,
                (c["shift"].as_i64().unwrap() + 3) as u128,
                50
            ))
        );
        assert!(r.read_frame_raw().unwrap().is_none());
        assert!(
            r.seek_raw(Duration::from_millis(end + 20))
                .unwrap()
                .is_some()
        );
        assert!(r.read_frame_raw().unwrap().is_none());
    }
    assert!(failures.is_empty(), "phantom composition tail:{failures:?}");
}
