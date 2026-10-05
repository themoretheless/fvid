//! Explicit fixture regeneration, never executed by ordinary tests.
use super::{
    config::{HevcConfig, NalUnits},
    hevc_decoder::HevcDecoder,
    hevc_nal::{NalHeader, NalRbsp},
    hevc_slice::SliceHeader,
};

fn bits(bytes: &[u8]) -> Vec<bool> {
    bytes
        .iter()
        .flat_map(|b| (0..8).rev().map(move |i| b & (1 << i) != 0))
        .collect()
}
fn ue(value: usize, out: &mut Vec<bool>) {
    let value = value + 1;
    let length = usize::BITS - value.leading_zeros();
    out.extend(std::iter::repeat_n(false, (length - 1) as usize));
    out.extend((0..length).rev().map(|i| value & (1 << i) != 0));
}
fn escaped_nal(header: &[u8], mut payload: Vec<bool>) -> Vec<u8> {
    while payload.len() % 8 != 0 {
        payload.push(false);
    }
    let mut nal = header.to_vec();
    let mut zeros = 0;
    for byte in payload
        .chunks_exact(8)
        .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
    {
        if zeros >= 2 && byte <= 3 {
            nal.push(3);
            zeros = 0;
        }
        nal.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    nal
}

#[test]
#[ignore = "explicit synthetic fixture regeneration; requires FVID_HEVC_LONG_TERM_STREAM"]
fn generate_long_term_stream() {
    let destination = std::env::var_os("FVID_HEVC_LONG_TERM_STREAM").expect("explicit output path");
    let mode = std::env::var("FVID_HEVC_LONG_TERM_MODE").unwrap_or_else(|_| "explicit".into());
    assert!(matches!(
        mode.as_str(),
        "explicit" | "lsb" | "mixed" | "invalid-short" | "sps" | "b-mixed" | "b-mixed-l1"
    ));
    let source = if mode.starts_with("b-mixed") {
        include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-b-base-main8.mp4")
            .as_slice()
    } else {
        include_bytes!(
            "../../tests/fixtures/playback-errors/hevc-rext-explicit-rdpcm-8-skip-disabled.mp4"
        )
        .as_slice()
    };
    let mut input =
        crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(source), Default::default())
            .unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let config = HevcConfig::parse(&configuration).unwrap();
    let mut decoder =
        HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
    let (sps, pps) = decoder.parameters();
    let (sps, pps) = (sps.clone(), pps.clone());
    assert!(!sps.long_term_present);
    assert_eq!(sps.ordering.len(), 1);
    assert!(!pps.lists_modification);
    assert!(!pps.entropy_sync && pps.tiles.is_none());
    assert_eq!(sps.dimensions, [64, 64]);
    assert_eq!(sps.depth, [8, 8]);
    let mut stream = Vec::new();
    let mut emit = |nal: &[u8]| {
        stream.extend_from_slice(&[0, 0, 0, 1]);
        stream.extend_from_slice(nal);
    };
    for array in &config.arrays {
        for nal in &array.units {
            if array.nal_type == 33 {
                let rbsp = NalRbsp::parse(nal, 16 << 20).unwrap();
                let mut payload = bits(&rbsp.bytes);
                // Strip trailing padding, preserve rbsp_stop_one_bit, insert zero SPS entries.
                payload.truncate(payload.iter().rposition(|&b| b).unwrap() + 1);
                let mut long_syntax = vec![true];
                ue(if mode == "sps" { 2 } else { 0 }, &mut long_syntax);
                if mode == "sps" {
                    for poc in [0u32, 1] {
                        long_syntax.extend((0..sps.poc_bits).rev().map(|i| poc & (1 << i) != 0));
                        long_syntax.push(true);
                    }
                }
                payload.splice(
                    sps.long_term_flag_bit..sps.long_term_flag_bit + 1,
                    long_syntax,
                );
                if matches!(mode.as_str(), "mixed" | "invalid-short") {
                    let mut ordering = vec![false];
                    ue(2, &mut ordering);
                    ue(0, &mut ordering);
                    ue(0, &mut ordering);
                    payload.splice(sps.ordering_bit_range.clone(), ordering);
                }
                emit(&escaped_nal(&nal[..2], payload));
            } else if array.nal_type == 34 && mode == "b-mixed-l1" {
                let rbsp = NalRbsp::parse(nal, 16 << 20).unwrap();
                let mut payload = bits(&rbsp.bytes);
                payload[pps.lists_modification_bit] = true;
                emit(&escaped_nal(&nal[..2], payload));
            } else if array.nal_type == 32 && matches!(mode.as_str(), "mixed" | "invalid-short") {
                let vps = super::hevc_vps::Vps::parse(nal, 16 << 20).unwrap();
                assert_eq!(vps.ordering.len(), 1);
                let rbsp = NalRbsp::parse(nal, 16 << 20).unwrap();
                let mut payload = bits(&rbsp.bytes);
                payload.truncate(payload.iter().rposition(|&b| b).unwrap() + 1);
                let mut ordering = vec![false];
                ue(2, &mut ordering);
                ue(0, &mut ordering);
                ue(0, &mut ordering);
                payload.splice(vps.ordering_bit_range, ordering);
                emit(&escaped_nal(&nal[..2], payload));
            } else {
                emit(nal);
            }
        }
    }
    let mut packet = Vec::new();
    let mut used = 0;
    assert_eq!(
        input.tracks()[0].samples.len(),
        if mode.starts_with("b-mixed") { 4 } else { 3 }
    );
    for index in 0..input.tracks()[0].samples.len() {
        input.read_packet(0, index, &mut packet).unwrap();
        let poc = decoder.decode_packet(&packet).unwrap().unwrap().poc;
        assert_eq!(poc, index as i32, "fixture must be low delay");
        for nal in NalUnits::new(&packet, config.length_size).unwrap() {
            let nal = nal.unwrap();
            let header = NalHeader::parse(nal).unwrap();
            if !header.is_vcl() || header.is_idr() {
                emit(nal);
                continue;
            }
            let slice = SliceHeader::parse(nal, &sps, &pps, 16 << 20).unwrap();
            assert!(slice.first && slice.entry_point_offsets.is_empty());
            let mut syntax = vec![false]; // Slice-local short-term set.
            if !sps.short_term.is_empty() {
                syntax.push(false);
            }
            if mode == "mixed" && index == 1 {
                syntax = bits(&slice.rbsp)[slice.short_term_bit_range.clone()].to_vec();
                ue(0, &mut syntax); // Keep POC zero short-term for the next picture.
            } else {
                ue(
                    usize::from(
                        matches!(mode.as_str(), "mixed" | "invalid-short") && index == 2
                            || mode.starts_with("b-mixed") && index >= 2,
                    ),
                    &mut syntax,
                );
                ue(0, &mut syntax);
                if matches!(mode.as_str(), "mixed" | "invalid-short") && index == 2 {
                    ue(1, &mut syntax); // delta_poc_s0_minus1: retained POC zero.
                    syntax.push(true);
                }
                if mode.starts_with("b-mixed") && index >= 2 {
                    assert_eq!(slice.short_term[0].delta_poc, -1);
                    ue(0, &mut syntax);
                    syntax.push(slice.short_term[0].used);
                }
                let long_references =
                    &slice.short_term[usize::from(mode.starts_with("b-mixed") && index >= 2)..];
                if mode == "sps" {
                    ue(long_references.len(), &mut syntax);
                    ue(0, &mut syntax);
                } else {
                    ue(long_references.len(), &mut syntax);
                }
                let modulus = 1i32 << sps.poc_bits;
                let mut previous_cycle = 0;
                for reference in long_references {
                    let target = poc + reference.delta_poc;
                    let lsb = target.rem_euclid(modulus);
                    let cycle = (poc - poc.rem_euclid(modulus) - target + lsb) / modulus;
                    assert!(cycle >= previous_cycle);
                    if mode == "sps" {
                        assert_eq!(slice.short_term.len(), 1);
                        assert_eq!(target, index as i32 - 1);
                        assert!(reference.used);
                        syntax.push(target != 0); // lt_idx_sps selects one of two entries.
                    } else {
                        syntax.extend((0..sps.poc_bits).rev().map(|i| lsb & (1 << i) != 0));
                        syntax.push(reference.used);
                    }
                    syntax.push(mode != "lsb");
                    if mode != "lsb" {
                        ue((cycle - previous_cycle) as usize, &mut syntax);
                    }
                    previous_cycle = cycle;
                    used += usize::from(reference.used);
                }
            }
            let mut payload = bits(&slice.rbsp);
            let end = slice.entropy_byte_offset * 8;
            let alignment = payload[..end].iter().rposition(|&b| b).unwrap();
            let entropy = payload[end..].to_vec();
            payload.truncate(alignment);
            if mode == "b-mixed-l1" && index >= 2 {
                let modification = if index == 2 {
                    vec![false, true, true, false]
                } else {
                    vec![false, false]
                };
                payload.splice(slice.list_modification_bit_range.clone(), modification);
            }
            payload.splice(slice.short_term_bit_range.clone(), syntax);
            payload.push(true);
            while payload.len() % 8 != 0 {
                payload.push(false);
            }
            payload.extend(entropy);
            emit(&escaped_nal(&nal[..2], payload));
        }
    }
    assert!(
        used >= if matches!(mode.as_str(), "mixed" | "invalid-short") {
            1
        } else {
            2
        },
        "fixture must actually use long-term pictures"
    );
    std::fs::write(destination, stream).unwrap();
}

