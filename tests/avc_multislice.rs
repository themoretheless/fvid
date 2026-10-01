use fvid::{
    codec::{
        avc::{Pps, Sps},
        avc_decoder::AvcDecoder,
        avc_slice::{SliceHeader, SliceType},
        config::{AvcConfig, NalUnits},
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
const VIDEO: &[u8] = include_bytes!("fixtures/playback-errors/avc-multislice-ipb.mp4");
#[test]
fn actual_two_slice_ipb_stream_reproduces_current_decoder_refusal() {
    let mut reader = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let config = reader.tracks()[0].configuration.clone();
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
    let mut kinds = [false; 3];
    let mut packet = Vec::new();
    for index in 0..reader.tracks()[0].samples.len() {
        reader.read_packet(0, index, &mut packet).unwrap();
        let slices: Vec<_> = NalUnits::new(&packet, avc.length_size)
            .unwrap()
            .map(|n| n.unwrap())
            .filter(|n| matches!(n[0] & 31, 1 | 5))
            .map(|n| SliceHeader::parse(n, &sps, &pps).unwrap())
            .collect();
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[0].first_mb, 0);
        assert!(slices[1].first_mb > 0);
        assert_eq!(slices[0].frame_num, slices[1].frame_num);
        kinds[match slices[0].slice_type {
            SliceType::I => 0,
            SliceType::P => 1,
            SliceType::B => 2,
            _ => panic!("unexpected type"),
        }] = true;
        let error = decoder.decode(&packet).unwrap_err().to_string();
        assert!(
            error.contains("multiple AVC slices per access unit"),
            "{error}"
        );
        decoder.reset();
    }
    assert_eq!(kinds, [true; 3]);
}
#[test]
#[ignore = "acceptance gate: native multi-slice AVC reconstruction is not implemented"]
fn two_slice_ipb_matches_saved_yuv_and_rewind() {
    let mut reader =
        fvid::playback_mp4::Mp4VideoReader::open(Cursor::new(VIDEO), Default::default(), 16 << 20)
            .unwrap();
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut count = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            frame.picture.write_planar(&mut actual).unwrap();
            count += 1;
        }
        assert_eq!(count, 8);
        assert!(
            actual == include_bytes!("fixtures/playback-errors/avc-multislice-ipb.yuv"),
            "multi-slice AVC differs from independent decoder"
        );
        reader.rewind();
    }
}
