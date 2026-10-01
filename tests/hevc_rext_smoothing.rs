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
        assert_eq!(actual, oracle);
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
        assert_eq!(actual, oracle);
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
        compare(source, oracle, depth, disabled, false, None, false);
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
    for flag in (0..9).filter(|&i| i != 0 && i != 1 && i != 5) {
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
                compare(source, oracle, $depth, false, false, Some($bypass), enabled);
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
