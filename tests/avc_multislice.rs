use fvid::{
    codec::{
        avc::{Pps, Sps},
        avc_decoder::AvcDecoder,
        avc_slice::{SliceHeader, SliceType},
        config::{AvcConfig, NalUnits},
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
const VIDEO: &[u8] = include_bytes!("fixtures/playback-errors/avc-multislice-ipb.mp4");
#[test]
fn actual_two_slice_ipb_stream_decodes_all_picture_types() {
    let mut reader = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let config = reader.tracks()[0].configuration.clone();
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
    let mut kinds = [false; 3];
    let mut packet = Vec::new();
    for index in 0..reader.tracks()[0].samples.len() {
        reader.read_packet(0, index, &mut packet).unwrap();
        let slices: Vec<_> = NalUnits::new(&packet, avc.length_size)
            .unwrap()
            .map(|n| n.unwrap())
            .filter(|n| matches!(n[0] & 31, 1 | 5))
            .map(|n| SliceHeader::parse(n, &sps, &pps).unwrap())
            .collect();
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[0].first_mb, 0);
        assert!(slices[1].first_mb > 0);
        assert_eq!(slices[0].frame_num, slices[1].frame_num);
        kinds[match slices[0].slice_type {
            SliceType::I => 0,
            SliceType::P => 1,
            SliceType::B => 2,
            _ => panic!("unexpected type"),
        }] = true;
        assert!(decoder.decode_order(&packet).unwrap().is_some());
    }
    assert_eq!(kinds, [true; 3]);
}
#[test]
fn two_slice_ipb_matches_saved_yuv_and_rewind() {
    let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(VIDEO),
        Default::default(),
        16 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut count = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            frame.picture.write_planar(&mut actual).unwrap();
            count += 1;
        }
        assert_eq!(count, 8);
        assert!(
            actual == include_bytes!("fixtures/playback-errors/avc-multislice-ipb.yuv"),
            "multi-slice AVC differs from independent decoder"
        );
        reader.rewind();
    }
}

#[test]
fn prepare_two_slice_picture_ranges_and_reject_mixed_frames() {
    use fvid::codec::avc_access_unit::prepare;
    let mut reader = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let config = reader.tracks()[0].configuration.clone();
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut packet = Vec::new();
    let mut first = Vec::new();
    let mut raster_decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
    let mut aso_decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
    for index in 0..reader.tracks()[0].samples.len() {
        reader.read_packet(0, index, &mut packet).unwrap();
        let slices = prepare(&packet, avc.length_size, &sps, &pps, packet.len()).unwrap();
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[0].macroblocks, 0..24);
        assert_eq!(slices[1].macroblocks, 24..48);
        assert!(prepare(&packet, avc.length_size, &sps, &pps, packet.len() - 1).is_err());
        let encode = |nal: &[u8], out: &mut Vec<u8>| {
            let length = (nal.len() as u32).to_be_bytes();
            out.extend_from_slice(&length[4 - avc.length_size as usize..]);
            out.extend_from_slice(nal);
        };
        let mut duplicate = Vec::new();
        encode(slices[0].nal, &mut duplicate);
        encode(slices[0].nal, &mut duplicate);
        let error = prepare(&duplicate, avc.length_size, &sps, &pps, duplicate.len())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("overlap"), "{error}");
        let mut reversed = Vec::new();
        encode(slices[1].nal, &mut reversed);
        encode(slices[0].nal, &mut reversed);
        let normalized = prepare(&reversed, avc.length_size, &sps, &pps, reversed.len()).unwrap();
        assert_eq!(normalized.len(), slices.len());
        for (actual, expected) in normalized.iter().zip(&slices) {
            assert_eq!(actual.nal, expected.nal);
            assert_eq!(actual.macroblocks, expected.macroblocks);
        }
        let mut expected_pixels = Vec::new();
        raster_decoder
            .decode_order(&packet)
            .unwrap()
            .unwrap()
            .write_planar(&mut expected_pixels)
            .unwrap();
        let mut aso_pixels = Vec::new();
        aso_decoder
            .decode_order(&reversed)
            .unwrap()
            .unwrap()
            .write_planar(&mut aso_pixels)
            .unwrap();
        assert_eq!(
            aso_pixels, expected_pixels,
            "progressive ASO picture {index}"
        );
        if index == 0 {
            encode(slices[0].nal, &mut first);
        } else {
            let mut mixed = first.clone();
            encode(slices[1].nal, &mut mixed);
            let error = prepare(&mixed, avc.length_size, &sps, &pps, mixed.len())
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("picture identity"), "{error}");
            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
            let error = decoder.decode(&mixed).unwrap_err().to_string();
            assert!(error.contains("picture identity"), "{error}");
        }
    }
}

#[test]
fn reconstruct_two_slice_intra_matches_saved_yuv() {
    let mut reader = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let config = reader.tracks()[0].configuration.clone();
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut packet = Vec::new();
    reader.read_packet(0, 0, &mut packet).unwrap();
    let slices =
        fvid::codec::avc_access_unit::prepare(&packet, avc.length_size, &sps, &pps, 16 << 20)
            .unwrap();
    let headers: Vec<_> = slices.iter().map(|s| &s.header).collect();
    let picture =
        fvid::codec::avc_picture::decode_intra_slices(&headers, &sps, &pps, 16 << 20).unwrap();
    let mut actual = Vec::new();
    picture.write_planar(&mut actual).unwrap();
    assert!(
        actual
            == include_bytes!("fixtures/playback-errors/avc-multislice-ipb.yuv")
                [..128 * 96 * 3 / 2]
    );
}

