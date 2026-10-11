use fvid::{
    codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_pps::Pps, hevc_sps::Sps},
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

fn decode(name: &str) -> Vec<(i32, Vec<u16>)> {
    let mut movie = Mp4Reader::open(Cursor::new(fixture(name)), Default::default()).unwrap();
    let mut decoder =
        HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20).unwrap();
    let mut frames = Vec::new();
    let mut packet = Vec::new();
    for index in 0..movie.tracks()[0].samples.len() {
        movie.read_packet(0, index, &mut packet).unwrap();
        let frame = decoder
            .decode_packet(&packet)
            .unwrap_or_else(|e| panic!("{name} packet {index}: {e}"))
            .unwrap();
        frames.push((frame.poc, frame.picture.planes[0].samples().to_vec()));
    }
    frames
}

#[test]
fn owned_wrap_baseline_decodes_all_references() {
    let baseline = decode("hevc-poc-baseline-synthetic.mp4");
    assert_eq!(baseline.len(), 35);
    assert_eq!(
        baseline.iter().map(|f| f.0).collect::<Vec<_>>(),
        (0..35).collect::<Vec<_>>()
    );
    let original = decode("hevc-pcm-mono-reference-rext8.mp4");
    assert_eq!(&baseline[..3], original.as_slice());
    let gold = fixture("hevc-pcm-mono-reference-rext8.yuv");
    assert_eq!(gold.len(), 3 * 64 * 64);
    for (frame, bytes) in baseline.iter().take(3).zip(gold.chunks_exact(64 * 64)) {
        assert_eq!(
            frame.1,
            bytes.iter().copied().map(u16::from).collect::<Vec<_>>()
        );
    }
}

#[test]
fn reset_fixture_pps_flag_is_admitted() {
    for kind in ["msb-reset", "msb-reset-signalled"] {
        let bytes = fixture(&format!("hevc-poc-{kind}-synthetic.mp4"));
        let movie = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        let config = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
        let sps = Sps::parse(
            config
                .arrays
                .iter()
                .find(|a| a.nal_type == 33)
                .unwrap()
                .units[0],
            32 << 20,
        )
        .unwrap();
        let pps = Pps::parse(
            config
                .arrays
                .iter()
                .find(|a| a.nal_type == 34)
                .unwrap()
                .units[0],
            &sps,
            32 << 20,
        )
        .unwrap();
        assert!(pps.poc_reset_info_present && pps.slice_header_extension);
    }
}

#[test]
fn reset_video_preserves_pixels_and_translates_poc_once() {
    for prefix in ["", "multislice-", "dependent-"] {
        let baseline = decode(&format!("hevc-poc-{prefix}baseline-synthetic.mp4"));
        for kind in ["msb-reset", "msb-reset-signalled"] {
            let actual = decode(&format!("hevc-poc-{prefix}{kind}-synthetic.mp4"));
            assert_eq!(actual.len(), baseline.len());
            for (index, ((poc, pixels), (_, expected))) in actual.iter().zip(&baseline).enumerate()
            {
                assert_eq!(pixels, expected, "{kind} picture {index}");
                let decrement = if index >= 33 {
                    32
                } else if index >= 17 {
                    16
                } else {
                    0
                };
                assert_eq!(*poc, index as i32 - decrement, "{kind} picture {index}");
            }
        }
    }
}

