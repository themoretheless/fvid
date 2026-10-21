//! Explicit zero CLL must replace previously received light metadata, with no external test tools.
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
    std::fs::read(root().join("av1-hdr-sequence").join(format!("zero-{name}"))).unwrap()
}
fn expected() -> Vec<u8> {
    std::fs::read(root().join("av1-separate-tiles-w128-h128-groups2.yuv")).unwrap()
}
const LIGHT: [(f32, f32); 3] = [(1000., 400.), (0., 0.), (750., 250.)];
fn check(h: fvid::color::hdr::HdrMetadata, i: usize) {
    assert_eq!(
        (h.light.max_cll, h.light.max_fall),
        LIGHT[i],
        "displayed sequence {i}"
    );
}
#[test]
fn direct_decoder_receives_explicit_zero_sequence_metadata() {
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
fn mp4_received_hdr_zero_overrides_previous_metadata_on_every_frame() {
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
fn webm_received_hdr_zero_overrides_previous_metadata_on_every_frame() {
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

#[test]
fn zero_cll_presence_survives_reset_and_packet_scanning() {
    use fvid::{codec::av1_metadata, color::hdr::HdrMetadata, container::mp4::Mp4Reader};
    let fallback = HdrMetadata::from_clli(&[3, 232, 1, 144]).unwrap();
    let mut input = Mp4Reader::open(Cursor::new(data("sequence.mp4")), Limits::default()).unwrap();
    let mut configuration = input.tracks()[0].configuration.clone();
    // An explicit zero CLL in the recorded reset snapshot.
    configuration.extend_from_slice(&[0x2a, 6, 1, 0, 0, 0, 0, 0x80]);
    let mut d = Decoder::from_configuration(&configuration, BUDGET).unwrap();
    assert_eq!(d.hdr_with_fallback(fallback).light.max_cll, 0.);
    d.decode_container_packet(&data("0.packet")).unwrap();
    check(d.hdr(), 0);
    d.reset();
    assert_eq!(d.hdr_with_fallback(fallback).light.max_cll, 0.);
    // Padding is absence, not an instruction to change the received statement.
    assert!(d.decode_container_packet(&[0x7a, 1, 0]).unwrap().is_empty());
    assert_eq!(d.hdr_with_fallback(fallback).light.max_cll, 0.);
    let empty = Decoder::new(BUDGET);
    assert_eq!(empty.hdr_with_fallback(fallback), fallback);
    for (first, second, expected) in [(0, 1, 0.), (1, 0, 1000.)] {
        let mut packet = data(&format!("{first}.packet"));
        packet.extend_from_slice(&data(&format!("{second}.packet")));
        assert_eq!(
            av1_metadata::hdr_from_packet(&packet)
                .unwrap()
                .light
                .max_cll,
            expected
        );
        assert_eq!(
            av1_metadata::signal_from_bytes(&packet).1.light.max_cll,
            expected
        );
        assert_eq!(
            av1_metadata::signal_from_container_sample(&packet)
                .1
                .light
                .max_cll,
            expected
        );
    }
    // Read-only fixture control: no recorded CLL originally existed.
    let mut packet = Vec::new();
    input.read_packet(0, 1, &mut packet).unwrap();
    assert_eq!(packet, data("1.packet"));
}