#[test]
fn multislice_camera_frames_loop_and_seek_back_without_ffmpeg() {
    use fvid::{
        playback_native::NativeReader,
        virtual_camera::{CameraEndBehavior, CameraTick, LatestFrame, NativeCameraSource},
    };
    let mut reader = NativeReader::software(Cursor::new(VIDEO), 16 << 20).unwrap();
    let mut expected = Vec::new();
    let mut duration = 0;
    while reader.read_frame().unwrap() {
        assert_eq!(reader.dimensions(), [128, 96]);
        let (start, end, scale) = reader.frame_interval().unwrap();
        duration = (end * 1_000_000_000).div_ceil(u128::from(scale)) as u64;
        expected.push((
            (start * 1_000_000_000).div_ceil(u128::from(scale)) as u64,
            reader
                .rgb()
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[2], p[1], p[0], 255])
                .collect::<Vec<_>>(),
        ));
    }
    assert_eq!(expected.len(), 8);
    let mut source =
        NativeCameraSource::new(NativeReader::software(Cursor::new(VIDEO), 16 << 20).unwrap())
            .with_end_behavior(CameraEndBehavior::Loop);
    let output = LatestFrame::new(128, 96, 128 * 96 * 4).unwrap();
    let mut pixels = vec![0; 128 * 96 * 4];
    let positions: Vec<_> = (0..3)
        .flat_map(|cycle| {
            expected
                .iter()
                .map(move |(position, reference)| (*position + cycle * duration, reference))
        })
        .chain(
            expected
                .iter()
                .take(1)
                .map(|(position, reference)| (*position, reference)),
        )
        .collect();
    for (sequence, (position, reference)) in positions.into_iter().enumerate() {
        let tick = CameraTick {
            sequence: sequence as u64,
            host_time_ns: sequence as u64 + 1,
            media_time_ns: position,
        };
        assert!(source.publish(tick, &output).unwrap());
        assert_eq!(output.copy_latest(None, &mut pixels).unwrap(), Some(tick));
        assert_eq!(&pixels, reference);
    }
}

#[test]
fn cavlc_and_ten_bit_multislice_ipb_match_independent_yuv() {
    for (video, oracle, cabac, depth) in [
        (
            include_bytes!("fixtures/playback-errors/avc-multislice-cavlc.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-multislice-cavlc.yuv").as_slice(),
            false,
            8,
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-multislice-cabac10.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-multislice-cabac10.yuv").as_slice(),
            true,
            10,
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-multislice-cavlc10.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-multislice-cavlc10.yuv").as_slice(),
            false,
            10,
        ),
    ] {
        let mut packets = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let config = packets.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&config).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        assert_eq!(sps.bit_depth_luma, depth);
        assert_eq!(pps.cabac, cabac);
        let mut packet = Vec::new();
        let mut kinds = [false; 3];
        for index in 0..packets.tracks()[0].samples.len() {
            packets.read_packet(0, index, &mut packet).unwrap();
            let slices = fvid::codec::avc_access_unit::prepare(
                &packet,
                avc.length_size,
                &sps,
                &pps,
                16 << 20,
            )
            .unwrap();
            assert_eq!(slices.len(), 2);
            kinds[match slices[0].header.slice_type {
                SliceType::I => 0,
                SliceType::P => 1,
                SliceType::B => 2,
                _ => panic!("unexpected slice type"),
            }] = true;
        }
        assert_eq!(kinds, [true; 3]);
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        assert!(!reader.hardware_accelerated());
        for _ in 0..2 {
            let mut actual = Vec::new();
            let mut count = 0;
            while let Some(frame) = reader.read_frame().unwrap() {
                frame.picture.write_planar(&mut actual).unwrap();
                count += 1;
            }
            assert_eq!(count, 8);
            assert!(
                actual == oracle,
                "multi-slice cabac={cabac} depth={depth} differs from oracle"
            );
            reader.rewind();
        }
    }
}

#[test]
fn encoded_slice_list_modification_selects_different_reference_pictures() {
    const VIDEO: &[u8] = include_bytes!("fixtures/playback-errors/avc-slice-lists.mp4");
    const ORACLE: &[u8] = include_bytes!("fixtures/playback-errors/avc-slice-lists.yuv");
    assert_eq!(ORACLE.len(), 3 * 32 * 16 * 3 / 2);
    for row in ORACLE[2 * 32 * 16 * 3 / 2..][..32 * 16].as_chunks::<32>().0 {
        assert_eq!(&row[..16], &[180; 16]);
        assert_eq!(&row[16..], &[80; 16]);
    }
    let mut packets = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let config = packets.tracks()[0].configuration.clone();
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut packet = Vec::new();
    packets.read_packet(0, 2, &mut packet).unwrap();
    let slices =
        fvid::codec::avc_access_unit::prepare(&packet, avc.length_size, &sps, &pps, 1 << 20)
            .unwrap();
    assert_eq!(slices.len(), 2);
    assert!(slices[0].header.modifications_l0.is_empty());
    assert_eq!(
        slices[1].header.modifications_l0,
        vec![fvid::codec::avc_slice::RefModification::Subtract(1)]
    );
    let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(VIDEO),
        Default::default(),
        16 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut count = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            frame.picture.write_planar(&mut actual).unwrap();
            count += 1;
        }
        assert_eq!(count, 3);
        assert!(
            actual == ORACLE,
            "slice-local reference selection differs from independent decoder"
        );
        reader.rewind();
    }
}

