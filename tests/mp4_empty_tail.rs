use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-empty-tail")
        .join(n)
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(path(n)).unwrap()
}
fn pixels(f: &fvid::playback_mp4::VideoFrame) -> Vec<u8> {
    f.picture
        .y
        .iter()
        .chain(&f.picture.cb)
        .chain(&f.picture.cr)
        .flat_map(|v| v.to_le_bytes())
        .collect()
}
#[test]
fn explicit_zero_delta_is_preserved_by_both_parsers() {
    let data = bytes("empty-tail.mp4");
    let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
    let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
    for i in 0..3 {
        assert_eq!(
            a.tracks()[0].samples.get(i).unwrap().duration,
            if i == 2 { 0 } else { 1 }
        );
        assert_eq!(
            b.tracks()[0].samples.get(i).unwrap().duration,
            if i == 2 { 0 } else { 1 }
        );
    }
    assert_eq!(a.tracks()[0].duration, 2);
}
#[test]
fn empty_final_presentation_window_completes_without_emitting_or_failing() {
    let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("empty-tail.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        for i in 0..2 {
            let f = r.read_frame().unwrap().unwrap();
            assert_eq!(pixels(&f), bytes(&format!("frame-{i}.yuv16")));
            assert_eq!((f.presentation_time.ticks, f.duration.ticks), (i, 1));
        }
        assert!(
            r.read_frame()
                .expect("zero-duration final sample must not poison EOF")
                .is_none()
        );
        assert!(r.read_frame().unwrap().is_none());
        r.rewind();
    }
    let stats = fvid_media::decode_video(&path("empty-tail.mp4")).unwrap();
    assert_eq!(stats.video_frames, 2);
    assert_eq!(stats.decode_errors, 0);
}
#[test]
fn zero_duration_does_not_hide_a_broken_final_codec_packet() {
    let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("invalid-empty-tail.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    let error = loop {
        match r.read_frame() {
            Ok(Some(_)) => {}
            Ok(None) => panic!("invalid empty tail silently skipped"),
            Err(e) => break e,
        }
    };
    assert!(
        error.to_string().contains("invalid VP9 frame marker"),
        "{error}"
    );
    let owned_error = fvid_media::decode_video(&path("invalid-empty-tail.mp4")).unwrap_err();
    assert!(
        owned_error.contains("invalid VP9 frame marker"),
        "{owned_error}"
    );
}
#[cfg(feature = "player")]
#[test]
fn native_empty_tail_preserves_duration_seek_rewind_and_eof() {
    use std::time::Duration;
    let mut r = fvid::playback_native::NativeReader::software(
        Cursor::new(bytes("empty-tail.mp4")),
        32 << 20,
    )
    .unwrap();
    assert_eq!(r.duration(), Some(Duration::from_millis(40)));
    let mut expected = Vec::new();
    for i in 0..2 {
        expected.push(
            r.read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
        );
        assert_eq!(r.frame_interval(), Some((i, i + 1, 50)));
    }
    assert!(
        r.read_frame_raw()
            .expect("empty tail must complete native EOF")
            .is_none()
    );
    r.rewind().unwrap();
    assert_eq!(
        r.read_frame_raw()
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        expected[0]
    );
    for ms in [0, 19, 20, 39, 40, 80] {
        let frame = r
            .seek_raw(Duration::from_millis(ms))
            .unwrap()
            .expect("seek past empty tail must land on last visible frame");
        let i = (ms.min(39) / 20) as usize;
        assert_eq!(frame.into_rgb(32 << 20).unwrap(), expected[i]);
        assert_eq!(r.frame_interval(), Some((i as u128, i as u128 + 1, 50)));
        if ms >= 20 {
            assert!(r.read_frame_raw().unwrap().is_none());
        }
    }
}