#[test]
fn reset_video_software_seek_and_rewind_match_baseline() {
    use fvid::playback_mp4::Mp4VideoReader;
    let open = |name: &str| {
        Mp4VideoReader::open_software(Cursor::new(fixture(name)), Default::default(), 32 << 20)
            .unwrap()
    };
    for (kind, baseline_name) in [
        ("msb-reset", "hevc-poc-baseline-synthetic.mp4"),
        ("msb-reset-signalled", "hevc-poc-baseline-synthetic.mp4"),
        ("full-reset", "hevc-poc-idr-baseline-synthetic.mp4"),
        (
            "multislice-msb-reset",
            "hevc-poc-multislice-baseline-synthetic.mp4",
        ),
        (
            "multislice-msb-reset-signalled",
            "hevc-poc-multislice-baseline-synthetic.mp4",
        ),
        (
            "multislice-full-reset",
            "hevc-poc-multislice-idr-baseline-synthetic.mp4",
        ),
        (
            "dependent-msb-reset",
            "hevc-poc-dependent-baseline-synthetic.mp4",
        ),
        (
            "dependent-msb-reset-signalled",
            "hevc-poc-dependent-baseline-synthetic.mp4",
        ),
        (
            "dependent-full-reset",
            "hevc-poc-dependent-idr-baseline-synthetic.mp4",
        ),
        (
            "long-term-msb-reset",
            "hevc-poc-long-term-baseline-synthetic.mp4",
        ),
        (
            "long-term-msb-reset-signalled",
            "hevc-poc-long-term-baseline-synthetic.mp4",
        ),
        (
            "long-term-full-reset",
            "hevc-poc-long-term-idr-baseline-synthetic.mp4",
        ),
    ] {
        let mut baseline = open(baseline_name);
        let mut actual = open(&format!("hevc-poc-{kind}-synthetic.mp4"));
        assert!(!actual.hardware_accelerated());
        for target in [None, Some(18), Some(34), None] {
            if let Some(index) = target {
                assert_eq!(actual.seek_to_sync(index), baseline.seek_to_sync(index));
            } else {
                actual.rewind();
                baseline.rewind();
            }
            let mut count = 0;
            loop {
                match (actual.read_frame().unwrap(), baseline.read_frame().unwrap()) {
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
                    _ => panic!("reset playback frame count differs"),
                }
            }
            let expected = if kind.ends_with("full-reset") {
                match target {
                    Some(18) => 18,
                    Some(34) => 2,
                    _ => 35,
                }
            } else {
                35
            };
            assert_eq!(count, expected, "{kind}: sync preroll frame count");
        }
    }
}

#[test]
fn full_reset_at_idr_and_same_period_signalled_reset_preserve_pixels() {
    for prefix in ["", "420-", "long-term-", "multislice-", "dependent-"] {
        let baseline = decode(&format!("hevc-poc-{prefix}idr-baseline-synthetic.mp4"));
        let actual = decode(&format!("hevc-poc-{prefix}full-reset-synthetic.mp4"));
        assert_eq!(baseline.len(), 35);
        assert_eq!(actual, baseline);
        for (index, (poc, _)) in actual.iter().enumerate() {
            let epoch = index
                - if index >= 33 {
                    33
                } else if index >= 17 {
                    17
                } else {
                    0
                };
            assert_eq!(*poc, epoch as i32);
        }
    }
}

#[test]
fn malformed_reset_video_fails_at_specific_picture_and_reset_recovers() {
    for prefix in ["", "420-", "long-term-", "multislice-", "dependent-"] {
        let baseline = decode(&format!("hevc-poc-{prefix}baseline-synthetic.mp4"));
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
            (
                "bad-slice-period",
                17,
                "HEVC slices disagree on POC reset syntax",
            ),
            (
                "bad-slice-reset",
                17,
                "HEVC slices disagree on POC reset syntax",
            ),
        ] {
            if kind.starts_with("bad-slice-") && !matches!(prefix, "multislice-" | "dependent-") {
                continue;
            }
            let mut movie = Mp4Reader::open(
                Cursor::new(fixture(&format!("hevc-poc-{prefix}{kind}-synthetic.mp4"))),
                Default::default(),
            )
            .unwrap();
            let mut decoder =
                HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20)
                    .unwrap();
            let mut packet = Vec::new();
            let mut first = Vec::new();
            for index in 0..failure {
                movie.read_packet(0, index, &mut packet).unwrap();
                if index == 0 {
                    first.clone_from(&packet);
                }
                let frame = decoder
                    .decode_packet(&packet)
                    .unwrap_or_else(|e| panic!("{kind} before failure at {index}: {e}"))
                    .unwrap();
                assert_eq!(
                    frame.picture.planes[0].samples(),
                    baseline[index].1,
                    "{kind} picture {index}"
                );
            }
            movie.read_packet(0, failure, &mut packet).unwrap();
            let error = decoder
                .decode_packet(&packet)
                .err()
                .expect("malformed reset must fail");
            assert!(matches!(error, fvid::Error::Invalid(_)), "{kind}: {error}");
            assert_eq!(error.to_string(), message, "{kind}");
            assert!(
                decoder.decode_packet(&first).is_err(),
                "error must poison decoder"
            );
            decoder.reset();
            let frame = decoder.decode_packet(&first).unwrap().unwrap();
            assert_eq!(frame.poc, 0);
            assert_eq!(frame.picture.planes[0].samples(), baseline[0].1);
        }
    }
}