fn check_video(data: &[u8], expected: &[u8], mode: &str) {
    let mut input =
        crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default())
            .unwrap();
    let mut decoder =
        HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
    let mut packet = Vec::new();
    for pass in 0..2 {
        if pass != 0 {
            decoder.reset();
        }
        let mut pixels = Vec::new();
        let mut used = 0;
        for index in 0..3 {
            input.read_packet(0, index, &mut packet).unwrap();
            let (sps, pps) = decoder.parameters();
            assert!(sps.long_term_present);
            if mode == "sps" {
                assert_eq!(sps.long_term, [(0, true), (1, true)]);
            }
            for nal in NalUnits::new(&packet, 4).unwrap() {
                let nal = nal.unwrap();
                if NalHeader::parse(nal).unwrap().is_vcl() {
                    let header = SliceHeader::parse(nal, sps, pps, 16 << 20).unwrap();
                    if mode == "sps" && index != 0 {
                        assert_eq!(header.long_term.len(), 1);
                        assert_eq!(header.long_term[0].poc_lsb, index as u32 - 1);
                    }
                    if index != 0 {
                        if mode == "mixed" && index == 1 {
                            assert!(header.long_term.is_empty());
                            assert_eq!(header.short_term.len(), 1);
                        } else {
                            assert!(header.long_term.iter().any(|r| r.used));
                            assert!(
                                header
                                    .long_term
                                    .iter()
                                    .all(|r| r.msb_cycles.is_none() == (mode == "lsb"))
                            );
                            if mode == "mixed" {
                                assert_eq!(header.short_term.len(), 1);
                                assert!(header.short_term[0].used);
                                assert_eq!(header.short_term[0].delta_poc, -2);
                            } else {
                                assert!(header.short_term.is_empty());
                            }
                            used += 1;
                        }
                    }
                }
            }
            let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
            assert_eq!(decoded.poc, index as i32);
            assert!(decoded.output);
            assert_eq!(decoded.picture.dimensions, [64, 64]);
            assert_eq!(decoded.picture.depth, [8, 8]);
            assert_eq!(decoded.picture.crop, [0; 4]);
            for plane in &decoded.picture.planes {
                pixels.extend(
                    plane
                        .samples()
                        .iter()
                        .map(|&value| u8::try_from(value).unwrap()),
                );
            }
        }
        assert_eq!(used, if mode == "mixed" { 1 } else { 2 });
        assert_eq!(
            pixels, expected,
            "independent HM reconstruction, pass {pass}"
        );
    }
}

