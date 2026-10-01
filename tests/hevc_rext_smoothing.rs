use fvid::{
    codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_sps::Sps},
    container::mp4::Mp4Reader,
};
use std::io::Cursor;

fn compare(
    source: &[u8],
    oracle: &[u8],
    depth: u8,
    disabled: bool,
    rotation: bool,
    bypass: Option<bool>,
    context: bool,
    rdpcm: bool,
    explicit: bool,
) {
    let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let parsed = HevcConfig::parse(&configuration).unwrap();
    let sps = parsed
        .arrays
        .iter()
        .find(|a| a.nal_type == 33)
        .unwrap()
        .units[0];
    let sps = Sps::parse(sps, 16 << 20).unwrap();
    assert_eq!(sps.profile.profile.unwrap().idc, 4);
    assert_eq!(sps.depth, [depth; 2]);
    assert_eq!(sps.intra_smoothing_disabled, disabled);
    assert_eq!(sps.transform_skip_rotation, rotation);
    assert_eq!(sps.transform_skip_context, context);
    assert_eq!(sps.implicit_rdpcm, rdpcm);
    assert_eq!(sps.explicit_rdpcm, explicit);
    let mut decoder = HevcDecoder::from_configuration(&configuration, 16 << 20).unwrap();
    if let Some(bypass) = bypass {
        assert_eq!(decoder.parameters().1.transquant_bypass, bypass);
        assert!(decoder.parameters().1.transform_skip);
    }
    let count = input.tracks()[0].samples.len();
    assert_eq!(count, 3);
    for _ in 0..2 {
        let mut actual = Vec::new();
        for index in 0..count {
            let mut packet = Vec::new();
            input.read_packet(0, index, &mut packet).unwrap();
            let frame = decoder.decode_packet(&packet).unwrap().unwrap();
            for plane in &frame.picture.planes {
                for &value in plane.samples() {
                    if depth == 8 {
                        actual.push(value as u8);
                    } else {
                        actual.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        assert_eq!(actual.len(), oracle.len());
        assert!(actual == oracle, "depth {depth}, first mismatch {:?}",
            actual.iter().zip(oracle).position(|(a, b)| a != b));
        decoder.reset();
    }
    let mut reader =
        fvid::playback_mp4::Mp4VideoReader::open(Cursor::new(source), Default::default(), 16 << 20)
            .unwrap();
    assert!(!reader.hardware_accelerated());
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut frames = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            for plane in [&frame.picture.y, &frame.picture.cb, &frame.picture.cr] {
                for &value in plane {
                    if depth == 8 {
                        actual.push(value as u8);
                    } else {
                        actual.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
            frames += 1;
        }
        assert_eq!(frames, 3);
        assert_eq!(actual.len(), oracle.len());
        assert!(actual == oracle, "depth {depth}, first mismatch {:?}",
            actual.iter().zip(oracle).position(|(a, b)| a != b));
        reader.rewind();
    }
}

#[test]
fn rext_intra_reference_filtering_matches_oracles_and_reset() {
    for (source, oracle, depth, disabled) in [
        (
            &include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-8-enabled.mp4")[..],
            &include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-8-enabled.yuv")[..],
            8,
            false,
        ),
        (
            &include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-8-disabled.mp4")[..],
            &include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-8-disabled.yuv")[..],
            8,
            true,
        ),
        (
            &include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-10-enabled.mp4")[..],
            &include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-10-enabled.yuv")[..],
            10,
            false,
        ),
        (
            &include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-10-disabled.mp4")[..],
            &include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-10-disabled.yuv")[..],
            10,
            true,
        ),
    ] {
        compare(
            source, oracle, depth, disabled, false, None, false, false, false,
        );
    }
    assert_ne!(
        include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-8-enabled.yuv"),
        include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-8-disabled.yuv")
    );
    assert_ne!(
        include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-10-enabled.yuv"),
        include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-10-disabled.yuv")
    );
}

#[test]
fn unsupported_range_tools_are_not_silently_ignored() {
    use fvid::codec::hevc_nal::NalRbsp;
    let source = include_bytes!("fixtures/playback-errors/hevc-rext-smoothing-8-enabled.mp4");
    let input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let parsed = HevcConfig::parse(&configuration).unwrap();
    let nal = parsed
        .arrays
        .iter()
        .find(|a| a.nal_type == 33)
        .unwrap()
        .units[0];
    let rbsp = NalRbsp::parse(nal, 16 << 20).unwrap().bytes;
    let stop = (0..rbsp.len() * 8)
        .rev()
        .find(|&i| rbsp[i / 8] & (1 << (7 - i % 8)) != 0)
        .unwrap();
    for flag in (0..9).filter(|&i| i != 0 && i != 1 && i != 2 && i != 3 && i != 5 && i != 6) {
        let mut bytes = rbsp.clone();
        let bit = stop - 9 + flag;
        bytes[bit / 8] |= 1 << (7 - bit % 8);
        let mut updated = nal[..2].to_vec();
        let mut zeros = 0;
        for byte in bytes {
            if zeros == 2 && byte <= 3 {
                updated.push(3);
                zeros = 0;
            }
            updated.push(byte);
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        assert!(
            Sps::parse(&updated, 16 << 20)
                .unwrap_err()
                .to_string()
                .contains("remaining HEVC SPS range-extension tools")
        );
    }
}

#[test]
fn rext_transform_skip_rotation_matches_oracle() {
    macro_rules! check {
        ($stem:literal, $depth:literal, $rotation:literal, $bypass:literal) => {
            compare(
                include_bytes!(concat!("fixtures/playback-errors/", $stem, ".mp4")),
                include_bytes!(concat!("fixtures/playback-errors/", $stem, ".yuv")),
                $depth,
                false,
                $rotation,
                Some($bypass),
                false,
                false,
                false,
            );
        };
    }
    check!("hevc-rext-rotation-8-skip-disabled", 8, false, false);
    check!("hevc-rext-rotation-8-skip-enabled", 8, true, false);
    check!("hevc-rext-rotation-8-bypass-disabled", 8, false, true);
    check!("hevc-rext-rotation-8-bypass-enabled", 8, true, true);
    check!("hevc-rext-rotation-10-skip-disabled", 10, false, false);
    check!("hevc-rext-rotation-10-skip-enabled", 10, true, false);
    check!("hevc-rext-rotation-10-bypass-disabled", 10, false, true);
    check!("hevc-rext-rotation-10-bypass-enabled", 10, true, true);
    macro_rules! differs {
        ($base:literal) => {
            assert_ne!(
                include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.yuv")),
                include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.yuv")),
            );
        };
    }
    differs!("hevc-rext-rotation-8-skip");
    differs!("hevc-rext-rotation-8-bypass");
    differs!("hevc-rext-rotation-10-skip");
    differs!("hevc-rext-rotation-10-bypass");
}

fn coded_units(source: &[u8]) -> Vec<Vec<u8>> {
    use fvid::codec::config::NalUnits;
    let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let size = HevcConfig::parse(&configuration).unwrap().length_size;
    let mut coded = Vec::new();
    for index in 0..input.tracks()[0].samples.len() {
        let mut packet = Vec::new();
        input.read_packet(0, index, &mut packet).unwrap();
        for nal in NalUnits::new(&packet, size).unwrap() {
            let nal = nal.unwrap();
            if ((nal[0] >> 1) & 63) < 32 {
                coded.push(nal.to_vec());
            }
        }
    }
    coded
}

#[test]
fn rext_significance_contexts_match_oracle() {
    macro_rules! check_pair {
        ($base:literal, $depth:literal, $bypass:literal) => {
            for (source, oracle, enabled) in [
                (&include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.mp4"))[..],
                 &include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.yuv"))[..], false),
                (&include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.mp4"))[..],
                 &include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.yuv"))[..], true),
            ] {
                compare(source, oracle, $depth, false, false, Some($bypass), enabled, false, false);
            }
            // Context selection changes actual VCL coding, not merely SPS bytes.
            assert_ne!(
                coded_units(include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.mp4"))),
                coded_units(include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.mp4"))),
            );
        };
    }
    check_pair!("hevc-rext-context-8-skip", 8, false);
    check_pair!("hevc-rext-context-8-bypass", 8, true);
    check_pair!("hevc-rext-context-10-skip", 10, false);
    check_pair!("hevc-rext-context-10-bypass", 10, true);
}

#[test]
fn implicit_rdpcm_matches_oracles() {
    macro_rules! check_pair {
        ($base:literal, $depth:literal, $bypass:literal) => {
            for (source, oracle, enabled) in [
                (&include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.mp4"))[..],
                 &include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.yuv"))[..], false),
                (&include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.mp4"))[..],
                 &include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.yuv"))[..], true),
            ] {
                compare(source, oracle, $depth, false, false, Some($bypass), false, enabled, false);
            }
            assert_ne!(
                coded_units(include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.mp4"))),
                coded_units(include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.mp4"))),
            );
        };
    }
    check_pair!("hevc-rext-rdpcm-8-skip", 8, false);
    check_pair!("hevc-rext-rdpcm-8-bypass", 8, true);
    check_pair!("hevc-rext-rdpcm-10-skip", 10, false);
    check_pair!("hevc-rext-rdpcm-10-bypass", 10, true);
}

fn assert_lowdelay_inter(source: &[u8]) {
    use fvid::codec::hevc_cabac::SliceType;
    let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let decoder = HevcDecoder::from_configuration(&configuration, 16 << 20).unwrap();
    for index in 0..3 {
        let mut packet = Vec::new();
        input.read_packet(0, index, &mut packet).unwrap();
        let headers = decoder.slice_headers(&packet).unwrap();
        assert!(!headers.is_empty());
        let expected = if index == 0 {
            SliceType::I
        } else {
            SliceType::P
        };
        assert!(headers.iter().all(|header| header.slice_type == expected));
    }
}

#[test]
fn explicit_rdpcm_matches_inter_oracles() {
    macro_rules! check_pair {
        ($base:literal, $depth:literal, $bypass:literal) => {
            for (source, oracle, enabled) in [
                (&include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.mp4"))[..],
                 &include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.yuv"))[..], false),
                (&include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.mp4"))[..],
                 &include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.yuv"))[..], true),
            ] {
                assert_lowdelay_inter(source);
                compare(source, oracle, $depth, false, false, Some($bypass), false, false, enabled);
            }
            assert_ne!(
                coded_units(include_bytes!(concat!("fixtures/playback-errors/", $base, "-disabled.mp4"))),
                coded_units(include_bytes!(concat!("fixtures/playback-errors/", $base, "-enabled.mp4"))),
            );
        };
    }
    check_pair!("hevc-rext-explicit-rdpcm-8-skip", 8, false);
    check_pair!("hevc-rext-explicit-rdpcm-8-bypass", 8, true);
    check_pair!("hevc-rext-explicit-rdpcm-10-skip", 10, false);
    check_pair!("hevc-rext-explicit-rdpcm-10-bypass", 10, true);
}

#[test]
fn large_transform_skip_matches_oracle() {
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip8-8-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip8-8-skip-enabled.yuv"),
        8, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip8-8-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip8-8-skip-disabled.yuv"),
        8, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip8-10-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip8-10-skip-enabled.yuv"),
        10, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip8-10-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip8-10-skip-disabled.yuv"),
        10, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip16-8-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip16-8-skip-enabled.yuv"),
        8, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip16-8-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip16-8-skip-disabled.yuv"),
        8, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip16-10-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip16-10-skip-enabled.yuv"),
        10, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip16-10-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip16-10-skip-disabled.yuv"),
        10, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip32-8-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip32-8-skip-enabled.yuv"),
        8, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip32-8-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip32-8-skip-disabled.yuv"),
        8, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip32-10-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip32-10-skip-enabled.yuv"),
        10, false, false, Some(false), false, false, false,
    );
    compare(
        include_bytes!("fixtures/playback-errors/hevc-rext-skip32-10-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-skip32-10-skip-disabled.yuv"),
        10, false, false, Some(false), false, false, false,
    );
}

fn assert_high_precision_weights(source: &[u8], enabled: bool) {
    let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let decoder = HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
    assert_eq!(decoder.parameters().0.high_precision_offsets, enabled);
    let mut nonzero = false;
    let mut wide = false;
    for index in 1..3 {
        let mut packet = Vec::new();
        input.read_packet(0, index, &mut packet).unwrap();
        for header in decoder.slice_headers(&packet).unwrap() {
            let weights = header.weights.unwrap();
            assert_eq!(weights.high_precision_offsets, enabled);
            wide |= weights.lists.iter().flatten().any(|w| w.offsets.iter().any(|&v| !(-128..=127).contains(&v)));
            nonzero |= weights.lists.iter().flatten().any(|w| w.offsets.iter().any(|&v| v != 0));
        }
    }
    assert!(nonzero, "fixture must exercise weighted offsets");
    if enabled && decoder.parameters().0.depth[0] == 10 {
        assert!(wide, "10-bit fixture must exercise offsets outside 8-bit bounds");
    }
}

#[test]
fn high_precision_weighted_prediction_matches_oracle() {
    assert_high_precision_weights(include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-8-skip-enabled.mp4"), true);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-8-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-8-skip-enabled.yuv"),
        8, false, false, Some(false), false, false, false);
    assert_high_precision_weights(include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-8-skip-disabled.mp4"), false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-8-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-8-skip-disabled.yuv"),
        8, false, false, Some(false), false, false, false);
    assert_high_precision_weights(include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-10-skip-enabled.mp4"), true);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-10-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-10-skip-enabled.yuv"),
        10, false, false, Some(false), false, false, false);
    assert_high_precision_weights(include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-10-skip-disabled.mp4"), false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-10-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-high-precision-10-skip-disabled.yuv"),
        10, false, false, Some(false), false, false, false);
}

#[test]
fn twelve_bit_420_matches_oracle() {
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-enabled.yuv"),
        12, false, false, Some(false), true, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-disabled.yuv"),
        12, false, false, Some(false), false, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled.yuv"),
        12, false, false, Some(true), true, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled.yuv"),
        12, false, false, Some(true), false, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-enabled-filters.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-enabled-filters.yuv"),
        12, false, false, Some(false), true, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-disabled-filters.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-disabled-filters.yuv"),
        12, false, false, Some(false), false, false, false);
    assert_lowdelay_inter(include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-skip-enabled.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-skip-enabled.yuv"),
        12, false, false, Some(false), false, false, true);
    assert_lowdelay_inter(include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-skip-disabled.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-skip-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-skip-disabled.yuv"),
        12, false, false, Some(false), false, false, false);
    assert_lowdelay_inter(include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-bypass-enabled.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-bypass-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-bypass-enabled.yuv"),
        12, false, false, Some(true), false, false, true);
    assert_lowdelay_inter(include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-bypass-disabled.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-bypass-disabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-explicit-rdpcm-12-bypass-disabled.yuv"),
        12, false, false, Some(true), false, false, false);
}

#[test]
fn lossless_bypass_with_filters_matches_oracle() {
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-filters.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-filters.yuv"),
        12, false, false, Some(true), true, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-filters.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-filters.yuv"),
        12, false, false, Some(true), false, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-8-bypass-enabled-filters.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-8-bypass-enabled-filters.yuv"),
        8, false, false, Some(true), true, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-8-bypass-disabled-filters.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-8-bypass-disabled-filters.yuv"),
        8, false, false, Some(true), false, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-10-bypass-enabled-filters.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-10-bypass-enabled-filters.yuv"),
        10, false, false, Some(true), true, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-10-bypass-disabled-filters.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-10-bypass-disabled-filters.yuv"),
        10, false, false, Some(true), false, false, false);
}

#[test]
fn twelve_bit_high_qp_filters_match_oracle() {
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-enabled-filters-qp51.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-enabled-filters-qp51.yuv"),
        12, false, false, Some(false), true, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-disabled-filters-qp51.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-disabled-filters-qp51.yuv"),
        12, false, false, Some(false), false, false, false);
}

fn assert_scaled_sao(source: &[u8]) {
    use fvid::codec::hevc_sao::Sao;
    let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let mut decoder = HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
    assert_eq!(decoder.parameters().1.sao_offset_scale, [2, 2]);
    let mut nonzero = false;
    for index in 0..3 {
        let mut packet = Vec::new();
        input.read_packet(0, index, &mut packet).unwrap();
        let frame = decoder.decode_packet(&packet).unwrap().unwrap();
        for mode in frame.picture.sao.iter().flatten() {
            if let Sao::Band { offsets, .. } | Sao::Edge { offsets, .. } = mode {
                assert!(offsets.iter().all(|v| v % 4 == 0));
                nonzero |= offsets.iter().any(|&v| v != 0);
            }
        }
    }
    assert!(nonzero, "fixture must exercise actual scaled SAO offsets");
}

#[test]
fn twelve_bit_scaled_sao_matches_oracle() {
    assert_scaled_sao(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-enabled-filters-sao2.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-enabled-filters-sao2.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-enabled-filters-sao2.yuv"),
        12, false, false, Some(false), true, false, false);
    assert_scaled_sao(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-disabled-filters-sao2.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-disabled-filters-sao2.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-skip-disabled-filters-sao2.yuv"),
        12, false, false, Some(false), false, false, false);
}

#[test]
fn mixed_bypass_with_filters_matches_oracle() {
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-mixed-filters-qp-24.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-mixed-filters-qp-24.yuv"),
        12, false, false, Some(true), true, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-mixed-filters-qp-24.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-mixed-filters-qp-24.yuv"),
        12, false, false, Some(true), false, false, false);
}

fn assert_two_filtered_slices(source: &[u8]) {
    let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let decoder = HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
    assert!(decoder.parameters().1.transquant_bypass);
    for index in 0..3 {
        let mut packet = Vec::new();
        input.read_packet(0, index, &mut packet).unwrap();
        let headers = decoder.slice_headers(&packet).unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!((headers[0].address, headers[1].address), (0, 2));
        assert!(headers.iter().all(|h| !h.deblocking.disabled && h.sao != [false; 2]));
    }
}

#[test]
fn multislice_bypass_with_filters_matches_oracle() {
    assert_two_filtered_slices(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-filters-slices2.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-filters-slices2.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-filters-slices2.yuv"),
        12, false, false, Some(true), true, false, false);
    assert_two_filtered_slices(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-filters-slices2.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-filters-slices2.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-filters-slices2.yuv"),
        12, false, false, Some(true), false, false, false);
    assert_two_filtered_slices(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-mixed-filters-qp-24-slices2.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-mixed-filters-qp-24-slices2.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-enabled-mixed-filters-qp-24-slices2.yuv"),
        12, false, false, Some(true), true, false, false);
    assert_two_filtered_slices(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-mixed-filters-qp-24-slices2.mp4"));
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-mixed-filters-qp-24-slices2.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-context-12-bypass-disabled-mixed-filters-qp-24-slices2.yuv"),
        12, false, false, Some(true), false, false, false);
}

#[test]
fn persistent_rice_current_refusal_is_specific() {
    {
        let source = include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-skip-enabled.mp4");
        let input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
        let config = HevcConfig::parse(&input.tracks()[0].configuration).unwrap();
        let sps = config.arrays.iter().find(|a| a.nal_type == 33).unwrap().units[0];
        let error = Sps::parse(sps, 16 << 20).err().unwrap();
        assert!(error.to_string().contains("remaining HEVC SPS range-extension tools"));
        compare(include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-skip-disabled.mp4"),
            include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-skip-disabled.yuv"),
            8, false, false, Some(false), false, false, false);
    }
    {
        let source = include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-bypass-enabled.mp4");
        let input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
        let config = HevcConfig::parse(&input.tracks()[0].configuration).unwrap();
        let sps = config.arrays.iter().find(|a| a.nal_type == 33).unwrap().units[0];
        let error = Sps::parse(sps, 16 << 20).err().unwrap();
        assert!(error.to_string().contains("remaining HEVC SPS range-extension tools"));
        compare(include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-bypass-disabled.mp4"),
            include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-bypass-disabled.yuv"),
            8, false, false, Some(true), false, false, false);
    }
    {
        let source = include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-skip-enabled.mp4");
        let input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
        let config = HevcConfig::parse(&input.tracks()[0].configuration).unwrap();
        let sps = config.arrays.iter().find(|a| a.nal_type == 33).unwrap().units[0];
        let error = Sps::parse(sps, 16 << 20).err().unwrap();
        assert!(error.to_string().contains("remaining HEVC SPS range-extension tools"));
        compare(include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-skip-disabled.mp4"),
            include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-skip-disabled.yuv"),
            10, false, false, Some(false), false, false, false);
    }
    {
        let source = include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-bypass-enabled.mp4");
        let input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
        let config = HevcConfig::parse(&input.tracks()[0].configuration).unwrap();
        let sps = config.arrays.iter().find(|a| a.nal_type == 33).unwrap().units[0];
        let error = Sps::parse(sps, 16 << 20).err().unwrap();
        assert!(error.to_string().contains("remaining HEVC SPS range-extension tools"));
        compare(include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-bypass-disabled.mp4"),
            include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-bypass-disabled.yuv"),
            10, false, false, Some(true), false, false, false);
    }
}

#[test]
#[ignore = "persistent Rice decoding not implemented; refusal coverage is not acceptance"]
fn persistent_rice_acceptance_matches_oracle() {
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-skip-enabled.yuv"),
        8, false, false, Some(false), false, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-bypass-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-8-bypass-enabled.yuv"),
        8, false, false, Some(true), false, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-skip-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-skip-enabled.yuv"),
        10, false, false, Some(false), false, false, false);
    compare(include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-bypass-enabled.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-rext-persistent-rice-10-bypass-enabled.yuv"),
        10, false, false, Some(true), false, false, false);
}
