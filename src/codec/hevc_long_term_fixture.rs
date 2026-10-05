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
    let source = include_bytes!(
        "../../tests/fixtures/playback-errors/hevc-rext-explicit-rdpcm-8-skip-disabled.mp4"
    );
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
                payload.splice(
                    sps.long_term_flag_bit..sps.long_term_flag_bit + 1,
                    [true, true],
                );
                emit(&escaped_nal(&nal[..2], payload));
            } else {
                emit(nal);
            }
        }
    }
    let mut packet = Vec::new();
    let mut used = 0;
    assert_eq!(input.tracks()[0].samples.len(), 3);
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
            ue(0, &mut syntax);
            ue(0, &mut syntax); // Empty short-term RPS.
            ue(slice.short_term.len(), &mut syntax);
            let modulus = 1i32 << sps.poc_bits;
            let mut previous_cycle = 0;
            for reference in &slice.short_term {
                let target = poc + reference.delta_poc;
                let lsb = target.rem_euclid(modulus);
                let cycle = (poc - poc.rem_euclid(modulus) - target + lsb) / modulus;
                assert!(cycle >= previous_cycle);
                syntax.extend((0..sps.poc_bits).rev().map(|i| lsb & (1 << i) != 0));
                syntax.push(reference.used);
                syntax.push(true); // Explicit MSB cycle.
                ue((cycle - previous_cycle) as usize, &mut syntax);
                previous_cycle = cycle;
                used += usize::from(reference.used);
            }
            let mut payload = bits(&slice.rbsp);
            let end = slice.entropy_byte_offset * 8;
            let alignment = payload[..end].iter().rposition(|&b| b).unwrap();
            let entropy = payload[end..].to_vec();
            payload.truncate(alignment);
            payload.splice(slice.short_term_bit_range.clone(), syntax);
            payload.push(true);
            while payload.len() % 8 != 0 {
                payload.push(false);
            }
            payload.extend(entropy);
            emit(&escaped_nal(&nal[..2], payload));
        }
    }
    assert!(used >= 2, "fixture must actually use long-term pictures");
    std::fs::write(destination, stream).unwrap();
}

#[test]
fn long_term_reference_video_matches_independent_hm_pixels_and_rewind() {
    let data = include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-rext8.mp4");
    let expected = include_bytes!("../../tests/fixtures/playback-errors/hevc-long-term-rext8.yuv");
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
            for nal in NalUnits::new(&packet, 4).unwrap() {
                let nal = nal.unwrap();
                if NalHeader::parse(nal).unwrap().is_vcl() {
                    let header = SliceHeader::parse(nal, sps, pps, 16 << 20).unwrap();
                    if index != 0 {
                        assert!(header.short_term.is_empty());
                        assert!(header.long_term.iter().any(|r| r.used));
                        used += 1;
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
        assert_eq!(used, 2);
        assert_eq!(
            pixels, expected,
            "independent HM reconstruction, pass {pass}"
        );
    }
}
