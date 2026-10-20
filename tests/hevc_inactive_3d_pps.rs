#![cfg(feature = "player")]
use fvid::{container::mp4::Limits, playback_mp4::Mp4VideoReader};
use std::{io::Cursor, path::Path};
#[test]
fn inactive_pps_3d_extension_preserves_frames_and_rewind() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let seed = std::fs::read(root.join("hevc/main-ipb.mp4")).unwrap();
    let mut baseline =
        Mp4VideoReader::open_software(Cursor::new(seed), Limits::default(), 16 << 20).unwrap();
    let mut expected = Vec::new();
    while let Some(frame) = baseline.read_frame().unwrap() {
        expected.push(frame);
    }
    assert_eq!(expected.len(), 17);
    for label in [
        "inactive",
        "inactive-future",
        "inactive-inband",
        "inactive-future-inband",
    ] {
        let bytes =
            std::fs::read(root.join(format!("playback-errors/hevc-3d-pps-{label}-synthetic.mp4")))
                .unwrap();
        let mp4 =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&bytes), Limits::default()).unwrap();
        let config =
            fvid::codec::config::HevcConfig::parse(&mp4.tracks()[0].configuration).unwrap();
        let sps_nal = &config
            .arrays
            .iter()
            .find(|a| a.nal_type == 33)
            .unwrap()
            .units[0];
        let pps_nal = &config
            .arrays
            .iter()
            .find(|a| a.nal_type == 34)
            .unwrap()
            .units[0];
        let sps = fvid::codec::hevc_sps::Sps::parse(sps_nal, 16 << 20).unwrap();
        fvid::codec::hevc_pps::Pps::parse(pps_nal, &sps, 16 << 20).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20).unwrap();
        for _ in 0..2 {
            for target in &expected {
                let frame = reader.read_frame().unwrap().unwrap();
                assert_eq!(frame.presentation_time, target.presentation_time);
                assert_eq!(frame.duration, target.duration);
                assert_eq!(frame.picture.y, target.picture.y);
                assert_eq!(frame.picture.cb, target.picture.cb);
                assert_eq!(frame.picture.cr, target.picture.cr);
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}

#[test]
fn active_or_truncated_pps_3d_extension_has_a_specific_refusal() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for (label, message) in [
        (
            "active",
            "HEVC active depth lookup tables are not implemented",
        ),
        ("missing-flag", "truncated HEVC PPS 3D extension"),
    ] {
        let bytes = std::fs::read(root.join(format!("hevc-3d-pps-{label}-synthetic.mp4"))).unwrap();
        let mp4 =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&bytes), Limits::default()).unwrap();
        let config =
            fvid::codec::config::HevcConfig::parse(&mp4.tracks()[0].configuration).unwrap();
        let sps_nal = &config
            .arrays
            .iter()
            .find(|a| a.nal_type == 33)
            .unwrap()
            .units[0];
        let pps_nal = &config
            .arrays
            .iter()
            .find(|a| a.nal_type == 34)
            .unwrap()
            .units[0];
        let sps = fvid::codec::hevc_sps::Sps::parse(sps_nal, 16 << 20).unwrap();
        assert_eq!(
            fvid::codec::hevc_pps::Pps::parse(pps_nal, &sps, 16 << 20)
                .unwrap_err()
                .to_string(),
            message
        );
        let error = Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20)
            .err()
            .expect("invalid or unsupported PPS must refuse");
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn in_band_pps_refusals_preserve_the_cause_after_rewind() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for (label, message) in [
        (
            "active",
            "HEVC active depth lookup tables are not implemented",
        ),
        ("missing-flag", "truncated HEVC PPS 3D extension"),
    ] {
        let bytes =
            std::fs::read(root.join(format!("hevc-3d-pps-{label}-inband-synthetic.mp4"))).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20).unwrap();
        for _ in 0..2 {
            let mut refused = false;
            for _ in 0..18 {
                match reader.read_frame() {
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("{label}: invalid PPS was accepted"),
                    Err(error) => {
                        assert_eq!(error.to_string(), message);
                        refused = true;
                        break;
                    }
                }
            }
            assert!(refused);
            reader.rewind();
        }
    }
}
