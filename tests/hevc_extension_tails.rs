use fvid::{container::mp4::Limits, playback_mp4::Mp4VideoReader};
use std::io::Cursor;

#[test]
fn future_parameter_extension_tails_preserve_base_picture_playback() {
    let seed = include_bytes!("fixtures/hevc/main-ipb.mp4");
    let mut baseline =
        Mp4VideoReader::open_software(Cursor::new(seed), Limits::default(), 16 << 20).unwrap();
    let mut expected = vec![];
    while let Some(frame) = baseline.read_frame().unwrap() {
        expected.push(frame);
    }
    assert_eq!(expected.len(), 17);
    for (label, bytes) in [
        (
            "sps",
            include_bytes!("fixtures/playback-errors/hevc-future-extension-sps-synthetic.mp4")
                .as_slice(),
        ),
        (
            "pps",
            include_bytes!("fixtures/playback-errors/hevc-future-extension-pps-synthetic.mp4")
                .as_slice(),
        ),
        (
            "both",
            include_bytes!("fixtures/playback-errors/hevc-future-extension-both-synthetic.mp4")
                .as_slice(),
        ),
    ] {
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20)
                .unwrap_or_else(|e| panic!("{label}: {e}"));
        for pass in 0..2 {
            for reference in &expected {
                let frame = reader.read_frame().unwrap().unwrap();
                assert_eq!(
                    frame.sample_index, reference.sample_index,
                    "{label}, pass {pass}"
                );
                assert_eq!(frame.presentation_time, reference.presentation_time);
                assert_eq!(frame.duration, reference.duration);
                assert_eq!(frame.picture.y, reference.picture.y);
                assert_eq!(frame.picture.cb, reference.picture.cb);
                assert_eq!(frame.picture.cr, reference.picture.cr);
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}

#[test]
fn future_extension_without_rbsp_stop_is_rejected() {
    let bytes =
        include_bytes!("fixtures/playback-errors/hevc-future-extension-bad-stop-synthetic.mp4");
    let error = Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20)
        .err()
        .expect("missing stop bit must fail");
    assert_eq!(error.to_string(), "truncated or oversized bit field");
}
