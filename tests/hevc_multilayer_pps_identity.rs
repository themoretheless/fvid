use fvid::{container::mp4::Limits, playback_mp4::Mp4VideoReader};
use std::io::Cursor;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}
fn open(bytes: Vec<u8>) -> fvid::Result<Mp4VideoReader<Cursor<Vec<u8>>>> {
    Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20)
}
fn parse_pps(bytes: Vec<u8>) -> fvid::Result<fvid::codec::hevc_pps::Pps> {
    let movie = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes), Limits::default())?;
    let config = fvid::codec::config::HevcConfig::parse(&movie.tracks()[0].configuration)?;
    let sps = fvid::codec::hevc_sps::Sps::parse(
        config
            .arrays
            .iter()
            .find(|a| a.nal_type == 33)
            .unwrap()
            .units[0],
        16 << 20,
    )?;
    fvid::codec::hevc_pps::Pps::parse(
        config
            .arrays
            .iter()
            .find(|a| a.nal_type == 34)
            .unwrap()
            .units[0],
        &sps,
        16 << 20,
    )
}
#[test]
fn identity_multilayer_pps_syntax_accepts() {
    for seed in ["main-ipb", "main10-ipb"] {
        for kind in ["identity", "identity-tail"] {
            parse_pps(fixture(&format!(
                "playback-errors/hevc-multilayer-pps-{seed}-{kind}-synthetic.mp4"
            )))
            .unwrap();
        }
    }
}
#[test]
fn identity_multilayer_pps_preserves_base_layer_pixels_timing_seek_and_rewind() {
    for seed in ["main-ipb", "main10-ipb"] {
        for kind in ["identity", "identity-tail"] {
            let name = format!("playback-errors/hevc-multilayer-pps-{seed}-{kind}-synthetic.mp4");
            let mut baseline = open(fixture(&format!("hevc/{seed}.mp4"))).unwrap();
            let mut reader = open(fixture(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!reader.hardware_accelerated());
            for target in [None, Some(3), Some(13), None] {
                if let Some(t) = target {
                    assert_eq!(reader.seek_to_sync(t), baseline.seek_to_sync(t));
                } else {
                    reader.rewind();
                    baseline.rewind();
                }
                let mut count = 0;
                loop {
                    let expected = baseline.read_frame().unwrap();
                    let actual = reader.read_frame().unwrap();
                    match (actual, expected) {
                        (Some(a), Some(b)) => {
                            assert_eq!(a.sample_index, b.sample_index, "{name}");
                            assert_eq!(a.presentation_time, b.presentation_time, "{name}");
                            assert_eq!(a.duration, b.duration, "{name}");
                            assert_eq!(a.picture.y, b.picture.y, "{name}");
                            assert_eq!(a.picture.cb, b.picture.cb, "{name}");
                            assert_eq!(a.picture.cr, b.picture.cr, "{name}");
                            count += 1;
                        }
                        (None, None) => break,
                        _ => panic!("{name}: frame count differs"),
                    }
                }
                assert!(count > 0);
                if target.is_none() {
                    assert_eq!(count, 17);
                }
            }
        }
    }
}
#[test]
fn poc_reset_flag_is_retained_for_slice_decoding() {
    for seed in ["main-ipb", "main10-ipb"] {
        let pps = parse_pps(fixture(&format!(
            "playback-errors/hevc-multilayer-pps-{seed}-poc-reset-synthetic.mp4"
        )))
        .unwrap();
        assert!(pps.poc_reset_info_present);
    }
}
#[test]
fn truncated_identity_syntax_is_invalid_not_an_unsupported_feature() {
    for seed in ["main-ipb", "main10-ipb"] {
        let error = parse_pps(fixture(&format!(
            "playback-errors/hevc-multilayer-pps-{seed}-truncated-synthetic.mp4"
        )))
        .unwrap_err();
        assert!(matches!(error, fvid::Error::Invalid(_)));
        assert_eq!(error.to_string(), "missing RBSP stop bit");
    }
}
#[test]
fn colour_mapping_requires_its_own_implementation() {
    for seed in ["main-ipb", "main10-ipb"] {
        let error = parse_pps(fixture(&format!(
            "playback-errors/hevc-multilayer-pps-{seed}-colour-mapping-synthetic.mp4"
        )))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "HEVC multilayer PPS colour mapping is not connected"
        );
    }
}
