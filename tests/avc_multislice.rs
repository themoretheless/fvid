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
        assert!(
            prepare(&reversed, avc.length_size, &sps, &pps, reversed.len())
                .err()
                .unwrap()
                .to_string()
                .contains("macroblock zero")
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
    assert!(decoder
        .decode_order(&saved[1])
        .unwrap_err()
        .to_string()
        .contains("requires reset"));
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
    assert!(refusal
        .unwrap()
        .contains("use decode_order for reordered pictures"));
    monotonic.reset();
    input.read_packet(0, 0, &mut packet).unwrap();
    assert!(monotonic.decode(&packet).unwrap().is_some());
    // Acceptance, independent of the narrow API refusal, includes oracle pixels
    // in presentation order and rewind, using this same owned synthetic stream.
    two_slice_ipb_matches_saved_yuv_and_rewind();
}
