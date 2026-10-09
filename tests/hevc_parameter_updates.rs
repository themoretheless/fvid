use fvid::{
    codec::{bits::BitReader, config::HevcConfig, hevc_decoder::HevcDecoder, hevc_nal::NalRbsp},
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
#[test]
fn changed_in_band_pps_decodes_and_reset_restores_configuration() {
    let mut source = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let config = source.tracks()[0].configuration.clone();
    let parsed = HevcConfig::parse(&config).unwrap();
    let pps = parsed
        .arrays
        .iter()
        .find(|a| a.nal_type == 34)
        .unwrap()
        .units[0];
    let mut rbsp = NalRbsp::parse(pps, 16 << 20).unwrap().bytes;
    let mut bits = BitReader::new(&rbsp);
    bits.unsigned_golomb().unwrap();
    bits.unsigned_golomb().unwrap();
    bits.skip(7).unwrap(); // dependent, output, extra bits, sign hiding, CABAC init
    bits.unsigned_golomb().unwrap();
    bits.unsigned_golomb().unwrap();
    bits.signed_golomb().unwrap();
    let position = bits.position(); // constrained_intra_pred_flag
    rbsp[position / 8] ^= 1 << (7 - position % 8);
    let mut nal = pps[..2].to_vec();
    let mut zeros = 0;
    for byte in rbsp {
        if zeros == 2 && byte <= 3 {
            nal.push(3);
            zeros = 0;
        }
        nal.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    let mut packet = Vec::new();
    source.read_packet(0, 0, &mut packet).unwrap();
    let mut updated = (nal.len() as u32).to_be_bytes().to_vec();
    updated.extend_from_slice(&nal);
    updated.extend_from_slice(&packet);
    assert_eq!(
        updated,
        include_bytes!("fixtures/playback-errors/hevc-pps-update.packet")
    );
    let mut reference = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
    let expected = reference.decode_packet(&packet).unwrap().unwrap();
    let original = reference.parameters().1.constrained_intra;
    let mut decoder = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
    assert_eq!(decoder.slice_headers(&updated).unwrap().len(), 2);
    let actual = decoder.decode_packet(&updated).unwrap().unwrap();
    for i in 0..3 {
        assert_eq!(
            actual.picture.planes[i].samples(),
            expected.picture.planes[i].samples()
        );
    }
    assert_ne!(decoder.parameters().1.constrained_intra, original);
    decoder.reset();
    assert_eq!(decoder.parameters().1.constrained_intra, original);
    assert_eq!(
        decoder
            .decode_packet(&packet)
            .unwrap()
            .unwrap()
            .picture
            .planes[0]
            .samples(),
        expected.picture.planes[0].samples()
    );
}

#[test]
fn changed_pps_after_slices_is_rejected_without_publishing_a_picture() {
    let mut source = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let config = source.tracks()[0].configuration.clone();
    let mut packet = Vec::new();
    source.read_packet(0, 0, &mut packet).unwrap();
    let update = include_bytes!("fixtures/playback-errors/hevc-pps-update.packet");
    let length = u32::from_be_bytes(update[..4].try_into().unwrap()) as usize;
    let mut late = packet.clone();
    late.extend_from_slice(&update[..length + 4]);
    let mut decoder = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
    let original = decoder.parameters().1.clone();
    assert!(decoder.decode_packet(&late).is_err());
    assert_eq!(decoder.parameters().1, &original);
    assert!(decoder.decode_packet(&packet).is_err());
    decoder.reset();
    assert!(decoder.decode_packet(&packet).unwrap().is_some());
}

#[test]
fn parameter_only_packet_persists_until_picture_and_matches_saved_oracle() {
    let mut source = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let config = source.tracks()[0].configuration.clone();
    let updated = include_bytes!("fixtures/playback-errors/hevc-pps-update.packet");
    let length = u32::from_be_bytes(updated[..4].try_into().unwrap()) as usize;
    let mut decoder = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
    let original = decoder.parameters().1.constrained_intra;
    assert!(
        decoder
            .decode_packet(&updated[..length + 4])
            .unwrap()
            .is_none()
    );
    assert_ne!(decoder.parameters().1.constrained_intra, original);
    let mut packet = Vec::new();
    source.read_packet(0, 0, &mut packet).unwrap();
    let picture = decoder.decode_packet(&packet).unwrap().unwrap().picture;
    let bytes: Vec<u8> = picture
        .planes
        .iter()
        .flat_map(|p| p.samples().iter().map(|v| *v as u8))
        .collect();
    assert_eq!(
        bytes,
        include_bytes!("fixtures/playback-errors/hevc-pps-update.yuv")
    );
    assert_ne!(decoder.parameters().1.constrained_intra, original);
}

#[test]
fn truncated_pps_updates_preserve_parameters_and_require_reset() {
    let source = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let config = &source.tracks()[0].configuration;
    let update = include_bytes!("fixtures/playback-errors/hevc-pps-update.packet");
    let length = u32::from_be_bytes(update[..4].try_into().unwrap()) as usize;
    for cut in 1..length {
        let mut truncated = (cut as u32).to_be_bytes().to_vec();
        truncated.extend_from_slice(&update[4..4 + cut]);
        let mut decoder = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
        let original = decoder.parameters().1.clone();
        assert!(decoder.decode_packet(&truncated).is_err(), "cut {cut}");
        assert_eq!(decoder.parameters().1, &original);
        assert!(decoder.decode_packet(update).is_err());
        decoder.reset();
        assert!(decoder.decode_packet(update).unwrap().is_some());
    }
}

#[test]
fn sps_resize_at_random_access_rebuilds_parameters_and_reference_storage() {
    let mut old = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let old_config = old.tracks()[0].configuration.clone();
    let mut new = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-sps-resize.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let new_config = new.tracks()[0].configuration.clone();
    let prefix = parameter_prefix(&new_config);
    let mut decoder = HevcDecoder::from_configuration(&old_config, 16 << 20).unwrap();
    for _ in 0..2 {
        let mut packet = Vec::new();
        for index in 0..old.tracks()[0].samples.len() {
            old.read_packet(0, index, &mut packet).unwrap();
            let picture = decoder.decode_packet(&packet).unwrap().unwrap().picture;
            assert_eq!(picture.dimensions, [128, 128]);
        }
        // Parameter-only updates must persist until the following random-access picture.
        assert!(decoder.decode_packet(&prefix).unwrap().is_none());
        let mut actual = Vec::new();
        for index in 0..new.tracks()[0].samples.len() {
            new.read_packet(0, index, &mut packet).unwrap();
            let picture = decoder.decode_packet(&packet).unwrap().unwrap().picture;
            assert_eq!(picture.dimensions, [96, 64]);
            actual.extend(
                picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples().iter().map(|v| *v as u8)),
            );
        }
        assert_eq!(
            actual,
            include_bytes!("fixtures/playback-errors/hevc-sps-resize.yuv")
        );
        decoder.reset();
    }
}

fn parameter_prefix(config: &[u8]) -> Vec<u8> {
    let parsed = HevcConfig::parse(config).unwrap();
    let mut prefix = Vec::new();
    for array in &parsed.arrays {
        if matches!(array.nal_type, 32..=34) {
            for nal in &array.units {
                prefix.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                prefix.extend_from_slice(nal);
            }
        }
    }
    prefix
}

#[test]
fn sps_resize_prefix_in_picture_packet_matches_oracle() {
    let old = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let mut new = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-sps-resize.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let mut decoder =
        HevcDecoder::from_configuration(&old.tracks()[0].configuration, 16 << 20).unwrap();
    let prefix = parameter_prefix(&new.tracks()[0].configuration);
    let mut actual = Vec::new();
    let mut packet = Vec::new();
    for index in 0..new.tracks()[0].samples.len() {
        new.read_packet(0, index, &mut packet).unwrap();
        let input = if index == 0 {
            [prefix.as_slice(), &packet].concat()
        } else {
            packet.clone()
        };
        assert_eq!(decoder.slice_headers(&input).unwrap().len(), 1);
        let picture = decoder.decode_packet(&input).unwrap().unwrap().picture;
        actual.extend(
            picture
                .planes
                .iter()
                .flat_map(|p| p.samples().iter().map(|v| *v as u8)),
        );
    }
    assert_eq!(
        actual,
        include_bytes!("fixtures/playback-errors/hevc-sps-resize.yuv")
    );
}

#[test]
fn sps_change_without_random_access_or_after_slices_is_rejected() {
    let mut old = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let mut new = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-sps-resize.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let config = &old.tracks()[0].configuration.clone();
    let prefix = parameter_prefix(&new.tracks()[0].configuration);
    let parsed = HevcConfig::parse(&new.tracks()[0].configuration).unwrap();
    let mut without_vps = Vec::new();
    for array in parsed.arrays.iter().filter(|a| matches!(a.nal_type, 33 | 34)) {
        for nal in &array.units {
            without_vps.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            without_vps.extend_from_slice(nal);
        }
    }
    let mut first = Vec::new();
    old.read_packet(0, 0, &mut first).unwrap();
    let mut inter = Vec::new();
    new.read_packet(0, 1, &mut inter).unwrap();
    for (input, expected) in [
        (
            [first.as_slice(), &prefix].concat(),
            "HEVC changed VPS follows picture slices",
        ),
        (
            [first.as_slice(), &without_vps].concat(),
            "HEVC changed SPS follows picture slices",
        ),
        (
            [prefix.as_slice(), &inter].concat(),
            "HEVC SPS change requires a random-access picture",
        ),
    ] {
        let mut decoder = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
        decoder.decode_packet(&first).unwrap().unwrap();
        let error = match decoder.decode_packet(&input) {
            Err(error) => error,
            Ok(_) => panic!("invalid transition accepted"),
        };
        assert!(error.to_string().contains(expected), "{error}");
        assert!(decoder.decode_packet(&first).is_err());
        decoder.reset();
        assert_eq!(
            decoder
                .decode_packet(&first)
                .unwrap()
                .unwrap()
                .picture
                .dimensions,
            [128, 128]
        );
    }
}