#[test]
fn temporal_direct_maps_colocated_slice_local_indices_by_picture_identity() {
    const VIDEO: &[u8] = include_bytes!("fixtures/playback-errors/avc-slice-lists-temporal.mp4");
    const ORACLE: &[u8] =
        include_bytes!("fixtures/playback-errors/avc-slice-lists-temporal-jm.yuv");
    assert_eq!(ORACLE.len(), 4 * 32 * 16 * 3 / 2);
    let mut packets = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let config = packets.tracks()[0].configuration.clone();
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut packet = Vec::new();
    packets.read_packet(0, 3, &mut packet).unwrap();
    let slices =
        fvid::codec::avc_access_unit::prepare(&packet, avc.length_size, &sps, &pps, 1 << 20)
            .unwrap();
    assert_eq!(slices.len(), 2);
    for slice in slices {
        assert_eq!(slice.header.slice_type, SliceType::B);
        assert!(!slice.header.direct_spatial_mv_pred);
        assert_eq!(slice.header.refs_l0, 2);
    }
    let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(VIDEO),
        Default::default(),
        16 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut count = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            frame.picture.write_planar(&mut actual).unwrap();
            count += 1;
        }
        assert_eq!(count, 4);
        assert!(
            actual == ORACLE,
            "temporal direct mismatch: actual {:?}, oracle {:?}",
            actual
                .as_chunks::<768>()
                .0
                .iter()
                .map(|f| (f[0], f[16]))
                .collect::<Vec<_>>(),
            ORACLE
                .as_chunks::<768>()
                .0
                .iter()
                .map(|f| (f[0], f[16]))
                .collect::<Vec<_>>()
        );
        reader.rewind();
    }
}

#[test]
fn temporal_direct_matches_analytic_picture_identity() {
    let video = include_bytes!("fixtures/playback-errors/avc-slice-lists-temporal.mp4");
    let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(video),
        Default::default(),
        16 << 20,
    )
    .unwrap();
    let mut actual = Vec::new();
    while let Some(frame) = reader.read_frame().unwrap() {
        frame.picture.write_planar(&mut actual).unwrap();
    }
    // H.264 8.4.1.2.3 maps the picture referenced by the co-located MB,
    // not a picture selected by another slice's local reference index.
    let mut specified = Vec::new();
    for (left, right) in [(80, 80), (180, 180), (180, 80), (180, 80)] {
        for _ in 0..16 {
            specified.extend_from_slice(&[left; 16]);
            specified.extend_from_slice(&[right; 16]);
        }
        specified.extend_from_slice(&[128; 256]);
    }
    assert_eq!(actual, specified);
}

#[test]
fn damaged_last_slice_requires_reset_and_restarts_with_identical_pictures() {
    let mut packets = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let config = packets.tracks()[0].configuration.clone();
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut packet = Vec::new();
    let mut saved = Vec::new();
    for index in 0..packets.tracks()[0].samples.len() {
        packets.read_packet(0, index, &mut packet).unwrap();
        saved.push(packet.clone());
    }
    let nals: Vec<_> = NalUnits::new(&saved[1], avc.length_size)
        .unwrap()
        .map(|n| n.unwrap())
        .collect();
    let last = nals
        .iter()
        .rposition(|n| matches!(n[0] & 31, 1 | 5))
        .unwrap();
    let shortened = &nals[last][..nals[last].len() / 2];
    // Header validation must pass: this test exercises entropy/reconstruction,
    // rather than failing only at the container's NAL length framing.
    SliceHeader::parse(shortened, &sps, &pps).unwrap();
    let mut damaged = Vec::new();
    for (index, nal) in nals.iter().enumerate() {
        let nal = if index == last { shortened } else { nal };
        damaged
            .extend_from_slice(&(nal.len() as u32).to_be_bytes()[4 - avc.length_size as usize..]);
        damaged.extend_from_slice(nal);
    }
    let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
    assert!(decoder.decode_order(&saved[0]).unwrap().is_some());
    let error = decoder.decode_order(&damaged).unwrap_err().to_string();
    assert!(!error.contains("slice header"), "{error}");
    assert!(
        decoder
            .decode_order(&saved[1])
            .unwrap_err()
            .to_string()
            .contains("requires reset")
    );
    decoder.reset();
    let mut fresh = AvcDecoder::new(&config, 16 << 20).unwrap();
    for packet in &saved {
        let restarted = decoder.decode_order(packet).unwrap().unwrap();
        let reference = fresh.decode_order(packet).unwrap().unwrap();
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        restarted.write_planar(&mut actual).unwrap();
        reference.write_planar(&mut expected).unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn mixed_slice_types_match_jm_and_rewind() {
    for (data, expected) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mixed-pb.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mixed-pb-jm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mixed-bp.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mixed-bp-jm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mixed-ip.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mixed-ip-jm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mixed-pi.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mixed-pi-jm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mixed-ib.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mixed-ib-jm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mixed-bi.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mixed-bi-jm.yuv").as_slice(),
        ),
    ] {
        let mut packets = Mp4Reader::open(Cursor::new(data), Default::default()).unwrap();
        let avc = AvcConfig::parse(&packets.tracks()[0].configuration).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        let length_size = avc.length_size;
        let mut last = Vec::new();
        packets.read_packet(0, 3, &mut last).unwrap();
        let slices =
            fvid::codec::avc_access_unit::prepare(&last, length_size, &sps, &pps, 16 << 20)
                .unwrap();
        assert_eq!(slices.len(), 2);
        assert_ne!(slices[0].header.slice_type, slices[1].header.slice_type);
        assert!(slices.iter().all(|s| matches!(
            s.header.slice_type,
            SliceType::I | SliceType::P | SliceType::B
        )));
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(data),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            let mut frames = 0;
            while let Some(frame) = reader.read_frame().unwrap() {
                frame.picture.write_planar(&mut actual).unwrap();
                frames += 1;
            }
            assert_eq!(frames, 4);
            assert!(
                actual.as_slice() == expected,
                "frame luma actual={:?}, expected={:?}",
                actual
                    .chunks(768)
                    .map(|f| (f[0], f[16]))
                    .collect::<Vec<_>>(),
                expected
                    .chunks(768)
                    .map(|f| (f[0], f[16]))
                    .collect::<Vec<_>>()
            );
            reader.rewind();
        }
    }
}

