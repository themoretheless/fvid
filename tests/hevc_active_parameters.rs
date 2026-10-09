use fvid::{container::mp4::Limits, playback_mp4::Mp4VideoReader};
use std::io::Cursor;

#[test]
fn active_parameter_signal_must_match_picture_parameter_binding() {
    for file in [
        include_bytes!(
            "fixtures/playback-errors/hevc-active-parameters-extension-tail-wrong-vps-synthetic.mp4"
        )
        .as_slice(),
        include_bytes!(
            "fixtures/playback-errors/hevc-active-parameters-config-wrong-vps-synthetic.mp4"
        )
        .as_slice(),
        include_bytes!("fixtures/playback-errors/hevc-active-parameters-wrong-vps-synthetic.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/hevc-active-parameters-wrong-sps-synthetic.mp4")
            .as_slice(),
    ] {
        let mut r =
            Mp4VideoReader::open_software(Cursor::new(file), Limits::default(), 16 << 20).unwrap();
        assert_eq!(
            r.read_frame()
                .err()
                .expect("SEI activation disagrees with picture")
                .to_string(),
            "HEVC active parameter SEI disagrees with picture binding"
        );
    }
}

#[test]
fn active_parameter_guidance_preserves_pixels_and_resets() {
    use fvid::{codec::hevc_decoder::HevcDecoder, container::mp4::Mp4Reader};
    let source = include_bytes!("fixtures/hevc/main-ipb.mp4");
    for (file, ids) in [
        (
            include_bytes!(
                "fixtures/playback-errors/hevc-active-parameters-extension-tail-synthetic.mp4"
            )
            .as_slice(),
            Some(vec![0]),
        ),
        (
            include_bytes!("fixtures/playback-errors/hevc-active-parameters-config-synthetic.mp4")
                .as_slice(),
            Some(vec![0]),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/hevc-active-parameters-repeated-synthetic.mp4"
            )
            .as_slice(),
            Some(vec![0]),
        ),
        (
            include_bytes!("fixtures/playback-errors/hevc-active-parameters-valid-synthetic.mp4")
                .as_slice(),
            Some(vec![0]),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/hevc-active-parameters-extra-ids-synthetic.mp4"
            )
            .as_slice(),
            Some(vec![0, 7, 15]),
        ),
        (
            include_bytes!("fixtures/playback-errors/hevc-active-parameters-mixed-synthetic.mp4")
                .as_slice(),
            None,
        ),
        // Malformed guidance stays opaque; it is not an activation acceptance.
        (
            include_bytes!("fixtures/playback-errors/hevc-active-parameters-empty-synthetic.mp4")
                .as_slice(),
            None,
        ),
    ] {
        let mut r = Mp4Reader::open(Cursor::new(file), Limits::default()).unwrap();
        let mut original = Mp4Reader::open(Cursor::new(source), Limits::default()).unwrap();
        let mut actual =
            HevcDecoder::from_configuration(&r.tracks()[0].configuration, 16 << 20).unwrap();
        let mut expected =
            HevcDecoder::from_configuration(&original.tracks()[0].configuration, 16 << 20).unwrap();
        for _ in 0..2 {
            let mut a_packet = vec![];
            let mut b_packet = vec![];
            for i in 0..17 {
                r.read_packet(0, i, &mut a_packet).unwrap();
                original.read_packet(0, i, &mut b_packet).unwrap();
                let a = actual.decode_packet(&a_packet).unwrap().unwrap();
                let b = expected.decode_packet(&b_packet).unwrap().unwrap();
                assert_eq!(a.poc, b.poc);
                if i == 0 {
                    let signal = actual.active_parameter_sets();
                    assert_eq!(signal.map(|s| &s.sps_ids), ids.as_ref());
                    if let Some(signal) = signal {
                        assert_eq!(signal.vps_id, 0);
                        assert!(!signal.self_contained_cvs && signal.no_parameter_set_update);
                    }
                }
                for p in 0..3 {
                    assert_eq!(a.picture.planes[p].samples(), b.picture.planes[p].samples());
                }
            }
            // This seed contains later IDR boundaries without a new declaration.
            assert!(actual.active_parameter_sets().is_none());
            actual.reset();
            expected.reset();
            assert!(actual.active_parameter_sets().is_none());
        }
        let mut playback =
            Mp4VideoReader::open_software(Cursor::new(file), Limits::default(), 16 << 20).unwrap();
        let mut baseline =
            Mp4VideoReader::open_software(Cursor::new(source), Limits::default(), 16 << 20)
                .unwrap();
        for _ in 0..17 {
            let a = playback.read_frame().unwrap().unwrap();
            let b = baseline.read_frame().unwrap().unwrap();
            assert_eq!(a.presentation_time, b.presentation_time);
            assert_eq!(a.duration, b.duration);
            assert_eq!(a.picture.y, b.picture.y);
            assert_eq!(a.picture.cb, b.picture.cb);
            assert_eq!(a.picture.cr, b.picture.cr);
        }
        assert!(playback.read_frame().unwrap().is_none());
    }
}

