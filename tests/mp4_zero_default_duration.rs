use fvid::container::mp4::Mp4Reader;
use std::io::Cursor;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-zero-default-duration")
            .join(name),
    )
    .unwrap()
}
#[test]
fn explicit_zero_fragment_duration_overrides_inherited_duration() {
    check("zero.mp4", 0);
}
#[test]
fn positive_and_absent_fragment_duration_controls() {
    for (name, step) in [("one.mp4", 1), ("two.mp4", 2), ("inherited.mp4", 1)] {
        check(name, step);
    }
}
fn check(name: &str, step: u32) {
    let mut r = Mp4Reader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
    let t = &r.tracks()[0];
    assert_eq!(t.samples.len(), 3);
    for (i, (dts, duration)) in [(0, step), (u64::from(step), 1), (u64::from(step) + 1, 1)]
        .into_iter()
        .enumerate()
    {
        let s = t.samples.get(i).unwrap();
        assert_eq!(
            (s.dts, s.pts, s.duration),
            (dts, dts as i64, duration),
            "{name} sample {i}"
        );
    }
    let expected = bytes("frame.vp9");
    for i in [2, 0, 1, 2] {
        let mut got = Vec::new();
        r.read_packet(0, i, &mut got).unwrap();
        assert_eq!(got, expected);
    }
}

#[test]
fn owned_parser_preserves_explicit_zero_duration() {
    let mut r =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes("zero.mp4")), Default::default())
            .unwrap();
    let stamps: Vec<_> = (0..3)
        .map(|i| {
            let s = r.tracks()[0].samples.get(i).unwrap();
            (s.dts, s.pts, s.duration)
        })
        .collect();
    assert_eq!(stamps, vec![(0, 0, 0), (0, 0, 1), (1, 1, 1)]);
    for i in [2, 0, 1] {
        let mut got = Vec::new();
        r.read_packet(0, i, &mut got).unwrap();
        assert_eq!(got, bytes("frame.vp9"));
    }
}

#[cfg(feature = "player")]
#[test]
fn zero_duration_override_preserves_playback_intervals_seek_rewind_and_eof() {
    use fvid::playback_native::NativeReader;
    use std::time::Duration;
    let mut r = NativeReader::software(Cursor::new(bytes("zero.mp4")), 32 << 20).unwrap();
    assert_eq!(r.duration(), Some(Duration::from_millis(40)));
    for i in 0..2 {
        let frame = r.read_frame_raw().unwrap().unwrap();
        assert_eq!(frame.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
        assert_eq!(r.frame_interval(), Some((i, i + 1, 50)));
    }
    assert!(r.read_frame_raw().unwrap().is_none());
    assert!(r.read_frame_raw().unwrap().is_none());
    r.rewind().unwrap();
    assert!(r.read_frame_raw().unwrap().is_some());
    assert_eq!(r.frame_interval(), Some((0, 1, 50)));
    for ms in [0, 19, 20, 39, 40, 80] {
        let frame = r.seek_raw(Duration::from_millis(ms)).unwrap().unwrap();
        assert_eq!(frame.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
        let i = u128::from(ms.min(39) / 20);
        assert_eq!(r.frame_interval(), Some((i, i + 1, 50)));
        assert_eq!(r.current_pts(), Some((i as i64, 50)));
    }
}