#[test]
fn automatic_reset_playback_matches_owned_software() {
    use fvid::playback_mp4::{Mp4VideoReader, VideoFrame};
    fn samples(frame: &VideoFrame) -> [Vec<u16>; 3] {
        if let Some(p) = &frame.planes8 {
            [&p.y, &p.cb, &p.cr].map(|v| v.iter().copied().map(u16::from).collect())
        } else {
            [
                frame.picture.y.clone(),
                frame.picture.cb.clone(),
                frame.picture.cr.clone(),
            ]
        }
    }
    for family in ["420", "long-term", "multislice", "dependent"] {
        for kind in ["msb-reset", "msb-reset-signalled", "full-reset"] {
            let name = format!("hevc-poc-{family}-{kind}-synthetic.mp4");
            let bytes = fixture(&name);
            let mut automatic =
                Mp4VideoReader::open(Cursor::new(bytes.clone()), Default::default(), 32 << 20)
                    .unwrap();
            let mut software =
                Mp4VideoReader::open_software(Cursor::new(bytes), Default::default(), 32 << 20)
                    .unwrap();
            eprintln!("{name}: hardware={}", automatic.hardware_accelerated());
            for target in [None, Some(18), Some(34), None] {
                if let Some(pts) = target {
                    assert_eq!(automatic.seek_to_sync(pts), software.seek_to_sync(pts));
                } else {
                    automatic.rewind();
                    software.rewind();
                }
                let mut count = 0;
                loop {
                    match (
                        automatic
                            .read_frame()
                            .unwrap_or_else(|e| panic!("{name} automatic picture {count}: {e}")),
                        software.read_frame().unwrap(),
                    ) {
                        (Some(a), Some(b)) => {
                            assert_eq!(a.sample_index, b.sample_index);
                            assert_eq!(a.presentation_time, b.presentation_time);
                            assert_eq!(a.duration, b.duration);
                            let [ay, acb, acr] = samples(&a);
                            let [by, bcb, bcr] = samples(&b);
                            assert!(ay == by, "{name} Y picture {count}");
                            assert!(acb == bcb, "{name} Cb picture {count}");
                            assert!(acr == bcr, "{name} Cr picture {count}");
                            count += 1;
                        }
                        (None, None) => break,
                        _ => panic!("{name} automatic/software frame count differs"),
                    }
                }
                let expected = if kind == "full-reset" {
                    match target {
                        Some(18) => 18,
                        Some(34) => 2,
                        _ => 35,
                    }
                } else {
                    35
                };
                assert_eq!(count, expected);
            }
        }
    }
}

