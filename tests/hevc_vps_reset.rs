use fvid::{
    codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_vps::Vps},
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn strict_vps_extensions_preserve_reset_pixels_and_poc() {
    for alignment in ["aligned", "unaligned"] {
        for kind in [
            "baseline",
            "idr-baseline",
            "msb-reset",
            "msb-reset-signalled",
            "full-reset",
        ] {
            let name = format!("hevc-poc-vps-{alignment}-{kind}-synthetic.mp4");
            let mut actual =
                Mp4Reader::open(Cursor::new(fixture(&name)), Default::default()).unwrap();
            let baseline_name = format!(
                "hevc-poc-{}-synthetic.mp4",
                if matches!(kind, "full-reset" | "idr-baseline") {
                    "idr-baseline"
                } else {
                    "baseline"
                }
            );
            let mut base =
                Mp4Reader::open(Cursor::new(fixture(&baseline_name)), Default::default()).unwrap();
            let config = HevcConfig::parse(&actual.tracks()[0].configuration).unwrap();
            let vps = Vps::parse(
                config
                    .arrays
                    .iter()
                    .find(|a| a.nal_type == 32)
                    .unwrap()
                    .units[0],
                32 << 20,
            )
            .unwrap();
            assert!(vps.extension_present);
            assert_eq!(vps.poc_lsb_aligned().unwrap(), alignment == "aligned");
            let mut decoder =
                HevcDecoder::from_configuration(&actual.tracks()[0].configuration, 32 << 20)
                    .unwrap();
            let mut oracle =
                HevcDecoder::from_configuration(&base.tracks()[0].configuration, 32 << 20).unwrap();
            let mut packet = Vec::new();
            let mut expected = Vec::new();
            for index in 0..35 {
                actual.read_packet(0, index, &mut packet).unwrap();
                base.read_packet(0, index, &mut expected).unwrap();
                let frame = decoder
                    .decode_packet(&packet)
                    .unwrap_or_else(|e| panic!("{name} picture {index}: {e}"))
                    .unwrap();
                let gold = oracle.decode_packet(&expected).unwrap().unwrap();
                for plane in 0..3 {
                    assert_eq!(
                        frame.picture.planes[plane].samples(),
                        gold.picture.planes[plane].samples()
                    );
                }
                let delta = if !kind.starts_with("msb-reset") {
                    0
                } else if index >= 33 {
                    32
                } else if index >= 17 {
                    16
                } else {
                    0
                };
                assert_eq!(frame.poc, gold.poc - delta);
            }
        }
    }
}

#[test]
fn vps_alignment_update_requires_new_sequence_and_reset_restores_configuration() {
    for (kind, picture) in [
        ("bad-in-sequence", 16),
        ("at-idr", 17),
        ("bad-after-slices", 16),
        ("annex-a-after-slices", 16),
        ("annex-a-to-reset-at-idr", 17),
        ("reserved-after-slices", 16),
    ] {
        let mut movie = Mp4Reader::open(
            Cursor::new(fixture(&format!(
                "hevc-poc-vps-alignment-update-{kind}-synthetic.mp4"
            ))),
            Default::default(),
        )
        .unwrap();
        let mut baseline = Mp4Reader::open(
            Cursor::new(fixture(&format!(
                "hevc-poc-vps-aligned-{}-synthetic.mp4",
                if kind.ends_with("at-idr") {
                    "full-reset"
                } else if kind == "annex-a-after-slices" {
                    "baseline"
                } else {
                    "msb-reset"
                }
            ))),
            Default::default(),
        )
        .unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20).unwrap();
        let mut oracle =
            HevcDecoder::from_configuration(&baseline.tracks()[0].configuration, 32 << 20).unwrap();
        let mut packet = Vec::new();
        let mut reference = Vec::new();
        let mut first = Vec::new();
        for index in 0..35 {
            movie.read_packet(0, index, &mut packet).unwrap();
            baseline.read_packet(0, index, &mut reference).unwrap();
            if index == 0 {
                first.clone_from(&packet);
            }
            if index == picture && kind.starts_with("bad-") {
                let error = decoder
                    .decode_packet(&packet)
                    .err()
                    .expect("must reject invalid video");
                assert!(matches!(error, fvid::Error::Invalid(_)));
                assert_eq!(
                    error.to_string(),
                    if kind == "bad-after-slices" {
                        "HEVC changed VPS follows picture slices"
                    } else {
                        "HEVC active VPS changed within sequence"
                    }
                );
                assert!(decoder.decode_packet(&first).is_err());
                decoder.reset();
                assert_eq!(decoder.decode_packet(&first).unwrap().unwrap().poc, 0);
                break;
            }
            let a = decoder
                .decode_packet(&packet)
                .unwrap_or_else(|e| panic!("{kind} picture {index}: {e}"))
                .unwrap();
            let b = oracle.decode_packet(&reference).unwrap().unwrap();
            assert_eq!(a.poc, b.poc);
            for plane in 0..3 {
                assert_eq!(
                    a.picture.planes[plane].samples(),
                    b.picture.planes[plane].samples()
                );
            }
        }
    }
}