#[test]
fn reordered_picture_api_refusal_is_distinct_from_playback_acceptance() {
    let mut input = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let mut monotonic = AvcDecoder::new(&configuration, 16 << 20).unwrap();
    let mut coded = AvcDecoder::new(&configuration, 16 << 20).unwrap();
    let mut packet = Vec::new();
    let mut refusal = None;
    let count = input.tracks()[0].samples.len();
    for index in 0..count {
        input.read_packet(0, index, &mut packet).unwrap();
        assert!(coded.decode_order(&packet).unwrap().is_some());
        if refusal.is_none() {
            match monotonic.decode(&packet) {
                Ok(Some(_)) => {}
                Ok(None) => panic!("missing picture"),
                Err(error) => refusal = Some(error.to_string()),
            }
        }
    }
    assert!(
        refusal
            .unwrap()
            .contains("use decode_order for reordered pictures")
    );
    monotonic.reset();
    input.read_packet(0, 0, &mut packet).unwrap();
    assert!(monotonic.decode(&packet).unwrap().is_some());
    // Acceptance, independent of the narrow API refusal, includes oracle pixels
    // in presentation order and rewind, using this same owned synthetic stream.
    two_slice_ipb_matches_saved_yuv_and_rewind();
}

#[test]
fn fmo_cavlc_syntax_preserves_group_addresses_and_owned_pcm_pixels() {
    use fvid::codec::avc_macroblock::{IntraCavlcReader, IntraLuma};
    for (video, oracle) in [
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-explicit-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-explicit-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-explicit-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-explicit-pcm-aso.yuv").as_slice(),
        ),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert!(
            matches!(&pps.slice_groups,fvid::codec::avc::SliceGroups::Explicit{groups:2,map} if map==&[0,1,0,1])
        );
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let mut playback = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let mut playback_pixels = Vec::new();
        playback
            .decode_order(&packet)
            .unwrap()
            .unwrap()
            .write_planar(&mut playback_pixels)
            .unwrap();
        assert_eq!(playback_pixels, oracle);
        let mut pixels = vec![0u8; 1536];
        let mut seen = [false; 4];
        for nal in NalUnits::new(&packet, config.length_size)
            .unwrap()
            .map(Result::unwrap)
        {
            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
            // Existing playback constructor must remain an explicit refusal,
            // independent of syntax-only acceptance below.
            assert!(
                IntraCavlcReader::new(&header, &sps, &pps, 4)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("without FMO")
            );
            let mut reader = IntraCavlcReader::new_fmo(&header, &sps, &pps, 4).unwrap();
            for address in [header.first_mb, header.first_mb + 2] {
                let block = reader.read_macroblock().unwrap().unwrap();
                assert_eq!(block.address, address);
                assert!(!seen[address as usize]);
                seen[address as usize] = true;
                let IntraLuma::Pcm { y, cb, cr } = block.luma else {
                    panic!("expected owned PCM")
                };
                for (component, plane) in [y.as_slice(), cb.as_slice(), cr.as_slice()]
                    .into_iter()
                    .enumerate()
                {
                    let side = if component == 0 { 16 } else { 8 };
                    let stride = side * 2;
                    let offset = [0, 1024, 1280][component];
                    for row in 0..side {
                        for column in 0..side {
                            pixels[offset
                                + (address as usize / 2 * side + row) * stride
                                + address as usize % 2 * side
                                + column] = plane[row * side + column] as u8;
                        }
                    }
                }
            }
            assert!(reader.read_macroblock().unwrap().is_none());
        }
        assert!(seen.into_iter().all(|v| v));
        assert_eq!(pixels, oracle);
    }
}