#[test]
fn long_term_reference_video_matches_independent_hm_pixels_and_rewind() {
    check_video(
        include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-rext8.mp4"),
        include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-rext8.yuv"),
        "explicit",
    );
}
#[test]
fn lsb_only_long_term_video_matches_independent_hm_pixels_and_rewind() {
    check_video(
        include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-lsb-rext8.mp4"),
        include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-lsb-rext8.yuv"),
        "lsb",
    );
}
#[test]
fn mixed_short_and_long_term_video_matches_independent_hm_pixels_and_rewind() {
    check_video(
        include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-mixed-rext8.mp4"),
        include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-mixed-rext8.yuv"),
        "mixed",
    );
}

#[test]
fn long_term_picture_cannot_be_reused_as_short_term_without_reset() {
    let data = include_bytes!(
        "../../tests/fixtures/playback-errors/hevc-long-term-invalid-short-rext8.mp4"
    );
    let mut input =
        crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default())
            .unwrap();
    let mut decoder =
        HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
    let mut packet = Vec::new();
    for index in 0..2 {
        input.read_packet(0, index, &mut packet).unwrap();
        assert_eq!(
            decoder.decode_packet(&packet).unwrap().unwrap().poc,
            index as i32
        );
    }
    input.read_packet(0, 2, &mut packet).unwrap();
    let (sps, pps) = decoder.parameters();
    let nal = NalUnits::new(&packet, 4)
        .unwrap()
        .map(Result::unwrap)
        .find(|n| NalHeader::parse(n).unwrap().is_vcl())
        .unwrap();
    let header = SliceHeader::parse(nal, sps, pps, 16 << 20).unwrap();
    assert_eq!(header.short_term[0].delta_poc, -2);
    assert!(header.short_term[0].used);
    let error = decoder
        .decode_packet(&packet)
        .err()
        .expect("must refuse invalid reference classification");
    assert!(
        error
            .to_string()
            .contains("long-term picture as short-term"),
        "{error}"
    );
    decoder.reset();
    input.read_packet(0, 0, &mut packet).unwrap();
    assert_eq!(decoder.decode_packet(&packet).unwrap().unwrap().poc, 0);
}

