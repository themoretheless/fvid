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
            "raised-vps-level",
            include_bytes!(
                "fixtures/playback-errors/hevc-future-extension-vps-level-change-synthetic.mp4"
            )
            .as_slice(),
        ),
        (
            "paired-id",
            include_bytes!(
                "fixtures/playback-errors/hevc-future-extension-paired-id-synthetic.mp4"
            )
            .as_slice(),
        ),
        (
            "vps",
            include_bytes!("fixtures/playback-errors/hevc-future-extension-vps-synthetic.mp4")
                .as_slice(),
        ),
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

#[test]
fn malformed_vps_extension_is_rejected_at_player_open() {
    let bytes =
        include_bytes!("fixtures/playback-errors/hevc-future-extension-vps-bad-stop-synthetic.mp4");
    let error = Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20)
        .err()
        .expect("VPS RBSP without a stop bit must fail");
    assert_eq!(error.to_string(), "truncated or oversized bit field");
}

#[test]
fn inband_vps_tail_validation_poison_and_reset() {
    use fvid::{
        codec::{config::HevcConfig, hevc_decoder::HevcDecoder},
        container::mp4::Mp4Reader,
    };
    let seed = include_bytes!("fixtures/hevc/main-ipb.mp4");
    let mut r = Mp4Reader::open(Cursor::new(seed), Limits::default()).unwrap();
    let configuration = &r.tracks()[0].configuration;
    let mut d = HevcDecoder::from_configuration(configuration, 16 << 20).unwrap();
    for (file, invalid) in [
        (
            include_bytes!("fixtures/playback-errors/hevc-future-extension-vps-synthetic.mp4")
                .as_slice(),
            false,
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/hevc-future-extension-vps-bad-stop-synthetic.mp4"
            )
            .as_slice(),
            true,
        ),
    ] {
        let fixture = Mp4Reader::open(Cursor::new(file), Limits::default()).unwrap();
        let config = HevcConfig::parse(&fixture.tracks()[0].configuration).unwrap();
        let nal = &config
            .arrays
            .iter()
            .find(|a| a.nal_type == 32)
            .unwrap()
            .units[0];
        let mut packet = (nal.len() as u32).to_be_bytes().to_vec();
        packet.extend_from_slice(nal);
        if invalid {
            assert_eq!(
                d.decode_packet(&packet).err().unwrap().to_string(),
                "truncated or oversized bit field"
            );
            assert_eq!(
                d.decode_packet(&packet).err().unwrap().to_string(),
                "HEVC decoder requires reset after error"
            );
        } else {
            assert!(d.decode_packet(&packet).unwrap().is_none());
        }
        d.reset();
        r.read_packet(0, 0, &mut packet).unwrap();
        assert!(d.decode_packet(&packet).unwrap().is_some());
    }
}

#[test]
fn activated_sps_requires_its_named_vps() {
    let bytes =
        include_bytes!("fixtures/playback-errors/hevc-future-extension-vps-wrong-id-synthetic.mp4");
    let mut reader =
        Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20).unwrap();
    assert_eq!(
        reader
            .read_frame()
            .err()
            .expect("SPS names missing VPS")
            .to_string(),
        "HEVC SPS references unavailable VPS"
    );
}