#[test]
fn owned_420_reset_pixels_match_baseline_and_saved_seed_oracle() {
    use fvid::playback_mp4::Mp4VideoReader;
    let open = |name: &str| {
        Mp4VideoReader::open_software(Cursor::new(fixture(name)), Default::default(), 32 << 20)
            .unwrap()
    };
    for (family, oracle) in [
        ("420", "hevc-pcm-reference-rext8.yuv"),
        ("long-term", "hevc-long-term-rext8.yuv"),
        ("multislice", "hevc-chroma-qp-list-slices-rext8.yuv"),
        ("dependent", "hevc-chroma-qp-list-dependent-rext8.yuv"),
    ] {
        let gold = fixture(oracle);
        assert_eq!(gold.len(), 3 * 64 * 64 * 3 / 2);
        let mut baseline = open(&format!("hevc-poc-{family}-baseline-synthetic.mp4"));
        for expected in gold.chunks_exact(64 * 64 * 3 / 2) {
            let frame = baseline.read_frame().unwrap().unwrap();
            let pixels = [
                &frame.picture.y[..],
                &frame.picture.cb[..],
                &frame.picture.cr[..],
            ]
            .concat();
            assert_eq!(
                pixels,
                expected.iter().copied().map(u16::from).collect::<Vec<_>>()
            );
        }
        for (kind, base) in [
            ("msb-reset", "baseline"),
            ("msb-reset-signalled", "baseline"),
            ("full-reset", "idr-baseline"),
        ] {
            let mut baseline = open(&format!("hevc-poc-{family}-{base}-synthetic.mp4"));
            let mut actual = open(&format!("hevc-poc-{family}-{kind}-synthetic.mp4"));
            for index in 0..35 {
                let a = actual.read_frame().unwrap().unwrap();
                let b = baseline.read_frame().unwrap().unwrap();
                assert_eq!(a.picture.y, b.picture.y, "{kind} Y {index}");
                assert_eq!(a.picture.cb, b.picture.cb, "{kind} Cb {index}");
                assert_eq!(a.picture.cr, b.picture.cr, "{kind} Cr {index}");
            }
            assert!(actual.read_frame().unwrap().is_none());
            assert!(baseline.read_frame().unwrap().is_none());
        }
    }
}

#[test]
fn multislice_reset_headers_agree_and_malformed_variants_diverge_only_at_reset() {
    use fvid::codec::hevc_poc::{Reset, ResetSyntax};
    for family in ["multislice", "dependent"] {
        for kind in ["msb-reset", "bad-slice-period", "bad-slice-reset"] {
            let name = format!("hevc-poc-{family}-{kind}-synthetic.mp4");
            let mut movie =
                Mp4Reader::open(Cursor::new(fixture(&name)), Default::default()).unwrap();
            let mut decoder =
                HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20)
                    .unwrap();
            let mut packet = Vec::new();
            for index in 0..35 {
                movie.read_packet(0, index, &mut packet).unwrap();
                let headers = decoder.slice_headers(&packet).unwrap();
                assert_eq!(headers.len(), 4);
                assert_eq!(
                    headers.iter().map(|h| h.address).collect::<Vec<_>>(),
                    [0, 1, 2, 3]
                );
                assert!(headers[0].first && headers[1..].iter().all(|h| !h.first));
                assert!(!headers[0].dependent);
                assert!(headers[1..]
                    .iter()
                    .all(|h| h.dependent == (family == "dependent")));
                if family == "dependent" {
                    for h in &headers[1..] {
                        assert_eq!(h.poc_lsb, headers[0].poc_lsb);
                        assert_eq!(h.qp, headers[0].qp);
                        assert_eq!(h.temporal_mvp, headers[0].temporal_mvp);
                        assert_eq!(h.references, headers[0].references);
                        assert_eq!(h.short_term, headers[0].short_term);
                    }
                }
                let syntax = headers
                    .iter()
                    .map(|h| ResetSyntax::parse_base(&h.extension, true, 4, h.nal, false).unwrap())
                    .collect::<Vec<_>>();
                if index == 17 && kind.starts_with("bad-") {
                    assert_eq!(syntax[0].reset, Some(Reset::Msb));
                    assert_eq!(syntax[0].period, Some(7));
                    assert!(syntax[1..].iter().all(|s| *s != syntax[0]));
                    if kind == "bad-slice-period" {
                        assert!(syntax[1..]
                            .iter()
                            .all(|s| s.reset == Some(Reset::Msb) && s.period == Some(8)));
                    } else {
                        assert!(syntax[1..].iter().all(|s| *s == ResetSyntax::default()));
                    }
                    break;
                }
                assert!(syntax.iter().all(|s| *s == syntax[0]));
                assert_eq!(
                    syntax[0].reset,
                    if matches!(index, 17 | 33) {
                        Some(Reset::Msb)
                    } else {
                        None
                    }
                );
                decoder.decode_packet(&packet).unwrap().unwrap();
            }
        }
    }
}
