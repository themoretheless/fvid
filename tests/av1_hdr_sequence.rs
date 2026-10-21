//! Received HDR must override the opening snapshot, with no external test tools.
use fvid::{
    codec::av1_decoder::Decoder,
    container::mp4::Limits,
    playback::video::{mp4::Mp4VideoReader, webm::WebmVideoReader},
    playback_native::RawFrame,
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn data(name: &str) -> Vec<u8> {
    std::fs::read(root().join("av1-hdr-sequence").join(name)).unwrap()
}
fn expected() -> Vec<u8> {
    std::fs::read(root().join("av1-separate-tiles-w128-h128-groups2.yuv")).unwrap()
}
const LIGHT: [(f32, f32); 3] = [(1000., 400.), (1400., 500.), (750., 250.)];
fn check(h: fvid::color::hdr::HdrMetadata, i: usize) {
    assert_eq!(
        (h.light.max_cll, h.light.max_fall),
        LIGHT[i],
        "displayed sequence {i}"
    );
}
#[test]
fn direct_decoder_receives_distinct_sequence_metadata() {
    let mut d = Decoder::new(BUDGET);
    for i in 0..3 {
        let frames = d
            .decode_container_packet(&data(&format!("{i}.packet")))
            .unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].show);
        check(d.hdr(), i);
    }
}
#[test]
fn mp4_received_hdr_overrides_opening_metadata_on_every_frame() {
    let mut r =
        Mp4VideoReader::open_software(Cursor::new(data("sequence.mp4")), Limits::default(), BUDGET)
            .unwrap();
    let pixels = expected();
    for _ in 0..2 {
        check(r.bitstream_hdr(), 0);
        for i in 0..3 {
            let f = r.read_frame().unwrap().unwrap();
            let p = f.planes8.as_ref().unwrap();
            let raw: Vec<_> = p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect();
            assert_eq!(raw, pixels);
            assert_eq!(f.sample_index, i);
            check(r.bitstream_hdr(), i);
        }
        assert!(r.read_frame().unwrap().is_none());
        check(r.bitstream_hdr(), 2);
        r.rewind();
    }
    for target in [2usize, 0, 1, 2] {
        assert_eq!(r.seek_to_sync(target as i64), 0);
        for i in 0..=target {
            let f = r.read_frame().unwrap().unwrap();
            assert_eq!(f.sample_index, i);
            let p = f.planes8.as_ref().unwrap();
            let raw: Vec<_> = p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect();
            assert_eq!(raw, pixels);
            check(r.bitstream_hdr(), i);
        }
    }
}
#[test]
fn webm_received_hdr_overrides_opening_metadata_on_every_frame() {
    let mut r = WebmVideoReader::open_software(Cursor::new(data("sequence.webm")), BUDGET).unwrap();
    let pixels = expected();
    for _ in 0..2 {
        check(r.bitstream_hdr(), 0);
        for i in 0..3 {
            let RawFrame::Planar8(p) = r.read_frame_raw().unwrap().unwrap() else {
                panic!("native AV1 planar output")
            };
            let raw: Vec<_> = p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect();
            assert_eq!(raw, pixels);
            check(r.bitstream_hdr(), i);
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        check(r.bitstream_hdr(), 2);
        r.rewind();
    }
    for target in [2usize, 0, 1, 2] {
        assert_eq!(r.seek_to_sync(target as i64 * 20_000_000).unwrap(), 0);
        for i in 0..=target {
            let RawFrame::Planar8(p) = r.read_frame_raw().unwrap().unwrap() else {
                panic!("native AV1 planar output")
            };
            let raw: Vec<_> = p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect();
            assert_eq!(raw, pixels);
            check(r.bitstream_hdr(), i);
        }
    }
}