#[test]
fn all_fmo_map_types_reconstruct_owned_pcm_and_reset() {
    for (video, oracle) in [
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-explicit-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-explicit-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-explicit-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-explicit-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type0-dir0-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type0-dir0-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type0-dir0-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type0-dir0-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type1-dir0-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type1-dir0-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type1-dir0-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type1-dir0-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type2-dir0-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type2-dir0-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type2-dir0-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type2-dir0-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type3-dir0-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type3-dir0-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type3-dir0-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type3-dir0-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type3-dir1-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type3-dir1-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type3-dir1-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type3-dir1-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type4-dir0-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type4-dir0-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type4-dir0-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type4-dir0-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type4-dir1-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type4-dir1-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type4-dir1-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type4-dir1-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type5-dir0-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type5-dir0-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type5-dir0-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type5-dir0-pcm.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type5-dir1-pcm-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type5-dir1-pcm-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-type5-dir1-pcm.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-type5-dir1-pcm.yuv").as_slice(),
        ),
    ] {
        let mut reader = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = reader.tracks()[0].configuration.clone();
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert!(!matches!(
            pps.slice_groups,
            fvid::codec::avc::SliceGroups::Single
        ));
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        for _ in 0..2 {
            let mut pixels = Vec::new();
            decoder
                .decode_order(&packet)
                .unwrap()
                .unwrap()
                .write_planar(&mut pixels)
                .unwrap();
            assert_eq!(pixels, oracle);
            decoder.reset();
        }
    }
}

#[test]
fn missing_fmo_group_never_publishes_a_partial_picture() {
    let video = include_bytes!("fixtures/playback-errors/avc-fmo-explicit-pcm.mp4");
    let mut input = Mp4Reader::open(Cursor::new(video.as_slice()), Default::default()).unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let config = AvcConfig::parse(&configuration).unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 0, &mut packet).unwrap();
    let nal = NalUnits::new(&packet, config.length_size)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let mut partial = (nal.len() as u32).to_be_bytes().to_vec();
    partial.extend_from_slice(nal);
    let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
    let error = decoder.decode_order(&partial).err().unwrap();
    assert!(
        error.to_string().contains("incomplete intra picture"),
        "{error}"
    );
    assert!(
        decoder
            .decode_order(&packet)
            .err()
            .unwrap()
            .to_string()
            .contains("requires reset")
    );
    decoder.reset();
    assert!(decoder.decode_order(&packet).unwrap().is_some());
}

#[test]
fn fmo_intra_prediction_dc_residual_and_slice_filters_match_jm() {
    let mut filtered = Vec::new();
    for (name, video, oracle, residual, filter) in [
        (
            "avc-fmo-intra-type0-dir0-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type0-dir0-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type0-dir0-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type1-dir0-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type1-dir0-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type2-dir0-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type2-dir0-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type3-dir0-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir0-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type3-dir1-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type3-dir1-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type4-dir0-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir0-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type4-dir1-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type4-dir1-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type5-dir0-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir0-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type5-dir1-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type5-dir1-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter0-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter0-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter0-dc0-aso.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter0-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter0-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter0-dc0.yuv")
                .as_slice(),
            false,
            0,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter0-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter0-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter0-dc1-aso.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter0-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter0-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter0-dc1.yuv")
                .as_slice(),
            true,
            0,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter1-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter1-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter1-dc0-aso.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter1-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter1-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter1-dc0.yuv")
                .as_slice(),
            false,
            1,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter1-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter1-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter1-dc1-aso.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter1-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter1-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter1-dc1.yuv")
                .as_slice(),
            true,
            1,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter2-dc0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter2-dc0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter2-dc0-aso.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter2-dc0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter2-dc0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter2-dc0.yuv")
                .as_slice(),
            false,
            2,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter2-dc1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter2-dc1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter2-dc1-aso.yuv")
                .as_slice(),
            true,
            2,
        ),
        (
            "avc-fmo-intra-type6-dir0-filter2-dc1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter2-dc1.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-intra-type6-dir0-filter2-dc1.yuv")
                .as_slice(),
            true,
            2,
        ),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let mut anchors = Vec::new();
        let mut pcm = 0;
        let mut predicted = 0;
        for nal in NalUnits::new(&packet, config.length_size)
            .unwrap()
            .map(Result::unwrap)
        {
            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
            assert_eq!(header.slice_qp, 50);
            assert_eq!(header.disable_deblocking_filter_idc, filter);
            if filter != 1 {
                assert_eq!([header.alpha_offset, header.beta_offset], [12, 12]);
            }
            let mut reader =
                fvid::codec::avc_macroblock::IntraCavlcReader::new_fmo(&header, &sps, &pps, 4)
                    .unwrap();
            while let Some(block) = reader.read_macroblock().unwrap() {
                match block.luma {
                    fvid::codec::avc_macroblock::IntraLuma::Pcm { y, cb, cr } => {
                        pcm += 1;
                        anchors.push(block.address as u16);
                        anchors.extend(y);
                        anchors.extend(cb);
                        anchors.extend(cr);
                    }
                    fvid::codec::avc_macroblock::IntraLuma::Block16(2) => {
                        predicted += 1;
                        assert_eq!(
                            block.luma_dc.iter().filter(|v| **v != 0).count(),
                            usize::from(residual)
                        );
                        assert_eq!(block.luma_dc.iter().sum::<i32>(), i32::from(residual));
                    }
                    _ => panic!("unexpected syntax in {name}"),
                }
            }
        }
        assert_eq!((pcm, predicted), (2, 2));
        if filter != 1 {
            filtered.push((name, filter, oracle, anchors));
        }
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        for _ in 0..2 {
            let mut pixels = Vec::new();
            decoder
                .decode_order(&packet)
                .unwrap()
                .unwrap()
                .write_planar(&mut pixels)
                .unwrap();
            assert!(
                pixels == oracle,
                "{name}: difference at {:?}",
                pixels.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    let mut categories = 0;
    for (name, filter, oracle, anchors) in &filtered {
        if *filter != 0 {
            continue;
        }
        let corresponding = name.replace("-filter0-", "-filter2-");
        let other = filtered
            .iter()
            .find(|(name, _, _, _)| *name == corresponding)
            .unwrap();
        assert_eq!(
            *anchors, other.3,
            "PCM anchors must not change across filter modes"
        );
        assert!(*oracle != other.2, "inactive cross-slice filter in {name}");
        categories += 1;
    }
    assert_eq!(categories, 40);
}

#[test]
fn fmo_inter_cavlc_reads_motion_and_skip_in_group_address_order() {
    use fvid::codec::avc_inter_slice::{InterCavlcSlice, InterMacroblock};
    for video in [
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-aso.mp4").as_slice(),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        for index in 1..3 {
            let mut packet = Vec::new();
            input.read_packet(0, index, &mut packet).unwrap();
            let mut seen = [false; 4];
            for nal in NalUnits::new(&packet, config.length_size)
                .unwrap()
                .map(Result::unwrap)
            {
                let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
                assert_eq!(
                    header.slice_type,
                    if index == 1 {
                        SliceType::P
                    } else {
                        SliceType::B
                    }
                );
                let mut reader = InterCavlcSlice::new_fmo(&header, &sps, &pps, 65536).unwrap();
                let first = header.first_mb as usize;
                let InterMacroblock::Coded {
                    address,
                    header: inter,
                    coefficients,
                } = reader.read_macroblock().unwrap().unwrap()
                else {
                    panic!("expected motion")
                };
                assert_eq!(address, first);
                seen[address] = true;
                assert_eq!(inter.partitions.len(), 1);
                assert_eq!(
                    inter.partitions[0].differences[0],
                    if index == 1 { [8, 4] } else { [4, 0] }
                );
                assert!(coefficients.luma_counts.iter().all(|v| *v == 0));
                let InterMacroblock::Skip { address, .. } =
                    reader.read_macroblock().unwrap().unwrap()
                else {
                    panic!("expected skip")
                };
                assert_eq!(address, first + 2);
                seen[address] = true;
                assert!(reader.read_macroblock().unwrap().is_none());
            }
            assert!(seen.into_iter().all(|v| v));
        }
    }
}

