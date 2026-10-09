use fvid::{codec::hevc_decoder::HevcDecoder, container::mp4::Mp4Reader};
use std::io::Cursor;

#[test]
fn eos_suffix_preserves_prior_picture_and_restarts_cra_rasl_state() {
    let bytes = include_bytes!("fixtures/playback-errors/hevc-eos-before-cra-valid-synthetic.mp4");
    let mut r = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    let config = &r.tracks()[0].configuration;
    let mut decoder = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
    let mut fresh = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
    let mut packet = vec![];
    let mut leading = 0;
    for i in 0..r.tracks()[0].samples.len() {
        r.read_packet(0, i, &mut packet).unwrap();
        let actual = decoder.decode_packet(&packet).unwrap();
        if i < 5 {
            assert!(actual.is_some());
            continue;
        }
        let expected = fresh.decode_packet(&packet).unwrap();
        if (6..=8).contains(&i) {
            assert!(expected.is_none());
            leading += 1;
        }
        assert_eq!(actual.is_some(), expected.is_some(), "sample {i}");
        if let (Some(a), Some(b)) = (actual, expected) {
            assert_eq!(a.poc, b.poc);
            for p in 0..3 {
                assert_eq!(a.picture.planes[p].samples(), b.picture.planes[p].samples());
            }
        }
    }
    assert_eq!(leading, 3);
}

#[test]
fn eos_rejects_invalid_trailing_bits_and_temporal_layer() {
    for (bytes, error) in [
        (
            include_bytes!(
                "fixtures/playback-errors/hevc-eos-before-cra-bad-trailing-synthetic.mp4"
            )
            .as_slice(),
            "invalid HEVC EOS/EOB trailing bits",
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/hevc-eos-before-cra-bad-temporal-synthetic.mp4"
            )
            .as_slice(),
            "HEVC EOS/EOB requires temporal layer zero",
        ),
    ] {
        let mut r = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        let mut d =
            HevcDecoder::from_configuration(&r.tracks()[0].configuration, 16 << 20).unwrap();
        let mut packet = vec![];
        for i in 0..4 {
            r.read_packet(0, i, &mut packet).unwrap();
            d.decode_packet(&packet).unwrap();
        }
        r.read_packet(0, 4, &mut packet).unwrap();
        assert_eq!(d.decode_packet(&packet).err().unwrap().to_string(), error);
    }
}