#[test]
fn inband_named_vps_becomes_available_and_reset_restores_initial_sets() {
    use fvid::{
        codec::{config::HevcConfig, hevc_decoder::HevcDecoder},
        container::mp4::Mp4Reader,
    };
    let wrong =
        include_bytes!("fixtures/playback-errors/hevc-future-extension-vps-wrong-id-synthetic.mp4");
    let mut r = Mp4Reader::open(Cursor::new(wrong), Limits::default()).unwrap();
    let mut d = HevcDecoder::from_configuration(&r.tracks()[0].configuration, 16 << 20).unwrap();
    let seed = Mp4Reader::open(
        Cursor::new(include_bytes!("fixtures/hevc/main-ipb.mp4")),
        Limits::default(),
    )
    .unwrap();
    let config = HevcConfig::parse(&seed.tracks()[0].configuration).unwrap();
    let nal = &config
        .arrays
        .iter()
        .find(|a| a.nal_type == 32)
        .unwrap()
        .units[0];
    let mut prefix = (nal.len() as u32).to_be_bytes().to_vec();
    prefix.extend_from_slice(nal);
    assert!(d.decode_packet(&prefix).unwrap().is_none());
    let mut packet = vec![];
    r.read_packet(0, 0, &mut packet).unwrap();
    let actual = d.decode_packet(&packet).unwrap().unwrap();
    let mut baseline =
        HevcDecoder::from_configuration(&seed.tracks()[0].configuration, 16 << 20).unwrap();
    let expected = baseline.decode_packet(&packet).unwrap().unwrap();
    for plane in 0..3 {
        assert_eq!(
            actual.picture.planes[plane].samples(),
            expected.picture.planes[plane].samples()
        );
    }
    d.reset();
    assert_eq!(
        d.decode_packet(&packet).err().unwrap().to_string(),
        "HEVC SPS references unavailable VPS"
    );
    d.reset();
    assert!(d.decode_packet(&prefix).unwrap().is_none());
    assert!(d.decode_packet(&packet).unwrap().is_some());
}

#[test]
fn active_vps_cannot_change_between_dependent_pictures() {
    let file = include_bytes!("fixtures/playback-errors/hevc-vps-change-inter-synthetic.mp4");
    let mut reader =
        Mp4VideoReader::open_software(Cursor::new(file), Limits::default(), 16 << 20).unwrap();
    assert_eq!(
        reader
            .read_frame()
            .err()
            .expect("active VPS changed mid-sequence")
            .to_string(),
        "HEVC active VPS changed within sequence"
    );
}

#[test]
fn changed_vps_is_accepted_at_idr_and_post_eos_cra() {
    use fvid::{
        codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_vps::Vps},
        container::mp4::Mp4Reader,
    };
    let changed = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-future-extension-vps-level-change-synthetic.mp4"
        )),
        Limits::default(),
    )
    .unwrap();
    let config = HevcConfig::parse(&changed.tracks()[0].configuration).unwrap();
    let nal = &config
        .arrays
        .iter()
        .find(|a| a.nal_type == 32)
        .unwrap()
        .units[0];
    assert_eq!(Vps::parse(nal, 16 << 20).unwrap().profile.level, 60);
    let mut prefix = (nal.len() as u32).to_be_bytes().to_vec();
    prefix.extend_from_slice(nal);
    for (bytes, restart) in [
        (include_bytes!("fixtures/hevc/main-ipb.mp4").as_slice(), 0),
        (
            include_bytes!("fixtures/playback-errors/hevc-eos-before-cra-valid-synthetic.mp4")
                .as_slice(),
            5,
        ),
        (
            include_bytes!("fixtures/playback-errors/hevc-bla-w-lp-synthetic.mp4").as_slice(),
            5,
        ),
    ] {
        let mut r = Mp4Reader::open(Cursor::new(bytes), Limits::default()).unwrap();
        let config = &r.tracks()[0].configuration;
        let mut actual = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
        let mut expected = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
        let mut packet = vec![];
        for _ in 0..2 {
            actual.reset();
            expected.reset();
            // Establish an active old VPS, including the EOS suffix for CRA.
            for i in 0..restart.max(1) {
                r.read_packet(0, i, &mut packet).unwrap();
                assert!(actual.decode_packet(&packet).unwrap().is_some());
            }
            assert!(actual.decode_packet(&prefix).unwrap().is_none());
            let mut count = 0;
            for i in restart..r.tracks()[0].samples.len() {
                r.read_packet(0, i, &mut packet).unwrap();
                let a = actual.decode_packet(&packet).unwrap();
                let b = expected.decode_packet(&packet).unwrap();
                assert_eq!(a.is_some(), b.is_some(), "restart {restart}, sample {i}");
                if let (Some(a), Some(b)) = (a, b) {
                    assert_eq!(a.poc, b.poc);
                    for plane in 0..3 {
                        assert_eq!(
                            a.picture.planes[plane].samples(),
                            b.picture.planes[plane].samples()
                        );
                    }
                    count += 1;
                }
            }
            assert_eq!(count, if restart == 0 { 17 } else { 9 });
        }
    }
}