#[test]
fn fmo_skip_run_cannot_escape_its_group_and_poisoned_reader_refuses_more_data() {
    let video = include_bytes!("fixtures/playback-errors/avc-fmo-inter-invalid-skip.mp4");
    let mut input = Mp4Reader::open(Cursor::new(video.as_slice()), Default::default()).unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let config = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let pps = Pps::parse(config.pps[0], &sps).unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 1, &mut packet).unwrap();
    let nal = NalUnits::new(&packet, config.length_size)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
    let mut reader =
        fvid::codec::avc_inter_slice::InterCavlcSlice::new_fmo(&header, &sps, &pps, 65536).unwrap();
    assert!(matches!(
        reader.read_macroblock().unwrap(),
        Some(fvid::codec::avc_inter_slice::InterMacroblock::Coded { address: 0, .. })
    ));
    let error = reader.read_macroblock().err().unwrap();
    assert!(
        error.to_string().contains("skip run exceeds slice group"),
        "{error}"
    );
    assert!(
        reader
            .read_macroblock()
            .err()
            .unwrap()
            .to_string()
            .contains("previously failed")
    );
    let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
    let mut idr = Vec::new();
    input.read_packet(0, 0, &mut idr).unwrap();
    assert!(decoder.decode_order(&idr).unwrap().is_some());
    let error = decoder.decode_order(&packet).err().unwrap();
    assert!(
        error.to_string().contains("skip run exceeds slice group"),
        "{error}"
    );
    assert!(
        decoder
            .decode_order(&idr)
            .err()
            .unwrap()
            .to_string()
            .contains("requires reset")
    );
    decoder.reset();
    assert!(decoder.decode_order(&idr).unwrap().is_some());
}

