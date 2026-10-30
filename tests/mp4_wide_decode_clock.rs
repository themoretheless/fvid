use std::{io::Cursor, path::Path};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-wide-decode-clock")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn wide_unsigned_decode_time_can_have_a_representable_signed_composition_time() {
    let mut failures = Vec::new();
    for c in cases().into_iter().filter(|c| c["accepted"] == true) {
        let n = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default());
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default());
        let (mut a, mut b) = match (a, b) {
            (Ok(a), Ok(b)) => (a, b),
            (a, b) => {
                failures.push(format!(
                    "{n}: root={}, owned={}",
                    a.err().map_or(String::new(), |e| e.to_string()),
                    b.err().map_or(String::new(), |e| e.to_string())
                ));
                continue;
            }
        };
        let dts = c["dts"].as_u64().unwrap();
        let pts = c["pts"].as_i64().unwrap();
        let x = a.tracks()[0].samples.get(0).unwrap();
        let y = b.tracks()[0].samples.get(0).unwrap();
        assert_eq!((x.dts, x.pts, x.duration), (dts, pts, 1));
        assert_eq!((y.dts, y.pts, y.duration), (dts, pts, 1));
        assert_eq!(a.tracks()[0].duration, dts + 1);
        let (mut x, mut y) = (Vec::new(), Vec::new());
        a.read_packet(0, 0, &mut x).unwrap();
        b.read_packet(0, 0, &mut y).unwrap();
        assert_eq!(x, bytes("frame.packet"));
        assert_eq!(x, y);
    }
    assert!(
        failures.is_empty(),
        "representable composition time refused: {failures:?}"
    );
}
#[test]
fn actual_composition_overflow_is_still_refused() {
    let a = fvid::container::mp4::Mp4Reader::open(
        Cursor::new(bytes("true-overflow.mp4")),
        Default::default(),
    )
    .err()
    .expect("overflow accepted");
    let b = fvid_media::owned_mp4::Mp4Reader::open(
        Cursor::new(bytes("true-overflow.mp4")),
        Default::default(),
    )
    .err()
    .expect("overflow accepted");
    assert!(a.to_string().contains("overflow"));
    assert!(b.to_string().contains("overflow"));
}
#[cfg(feature = "player")]
#[test]
fn wide_decode_clock_plays_the_exact_picture_and_seeks_on_its_signed_pts() {
    let mut failures = Vec::new();
    for c in cases().into_iter().filter(|c| c["accepted"] == true) {
        let n = c["file"].as_str().unwrap();
        let pts = c["pts"].as_i64().unwrap();
        let mut r = match fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
            Default::default(),
            32 << 20,
        ) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{n}: {e}"));
                continue;
            }
        };
        for replay in 0..2 {
            if replay != 0 {
                assert_eq!(r.seek_to_sync(pts), pts);
            }
            let f = r.read_frame().unwrap().unwrap();
            assert_eq!(
                (
                    f.presentation_time.ticks,
                    f.presentation_time.timescale,
                    f.duration.ticks
                ),
                (pts, 1_000_000_000, 1)
            );
            let pcm: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(pcm, bytes("frame.yuv16"));
            assert!(r.read_frame().unwrap().is_none());
        }
    }
    assert!(
        failures.is_empty(),
        "valid wide clock playback refused: {failures:?}"
    );
}
