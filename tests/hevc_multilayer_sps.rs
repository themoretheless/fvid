use fvid::{codec::hevc_sps::Sps, container::mp4::Limits, playback_mp4::Mp4VideoReader};
use std::io::Cursor;
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn name(seed: &str, kind: &str) -> String {
    format!("playback-errors/hevc-multilayer-sps-{seed}-{kind}-synthetic.mp4")
}
fn parse(bytes: Vec<u8>) -> fvid::Result<Sps> {
    let movie = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes), Limits::default())?;
    let config = fvid::codec::config::HevcConfig::parse(&movie.tracks()[0].configuration)?;
    Sps::parse(
        config
            .arrays
            .iter()
            .find(|a| a.nal_type == 33)
            .unwrap()
            .units[0],
        16 << 20,
    )
}
#[test]
fn multilayer_sps_constraint_and_extension_order_accept() {
    for seed in ["main-ipb", "main10-ipb"] {
        for kind in ["unconstrained", "constrained", "range", "range-future"] {
            let sps = parse(data(&name(seed, kind))).unwrap();
            assert_eq!(sps.inter_view_mv_vert_constraint, kind != "unconstrained");
            let baseline = parse(data(&format!("hevc/{seed}.mp4"))).unwrap();
            let mut normalized = sps;
            normalized.inter_view_mv_vert_constraint = false;
            assert_eq!(normalized, baseline, "{seed}/{kind}");
        }
    }
}
#[test]
fn multilayer_sps_base_layer_pixels_timing_seek_and_rewind_accept() {
    for seed in ["main-ipb", "main10-ipb"] {
        for kind in ["unconstrained", "constrained", "range", "range-future"] {
            let open = |bytes| {
                Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20)
                    .unwrap()
            };
            let mut baseline = open(data(&format!("hevc/{seed}.mp4")));
            let mut reader = open(data(&name(seed, kind)));
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
                    match (reader.read_frame().unwrap(), baseline.read_frame().unwrap()) {
                        (Some(a), Some(b)) => {
                            assert_eq!(a.sample_index, b.sample_index);
                            assert_eq!(a.presentation_time, b.presentation_time);
                            assert_eq!(a.duration, b.duration);
                            assert_eq!(a.picture.y, b.picture.y);
                            assert_eq!(a.picture.cb, b.picture.cb);
                            assert_eq!(a.picture.cr, b.picture.cr);
                            count += 1;
                        }
                        (None, None) => break,
                        _ => panic!("{seed}/{kind}: frame count differs"),
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
fn multilayer_sps_malformed_rbsp_is_invalid_and_three_d_remains_refusal() {
    for seed in ["main-ipb", "main10-ipb"] {
        for kind in ["missing-constraint", "missing-stop"] {
            let bytes = data(&name(seed, kind));
            let e = parse(bytes.clone()).unwrap_err();
            assert!(matches!(e, fvid::Error::Invalid(_)), "{seed}/{kind}: {e}");
            assert!(e.to_string().contains("RBSP"), "{seed}/{kind}: {e}");
            let result =
                Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20);
            assert!(matches!(result, Err(fvid::Error::Invalid(_))));
        }
        let e = parse(data(&name(seed, "three-d"))).unwrap_err();
        assert!(matches!(e, fvid::Error::Unsupported(_)));
        assert_eq!(e.to_string(), "HEVC 3D SPS extension is not implemented");
    }
}