#[test]
fn sps_selected_long_term_video_matches_independent_hm_pixels_and_rewind() {
    check_video(
        include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-sps-rext8.mp4"),
        include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-sps-rext8.yuv"),
        "sps",
    );
}

#[test]
fn b_frames_with_active_mixed_l0_l1_match_hm_pixels_and_rewind() {
    use super::hevc_cabac::SliceType;
    let mut mixed_motion_coverage = [[false; 2]; 2];
    for (data, expected, mixed) in [
        (
            include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-b-base-main8.mp4")
                .as_slice(),
            include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-b-base-main8.yuv")
                .as_slice(),
            false,
        ),
        (
            include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-b-mixed-main8.mp4")
                .as_slice(),
            include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-b-mixed-main8.yuv")
                .as_slice(),
            true,
        ),
        (
            include_bytes!(
                "../../tests/fixtures/playback-errors/hevc-long-term-b-mixed-l1-main8.mp4"
            )
            .as_slice(),
            include_bytes!(
                "../../tests/fixtures/playback-errors/hevc-long-term-b-mixed-l1-main8.yuv"
            )
            .as_slice(),
            true,
        ),
    ] {
        let mut input =
            crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default())
                .unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        for pass in 0..2 {
            if pass != 0 {
                decoder.reset();
            }
            let mut pixels = Vec::new();
            let mut observed = [[false; 2]; 2];
            for index in 0..4 {
                input.read_packet(0, index, &mut packet).unwrap();
                let (sps, pps) = decoder.parameters();
                let nal = NalUnits::new(&packet, 4)
                    .unwrap()
                    .map(Result::unwrap)
                    .find(|n| NalHeader::parse(n).unwrap().is_vcl())
                    .unwrap();
                let header = SliceHeader::parse(nal, sps, pps, 16 << 20).unwrap();
                if index > 0 {
                    assert_eq!(header.slice_type, SliceType::B);
                }
                if index >= 2 {
                    assert_eq!(header.references, [2, 2]);
                    if mixed {
                        assert_eq!(header.short_term.len(), 1);
                        assert_eq!(header.short_term[0].delta_poc, -1);
                        assert!(header.short_term[0].used);
                        assert_eq!(header.long_term.len(), 1);
                        assert_eq!(header.long_term[0].poc_lsb, index as u32 - 2);
                        assert!(header.long_term[0].used);
                    }
                }
                let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                assert_eq!(decoded.poc, index as i32);
                for motion in &decoded.picture.motion {
                    for (list, vector) in motion.iter().enumerate() {
                        if let Some(vector) = vector {
                            observed[list][usize::from(
                                decoded.picture.reference_long_term[list]
                                    [vector.reference as usize],
                            )] = true;
                        }
                    }
                }
                for plane in &decoded.picture.planes {
                    pixels.extend(plane.samples().iter().map(|&v| u8::try_from(v).unwrap()));
                }
            }
            assert_eq!(pixels, expected, "HM oracle, mixed {mixed}, pass {pass}");
            if mixed {
                assert_eq!(observed[0], [true, true]);
                let swapped = decoder.parameters().1.lists_modification;
                assert_eq!(
                    observed[1],
                    if swapped {
                        [false, true]
                    } else {
                        [true, true]
                    }
                );
                for list in 0..2 {
                    for kind in 0..2 {
                        mixed_motion_coverage[list][kind] |= observed[list][kind];
                    }
                }
            }
        }
    }
    assert_eq!(
        mixed_motion_coverage,
        [[true, true], [true, true]],
        "both reference types must be used by mixed B-picture motion in both lists"
    );
}