#[test]
fn fmo_inter_playback_matches_jm_and_reset() {
    for (video, oracle) in [
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter0.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter2.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter0.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter2.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter0.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter2.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter0.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter2.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter0.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter2.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter0.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter2.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter0.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter2.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter0.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter2.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter0.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter2.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-aso.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-aso.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter0.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter0.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter2.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1.yuv").as_slice(),
        ),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        for _ in 0..2 {
            let mut frames = Vec::new();
            for index in 0..3 {
                let mut packet = Vec::new();
                input.read_packet(0, index, &mut packet).unwrap();
                let mut pixels = Vec::new();
                decoder
                    .decode_order(&packet)
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut pixels)
                    .unwrap();
                frames.push(([0, 2, 1][index], pixels));
            }
            frames.sort_by_key(|f| f.0);
            let actual: Vec<_> = frames.into_iter().flat_map(|f| f.1).collect();
            assert!(
                actual == oracle,
                "pixel mismatch at {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    for (all, isolated) in [
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter0.yuv").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter2.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-skip-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter0.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter0.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter0.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter0.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter0.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter0.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter0.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter0.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter2.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter0-aso.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter0.yuv")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter2.yuv")
                .as_slice(),
        ),
    ] {
        assert_eq!(&all[..1536], &isolated[..1536]);
        for frame in [1, 2] {
            assert_ne!(
                &all[frame * 1536..frame * 1536 + 512],
                &isolated[frame * 1536..frame * 1536 + 512],
                "active B/P top-row cross-slice filter"
            );
        }
    }
}

#[test]
fn fmo_all_inter_maps_gate_motion_skip_and_complete_group_coverage() {
    for video in [
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-aso.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter0-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter2-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0-filter2.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type0-dir0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-aso.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter0-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter2-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0-filter2.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type1-dir0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-aso.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter0-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter2-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0-filter2.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type2-dir0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-aso.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter0-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter2-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0-filter2.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-aso.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter0-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter2-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1-filter2.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type3-dir1.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-aso.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter0-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter2-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0-filter2.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-aso.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter0-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter2-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1-filter2.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type4-dir1.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-aso.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter0-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter2-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0-filter2.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-aso.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter0-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter0.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter2-aso.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1-filter2.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-fmo-inter-type5-dir1.mp4").as_slice(),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        assert_eq!(sps.max_num_ref_frames, 2);
        for index in 1..3 {
            let mut packet = Vec::new();
            input.read_packet(0, index, &mut packet).unwrap();
            let mut seen = [false; 4];
            for nal in NalUnits::new(&packet, config.length_size)
                .unwrap()
                .map(Result::unwrap)
            {
                let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
                assert_eq!(
                    header.slice_type,
                    if index == 1 {
                        SliceType::P
                    } else {
                        SliceType::B
                    }
                );
                let map = fvid::codec::avc_slice_group_map::map_units(
                    &pps.slice_groups,
                    2,
                    2,
                    header.slice_group_change_cycle,
                    4,
                )
                .unwrap();
                let group = map[header.first_mb as usize];
                let expected: Vec<_> = map
                    .iter()
                    .enumerate()
                    .filter_map(|(i, g)| (*g == group).then_some(i))
                    .collect();
                let mut reader = fvid::codec::avc_inter_slice::InterCavlcSlice::new_fmo(
                    &header, &sps, &pps, 65536,
                )
                .unwrap();
                for (position, address) in expected.iter().enumerate() {
                    let block = reader.read_macroblock().unwrap().unwrap();
                    match block {
                        fvid::codec::avc_inter_slice::InterMacroblock::Coded {
                            address: actual,
                            header: motion,
                            coefficients,
                        } if position == 0 => {
                            assert_eq!(actual, *address);
                            assert_eq!(motion.partitions.len(), 1);
                            let extra = if header.disable_deblocking_filter_idc == 1 {
                                0
                            } else {
                                i32::from(group) * 4
                            };
                            assert_eq!(
                                motion.partitions[0].differences[0],
                                if index == 1 {
                                    [8 + extra, 4]
                                } else {
                                    [4 + extra, 0]
                                }
                            );
                            assert!(coefficients.luma_counts.iter().all(|v| *v == 0));
                        }
                        fvid::codec::avc_inter_slice::InterMacroblock::Skip {
                            address: actual,
                            ..
                        } if position > 0 => assert_eq!(actual, *address),
                        _ => panic!("unexpected group syntax"),
                    }
                    assert!(!seen[*address]);
                    seen[*address] = true;
                }
                assert!(reader.read_macroblock().unwrap().is_none());
            }
            assert!(seen.into_iter().all(|v| v));
        }
    }
}