#[test]
fn parameter_only_sei_is_applied_to_the_following_picture() {
    use fvid::{
        codec::{
            config::{HevcConfig, NalUnits},
            hevc_decoder::HevcDecoder,
            hevc_sei,
        },
        container::mp4::Mp4Reader,
    };
    let file =
        include_bytes!("fixtures/playback-errors/hevc-active-parameters-valid-synthetic.mp4");
    let mut r = Mp4Reader::open(Cursor::new(file), Limits::default()).unwrap();
    let config = r.tracks()[0].configuration.clone();
    let mut d = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
    let mut packet = vec![];
    r.read_packet(0, 0, &mut packet).unwrap();
    let length = HevcConfig::parse(&config).unwrap().length_size;
    let nal = NalUnits::new(&packet, length)
        .unwrap()
        .map(Result::unwrap)
        .find(|n| {
            hevc_sei::active_parameters_from_nal(n, 16 << 20)
                .ok()
                .flatten()
                .is_some()
        })
        .unwrap();
    let mut prefix = (nal.len() as u32).to_be_bytes().to_vec();
    prefix.extend_from_slice(nal);
    assert!(d.decode_packet(&prefix).unwrap().is_none());
    assert!(d.active_parameter_sets().is_none());
    let mut original = Mp4Reader::open(
        Cursor::new(include_bytes!("fixtures/hevc/main-ipb.mp4")),
        Limits::default(),
    )
    .unwrap();
    original.read_packet(0, 0, &mut packet).unwrap();
    assert!(d.decode_packet(&packet).unwrap().is_some());
    assert_eq!(d.active_parameter_sets().unwrap().sps_ids, [0]);
    d.reset();
    assert!(d.decode_packet(&packet).unwrap().is_some());
    assert!(d.active_parameter_sets().is_none());
}

#[test]
fn reset_restores_configured_declaration_after_inband_override() {
    use fvid::{
        codec::{
            config::{HevcConfig, NalUnits},
            hevc_decoder::HevcDecoder,
            hevc_sei,
        },
        container::mp4::Mp4Reader,
    };
    let file = include_bytes!(
        "fixtures/playback-errors/hevc-active-parameters-config-wrong-vps-synthetic.mp4"
    );
    let mut r = Mp4Reader::open(Cursor::new(file), Limits::default()).unwrap();
    let mut d = HevcDecoder::from_configuration(&r.tracks()[0].configuration, 16 << 20).unwrap();
    let mut packet = vec![];
    r.read_packet(0, 0, &mut packet).unwrap();
    assert_eq!(
        d.decode_packet(&packet).err().unwrap().to_string(),
        "HEVC active parameter SEI disagrees with picture binding"
    );
    d.reset();
    let mut valid = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-active-parameters-valid-synthetic.mp4"
        )),
        Limits::default(),
    )
    .unwrap();
    let length = HevcConfig::parse(&valid.tracks()[0].configuration)
        .unwrap()
        .length_size;
    let mut valid_packet = vec![];
    valid.read_packet(0, 0, &mut valid_packet).unwrap();
    let nal = NalUnits::new(&valid_packet, length)
        .unwrap()
        .map(Result::unwrap)
        .find(|n| {
            hevc_sei::active_parameters_from_nal(n, 16 << 20)
                .ok()
                .flatten()
                .is_some()
        })
        .unwrap();
    let mut prefix = (nal.len() as u32).to_be_bytes().to_vec();
    prefix.extend_from_slice(nal);
    assert!(d.decode_packet(&prefix).unwrap().is_none());
    assert!(d.decode_packet(&packet).unwrap().is_some());
    assert_eq!(d.active_parameter_sets().unwrap().vps_id, 0);
    d.reset();
    assert!(d.active_parameter_sets().is_none());
    assert_eq!(
        d.decode_packet(&packet).err().unwrap().to_string(),
        "HEVC active parameter SEI disagrees with picture binding"
    );
}
