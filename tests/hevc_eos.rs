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

#[test]
fn eos_playback_preserves_pixels_timestamps_and_rewind() {
    use fvid::{container::mp4::Limits, playback_mp4::Mp4VideoReader};
    let original = include_bytes!("fixtures/hevc/weighted-tmvp.mp4");
    let mut baseline =
        Mp4VideoReader::open_software(Cursor::new(original), Limits::default(), 16 << 20).unwrap();
    let mut expected = Vec::new();
    while let Some(frame) = baseline.read_frame().unwrap() {
        if !(6..=8).contains(&frame.sample_index) {
            expected.push(frame);
        }
    }
    assert_eq!(expected.len(), 14);
    for ended in [
        include_bytes!("fixtures/playback-errors/hevc-eos-before-cra-valid-synthetic.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/hevc-bla-w-lp-synthetic.mp4").as_slice(),
    ] {
        let mut source =
            Mp4VideoReader::open_software(Cursor::new(ended), Limits::default(), 16 << 20).unwrap();
        for pass in 0..2 {
            let mut previous_end = None;
            for reference in &expected {
                let frame = source.read_frame().unwrap().unwrap();
                assert_eq!(frame.sample_index, reference.sample_index, "pass {pass}");
                assert_eq!(frame.presentation_time, reference.presentation_time);
                assert_eq!(frame.picture.y, reference.picture.y);
                assert_eq!(frame.picture.cb, reference.picture.cb);
                assert_eq!(frame.picture.cr, reference.picture.cr);
                if let Some(end) = previous_end {
                    assert_eq!(
                        end, frame.presentation_time.ticks,
                        "sample {}",
                        frame.sample_index
                    );
                }
                previous_end = Some(frame.presentation_time.ticks + frame.duration.ticks);
            }
            assert!(source.read_frame().unwrap().is_none());
            source.rewind();
        }
    }
}

#[test]
fn eos_native_seek_matches_the_output_timeline() {
    use fvid::playback_native::NativeReader;
    use std::time::Duration;
    for file in [
        include_bytes!("fixtures/playback-errors/hevc-eos-before-cra-valid-synthetic.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/hevc-bla-w-lp-synthetic.mp4").as_slice(),
    ] {
        let mut reader = NativeReader::software(Cursor::new(file), 32 << 20).unwrap();
        let mut frames = Vec::new();
        while reader.read_frame().unwrap() {
            frames.push((reader.frame_interval().unwrap(), reader.rgb().to_vec()));
        }
        assert_eq!(frames.len(), 14);
        // Probe both sides of frame boundaries and the interval extended across
        // suppressed RASL samples, alternating forward and backward seeks.
        for millis in [
            0, 566, 100, 533, 166, 500, 199, 433, 200, 400, 233, 399, 266, 350, 267, 334, 299, 333,
            300, 332,
        ] {
            let expected = frames
                .iter()
                .find(|((start, end, scale), _)| {
                    *start * 1000 <= u128::from(millis) * u128::from(*scale)
                        && *end * 1000 > u128::from(millis) * u128::from(*scale)
                })
                .unwrap();
            reader.seek(Duration::from_millis(millis)).unwrap();
            assert_eq!(
                reader.frame_interval().unwrap(),
                expected.0,
                "seek {millis}"
            );
            assert_eq!(reader.rgb(), expected.1, "seek {millis}");
        }
        reader.rewind().unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.rgb(), frames[0].1);
    }
}
