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
        assert_eq!(
            d.decode_packet(&packet).err().unwrap().to_string(),
            "HEVC decoder requires reset after error"
        );
        d.reset();
        r.read_packet(0, 0, &mut packet).unwrap();
        assert!(d.decode_packet(&packet).unwrap().is_some());
    }
}

#[test]
fn standalone_end_markers_restart_cra_without_emitting_a_picture() {
    // All source pictures and parameter sets are our authored short fixture.
    let bytes = include_bytes!("fixtures/playback-errors/hevc-eos-before-cra-valid-synthetic.mp4");
    let mut r = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    let config = &r.tracks()[0].configuration;
    let mut d = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
    let mut fresh = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
    let mut packet = vec![];
    // EOS and EOB both have a header and rbsp_trailing_bits only.
    for kind in [36u8, 37] {
        for _ in 0..2 {
            d.reset();
            fresh.reset();
            for i in 0..5 {
                r.read_packet(0, i, &mut packet).unwrap();
                if i == 4 {
                    // Move the fixture's seven-byte suffix into its own packet.
                    packet.truncate(packet.len() - 7);
                }
                assert!(d.decode_packet(&packet).unwrap().is_some());
            }
            let marker = [0, 0, 0, 3, kind << 1, 1, 128];
            assert!(d.decode_packet(&marker).unwrap().is_none());
            assert!(d.decode_packet(&marker).unwrap().is_none());
            let mut discarded = 0;
            for i in 5..r.tracks()[0].samples.len() {
                r.read_packet(0, i, &mut packet).unwrap();
                let a = d.decode_packet(&packet).unwrap();
                let b = fresh.decode_packet(&packet).unwrap();
                assert_eq!(a.is_some(), b.is_some(), "kind {kind}, sample {i}");
                if (6..=8).contains(&i) {
                    assert!(a.is_none());
                    discarded += 1;
                }
                if let (Some(a), Some(b)) = (a, b) {
                    assert_eq!(a.poc, b.poc);
                    for plane in 0..3 {
                        assert_eq!(
                            a.picture.planes[plane].samples(),
                            b.picture.planes[plane].samples()
                        );
                    }
                }
            }
            assert_eq!(discarded, 3);
        }
    }
}