#[test]
fn strict_vps_reset_malformed_payloads_keep_exact_errors_and_recovery() {
    for alignment in ["aligned", "unaligned"] {
        for (kind, failure, message) in [
            (
                "bad-full-nonidr",
                17,
                "HEVC internal base-layer full POC reset requires IDR",
            ),
            (
                "bad-period-repeat",
                18,
                "consecutive HEVC initial resets reuse a period ID",
            ),
            (
                "bad-anchor",
                18,
                "HEVC signalled reset disagrees with period anchor",
            ),
            ("bad-truncated", 18, "truncated or oversized bit field"),
            ("bad-empty", 17, "truncated or oversized bit field"),
        ] {
            let mut movie = Mp4Reader::open(
                Cursor::new(fixture(&format!(
                    "hevc-poc-vps-{alignment}-{kind}-synthetic.mp4"
                ))),
                Default::default(),
            )
            .unwrap();
            let mut baseline = Mp4Reader::open(
                Cursor::new(fixture(&format!(
                    "hevc-poc-vps-{alignment}-msb-reset-synthetic.mp4"
                ))),
                Default::default(),
            )
            .unwrap();
            let mut decoder =
                HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20)
                    .unwrap();
            let mut oracle =
                HevcDecoder::from_configuration(&baseline.tracks()[0].configuration, 32 << 20)
                    .unwrap();
            let mut packet = Vec::new();
            let mut reference = Vec::new();
            let mut first = Vec::new();
            for index in 0..failure {
                movie.read_packet(0, index, &mut packet).unwrap();
                baseline.read_packet(0, index, &mut reference).unwrap();
                if index == 0 {
                    first.clone_from(&packet);
                }
                let a = decoder.decode_packet(&packet).unwrap().unwrap();
                let b = oracle.decode_packet(&reference).unwrap().unwrap();
                assert_eq!(a.poc, b.poc);
                for plane in 0..3 {
                    assert_eq!(
                        a.picture.planes[plane].samples(),
                        b.picture.planes[plane].samples()
                    );
                }
            }
            movie.read_packet(0, failure, &mut packet).unwrap();
            let error = decoder
                .decode_packet(&packet)
                .err()
                .expect("must reject invalid video");
            assert!(matches!(error, fvid::Error::Invalid(_)));
            assert_eq!(error.to_string(), message, "{alignment}/{kind}");
            assert!(decoder.decode_packet(&first).is_err());
            decoder.reset();
            assert_eq!(decoder.decode_packet(&first).unwrap().unwrap().poc, 0);
        }
    }
}

#[test]
fn malformed_vps_extension_is_rejected_by_annex_f_without_changing_annex_a() {
    for alignment in ["aligned", "unaligned"] {
        for (kind, message) in [
            (
                "bad-vps-alignment",
                "HEVC VPS extension alignment bit is not one",
            ),
            (
                "bad-vps-splitting",
                "HEVC VPS splitting has no scalability dimensions",
            ),
            (
                "bad-vps-dimensions",
                "zero HEVC VPS representation dimensions",
            ),
            (
                "bad-vps-no-format",
                "first HEVC VPS representation format lacks chroma/depth",
            ),
        ] {
            let mut movie = Mp4Reader::open(
                Cursor::new(fixture(&format!(
                    "hevc-poc-vps-{alignment}-{kind}-synthetic.mp4"
                ))),
                Default::default(),
            )
            .unwrap();
            let config = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
            let vps = Vps::parse(
                config
                    .arrays
                    .iter()
                    .find(|a| a.nal_type == 32)
                    .unwrap()
                    .units[0],
                32 << 20,
            )
            .unwrap();
            let error = vps.poc_lsb_aligned().unwrap_err();
            assert!(matches!(error, fvid::Error::Invalid(_)));
            assert_eq!(error.to_string(), message);
            let mut decoder =
                HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20)
                    .unwrap();
            let mut packet = Vec::new();
            movie.read_packet(0, 0, &mut packet).unwrap();
            let error = decoder
                .decode_packet(&packet)
                .err()
                .expect("strict Annex F must reject malformed VPS");
            assert!(matches!(error, fvid::Error::Invalid(_)));
            assert_eq!(error.to_string(), message);
            assert!(decoder.decode_packet(&packet).is_err());
            decoder.reset();
            assert_eq!(
                decoder.decode_packet(&packet).err().unwrap().to_string(),
                message
            );
        }
    }
}

#[test]
fn strict_vps_reset_software_seek_and_rewind_match_existing_reset_oracle() {
    use fvid::playback_mp4::Mp4VideoReader;
    let mut cases = Vec::new();
    for alignment in ["aligned", "unaligned"] {
        for kind in ["msb-reset", "msb-reset-signalled", "full-reset"] {
            cases.push((
                format!("hevc-poc-vps-{alignment}-{kind}-synthetic.mp4"),
                kind,
            ));
        }
    }
    cases.push((
        "hevc-poc-vps-alignment-update-at-idr-synthetic.mp4".to_owned(),
        "full-reset",
    ));
    cases.push((
        "hevc-poc-vps-alignment-update-annex-a-to-reset-at-idr-synthetic.mp4".to_owned(),
        "full-reset",
    ));
    for (name, kind) in cases {
        let mut actual = Mp4VideoReader::open_software(
            Cursor::new(fixture(&name)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        let mut baseline = Mp4VideoReader::open_software(
            Cursor::new(fixture(&format!("hevc-poc-{kind}-synthetic.mp4"))),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for target in [None, Some(18), Some(34), None] {
            if let Some(pts) = target {
                assert_eq!(actual.seek_to_sync(pts), baseline.seek_to_sync(pts));
            } else {
                actual.rewind();
                baseline.rewind();
            }
            let mut count = 0;
            loop {
                match (
                    actual
                        .read_frame()
                        .unwrap_or_else(|e| panic!("{name}, seek {target:?}, output {count}: {e}")),
                    baseline.read_frame().unwrap(),
                ) {
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
                    _ => panic!("{name}: unequal output frame counts"),
                }
            }
            assert_eq!(
                count,
                match (kind, target) {
                    ("full-reset", Some(18)) => 18,
                    ("full-reset", Some(34)) => 2,
                    _ => 35,
                }
            );
        }
    }
}