#[test]
fn fmo_three_to_eight_groups_match_jm_with_motion_skip_and_reset() {
    let mut cases = 0;
    let mut filtered = Vec::new();
    for (name, video, oracle) in [
        (
            "avc-fmo-groups3-type0-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type0-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type0-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type0-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type1-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type1-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type1-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type1-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type2-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type2-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type2-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type2-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type2-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type2-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type2-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type6-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type6-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type6-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type6-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups3-type6-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups3-type6-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups3-type6-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type0-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type0-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type0-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type0-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type1-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type1-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type1-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type1-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type2-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type2-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type2-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type2-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type2-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type2-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type2-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type6-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type6-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type6-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type6-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups4-type6-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups4-type6-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups4-type6-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type0-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type0-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type0-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type0-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type1-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type1-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type1-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type1-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type2-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type2-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type2-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type2-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type2-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type2-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type2-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type6-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type6-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type6-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type6-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups5-type6-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups5-type6-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups5-type6-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type0-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type0-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type0-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type0-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type1-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type1-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type1-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type1-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type2-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type2-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type2-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type2-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type2-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type2-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type2-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type6-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type6-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type6-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type6-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups6-type6-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups6-type6-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups6-type6-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type0-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type0-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type0-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type0-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type1-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type1-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type1-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type1-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type2-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type2-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type2-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type2-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type2-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type2-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type2-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type6-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type6-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type6-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type6-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups7-type6-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups7-type6-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups7-type6-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type0-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type0-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type0-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type0-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type1-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type1-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type1-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type1-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type2-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type2-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type2-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type2-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type2-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type2-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type2-filter2.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type6-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter0-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter0-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type6-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter0.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter0.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type6-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter1-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter1-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type6-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter1.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter1.yuv").as_slice(),
        ),
        (
            "avc-fmo-groups8-type6-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter2-aso.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter2-aso.yuv")
                .as_slice(),
        ),
        (
            "avc-fmo-groups8-type6-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter2.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-fmo-groups8-type6-filter2.yuv").as_slice(),
        ),
    ] {
        cases += 1;
        if name.contains("-filter0") || name.contains("-filter2") {
            filtered.push((name, oracle));
        }
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        assert_eq!(sps.coded_dimensions(), (64, 64));
        let map =
            fvid::codec::avc_slice_group_map::map_units(&pps.slice_groups, 4, 4, None, 16).unwrap();
        let groups = usize::from(*map.iter().max().unwrap()) + 1;
        assert_eq!(
            groups,
            name.split("groups")
                .nth(1)
                .unwrap()
                .chars()
                .next()
                .unwrap()
                .to_digit(10)
                .unwrap() as usize
        );
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        for pass in 0..2 {
            let mut pictures = Vec::new();
            for index in 0..3 {
                let mut packet = Vec::new();
                input.read_packet(0, index, &mut packet).unwrap();
                if pass == 0 && index > 0 {
                    let mut covered = [false; 16];
                    let mut slice_groups = Vec::new();
                    for nal in NalUnits::new(&packet, config.length_size)
                        .unwrap()
                        .map(Result::unwrap)
                    {
                        let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
                        assert_eq!(
                            header.disable_deblocking_filter_idc,
                            name.split("-filter")
                                .nth(1)
                                .unwrap()
                                .chars()
                                .next()
                                .unwrap()
                                .to_digit(10)
                                .unwrap()
                        );
                        let group = map[header.first_mb as usize];
                        slice_groups.push(group);
                        assert_eq!(
                            header.slice_type,
                            if index == 1 {
                                SliceType::P
                            } else {
                                SliceType::B
                            }
                        );
                        let expected: Vec<_> = map
                            .iter()
                            .enumerate()
                            .filter_map(|(i, g)| (*g == group).then_some(i))
                            .collect();
                        let mut reader = fvid::codec::avc_inter_slice::InterCavlcSlice::new_fmo(
                            &header, &sps, &pps, 65536,
                        )
                        .unwrap();
                        for (position, address) in expected.iter().enumerate() {
                            let block = reader.read_macroblock().unwrap().unwrap();
                            match block {
                                fvid::codec::avc_inter_slice::InterMacroblock::Coded {
                                    address: actual,
                                    header: motion,
                                    ..
                                } if position == 0 => {
                                    assert_eq!(actual, *address);
                                    assert_eq!(motion.partitions.len(), 1);
                                    let extra = if header.disable_deblocking_filter_idc == 1 {
                                        0
                                    } else {
                                        i32::from(group) * 4
                                    };
                                    assert_eq!(
                                        motion.partitions[0].differences[0],
                                        if index == 1 {
                                            [8 + extra, 4]
                                        } else {
                                            [4 + extra, 0]
                                        }
                                    );
                                }
                                fvid::codec::avc_inter_slice::InterMacroblock::Skip {
                                    address: actual,
                                    ..
                                } if position > 0 => assert_eq!(actual, *address),
                                _ => panic!("unexpected syntax in {name}"),
                            }
                            assert!(!covered[*address]);
                            covered[*address] = true;
                        }
                        assert!(reader.read_macroblock().unwrap().is_none());
                    }
                    let mut expected: Vec<_> = (0..groups as u8).collect();
                    if name.contains("-aso") {
                        expected.reverse();
                    }
                    assert_eq!(slice_groups, expected);
                    assert!(covered.into_iter().all(|v| v));
                }
                let mut pixels = Vec::new();
                decoder
                    .decode_order(&packet)
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut pixels)
                    .unwrap();
                pictures.push(([0, 2, 1][index], pixels));
            }
            pictures.sort_by_key(|v| v.0);
            let actual: Vec<_> = pictures.into_iter().flat_map(|v| v.1).collect();
            assert!(
                actual == oracle,
                "{name}: mismatch at {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    assert_eq!(cases, 144);
    let mut active = 0;
    for (name, all) in &filtered {
        if !name.contains("-filter0") {
            continue;
        }
        let matching = name.replace("-filter0", "-filter2");
        let isolated = filtered
            .iter()
            .find(|(name, _)| *name == matching)
            .unwrap()
            .1;
        assert_eq!(&all[..6144], &isolated[..6144], "unchanged PCM anchor");
        for frame in [1, 2] {
            assert_ne!(
                &all[frame * 6144..frame * 6144 + 4096],
                &isolated[frame * 6144..frame * 6144 + 4096],
                "active B/P filter in {name}"
            );
        }
        active += 1;
    }
    assert_eq!(active, 48);
}
