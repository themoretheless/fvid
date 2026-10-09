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
