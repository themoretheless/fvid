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

#[test]
fn fmo_mixed_inter_intra_dc_matches_jm_and_reset() {
    use fvid::codec::{
        avc_inter_slice::{InterCavlcSlice, InterMacroblock},
        avc_macroblock::IntraLuma,
    };
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "avc-fmo-mixed-residual-type0-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type0-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type0-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type0-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type0-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type0-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type0-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type1-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type1-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type1-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type1-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type1-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type1-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type1-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type2-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type2-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type2-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type2-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type2-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type2-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type2-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir1-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir1-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir1-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type3-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type3-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir1-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir1-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir1-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type4-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type4-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir1-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir1-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir1-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type5-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type5-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type6-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type6-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type6-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type6-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type6-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-residual-type6-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-residual-type6-dir0-filter2-aso.yuv"
            ),
        ),
    ];
    assert_eq!(cases.len(), 60);
    for &(name, video, oracle) in cases {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        for pass in 0..2 {
            let mut pictures = Vec::new();
            for index in 0..3 {
                input.read_packet(0, index, &mut packet).unwrap();
                if index != 0 && pass == 0 {
                    let mut covered = [false; 4];
                    let mut intra = 0;
                    let mut motion = 0;
                    for nal in NalUnits::new(&packet, config.length_size).unwrap() {
                        let header = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                        assert_eq!(
                            header.slice_type,
                            if index == 1 {
                                SliceType::P
                            } else {
                                SliceType::B
                            }
                        );
                        let mut reader =
                            InterCavlcSlice::new_fmo(&header, &sps, &pps, 65536).unwrap();
                        while let Some(block) = reader.read_macroblock().unwrap() {
                            let address = match block {
                                InterMacroblock::Intra(block) => {
                                    assert!(matches!(block.luma, IntraLuma::Block16(2)));
                                    assert_eq!(block.luma_dc.iter().sum::<i32>(), 1);
                                    assert_eq!(
                                        block.luma_dc.iter().filter(|v| **v != 0).count(),
                                        1
                                    );
                                    intra += 1;
                                    block.address as usize
                                }
                                InterMacroblock::Coded { address, .. } => {
                                    motion += 1;
                                    address
                                }
                                InterMacroblock::Skip { address, .. } => address,
                            };
                            assert!(!covered[address], "duplicate {name}");
                            covered[address] = true;
                        }
                    }
                    assert!(covered.into_iter().all(|v| v));
                    assert!(intra > 0 && motion == 2, "mixed syntax {name}");
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
            let pixels: Vec<_> = pictures.into_iter().flat_map(|v| v.1).collect();
            assert!(
                pixels == oracle,
                "{name}: pixel mismatch {:?}",
                pixels.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    let mut pairs = 0;
    for &(name, _, all) in cases {
        if !name.contains("-filter0") {
            continue;
        }
        let other = name.replace("-filter0", "-filter2");
        let isolated = cases.iter().find(|v| v.0 == other).unwrap().2;
        assert_eq!(&all[..1536], &isolated[..1536], "PCM anchor {name}");
        for frame in [1, 2] {
            assert_ne!(
                &all[frame * 1536..frame * 1536 + 1024],
                &isolated[frame * 1536..frame * 1536 + 1024],
                "active mixed filter {name}"
            );
        }
        pairs += 1;
    }
    assert_eq!(pairs, 20);
}

#[test]
fn fmo_mixed_signed_ac_and_chroma_matches_jm_and_reset() {
    use fvid::codec::{
        avc_inter_slice::{InterCavlcSlice, InterMacroblock},
        avc_macroblock::IntraLuma,
    };
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "avc-fmo-mixed-luma-ac-type0-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type0-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type0-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type0-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type0-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type0-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type0-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type1-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type1-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type1-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type1-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type1-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type1-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type1-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type2-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type2-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type2-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type2-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type2-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type2-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type2-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type3-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type3-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type4-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type4-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type5-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type5-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type6-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type6-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type6-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-luma-ac-type6-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type6-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-luma-ac-type6-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-luma-ac-type6-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type0-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type0-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type0-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type0-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type0-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type0-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type0-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type1-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type1-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type1-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type1-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type1-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type1-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type1-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type2-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type2-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type2-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type2-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type2-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type2-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type2-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir1-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir1-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir1-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type3-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type3-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir1-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir1-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir1-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type4-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type4-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir1-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir1-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir1-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type5-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type5-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type6-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type6-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type6-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type6-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type6-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-chroma-dc-type6-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-chroma-dc-type6-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type0-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type0-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type0-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type0-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type0-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type0-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type0-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type1-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type1-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type1-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type1-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type1-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type1-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type1-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type2-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type2-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type2-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type2-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type2-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type2-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type2-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type3-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type3-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type4-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type4-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type5-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type5-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type6-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type6-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type6-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-mixed-all-ac-type6-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type6-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-mixed-all-ac-type6-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-mixed-all-ac-type6-dir0-filter2-aso.yuv"
            ),
        ),
    ];
    assert_eq!(cases.len(), 180);
    for &(name, video, oracle) in cases {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        for pass in 0..2 {
            let mut pictures = Vec::new();
            for index in 0..3 {
                input.read_packet(0, index, &mut packet).unwrap();
                if index != 0 && pass == 0 {
                    let mut covered = [false; 4];
                    let mut intra = 0;
                    let mut motion = 0;
                    for nal in NalUnits::new(&packet, config.length_size).unwrap() {
                        let header = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                        assert_eq!(
                            header.slice_type,
                            if index == 1 {
                                SliceType::P
                            } else {
                                SliceType::B
                            }
                        );
                        let mut reader =
                            InterCavlcSlice::new_fmo(&header, &sps, &pps, 65536).unwrap();
                        while let Some(block) = reader.read_macroblock().unwrap() {
                            let address = match block {
                                InterMacroblock::Intra(block) => {
                                    assert!(matches!(block.luma, IntraLuma::Block16(2)));
                                    assert_eq!(block.luma_dc.iter().sum::<i32>(), 1);
                                    assert_eq!(
                                        block.luma_dc.iter().filter(|v| **v != 0).count(),
                                        1
                                    );
                                    let ac = block
                                        .luma_levels
                                        .iter()
                                        .flatten()
                                        .filter(|v| **v != 0)
                                        .count();
                                    let chroma_dc = block
                                        .chroma_dc
                                        .iter()
                                        .flatten()
                                        .filter(|v| **v != 0)
                                        .count();
                                    let chroma_ac = block
                                        .chroma_ac
                                        .iter()
                                        .flatten()
                                        .flatten()
                                        .filter(|v| **v != 0)
                                        .count();
                                    let expected = if name.contains("luma-ac") {
                                        (16, 0, 0)
                                    } else if name.contains("chroma-dc") {
                                        (0, 2, 0)
                                    } else {
                                        (16, 2, 8)
                                    };
                                    assert_eq!(
                                        (ac, chroma_dc, chroma_ac),
                                        expected,
                                        "residual syntax {name}"
                                    );
                                    if ac != 0 {
                                        assert!(block.luma_levels.iter().flatten().any(|v| *v < 0));
                                        assert!(block.luma_levels.iter().flatten().any(|v| *v > 0));
                                    }
                                    intra += 1;
                                    block.address as usize
                                }
                                InterMacroblock::Coded { address, .. } => {
                                    motion += 1;
                                    address
                                }
                                InterMacroblock::Skip { address, .. } => address,
                            };
                            assert!(!covered[address], "duplicate {name}");
                            covered[address] = true;
                        }
                    }
                    assert!(covered.into_iter().all(|v| v));
                    assert!(intra > 0 && motion == 2, "mixed syntax {name}");
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
            let pixels: Vec<_> = pictures.into_iter().flat_map(|v| v.1).collect();
            assert!(
                pixels == oracle,
                "{name}: pixel mismatch {:?}",
                pixels.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    let mut pairs = 0;
    for &(name, _, all) in cases {
        if !name.contains("-filter0") {
            continue;
        }
        let other = name.replace("-filter0", "-filter2");
        let isolated = cases.iter().find(|v| v.0 == other).unwrap().2;
        assert_eq!(&all[..1536], &isolated[..1536], "PCM anchor {name}");
        for frame in [1, 2] {
            assert_ne!(
                &all[frame * 1536..frame * 1536 + 1024],
                &isolated[frame * 1536..frame * 1536 + 1024],
                "active mixed filter {name}"
            );
        }
        pairs += 1;
    }
    assert_eq!(pairs, 60);
}

#[test]
fn fmo_inter_signed_residual_matches_jm_and_reset() {
    use fvid::codec::avc_inter_slice::{InterCavlcSlice, InterMacroblock};
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "avc-fmo-inter-luma-ac-type0-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type0-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type0-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type0-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type0-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type0-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type0-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type1-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type1-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type1-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type1-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type1-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type1-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type1-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type2-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type2-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type2-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type2-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type2-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type2-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type2-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type3-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type3-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type4-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type4-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type5-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type5-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type6-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type6-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type6-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-luma-ac-type6-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type6-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-luma-ac-type6-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-luma-ac-type6-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type0-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type0-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type0-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type0-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type0-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type0-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type0-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type1-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type1-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type1-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type1-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type1-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type1-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type1-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type2-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type2-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type2-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type2-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type2-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type2-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type2-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir1-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir1-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir1-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type3-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type3-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir1-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir1-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir1-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type4-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type4-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir1-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir1-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir1-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type5-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type5-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type6-dir0-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter0.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type6-dir0-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter1.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type6-dir0-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter2.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type6-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type6-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-chroma-dc-type6-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-chroma-dc-type6-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type0-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type0-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type0-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type0-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type0-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type0-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type0-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type1-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type1-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type1-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type1-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type1-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type1-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type1-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type2-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type2-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type2-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type2-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type2-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type2-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type2-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type3-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type3-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type4-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type4-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir0-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir1-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir1-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir1-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir1-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir1-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type5-dir1-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type5-dir1-filter2-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type6-dir0-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter0.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type6-dir0-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter1.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type6-dir0-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter2.yuv"),
        ),
        (
            "avc-fmo-inter-all-ac-type6-dir0-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter0-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type6-dir0-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter1-aso.yuv"
            ),
        ),
        (
            "avc-fmo-inter-all-ac-type6-dir0-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-fmo-inter-all-ac-type6-dir0-filter2-aso.yuv"
            ),
        ),
    ];
    assert_eq!(cases.len(), 180);
    for &(name, video, oracle) in cases {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        for pass in 0..2 {
            let mut pictures = Vec::new();
            for index in 0..3 {
                input.read_packet(0, index, &mut packet).unwrap();
                if index != 0 && pass == 0 {
                    let mut covered = [false; 4];

                    let mut motion = 0;
                    for nal in NalUnits::new(&packet, config.length_size).unwrap() {
                        let header = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                        assert_eq!(
                            header.slice_type,
                            if index == 1 {
                                SliceType::P
                            } else {
                                SliceType::B
                            }
                        );
                        let mut reader =
                            InterCavlcSlice::new_fmo(&header, &sps, &pps, 65536).unwrap();
                        while let Some(block) = reader.read_macroblock().unwrap() {
                            let address = match block {
                                InterMacroblock::Intra(_) => panic!("unexpected intra {name}"),
                                InterMacroblock::Coded {
                                    address,
                                    header: inter,
                                    coefficients,
                                } => {
                                    assert_eq!(inter.partitions.len(), 1);
                                    assert!(
                                        inter.partitions[0].differences[0].iter().any(|v| *v != 0)
                                    );
                                    let luma = coefficients
                                        .luma4
                                        .iter()
                                        .flatten()
                                        .filter(|v| **v != 0)
                                        .count();
                                    let dc = coefficients
                                        .chroma_dc
                                        .iter()
                                        .flatten()
                                        .filter(|v| **v != 0)
                                        .count();
                                    let ac = coefficients
                                        .chroma_ac
                                        .iter()
                                        .flatten()
                                        .flatten()
                                        .filter(|v| **v != 0)
                                        .count();
                                    let expected = if name.contains("luma-ac") {
                                        (16, 0, 0)
                                    } else if name.contains("chroma-dc") {
                                        (0, 2, 0)
                                    } else {
                                        (16, 2, 8)
                                    };
                                    assert_eq!((luma, dc, ac), expected, "inter residual {name}");
                                    if luma > 0 {
                                        assert!(
                                            coefficients.luma4.iter().flatten().any(|v| *v < 0)
                                        );
                                        assert!(
                                            coefficients.luma4.iter().flatten().any(|v| *v > 0)
                                        );
                                    }
                                    motion += 1;
                                    address
                                }
                                InterMacroblock::Skip { address, .. } => address,
                            };
                            assert!(!covered[address], "duplicate {name}");
                            covered[address] = true;
                        }
                    }
                    assert!(covered.into_iter().all(|v| v));
                    assert!(motion == 2, "inter syntax {name}");
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
            let pixels: Vec<_> = pictures.into_iter().flat_map(|v| v.1).collect();
            assert!(
                pixels == oracle,
                "{name}: pixel mismatch {:?}",
                pixels.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    let mut pairs = 0;
    for &(name, _, all) in cases {
        if !name.contains("-filter0") {
            continue;
        }
        let other = name.replace("-filter0", "-filter2");
        let isolated = cases.iter().find(|v| v.0 == other).unwrap().2;
        assert_eq!(&all[..1536], &isolated[..1536], "PCM anchor {name}");
        for frame in [1, 2] {
            assert_ne!(
                &all[frame * 1536..frame * 1536 + 1024],
                &isolated[frame * 1536..frame * 1536 + 1024],
                "active inter filter {name}"
            );
        }
        pairs += 1;
    }
    assert_eq!(pairs, 60);
}

#[test]
fn mbaff_fmo_pcm_matches_jm_with_physical_pair_modes_and_reset() {
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "avc-mbaff-fmo-frame-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-frame-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-frame-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-frame-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter0-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-frame-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter1-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-frame-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter2-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-frame-filter2-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-field-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-field-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-field-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-field-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter0-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-field-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter1-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-field-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter2-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-field-filter2-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-mixed-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-mixed-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-mixed-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-mixed-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter0-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-mixed-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter1-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-mixed-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter2-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-mixed-filter2-aso.yuv"),
        ),
    ];
    assert_eq!(cases.len(), 18);
    for &(name, video, oracle) in cases {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        assert!(!sps.frame_mbs_only && sps.mb_adaptive_frame_field);
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let mut covered = [false; 8];
        let mut wire = Vec::new();
        for nal in NalUnits::new(&packet, config.length_size).unwrap() {
            let header = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
            assert_eq!(header.slice_type, SliceType::I);
            wire.push(header.first_mb);
            let mut reader =
                fvid::codec::avc_macroblock::IntraCavlcReader::new_fmo(&header, &sps, &pps, 8)
                    .unwrap();
            let expected = if header.first_mb == 0 {
                [0, 1, 4, 5]
            } else {
                [2, 3, 6, 7]
            };
            for address in expected {
                let block = reader.read_macroblock().unwrap().unwrap();
                assert_eq!(block.address, address);
                assert!(matches!(
                    block.luma,
                    fvid::codec::avc_macroblock::IntraLuma::Pcm { .. }
                ));
                let pair = address as usize / 2;
                let field = if name.contains("mixed") {
                    [false, true, true, false][pair]
                } else {
                    name.contains("-field-")
                };
                assert_eq!(
                    reader.field_decoding(),
                    field,
                    "physical pair {name} address {address}"
                );
                assert!(!covered[address as usize]);
                covered[address as usize] = true;
            }
            assert!(reader.read_macroblock().unwrap().is_none());
        }
        assert_eq!(
            wire,
            if name.contains("-aso") {
                vec![1, 0]
            } else {
                vec![0, 1]
            }
        );
        assert!(covered.into_iter().all(|v| v));
        if name == "avc-mbaff-fmo-frame-filter1.mp4" {
            let length = u32::from_be_bytes(packet[..4].try_into().unwrap()) as usize;
            let incomplete = &packet[..4 + length];
            let error = decoder.decode_order(incomplete).err().unwrap();
            assert!(
                error.to_string().contains("incomplete MBAFF intra picture"),
                "{error}"
            );
            assert!(
                decoder.decode_order(&packet).is_err(),
                "failed decoder requires reset"
            );
            decoder.reset();
        }
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
                "{name}: mismatch {:?}",
                pixels.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
}

#[test]
fn mbaff_fmo_intra_dc_matches_jm_with_prediction_and_reset() {
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc0-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter0-dc1-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc0-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter1-dc1-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc0-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-frame-filter2-dc1-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc0-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc0-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc0-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc0-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc1-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc1-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc1-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc1-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc0-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc0-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc0-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc0-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc1-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc1-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc1-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc1-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc0-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc0-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc0-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc0-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc1-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc1-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc1-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc1-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc0-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc0-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc0-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc0-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc0-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc1-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc1-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc1-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter0-dc1-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter0-dc1-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc0-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc0-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc0-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc0-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc0-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc1-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc1-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc1-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter1-dc1-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter1-dc1-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc0-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc0-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc0-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc0-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc0-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc1-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc1-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc1-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-field-filter2-dc1-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-field-filter2-dc1-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc0-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter0-dc1-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc0-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter1-dc1-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc0-l2-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l0-c2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-intra-mixed-filter2-dc1-l2-c2-aso.yuv"
            ),
        ),
    ];
    assert_eq!(cases.len(), 144);
    for &(name, video, oracle) in cases {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        assert!(!sps.frame_mbs_only && sps.mb_adaptive_frame_field);
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let mut covered = [false; 8];
        let mut wire = Vec::new();
        for nal in NalUnits::new(&packet, config.length_size).unwrap() {
            let header = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
            assert_eq!(header.slice_type, SliceType::I);
            wire.push(header.first_mb);
            let mut reader =
                fvid::codec::avc_macroblock::IntraCavlcReader::new_fmo(&header, &sps, &pps, 8)
                    .unwrap();
            let expected = if header.first_mb == 0 {
                [0, 1, 4, 5]
            } else {
                [2, 3, 6, 7]
            };
            for address in expected {
                let block = reader.read_macroblock().unwrap().unwrap();
                assert_eq!(block.address, address);
                if address < 4 {
                    assert!(matches!(
                        block.luma,
                        fvid::codec::avc_macroblock::IntraLuma::Pcm { .. }
                    ));
                } else {
                    assert!(matches!(
                        block.luma,
                        fvid::codec::avc_macroblock::IntraLuma::Block16(mode) if mode == if name.contains("-l0") {0} else {2}
                    ));
                    assert_eq!(block.qp, 50);
                    assert_eq!(
                        matches!(
                            block.chroma_mode,
                            fvid::codec::avc_intra::ChromaMode::Vertical
                        ),
                        name.contains("-c2")
                    );
                    assert_eq!(
                        matches!(block.chroma_mode, fvid::codec::avc_intra::ChromaMode::Dc),
                        name.contains("-c0")
                    );
                    let expected = if name.contains("-dc1") { 1 } else { 0 };
                    assert_eq!(block.luma_dc.iter().filter(|v| **v != 0).count(), expected);
                    assert_eq!(block.luma_dc.iter().sum::<i32>(), expected as i32);
                }
                let pair = address as usize / 2;
                let field = if name.contains("mixed") {
                    [false, true, true, false][pair]
                } else {
                    name.contains("-field-")
                };
                assert_eq!(
                    reader.field_decoding(),
                    field,
                    "physical pair {name} address {address}"
                );
                assert!(!covered[address as usize]);
                covered[address as usize] = true;
            }
            assert!(reader.read_macroblock().unwrap().is_none());
        }
        assert_eq!(
            wire,
            if name.contains("-aso") {
                vec![1, 0]
            } else {
                vec![0, 1]
            }
        );
        assert!(covered.into_iter().all(|v| v));
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
                "{name}: mismatch {:?}",
                pixels.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    let mut pairs = 0;
    for &(name, _, all) in cases {
        if !name.contains("-filter0") {
            continue;
        }
        let other = name.replace("-filter0", "-filter2");
        let isolated = cases.iter().find(|v| v.0 == other).unwrap().2;
        assert_ne!(all, isolated, "active filtering {name}");
        pairs += 1;
    }
    assert_eq!(pairs, 48);
    let mut predictions = 0;
    for &(name, _, vertical) in cases {
        if !name.contains("-filter1") || !name.contains("-l0-c0") {
            continue;
        }
        let luma_dc = cases
            .iter()
            .find(|v| v.0 == name.replace("-l0-c0", "-l2-c0"))
            .unwrap()
            .2;
        let chroma_vertical = cases
            .iter()
            .find(|v| v.0 == name.replace("-l0-c0", "-l0-c2"))
            .unwrap()
            .2;
        assert_ne!(
            &vertical[..2048],
            &luma_dc[..2048],
            "active luma prediction {name}"
        );
        assert_ne!(
            &vertical[2048..],
            &chroma_vertical[2048..],
            "active chroma prediction {name}"
        );
        predictions += 1;
    }
    assert_eq!(predictions, 12);
}

#[test]
fn mbaff_fmo_inter_matches_jm_and_reset() {
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "avc-mbaff-fmo-inter-frame-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter0-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter1-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter2-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter2-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter0-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter1-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter2-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter2-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter0-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter1-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter2-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter2-aso.yuv"),
        ),
    ];
    assert_eq!(cases.len(), 18);
    for &(name, video, oracle) in cases {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        assert!(!sps.frame_mbs_only && sps.mb_adaptive_frame_field);
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        for _ in 0..2 {
            let mut frames = Vec::new();
            for index in 0..3 {
                input.read_packet(0, index, &mut packet).unwrap();
                if index > 0 {
                    let mut covered = [false; 8];
                    let mut wire = Vec::new();
                    for nal in NalUnits::new(&packet, config.length_size).unwrap() {
                        let header = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                        assert_eq!(
                            header.slice_type,
                            if index == 1 {
                                SliceType::P
                            } else {
                                SliceType::B
                            }
                        );
                        wire.push(header.first_mb);
                        let mut reader = fvid::codec::avc_inter_slice::InterCavlcSlice::new_fmo(
                            &header, &sps, &pps, 65536,
                        )
                        .unwrap();
                        let expected = if header.first_mb == 0 {
                            [0, 1, 4, 5]
                        } else {
                            [2, 3, 6, 7]
                        };
                        for address in expected {
                            let Some(fvid::codec::avc_inter_slice::InterMacroblock::Coded {
                                address: actual,
                                header: inter,
                                coefficients,
                            }) = reader.read_macroblock().unwrap()
                            else {
                                panic!("expected motion {name}")
                            };
                            assert_eq!(actual, address);
                            let pair = address / 2;
                            let field = if name.contains("mixed") {
                                [false, true, true, false][pair]
                            } else {
                                name.contains("-field-")
                            };
                            assert_eq!(reader.field_decoding(), field);
                            let mvd = if address % 2 == 0 || field {
                                [8 + header.first_mb as i32 * 4, 4]
                            } else {
                                [0, 0]
                            };
                            assert_eq!(inter.partitions[0].differences[0], mvd);
                            assert!(coefficients.luma_counts.iter().all(|v| *v == 0));
                            assert!(!covered[address]);
                            covered[address] = true;
                        }
                        assert!(reader.read_macroblock().unwrap().is_none());
                    }
                    assert_eq!(
                        wire,
                        if name.contains("-aso") {
                            vec![1, 0]
                        } else {
                            vec![0, 1]
                        }
                    );
                    assert!(covered.into_iter().all(|v| v));
                }
                let mut pixels = Vec::new();
                decoder
                    .decode_order(&packet)
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut pixels)
                    .unwrap();
                frames.push(([0, 2, 1][index], pixels));
            }
            frames.sort_by_key(|v| v.0);
            let actual: Vec<_> = frames.into_iter().flat_map(|v| v.1).collect();
            assert!(
                actual == oracle,
                "{name}: mismatch {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    let mut pairs = 0;
    for &(name, _, all) in cases {
        if !name.contains("filter0") {
            continue;
        }
        let other = name.replace("filter0", "filter2");
        let isolated = cases.iter().find(|v| v.0 == other).unwrap().2;
        assert_eq!(&all[..3072], &isolated[..3072]);
        for frame in [1, 2] {
            assert_ne!(
                &all[frame * 3072..frame * 3072 + 2048],
                &isolated[frame * 3072..frame * 3072 + 2048],
                "active inter filtering {name}"
            );
        }
        pairs += 1;
    }
    assert_eq!(pairs, 6);
}

#[test]
fn mbaff_fmo_skip_matches_jm_and_reset() {
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "avc-mbaff-fmo-skip-frame-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-frame-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-frame-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-frame-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter0-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-frame-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter1-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-frame-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter2-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-frame-filter2-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-field-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-field-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-field-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-field-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter0-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-field-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter1-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-field-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter2-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-field-filter2-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-mixed-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter0.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-mixed-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter1.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-mixed-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter2.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-mixed-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter0-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-mixed-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter1-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-skip-mixed-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter2-aso.mp4"),
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-skip-mixed-filter2-aso.yuv"),
        ),
    ];
    assert_eq!(cases.len(), 18);
    for &(name, video, oracle) in cases {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        assert!(!sps.frame_mbs_only && sps.mb_adaptive_frame_field);
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        for _ in 0..2 {
            let mut frames = Vec::new();
            for index in 0..3 {
                input.read_packet(0, index, &mut packet).unwrap();
                if index > 0 {
                    let mut covered = [false; 8];
                    let mut wire = Vec::new();
                    for nal in NalUnits::new(&packet, config.length_size).unwrap() {
                        let header = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                        assert_eq!(
                            header.slice_type,
                            if index == 1 {
                                SliceType::P
                            } else {
                                SliceType::B
                            }
                        );
                        wire.push(header.first_mb);
                        let mut reader = fvid::codec::avc_inter_slice::InterCavlcSlice::new_fmo(
                            &header, &sps, &pps, 65536,
                        )
                        .unwrap();
                        let expected = if header.first_mb == 0 {
                            [0, 1, 4, 5]
                        } else {
                            [2, 3, 6, 7]
                        };
                        for address in expected {
                            let block = reader.read_macroblock().unwrap().unwrap();
                            let pair = address / 2;
                            let field = if name.contains("mixed") {
                                [false, true, true, false][pair]
                            } else {
                                name.contains("-field-")
                            };
                            assert_eq!(
                                reader.field_decoding(),
                                field,
                                "pair mode {name} address {address}"
                            );
                            match block {
                                fvid::codec::avc_inter_slice::InterMacroblock::Skip {
                                    address: actual,
                                    ..
                                } => {
                                    assert_eq!(address % 2, 1);
                                    assert_eq!(actual, address);
                                }
                                fvid::codec::avc_inter_slice::InterMacroblock::Coded {
                                    address: actual,
                                    header: inter,
                                    coefficients,
                                } => {
                                    assert_eq!(address % 2, 0);
                                    assert_eq!(actual, address);
                                    assert_eq!(
                                        inter.partitions[0].differences[0],
                                        [8 + header.first_mb as i32 * 4, 4]
                                    );
                                    assert!(coefficients.luma_counts.iter().all(|v| *v == 0));
                                }
                                _ => panic!("unexpected macroblock {name}"),
                            }
                            assert!(!covered[address]);
                            covered[address] = true;
                        }
                        assert!(reader.read_macroblock().unwrap().is_none());
                    }
                    assert_eq!(
                        wire,
                        if name.contains("-aso") {
                            vec![1, 0]
                        } else {
                            vec![0, 1]
                        }
                    );
                    assert!(covered.into_iter().all(|v| v));
                }
                let mut pixels = Vec::new();
                decoder
                    .decode_order(&packet)
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut pixels)
                    .unwrap();
                frames.push(([0, 2, 1][index], pixels));
            }
            frames.sort_by_key(|v| v.0);
            let actual: Vec<_> = frames.into_iter().flat_map(|v| v.1).collect();
            assert!(
                actual == oracle,
                "{name}: mismatch {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    let mut pairs = 0;
    for &(name, _, all) in cases {
        if !name.contains("filter0") {
            continue;
        }
        let other = name.replace("filter0", "filter2");
        let isolated = cases.iter().find(|v| v.0 == other).unwrap().2;
        assert_eq!(&all[..3072], &isolated[..3072]);
        for frame in [1, 2] {
            assert_ne!(
                &all[frame * 3072..frame * 3072 + 2048],
                &isolated[frame * 3072..frame * 3072 + 2048],
                "active skip filtering {name}"
            );
        }
        pairs += 1;
    }
    assert_eq!(pairs, 6);
    let explicit: &[(&str, &[u8])] = &[
        (
            "avc-mbaff-fmo-inter-frame-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-frame-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-frame-filter2-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-field-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-field-filter2-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter0.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter0.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter1.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter1.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter2.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter2.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter0-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter0-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter1-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter1-aso.yuv"),
        ),
        (
            "avc-mbaff-fmo-inter-mixed-filter2-aso.mp4",
            include_bytes!("fixtures/playback-errors/avc-mbaff-fmo-inter-mixed-filter2-aso.yuv"),
        ),
    ];
    for &(name, _, skip) in cases {
        let other = name.replace("fmo-skip-", "fmo-inter-");
        let coded = explicit.iter().find(|v| v.0 == other).unwrap().1;
        assert_eq!(&skip[..3072], &coded[..3072], "same IDR anchor");
        assert_ne!(
            &skip[3072..5120],
            &coded[3072..5120],
            "active direct B prediction {name}"
        );
    }
}

#[test]
fn mbaff_fmo_residual_matches_jm_and_reset() {
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "avc-mbaff-fmo-residual-luma-frame-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-frame-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-frame-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-frame-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter1.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-frame-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter1.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-frame-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter1.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-frame-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-frame-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-frame-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-frame-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-frame-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-frame-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-frame-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter1-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-frame-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter1-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-frame-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter1-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-frame-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-frame-filter2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-frame-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-frame-filter2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-frame-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-frame-filter2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-field-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-field-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-field-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-field-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter1.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-field-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter1.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-field-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter1.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-field-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-field-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-field-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-field-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-field-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-field-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-field-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter1-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-field-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter1-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-field-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter1-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-field-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-field-filter2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-field-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-field-filter2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-field-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-field-filter2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-mixed-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-mixed-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-mixed-filter0.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter0.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter0.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-mixed-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter1.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-mixed-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter1.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-mixed-filter1.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter1.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter1.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-mixed-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-mixed-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-mixed-filter2.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter2.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter2.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-mixed-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-mixed-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-mixed-filter0-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter0-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter0-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-mixed-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter1-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-mixed-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter1-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-mixed-filter1-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter1-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter1-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-luma-mixed-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-luma-mixed-filter2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-chroma-mixed-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-chroma-mixed-filter2-aso.yuv"
            ),
        ),
        (
            "avc-mbaff-fmo-residual-combined-mixed-filter2-aso.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter2-aso.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-fmo-residual-combined-mixed-filter2-aso.yuv"
            ),
        ),
    ];
    assert_eq!(cases.len(), 54);
    for &(name, video, oracle) in cases {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        assert!(!sps.frame_mbs_only && sps.mb_adaptive_frame_field);
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        for _ in 0..2 {
            let mut frames = Vec::new();
            for index in 0..3 {
                input.read_packet(0, index, &mut packet).unwrap();
                if index > 0 {
                    let mut covered = [false; 8];
                    let mut wire = Vec::new();
                    for nal in NalUnits::new(&packet, config.length_size).unwrap() {
                        let header = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                        assert_eq!(
                            header.slice_type,
                            if index == 1 {
                                SliceType::P
                            } else {
                                SliceType::B
                            }
                        );
                        wire.push(header.first_mb);
                        let mut reader = fvid::codec::avc_inter_slice::InterCavlcSlice::new_fmo(
                            &header, &sps, &pps, 65536,
                        )
                        .unwrap();
                        let expected = if header.first_mb == 0 {
                            [0, 1, 4, 5]
                        } else {
                            [2, 3, 6, 7]
                        };
                        for address in expected {
                            let Some(fvid::codec::avc_inter_slice::InterMacroblock::Coded {
                                address: actual,
                                header: inter,
                                coefficients,
                            }) = reader.read_macroblock().unwrap()
                            else {
                                panic!("expected motion {name}")
                            };
                            assert_eq!(actual, address);
                            let pair = address / 2;
                            let field = if name.contains("mixed") {
                                [false, true, true, false][pair]
                            } else {
                                name.contains("-field-")
                            };
                            assert_eq!(reader.field_decoding(), field);
                            let mvd = if address % 2 == 0 || field {
                                [8 + header.first_mb as i32 * 4, 4]
                            } else {
                                [0, 0]
                            };
                            assert_eq!(inter.partitions[0].differences[0], mvd);
                            let luma = coefficients
                                .luma4
                                .iter()
                                .flatten()
                                .filter(|v| **v != 0)
                                .count();
                            let dc = coefficients
                                .chroma_dc
                                .iter()
                                .flatten()
                                .filter(|v| **v != 0)
                                .count();
                            let ac = coefficients
                                .chroma_ac
                                .iter()
                                .flatten()
                                .flatten()
                                .filter(|v| **v != 0)
                                .count();
                            let expected = if name.contains("-luma-") {
                                (16, 0, 0)
                            } else if name.contains("-chroma-") {
                                (0, 2, 0)
                            } else {
                                (16, 2, 8)
                            };
                            assert_eq!(
                                (luma, dc, ac),
                                expected,
                                "residual {name} address {address}"
                            );
                            if luma > 0 {
                                assert!(coefficients.luma4.iter().flatten().any(|v| *v < 0));
                                assert!(coefficients.luma4.iter().flatten().any(|v| *v > 0));
                            }
                            assert!(!covered[address]);
                            covered[address] = true;
                        }
                        assert!(reader.read_macroblock().unwrap().is_none());
                    }
                    assert_eq!(
                        wire,
                        if name.contains("-aso") {
                            vec![1, 0]
                        } else {
                            vec![0, 1]
                        }
                    );
                    assert!(covered.into_iter().all(|v| v));
                }
                let mut pixels = Vec::new();
                decoder
                    .decode_order(&packet)
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut pixels)
                    .unwrap();
                frames.push(([0, 2, 1][index], pixels));
            }
            frames.sort_by_key(|v| v.0);
            let actual: Vec<_> = frames.into_iter().flat_map(|v| v.1).collect();
            assert!(
                actual == oracle,
                "{name}: mismatch {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
    let mut pairs = 0;
    for &(name, _, all) in cases {
        if !name.contains("filter0") {
            continue;
        }
        let other = name.replace("filter0", "filter2");
        let isolated = cases.iter().find(|v| v.0 == other).unwrap().2;
        assert_eq!(&all[..3072], &isolated[..3072]);
        for frame in [1, 2] {
            assert_ne!(
                &all[frame * 3072..frame * 3072 + 2048],
                &isolated[frame * 3072..frame * 3072 + 2048],
                "active inter filtering {name}"
            );
        }
        pairs += 1;
    }
    assert_eq!(pairs, 18);
}

#[test]
fn separate_pcm_fields_match_jm_with_native_pairing_and_reset() {
    use fvid::codec::avc_field_picture::{decode_pcm_slices, weave_pair};
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "avc-field-pcm-8bit-top-first.mp4",
            include_bytes!("fixtures/playback-errors/avc-field-pcm-8bit-top-first.mp4"),
            include_bytes!("fixtures/playback-errors/avc-field-pcm-8bit-top-first.yuv"),
        ),
        (
            "avc-field-pcm-8bit-top-first-mixed-long.mp4",
            include_bytes!("fixtures/playback-errors/avc-field-pcm-8bit-top-first-mixed-long.mp4"),
            include_bytes!("fixtures/playback-errors/avc-field-pcm-8bit-top-first-mixed-long.yuv"),
        ),
        (
            "avc-field-pcm-8bit-bottom-first.mp4",
            include_bytes!("fixtures/playback-errors/avc-field-pcm-8bit-bottom-first.mp4"),
            include_bytes!("fixtures/playback-errors/avc-field-pcm-8bit-bottom-first.yuv"),
        ),
        (
            "avc-field-pcm-8bit-bottom-first-mixed-long.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-field-pcm-8bit-bottom-first-mixed-long.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-field-pcm-8bit-bottom-first-mixed-long.yuv"
            ),
        ),
        (
            "avc-field-pcm-10bit-top-first.mp4",
            include_bytes!("fixtures/playback-errors/avc-field-pcm-10bit-top-first.mp4"),
            include_bytes!("fixtures/playback-errors/avc-field-pcm-10bit-top-first.yuv"),
        ),
        (
            "avc-field-pcm-10bit-top-first-mixed-long.mp4",
            include_bytes!("fixtures/playback-errors/avc-field-pcm-10bit-top-first-mixed-long.mp4"),
            include_bytes!("fixtures/playback-errors/avc-field-pcm-10bit-top-first-mixed-long.yuv"),
        ),
        (
            "avc-field-pcm-10bit-bottom-first.mp4",
            include_bytes!("fixtures/playback-errors/avc-field-pcm-10bit-bottom-first.mp4"),
            include_bytes!("fixtures/playback-errors/avc-field-pcm-10bit-bottom-first.yuv"),
        ),
        (
            "avc-field-pcm-10bit-bottom-first-mixed-long.mp4",
            include_bytes!(
                "fixtures/playback-errors/avc-field-pcm-10bit-bottom-first-mixed-long.mp4"
            ),
            include_bytes!(
                "fixtures/playback-errors/avc-field-pcm-10bit-bottom-first-mixed-long.yuv"
            ),
        ),
    ];
    assert_eq!(cases.len(), 8);
    for &(name, video, oracle) in cases {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let mut buffer = fvid::codec::avc_field_dpb::FieldBuffer::new(
            sps.frame_num_bits,
            sps.max_num_ref_frames,
        )
        .unwrap();
        let mut fields = Vec::new();
        let mut packet = Vec::new();
        for index in 0..2 {
            input.read_packet(0, index, &mut packet).unwrap();
            let nal = NalUnits::new(&packet, config.length_size)
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
            assert!(header.field_pic);
            assert_eq!(
                header.bottom_field,
                if name.contains("bottom-first") {
                    index == 0
                } else {
                    index == 1
                }
            );
            assert_eq!(header.idr, index == 0);
            if index == 1 && name.contains("mixed-long") {
                assert_eq!(
                    header.memory_operations,
                    vec![
                        fvid::codec::avc_slice::MemoryOperation::LimitLong(3),
                        fvid::codec::avc_slice::MemoryOperation::ShortToLong {
                            difference: 0,
                            index: 2
                        }
                    ]
                );
            }

            let field = decode_pcm_slices(&[&header], &sps, &pps, 1538).unwrap();
            assert_eq!(
                (field.picture.coded_width, field.picture.coded_height),
                (32, 16)
            );
            assert!(decode_pcm_slices(&[&header], &sps, &pps, 1537).is_err());
            let mut unsupported = SliceHeader::parse(nal, &sps, &pps).unwrap();
            let bit = unsupported.entropy_bit_offset;
            unsupported.rbsp[bit / 8] |= 1 << (7 - bit % 8);
            assert!(
                decode_pcm_slices(&[&unsupported], &sps, &pps, 1538)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("non-PCM AVC field")
            );
            buffer
                .finish(
                    &header,
                    index as i32,
                    7 + index as u64,
                    std::sync::Arc::new(index),
                )
                .unwrap();
            assert_eq!(**buffer.get(7, header.bottom_field).unwrap(), index);
            fields.push(field);
        }
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        for _ in 0..2 {
            input.read_packet(0, 0, &mut packet).unwrap();
            assert!(decoder.decode_order(&packet).unwrap().is_none());
            assert!(decoder.has_pending_field() && !decoder.output_is_field_pair());
            input.read_packet(0, 1, &mut packet).unwrap();
            let picture = decoder.decode_order(&packet).unwrap().unwrap();
            assert!(!decoder.has_pending_field() && decoder.output_is_field_pair());
            let mut pixels = Vec::new();
            picture.write_planar(&mut pixels).unwrap();
            assert!(pixels == oracle, "native field pair {name}");
            decoder.reset();
            assert!(!decoder.has_pending_field() && !decoder.output_is_field_pair());
        }
        let mut playback = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let frame = playback.read_frame().unwrap().unwrap();
            assert_eq!(frame.sample_index, 0, "{name}");
            assert_eq!(frame.presentation_time.ticks, 0, "{name}");
            assert_eq!(frame.duration.ticks, 2, "{name}");
            assert_eq!(frame.duration.timescale, 50, "{name}");
            let mut pixels = Vec::new();
            frame.picture.write_planar(&mut pixels).unwrap();
            assert!(pixels == oracle, "playback field pair {name}");
            assert!(playback.read_frame().unwrap().is_none());
            playback.rewind();
        }
        let expected_reference = fvid::codec::avc_field_references::FieldFrameReference {
            id: 7,
            frame_num: fields[0].frame_num,
            top_poc: Some(if name.contains("bottom-first") { 1 } else { 0 }),
            bottom_poc: Some(if name.contains("bottom-first") { 0 } else { 1 }),
            long_term_indices: if name.contains("mixed-long") {
                if name.contains("bottom-first") {
                    [None, Some(2)]
                } else {
                    [Some(2), None]
                }
            } else {
                [None; 2]
            },
        };
        assert_eq!(buffer.references(), vec![expected_reference]);
        let mut list_header = SliceHeader::parse(
            NalUnits::new(&packet, config.length_size)
                .unwrap()
                .next()
                .unwrap()
                .unwrap(),
            &sps,
            &pps,
        )
        .unwrap();
        list_header.slice_type = SliceType::P;
        list_header.frame_num = 1;
        list_header.refs_l0 = 2;
        assert_eq!(buffer.lists(&list_header, 2).unwrap().l0.len(), 2);

        let reference = buffer.references()[0];
        let before = buffer.references();
        let nal = NalUnits::new(&packet, config.length_size)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let mut damaged = SliceHeader::parse(nal, &sps, &pps).unwrap();
        damaged.frame_num = 1;
        damaged.adaptive_reference_marking = true;
        damaged.memory_operations = vec![
            fvid::codec::avc_slice::MemoryOperation::LimitLong(0),
            fvid::codec::avc_slice::MemoryOperation::ForgetShort(31),
        ];
        assert!(
            buffer
                .finish(&damaged, 2, 9, std::sync::Arc::new(9usize))
                .is_err()
        );
        assert_eq!(
            buffer.references(),
            before,
            "failed field marking must be atomic"
        );
        for bottom in [false, true] {
            let lists = fvid::codec::avc_field_references::field_lists(
                &[reference],
                sps.frame_num_bits,
                1,
                2,
                bottom,
                SliceType::P,
                [2, 0],
                [&[], &[]],
            )
            .unwrap();
            let expected = if name.contains("mixed-long") {
                vec![
                    fvid::codec::avc_field_references::FieldSelection {
                        id: 7,
                        bottom: !fields[0].bottom,
                    },
                    fvid::codec::avc_field_references::FieldSelection {
                        id: 7,
                        bottom: fields[0].bottom,
                    },
                ]
            } else {
                vec![
                    fvid::codec::avc_field_references::FieldSelection { id: 7, bottom },
                    fvid::codec::avc_field_references::FieldSelection {
                        id: 7,
                        bottom: !bottom,
                    },
                ]
            };
            assert_eq!(lists.l0, expected);
            if name.contains("mixed-long") {
                let number = 4 + u32::from(fields[0].bottom == bottom);
                let selected = fvid::codec::avc_field_references::field_lists(
                    &[reference],
                    sps.frame_num_bits,
                    1,
                    2,
                    bottom,
                    SliceType::P,
                    [1, 0],
                    [
                        &[fvid::codec::avc_slice::RefModification::LongTerm(number)],
                        &[],
                    ],
                )
                .unwrap();
                assert_eq!(
                    selected.l0,
                    vec![fvid::codec::avc_field_references::FieldSelection {
                        id: 7,
                        bottom: fields[0].bottom
                    }]
                );
            }
        }
        for (first, second) in [(&fields[0], &fields[1]), (&fields[1], &fields[0])] {
            let picture = weave_pair(first, second, 3072).unwrap();
            let mut actual = Vec::new();
            picture.write_planar(&mut actual).unwrap();
            assert!(
                actual == oracle,
                "{name}: mismatch {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            assert!(weave_pair(first, second, 3071).is_err());
            assert!(weave_pair(first, first, 3072).is_err());
        }
        if name == "avc-field-pcm-8bit-top-first-mixed-long.mp4" {
            use fvid::codec::avc_slice::MemoryOperation as M;
            damaged.memory_operations = vec![M::CurrentLong(2)];
            buffer
                .finish(&damaged, 2, 9, std::sync::Arc::new(9usize))
                .unwrap();
            assert!(buffer.get(7, false).is_none());
            assert!(buffer.get(7, true).is_some());
            assert_eq!(
                buffer
                    .references()
                    .iter()
                    .find(|r| r.id == 9)
                    .unwrap()
                    .long_term_indices,
                [None, Some(2)]
            );
            damaged.frame_num = 2;
            damaged.bottom_field = false;
            damaged.memory_operations = vec![M::ForgetLong(4)];
            buffer
                .finish(&damaged, 4, 10, std::sync::Arc::new(10usize))
                .unwrap();
            assert!(buffer.get(9, true).is_none());
            damaged.frame_num = 3;
            damaged.memory_operations = vec![M::ForgetShort(4)]; // CurrPicNum 7 -> frame 1? missing
            let unchanged = buffer.references();
            assert!(
                buffer
                    .finish(&damaged, 6, 11, std::sync::Arc::new(11usize))
                    .is_err()
            );
            assert_eq!(buffer.references(), unchanged);
            damaged.memory_operations = vec![M::ForgetShort(0)]; // opposite field? frame 3 missing
            assert!(
                buffer
                    .finish(&damaged, 6, 11, std::sync::Arc::new(11usize))
                    .is_err()
            );
            damaged.memory_operations = vec![M::ForgetShort(1)]; // frame 2 top has PicNum 5
            buffer
                .finish(&damaged, 6, 11, std::sync::Arc::new(11usize))
                .unwrap();
            assert!(buffer.get(10, false).is_none());
            damaged.frame_num = 4;
            damaged.memory_operations = vec![M::Reset];
            buffer
                .finish(&damaged, 0, 12, std::sync::Arc::new(12usize))
                .unwrap();
            assert_eq!(buffer.references().len(), 1);
            assert_eq!(buffer.references()[0].frame_num, 0);
            let mut anchor = Vec::new();
            input.read_packet(0, 0, &mut anchor).unwrap();
            let first_nal = NalUnits::new(&anchor, config.length_size)
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            let mut first_header = SliceHeader::parse(first_nal, &sps, &pps).unwrap();
            let mut window =
                fvid::codec::avc_field_dpb::FieldBuffer::new(sps.frame_num_bits, 1).unwrap();
            window
                .finish(&first_header, 0, 20, std::sync::Arc::new(0usize))
                .unwrap();
            damaged.frame_num = 0;
            damaged.bottom_field = true;
            damaged.adaptive_reference_marking = false;
            damaged.memory_operations.clear();
            window
                .finish(&damaged, 1, 21, std::sync::Arc::new(1usize))
                .unwrap();
            assert_eq!(window.references().len(), 1);
            assert!(window.get(20, false).is_some() && window.get(20, true).is_some());
            damaged.frame_num = 1;
            damaged.bottom_field = false;
            window
                .finish(&damaged, 2, 22, std::sync::Arc::new(2usize))
                .unwrap();
            assert!(window.get(20, false).is_none());
            damaged.bottom_field = true;
            window
                .finish(&damaged, 3, 23, std::sync::Arc::new(3usize))
                .unwrap();
            assert!(window.get(22, false).is_some() && window.get(22, true).is_some());
            first_header.long_term_reference = true;
            let mut locked =
                fvid::codec::avc_field_dpb::FieldBuffer::new(sps.frame_num_bits, 1).unwrap();
            locked
                .finish(&first_header, 0, 30, std::sync::Arc::new(0usize))
                .unwrap();
            let unchanged = locked.references();
            assert!(
                locked
                    .finish(&damaged, 3, 31, std::sync::Arc::new(1usize))
                    .is_err()
            );
            assert_eq!(locked.references(), unchanged);
        }
    }
}

#[test]
fn unpaired_pcm_field_is_not_silently_discarded_at_eof() {
    let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(
            include_bytes!("fixtures/playback-errors/avc-field-pcm-unpaired.mp4").as_slice(),
        ),
        Default::default(),
        16 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let error = reader.read_frame().err().expect("missing field must fail");
        assert!(
            error
                .to_string()
                .contains("unpaired AVC field at end of MP4"),
            "{error}"
        );
        reader.rewind();
    }
}

#[test]
fn separate_skip_fields_match_jm_with_native_references_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            let name = format!("avc-field-skip-{depth}bit-{order}");
            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
            let mut input = Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
            let config = input.tracks()[0].configuration.clone();
            let avc = AvcConfig::parse(&config).unwrap();
            let sps = Sps::parse(avc.sps[0]).unwrap();
            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
            let mut decoder = AvcDecoder::new(&config, 15362).unwrap();
            let mut packet = Vec::new();
            input.read_packet(0, 0, &mut packet).unwrap();
            let first_nal = NalUnits::new(&packet, avc.length_size)
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            let first_header = SliceHeader::parse(first_nal, &sps, &pps).unwrap();
            let reference = fvid::codec::avc_field_picture::decode_pcm_slices(
                &[&first_header],
                &sps,
                &pps,
                1538,
            )
            .unwrap();
            input.read_packet(0, 2, &mut packet).unwrap();
            let skip_nal = NalUnits::new(&packet, avc.length_size)
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            let skip_header = SliceHeader::parse(skip_nal, &sps, &pps).unwrap();
            assert!(
                fvid::codec::avc_field_picture::decode_skip_field(
                    &[&skip_header],
                    &sps,
                    &pps,
                    &reference,
                    1535
                )
                .is_err()
            );
            let copied = fvid::codec::avc_field_picture::decode_skip_field(
                &[&skip_header],
                &sps,
                &pps,
                &reference,
                1536,
            )
            .unwrap();
            assert_eq!(copied.picture.y, reference.picture.y);
            assert_eq!(copied.picture.cb, reference.picture.cb);
            assert_eq!(copied.picture.cr, reference.picture.cr);
            for _ in 0..2 {
                let mut pixels = Vec::new();
                for index in 0..4 {
                    input.read_packet(0, index, &mut packet).unwrap();
                    let nal = NalUnits::new(&packet, avc.length_size)
                        .unwrap()
                        .next()
                        .unwrap()
                        .unwrap();
                    let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
                    assert!(header.field_pic);
                    assert_eq!(
                        header.bottom_field,
                        (index % 2 == 1) ^ (order == "bottom-first")
                    );
                    assert_eq!(
                        header.slice_type,
                        if index < 2 {
                            SliceType::I
                        } else {
                            SliceType::P
                        }
                    );
                    if index >= 2 {
                        let mut bits = fvid::codec::bits::BitReader::new(&header.rbsp);
                        bits.skip(header.entropy_bit_offset).unwrap();
                        assert_eq!(bits.unsigned_golomb().unwrap(), 2);
                        bits.finish_rbsp().unwrap();
                    }
                    let output = decoder.decode_order(&packet).unwrap();
                    assert_eq!(output.is_some(), index % 2 == 1);
                    if let Some(picture) = output {
                        picture.write_planar(&mut pixels).unwrap();
                    }
                }
                assert!(pixels == oracle, "native I/P fields {name}");
                decoder.reset();
            }
            let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                Cursor::new(&video),
                Default::default(),
                16 << 20,
            )
            .unwrap();
            for _ in 0..2 {
                let mut pixels = Vec::new();
                for index in 0..2 {
                    let frame = reader.read_frame().unwrap().unwrap();
                    assert_eq!(
                        (
                            frame.sample_index,
                            frame.presentation_time.ticks,
                            frame.duration.ticks
                        ),
                        (index * 2, index as i64 * 2, 2)
                    );
                    frame.picture.write_planar(&mut pixels).unwrap();
                }
                assert!(reader.read_frame().unwrap().is_none());
                assert!(pixels == oracle, "playback I/P fields {name}");
                reader.rewind();
            }
        }
    }
}

#[test]
fn separate_motion_fields_match_every_jm_sample_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for vector in [[1, 1], [-1, -1], [5, -3], [-7, 6]] {
                let name = format!(
                    "avc-field-motion-{depth}bit-{order}-x{}-y{}",
                    vector[0], vector[1]
                );
                let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                assert_ne!(
                    &oracle[..oracle.len() / 2],
                    &oracle[oracle.len() / 2..],
                    "fixture must exercise actual motion"
                );
                let mut input = Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                let config = input.tracks()[0].configuration.clone();
                let avc = AvcConfig::parse(&config).unwrap();
                let sps = Sps::parse(avc.sps[0]).unwrap();
                let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                let mut decoder = AvcDecoder::new(&config, 33792).unwrap();
                let mut packet = Vec::new();
                for _ in 0..2 {
                    let mut pixels = Vec::new();
                    for index in 0..4 {
                        input.read_packet(0, index, &mut packet).unwrap();
                        if index >= 2 {
                            let nal = NalUnits::new(&packet, avc.length_size)
                                .unwrap()
                                .next()
                                .unwrap()
                                .unwrap();
                            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
                            assert!(header.field_pic);
                            assert_eq!(header.slice_type, SliceType::P);
                            let mut bits = fvid::codec::bits::BitReader::new(&header.rbsp);
                            bits.skip(header.entropy_bit_offset).unwrap();
                            for address in 0..2 {
                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                let parts = fvid::codec::avc_inter::read_prediction_field(
                                    &mut bits,
                                    SliceType::P,
                                    0,
                                    [1, 0],
                                    true,
                                )
                                .unwrap();
                                assert_eq!(
                                    parts[0].differences[0],
                                    if address == 0 { vector } else { [0, 0] }
                                );
                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                            }
                            bits.finish_rbsp().unwrap();
                        }
                        let output = decoder.decode_order(&packet).unwrap();
                        assert_eq!(output.is_some(), index % 2 == 1);
                        if let Some(picture) = output {
                            picture.write_planar(&mut pixels).unwrap();
                        }
                    }
                    assert!(
                        pixels == oracle,
                        "native {name}: {:?}",
                        pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                    );
                    decoder.reset();
                }
                let mut tight = AvcDecoder::new(&config, 33791).unwrap();
                for index in 0..2 {
                    input.read_packet(0, index, &mut packet).unwrap();
                    tight.decode_order(&packet).unwrap();
                }
                input.read_packet(0, 2, &mut packet).unwrap();
                let error = tight
                    .decode_order(&packet)
                    .err()
                    .expect("motion field storage exceeds this budget");
                assert!(
                    error
                        .to_string()
                        .contains("AVC P field exceeds memory budget"),
                    "{error}"
                );
                assert!(
                    tight
                        .decode_order(&packet)
                        .err()
                        .unwrap()
                        .to_string()
                        .contains("requires reset")
                );
                tight.reset();
                let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                    Cursor::new(&video),
                    Default::default(),
                    16 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut pixels = Vec::new();
                    for index in 0..2 {
                        let frame = reader.read_frame().unwrap().unwrap();
                        assert_eq!(
                            (
                                frame.sample_index,
                                frame.presentation_time.ticks,
                                frame.duration.ticks
                            ),
                            (index * 2, index as i64 * 2, 2)
                        );
                        frame.picture.write_planar(&mut pixels).unwrap();
                    }
                    assert!(reader.read_frame().unwrap().is_none());
                    assert!(pixels == oracle, "playback {name}");
                    reader.rewind();
                }
            }
        }
    }
}

#[test]
fn separate_partitioned_fields_match_jm_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for (code, sub) in [
                (1, 0),
                (2, 0),
                (3, 0),
                (3, 1),
                (3, 2),
                (3, 3),
                (4, 0),
                (4, 1),
                (4, 2),
                (4, 3),
            ] {
                let name = format!("avc-field-partition-{depth}bit-{order}-type{code}-sub{sub}");
                let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                assert_ne!(&oracle[..oracle.len() / 2], &oracle[oracle.len() / 2..]);
                let mut input = Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                let config = input.tracks()[0].configuration.clone();
                let avc = AvcConfig::parse(&config).unwrap();
                let sps = Sps::parse(avc.sps[0]).unwrap();
                let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                let mut packet = Vec::new();
                let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                for _ in 0..2 {
                    let mut pixels = Vec::new();
                    for index in 0..4 {
                        input.read_packet(0, index, &mut packet).unwrap();
                        if index >= 2 {
                            let nal = NalUnits::new(&packet, avc.length_size)
                                .unwrap()
                                .next()
                                .unwrap()
                                .unwrap();
                            let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                            assert!(h.field_pic);
                            let mut bits = fvid::codec::bits::BitReader::new(&h.rbsp);
                            bits.skip(h.entropy_bit_offset).unwrap();
                            for _ in 0..2 {
                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                assert_eq!(bits.unsigned_golomb().unwrap(), code);
                                let parts = fvid::codec::avc_inter::read_prediction_field(
                                    &mut bits,
                                    SliceType::P,
                                    code,
                                    [1, 0],
                                    true,
                                )
                                .unwrap();
                                assert_eq!(
                                    parts.len(),
                                    if code < 3 {
                                        2
                                    } else {
                                        4 * [1, 2, 2, 4][sub as usize]
                                    }
                                );
                                for (i, p) in parts.iter().enumerate() {
                                    assert_eq!(
                                        p.differences[0],
                                        [
                                            if i % 2 == 0 { 1 } else { -2 },
                                            if i % 3 == 0 { -1 } else { 2 }
                                        ]
                                    );
                                    assert_eq!(
                                        p.size,
                                        match code {
                                            1 => [16, 8],
                                            2 => [8, 16],
                                            _ => [[8, 8], [8, 4], [4, 8], [4, 4]][sub as usize],
                                        }
                                    );
                                }
                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                            }
                            bits.finish_rbsp().unwrap();
                        }
                        let output = decoder.decode_order(&packet).unwrap();
                        assert_eq!(output.is_some(), index % 2 == 1);
                        if let Some(p) = output {
                            p.write_planar(&mut pixels).unwrap();
                        }
                    }
                    assert!(
                        pixels == oracle,
                        "native {name}: {:?}",
                        pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                    );
                    decoder.reset();
                }
                let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                    Cursor::new(&video),
                    Default::default(),
                    16 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut pixels = Vec::new();
                    for index in 0..2 {
                        let frame = reader.read_frame().unwrap().unwrap();
                        assert_eq!(
                            (
                                frame.sample_index,
                                frame.presentation_time.ticks,
                                frame.duration.ticks
                            ),
                            (index * 2, index as i64 * 2, 2)
                        );
                        frame.picture.write_planar(&mut pixels).unwrap();
                    }
                    assert!(reader.read_frame().unwrap().is_none());
                    assert!(pixels == oracle, "playback {name}");
                    reader.rewind();
                }
            }
        }
    }
}

#[test]
fn separate_residual_fields_match_jm_field_scan_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["luma-ac", "chroma-dc", "all-ac"] {
                let name = format!("avc-field-residual-{depth}bit-{order}-{mode}");
                let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                let baseline = std::fs::read(
                    root.join(format!("avc-field-residual-{depth}bit-{order}-zero.yuv")),
                )
                .unwrap();
                let baseline_video = std::fs::read(
                    root.join(format!("avc-field-residual-{depth}bit-{order}-zero.mp4")),
                )
                .unwrap();
                let mut control_reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                    Cursor::new(&baseline_video),
                    Default::default(),
                    16 << 20,
                )
                .unwrap();
                let mut control_pixels = Vec::new();
                while let Some(frame) = control_reader.read_frame().unwrap() {
                    frame.picture.write_planar(&mut control_pixels).unwrap();
                }
                assert!(
                    control_pixels == baseline,
                    "zero-residual control must decode correctly"
                );
                assert_eq!(&oracle[..oracle.len() / 2], &baseline[..baseline.len() / 2]);
                assert_ne!(
                    &oracle[oracle.len() / 2..],
                    &baseline[baseline.len() / 2..],
                    "residual must change the moving output"
                );
                let mut input = Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                let config = input.tracks()[0].configuration.clone();
                let avc = AvcConfig::parse(&config).unwrap();
                let sps = Sps::parse(avc.sps[0]).unwrap();
                let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                let mut packet = Vec::new();
                let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                for _ in 0..2 {
                    let mut pixels = Vec::new();
                    for index in 0..4 {
                        input.read_packet(0, index, &mut packet).unwrap();
                        if index >= 2 {
                            let nal = NalUnits::new(&packet, avc.length_size)
                                .unwrap()
                                .next()
                                .unwrap()
                                .unwrap();
                            let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                            assert!(h.field_pic);
                            let mut bits = fvid::codec::bits::BitReader::new(&h.rbsp);
                            bits.skip(h.entropy_bit_offset).unwrap();
                            assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                            let syntax = fvid::codec::avc_inter::InterSyntax {
                                slice: SliceType::P,
                                active_references: [1, 0],
                                previous_qp: h.slice_qp,
                                bit_depth: sps.bit_depth_luma,
                                chroma_array_type: 1,
                                transform8_enabled: false,
                                direct8_inference: sps.direct_8x8_inference,
                            };
                            let mb = fvid::codec::avc_inter::read_inter_header_field(
                                &mut bits, &syntax, true,
                            )
                            .unwrap();
                            assert_eq!(mb.partitions[0].differences[0], [1, -1]);
                            assert_eq!(
                                mb.residual.pattern,
                                match mode {
                                    "luma-ac" => 15,
                                    "chroma-dc" => 16,
                                    _ => 47,
                                }
                            );
                            let c =
                                fvid::codec::avc_inter_coefficients::read_inter_coefficients_field(
                                    &mut bits,
                                    mb.residual.pattern,
                                    false,
                                    Default::default(),
                                    true,
                                )
                                .unwrap();
                            if mode != "chroma-dc" {
                                assert_eq!(c.luma_counts, [1; 16]);
                                assert_eq!(c.luma4[0][4], 1);
                                assert_eq!(c.luma4[0][1], 0);
                                assert!(c.luma4.iter().any(|v| v[4] < 0));
                            }
                            if mode != "luma-ac" {
                                assert_eq!(c.chroma_dc[0][0], 1);
                                assert_eq!(c.chroma_dc[1][0], -1);
                            }
                            if mode == "all-ac" {
                                assert_eq!(c.chroma_counts, [[1; 4]; 2]);
                                assert_eq!(c.chroma_ac[0][0][4], 1);
                                assert_eq!(c.chroma_ac[0][0][1], 0);
                            }
                            assert_eq!(bits.unsigned_golomb().unwrap(), 1);
                            bits.finish_rbsp().unwrap();
                        }
                        let output = decoder.decode_order(&packet).unwrap();
                        assert_eq!(output.is_some(), index % 2 == 1);
                        if let Some(p) = output {
                            p.write_planar(&mut pixels).unwrap();
                        }
                    }
                    assert!(
                        pixels == oracle,
                        "native {name}: {:?}",
                        pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                    );
                    decoder.reset();
                }
                let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                    Cursor::new(&video),
                    Default::default(),
                    16 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut pixels = Vec::new();
                    for index in 0..2 {
                        let frame = reader.read_frame().unwrap().unwrap();
                        assert_eq!(
                            (
                                frame.sample_index,
                                frame.presentation_time.ticks,
                                frame.duration.ticks
                            ),
                            (index * 2, index as i64 * 2, 2)
                        );
                        frame.picture.write_planar(&mut pixels).unwrap();
                    }
                    assert!(reader.read_frame().unwrap().is_none());
                    assert!(pixels == oracle, "playback {name}");
                    reader.rewind();
                }
            }
        }
    }
}

#[test]
fn separate_field_transform_sizes_and_qp_match_jm_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for eight in [false, true] {
                for qp in [18, 26, 40] {
                    for scaled in [false, true] {
                        let suffix = if scaled { "-scale24" } else { "" };
                        let name = format!(
                            "avc-field-transform-{depth}bit-{order}-t{}-qp{qp}{suffix}",
                            if eight { 8 } else { 4 }
                        );
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        if eight && qp == 40 {
                            let four = std::fs::read(root.join(format!(
                                "avc-field-transform-{depth}bit-{order}-t4-qp{qp}{suffix}.yuv"
                            )))
                            .unwrap();
                            assert_ne!(
                                &oracle[oracle.len() / 2..],
                                &four[four.len() / 2..],
                                "transform size must change reconstructed pixels"
                            );
                        }
                        let mut input =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = input.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        assert!(pps.transform_8x8);
                        let matrices =
                            fvid::codec::avc_scaling::ScalingMatrices::new(&sps, &pps).unwrap();
                        assert_eq!(matrices.four[3], [if scaled { 24 } else { 16 }; 16]);
                        assert_eq!(matrices.eight[1], [if scaled { 24 } else { 16 }; 64]);
                        if scaled && qp == 40 {
                            let plain = std::fs::read(root.join(format!(
                                "avc-field-transform-{depth}bit-{order}-t{}-qp{qp}.yuv",
                                if eight { 8 } else { 4 }
                            )))
                            .unwrap();
                            assert_eq!(&oracle[..oracle.len() / 2], &plain[..plain.len() / 2]);
                            assert_ne!(
                                &oracle[oracle.len() / 2..],
                                &plain[plain.len() / 2..],
                                "scaling must change residual reconstruction"
                            );
                        }

                        let mut packet = Vec::new();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            for index in 0..4 {
                                input.read_packet(0, index, &mut packet).unwrap();
                                if index >= 2 {
                                    let nal = NalUnits::new(&packet, avc.length_size)
                                        .unwrap()
                                        .next()
                                        .unwrap()
                                        .unwrap();
                                    let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                    assert!(h.field_pic);
                                    assert_eq!(h.slice_qp, qp);
                                    let mut bits = fvid::codec::bits::BitReader::new(&h.rbsp);
                                    bits.skip(h.entropy_bit_offset).unwrap();
                                    assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                    let syntax = fvid::codec::avc_inter::InterSyntax {
                                        slice: SliceType::P,
                                        active_references: [1, 0],
                                        previous_qp: h.slice_qp,
                                        bit_depth: sps.bit_depth_luma,
                                        chroma_array_type: 1,
                                        transform8_enabled: true,
                                        direct8_inference: sps.direct_8x8_inference,
                                    };
                                    let mb = fvid::codec::avc_inter::read_inter_header_field(
                                        &mut bits, &syntax, true,
                                    )
                                    .unwrap();
                                    assert_eq!(mb.residual.pattern, 47);
                                    assert_eq!(mb.residual.transform8, eight);
                                    assert_eq!(
                                        mb.residual.qp,
                                        qp + if h.bottom_field { 3 } else { -3 }
                                    );
                                    let c=fvid::codec::avc_inter_coefficients::read_inter_coefficients_field(&mut bits,47,eight,Default::default(),true).unwrap();
                                    assert_eq!(c.luma_counts, [1; 16]);
                                    assert_eq!(c.chroma_counts, [[1; 4]; 2]);
                                    if eight {
                                        assert!(
                                            c.luma8.iter().all(|v| v
                                                .iter()
                                                .filter(|&&n| n != 0)
                                                .count()
                                                == 4)
                                        );
                                        assert!(c.luma4.iter().flatten().all(|&n| n == 0));
                                    } else {
                                        assert_eq!(c.luma4[0][4], 1);
                                    }
                                    assert_eq!(bits.unsigned_golomb().unwrap(), 1);
                                    bits.finish_rbsp().unwrap();
                                }
                                let output = decoder.decode_order(&packet).unwrap();
                                assert_eq!(output.is_some(), index % 2 == 1);
                                if let Some(p) = output {
                                    p.write_planar(&mut pixels).unwrap();
                                }
                            }
                            assert!(
                                pixels == oracle,
                                "native {name}: {:?}",
                                pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                            );
                            decoder.reset();
                        }
                        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            for index in 0..2 {
                                let frame = reader.read_frame().unwrap().unwrap();
                                assert_eq!(
                                    (
                                        frame.sample_index,
                                        frame.presentation_time.ticks,
                                        frame.duration.ticks
                                    ),
                                    (index * 2, index as i64 * 2, 2)
                                );
                                frame.picture.write_planar(&mut pixels).unwrap();
                            }
                            assert!(reader.read_frame().unwrap().is_none());
                            assert!(pixels == oracle, "playback {name}");
                            reader.rewind();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn separate_field_transform_bypass_matches_jm_and_ignores_scaling() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for eight in [false, true] {
                for bypass in [false, true] {
                    for scaled in [false, true] {
                        let prefix = format!(
                            "avc-field-bypass-{depth}bit-{order}-t{}",
                            if eight { 8 } else { 4 }
                        );
                        let name = format!(
                            "{prefix}-{}-scale{}",
                            if bypass { "enabled" } else { "control" },
                            if scaled { 24 } else { 16 }
                        );
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        if bypass {
                            let control = std::fs::read(root.join(format!(
                                "{prefix}-control-scale{}.yuv",
                                if scaled { 24 } else { 16 }
                            )))
                            .unwrap();
                            assert_ne!(
                                &oracle[oracle.len() / 2..],
                                &control[control.len() / 2..],
                                "bypass must change reconstructed residual"
                            );
                            let flat =
                                std::fs::read(root.join(format!("{prefix}-enabled-scale16.yuv")))
                                    .unwrap();
                            assert_eq!(oracle, flat, "bypass must ignore scaling matrices");
                        }
                        let mut input =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = input.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        assert_eq!(sps.transform_bypass, bypass);
                        assert_eq!(sps.profile, 244);
                        let mut packet = Vec::new();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            for index in 0..4 {
                                input.read_packet(0, index, &mut packet).unwrap();
                                if index >= 2 {
                                    let nal = NalUnits::new(&packet, avc.length_size)
                                        .unwrap()
                                        .next()
                                        .unwrap()
                                        .unwrap();
                                    let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                    assert!(h.field_pic);
                                    assert_eq!(h.slice_qp, -6 * (depth - 8));
                                    let mut bits = fvid::codec::bits::BitReader::new(&h.rbsp);
                                    bits.skip(h.entropy_bit_offset).unwrap();
                                    assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                    let syntax = fvid::codec::avc_inter::InterSyntax {
                                        slice: SliceType::P,
                                        active_references: [1, 0],
                                        previous_qp: h.slice_qp,
                                        bit_depth: sps.bit_depth_luma,
                                        chroma_array_type: 1,
                                        transform8_enabled: true,
                                        direct8_inference: sps.direct_8x8_inference,
                                    };
                                    let mb = fvid::codec::avc_inter::read_inter_header_field(
                                        &mut bits, &syntax, true,
                                    )
                                    .unwrap();
                                    assert_eq!(mb.residual.transform8, eight);
                                    assert_eq!(mb.residual.qp + 6 * (depth - 8), 0);
                                    assert_eq!(mb.residual.pattern, 47);
                                    let c=fvid::codec::avc_inter_coefficients::read_inter_coefficients_field(&mut bits,47,eight,Default::default(),true).unwrap();
                                    assert_eq!(c.luma_counts, [1; 16]);
                                    assert_eq!(c.chroma_counts, [[1; 4]; 2]);
                                    assert_eq!(c.chroma_dc[0][0], 1);
                                    assert_eq!(c.chroma_dc[1][0], -1);
                                    assert_eq!(bits.unsigned_golomb().unwrap(), 1);
                                    bits.finish_rbsp().unwrap();
                                }
                                let output = decoder.decode_order(&packet).unwrap();
                                assert_eq!(output.is_some(), index % 2 == 1);
                                if let Some(p) = output {
                                    p.write_planar(&mut pixels).unwrap();
                                }
                            }
                            assert!(
                                pixels == oracle,
                                "native {name}: {:?}",
                                pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                            );
                            decoder.reset();
                        }
                        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            for index in 0..2 {
                                let frame = reader.read_frame().unwrap().unwrap();
                                assert_eq!(
                                    (
                                        frame.sample_index,
                                        frame.presentation_time.ticks,
                                        frame.duration.ticks
                                    ),
                                    (index * 2, index as i64 * 2, 2)
                                );
                                frame.picture.write_planar(&mut pixels).unwrap();
                            }
                            assert!(reader.read_frame().unwrap().is_none());
                            assert!(pixels == oracle, "playback {name}");
                            reader.rewind();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn separate_field_filtering_matches_jm_and_field_motion_threshold() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["motion", "luma-ac", "all-ac"] {
                for filter in [0, 1, 2] {
                    let prefix = format!("avc-field-filter-{depth}bit-{order}-{mode}");
                    let name = format!("{prefix}-filter{filter}");
                    let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                    let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                    if filter != 1 {
                        let off =
                            std::fs::read(root.join(format!("{prefix}-filter1.yuv"))).unwrap();
                        assert_eq!(&oracle[..oracle.len() / 2], &off[..off.len() / 2]);
                        assert_ne!(
                            &oracle[oracle.len() / 2..],
                            &off[off.len() / 2..],
                            "filter must affect {name}"
                        );
                        let enabled =
                            std::fs::read(root.join(format!("{prefix}-filter0.yuv"))).unwrap();
                        assert_eq!(oracle, enabled, "same slice filter2 must match filter0");
                    }
                    let mut input =
                        Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                    let config = input.tracks()[0].configuration.clone();
                    let avc = AvcConfig::parse(&config).unwrap();
                    let sps = Sps::parse(avc.sps[0]).unwrap();
                    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                    let mut packet = Vec::new();
                    let mut decoder = AvcDecoder::new(&config, 35840).unwrap();
                    for _ in 0..2 {
                        let mut pixels = Vec::new();
                        for index in 0..4 {
                            input.read_packet(0, index, &mut packet).unwrap();
                            if index >= 2 {
                                let nal = NalUnits::new(&packet, avc.length_size)
                                    .unwrap()
                                    .next()
                                    .unwrap()
                                    .unwrap();
                                let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                assert!(h.field_pic);
                                assert_eq!(h.disable_deblocking_filter_idc, filter);
                                assert_eq!(h.slice_qp, 50);
                                assert_eq!(
                                    [h.alpha_offset, h.beta_offset],
                                    if filter == 1 { [0, 0] } else { [12, 12] }
                                );
                                let mut bits = fvid::codec::bits::BitReader::new(&h.rbsp);
                                bits.skip(h.entropy_bit_offset).unwrap();
                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                let syntax = fvid::codec::avc_inter::InterSyntax {
                                    slice: SliceType::P,
                                    active_references: [1, 0],
                                    previous_qp: h.slice_qp,
                                    bit_depth: sps.bit_depth_luma,
                                    chroma_array_type: 1,
                                    transform8_enabled: false,
                                    direct8_inference: sps.direct_8x8_inference,
                                };
                                let mb = fvid::codec::avc_inter::read_inter_header_field(
                                    &mut bits, &syntax, true,
                                )
                                .unwrap();
                                if mode == "motion" {
                                    assert_eq!(mb.partitions.len(), 2);
                                    assert_eq!(mb.partitions[0].differences[0], [0, 0]);
                                    assert_eq!(mb.partitions[1].differences[0], [0, 2]);
                                    assert_eq!(mb.residual.pattern, 0);
                                }
                                let c=fvid::codec::avc_inter_coefficients::read_inter_coefficients_field(&mut bits,mb.residual.pattern,false,Default::default(),true).unwrap();
                                assert_eq!(
                                    c.luma_counts,
                                    if mode == "motion" { [0; 16] } else { [1; 16] }
                                );
                                assert_eq!(bits.unsigned_golomb().unwrap(), 1);
                                bits.finish_rbsp().unwrap();
                            }
                            let output = decoder.decode_order(&packet).unwrap();
                            assert_eq!(output.is_some(), index % 2 == 1);
                            if let Some(p) = output {
                                p.write_planar(&mut pixels).unwrap();
                            }
                        }
                        assert!(
                            pixels == oracle,
                            "native {name}: {:?}",
                            pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                        );
                        decoder.reset();
                    }
                    if filter != 1 {
                        let mut tight = AvcDecoder::new(&config, 35839).unwrap();
                        for index in 0..2 {
                            input.read_packet(0, index, &mut packet).unwrap();
                            tight.decode_order(&packet).unwrap();
                        }
                        input.read_packet(0, 2, &mut packet).unwrap();
                        assert!(
                            tight
                                .decode_order(&packet)
                                .err()
                                .unwrap()
                                .to_string()
                                .contains("AVC P field exceeds memory budget")
                        );
                    }
                    let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                        Cursor::new(&video),
                        Default::default(),
                        16 << 20,
                    )
                    .unwrap();
                    for _ in 0..2 {
                        let mut pixels = Vec::new();
                        for index in 0..2 {
                            let frame = reader.read_frame().unwrap().unwrap();
                            assert_eq!(
                                (
                                    frame.sample_index,
                                    frame.presentation_time.ticks,
                                    frame.duration.ticks
                                ),
                                (index * 2, index as i64 * 2, 2)
                            );
                            frame.picture.write_planar(&mut pixels).unwrap();
                        }
                        assert!(reader.read_frame().unwrap().is_none());
                        assert!(pixels == oracle, "playback {name}");
                        reader.rewind();
                    }
                }
            }
        }
    }
}

#[test]
fn separate_multislice_fields_match_jm_and_filter_slice_edges() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["motion", "all-ac"] {
                for filter in [0, 1, 2] {
                    for aso in [false, true] {
                        let prefix = format!("avc-field-multislice-{depth}bit-{order}-{mode}");
                        let suffix = if aso { "-aso" } else { "" };
                        let name = format!("{prefix}-filter{filter}{suffix}");
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        let ordered =
                            std::fs::read(root.join(format!("{prefix}-filter{filter}.yuv")))
                                .unwrap();
                        assert_eq!(oracle, ordered, "ASO must preserve pixels");
                        if filter == 0 {
                            let suppressed =
                                std::fs::read(root.join(format!("{prefix}-filter2{suffix}.yuv")))
                                    .unwrap();
                            assert_ne!(
                                &oracle[oracle.len() / 2..],
                                &suppressed[suppressed.len() / 2..],
                                "slice boundary suppression must affect {name}"
                            );
                        }
                        if mode == "motion" && filter == 2 {
                            let off =
                                std::fs::read(root.join(format!("{prefix}-filter1{suffix}.yuv")))
                                    .unwrap();
                            assert_eq!(
                                oracle, off,
                                "motion fixture has only a slice boundary strength"
                            );
                        }
                        let mut input =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = input.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        let mut packet = Vec::new();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            for index in 0..4 {
                                input.read_packet(0, index, &mut packet).unwrap();
                                if index >= 2 {
                                    let nals = NalUnits::new(&packet, avc.length_size)
                                        .unwrap()
                                        .map(|n| n.unwrap())
                                        .collect::<Vec<_>>();
                                    assert_eq!(nals.len(), 2);
                                    for (wire, nal) in nals.iter().enumerate() {
                                        let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                        assert!(h.field_pic);
                                        assert_eq!(
                                            h.nal_ref_idc,
                                            if h.first_mb == 0 { 2 } else { 3 }
                                        );
                                        assert_eq!(
                                            h.first_mb,
                                            if aso { 1 - wire as u32 } else { wire as u32 }
                                        );
                                        assert_eq!(h.disable_deblocking_filter_idc, filter);
                                        let mut bits = fvid::codec::bits::BitReader::new(&h.rbsp);
                                        bits.skip(h.entropy_bit_offset).unwrap();
                                        assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                        let syntax = fvid::codec::avc_inter::InterSyntax {
                                            slice: SliceType::P,
                                            active_references: [1, 0],
                                            previous_qp: h.slice_qp,
                                            bit_depth: sps.bit_depth_luma,
                                            chroma_array_type: 1,
                                            transform8_enabled: false,
                                            direct8_inference: sps.direct_8x8_inference,
                                        };
                                        let mb = fvid::codec::avc_inter::read_inter_header_field(
                                            &mut bits, &syntax, true,
                                        )
                                        .unwrap();
                                        assert_eq!(
                                            mb.partitions[0].differences[0],
                                            if mode == "motion" {
                                                [0, h.first_mb as i32 * 2]
                                            } else {
                                                [1, -1]
                                            }
                                        );
                                        let c=fvid::codec::avc_inter_coefficients::read_inter_coefficients_field(&mut bits,mb.residual.pattern,false,Default::default(),true).unwrap();
                                        assert_eq!(
                                            c.luma_counts,
                                            if mode == "motion" { [0; 16] } else { [1; 16] }
                                        );
                                        bits.finish_rbsp().unwrap();
                                    }
                                }
                                let output = decoder.decode_order(&packet).unwrap();
                                assert_eq!(output.is_some(), index % 2 == 1);
                                if let Some(p) = output {
                                    p.write_planar(&mut pixels).unwrap();
                                }
                            }
                            assert!(
                                pixels == oracle,
                                "native {name}: {:?}",
                                pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                            );
                            decoder.reset();
                        }
                        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            for index in 0..2 {
                                let frame = reader.read_frame().unwrap().unwrap();
                                assert_eq!(
                                    (
                                        frame.sample_index,
                                        frame.presentation_time.ticks,
                                        frame.duration.ticks
                                    ),
                                    (index * 2, index as i64 * 2, 2)
                                );
                                frame.picture.write_planar(&mut pixels).unwrap();
                            }
                            assert!(reader.read_frame().unwrap().is_none());
                            assert!(pixels == oracle, "playback {name}");
                            reader.rewind();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn separate_multirow_fields_match_jm_contexts_and_filtering() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["motion", "all-ac"] {
                for layout in ["one", "rows", "blocks"] {
                    for filter in [0, 1, 2] {
                        for aso in [false, true] {
                            if layout == "one" && aso {
                                continue;
                            }
                            let prefix =
                                format!("avc-field-rows-{depth}bit-{order}-{mode}-{layout}");
                            let suffix = if aso { "-aso" } else { "" };
                            let name = format!("{prefix}-filter{filter}{suffix}");
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            let ordered =
                                std::fs::read(root.join(format!("{prefix}-filter{filter}.yuv")))
                                    .unwrap();
                            assert_eq!(oracle, ordered);
                            if filter == 0 {
                                let off = std::fs::read(
                                    root.join(format!("{prefix}-filter1{suffix}.yuv")),
                                )
                                .unwrap();
                                assert_ne!(
                                    &oracle[oracle.len() / 2..],
                                    &off[off.len() / 2..],
                                    "filter must affect {name}"
                                );
                                if layout != "one" {
                                    let suppressed = std::fs::read(
                                        root.join(format!("{prefix}-filter2{suffix}.yuv")),
                                    )
                                    .unwrap();
                                    assert_ne!(
                                        &oracle[oracle.len() / 2..],
                                        &suppressed[suppressed.len() / 2..],
                                        "row/slice edge suppression must affect {name}"
                                    );
                                }
                            }
                            let mut input =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = input.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            assert_eq!(sps.coded_dimensions(), (32, 64));
                            let mut packet = Vec::new();
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..4 {
                                    input.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 2 {
                                        let nals = NalUnits::new(&packet, avc.length_size)
                                            .unwrap()
                                            .map(|n| n.unwrap())
                                            .collect::<Vec<_>>();
                                        let per_slice = match layout {
                                            "one" => 4,
                                            "rows" => 2,
                                            _ => 1,
                                        };
                                        assert_eq!(nals.len(), 4 / per_slice);
                                        for (wire, nal) in nals.iter().enumerate() {
                                            let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                            assert!(h.field_pic);
                                            let start = if aso {
                                                (nals.len() - 1 - wire) * per_slice
                                            } else {
                                                wire * per_slice
                                            };
                                            assert_eq!(h.first_mb, start as u32);
                                            assert_eq!(h.disable_deblocking_filter_idc, filter);
                                            let mut bits =
                                                fvid::codec::bits::BitReader::new(&h.rbsp);
                                            bits.skip(h.entropy_bit_offset).unwrap();
                                            let mut counts=fvid::codec::avc_coefficient_field::CoefficientField::new(2,2,4096).unwrap();
                                            let mut qp = h.slice_qp;
                                            for address in start..start + per_slice {
                                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                                let syntax = fvid::codec::avc_inter::InterSyntax {
                                                    slice: SliceType::P,
                                                    active_references: [1, 0],
                                                    previous_qp: qp,
                                                    bit_depth: sps.bit_depth_luma,
                                                    chroma_array_type: 1,
                                                    transform8_enabled: false,
                                                    direct8_inference: sps.direct_8x8_inference,
                                                };
                                                let mb=fvid::codec::avc_inter::read_inter_header_field(&mut bits,&syntax,true).unwrap();
                                                qp = mb.residual.qp;
                                                assert_eq!(
                                                    mb.partitions[0].differences[0],
                                                    if mode == "motion" {
                                                        [0, (address / 2) as i32 * 2]
                                                    } else {
                                                        [1, -1]
                                                    }
                                                );
                                                let c=fvid::codec::avc_inter_coefficients::read_inter_coefficients_field(&mut bits,mb.residual.pattern,false,counts.neighbours(address,0).unwrap(),true).unwrap();
                                                assert_eq!(
                                                    c.luma_counts,
                                                    if mode == "motion" {
                                                        [0; 16]
                                                    } else {
                                                        [1; 16]
                                                    }
                                                );
                                                counts
                                                    .store(
                                                        address,
                                                        0,
                                                        c.luma_counts,
                                                        c.chroma_counts,
                                                    )
                                                    .unwrap();
                                            }
                                            bits.finish_rbsp().unwrap();
                                        }
                                    }
                                    let output = decoder.decode_order(&packet).unwrap();
                                    assert_eq!(output.is_some(), index % 2 == 1);
                                    if let Some(p) = output {
                                        assert_eq!(p.dimensions(), (32, 64));
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert!(
                                    pixels == oracle,
                                    "native {name}: {:?}",
                                    pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                                );
                                decoder.reset();
                            }
                            let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..2 {
                                    let frame = reader.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            frame.sample_index,
                                            frame.presentation_time.ticks,
                                            frame.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    frame.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert!(reader.read_frame().unwrap().is_none());
                                assert!(pixels == oracle, "playback {name}");
                                reader.rewind();
                            }
                            assert_eq!(reader.seek_to_sync(2), 0);
                            assert_eq!(reader.read_frame().unwrap().unwrap().sample_index, 0);
                            let resumed = reader.read_frame().unwrap().unwrap();
                            assert_eq!(resumed.presentation_time.ticks, 2);
                            let mut resumed_pixels = Vec::new();
                            resumed.picture.write_planar(&mut resumed_pixels).unwrap();
                            assert_eq!(resumed_pixels, &oracle[oracle.len() / 2..]);
                            assert!(reader.read_frame().unwrap().is_none());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn separate_field_slice_references_match_jm_and_filter_picture_identity() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for swap in [false, true] {
                for mode in ["skip", "coded"] {
                    for filter in [0, 1, 2] {
                        for aso in [false, true] {
                            let prefix = format!(
                                "avc-field-refs-{depth}bit-{order}-{}-{mode}",
                                if swap { "swap" } else { "normal" }
                            );
                            let suffix = if aso { "-aso" } else { "" };
                            let name = format!("{prefix}-filter{filter}{suffix}");
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            let frame_bytes = oracle.len() / 3;
                            let ordered =
                                std::fs::read(root.join(format!("{prefix}-filter{filter}.yuv")))
                                    .unwrap();
                            assert_eq!(oracle, ordered);
                            let switched=std::fs::read(root.join(format!("avc-field-refs-{depth}bit-{order}-{}-{mode}-filter{filter}{suffix}.yuv",if swap {"normal"} else {"swap"}))).unwrap();
                            assert_ne!(
                                &oracle[2 * frame_bytes..],
                                &switched[2 * frame_bytes..],
                                "reference modification must change pixels"
                            );
                            if filter == 0 {
                                let off = std::fs::read(
                                    root.join(format!("{prefix}-filter1{suffix}.yuv")),
                                )
                                .unwrap();
                                assert_ne!(
                                    &oracle[2 * frame_bytes..],
                                    &off[2 * frame_bytes..],
                                    "different reference identities must produce boundary strength"
                                );
                            }
                            if mode == "coded" {
                                let skipped=std::fs::read(root.join(format!("avc-field-refs-{depth}bit-{order}-{}-skip-filter{filter}{suffix}.yuv",if swap {"swap"} else {"normal"}))).unwrap();
                                assert_eq!(oracle, skipped);
                            }
                            let mut input =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = input.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            let mut packet = Vec::new();
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..6 {
                                    input.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 4 {
                                        let nals = NalUnits::new(&packet, avc.length_size)
                                            .unwrap()
                                            .map(|n| n.unwrap())
                                            .collect::<Vec<_>>();
                                        assert_eq!(nals.len(), 2);
                                        for (wire, nal) in nals.iter().enumerate() {
                                            let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                            assert!(h.field_pic);
                                            assert_eq!(h.frame_num, 2);
                                            assert_eq!(
                                                h.first_mb,
                                                if aso { 1 - wire as u32 } else { wire as u32 }
                                            );
                                            let old = (h.first_mb == 0) ^ swap;
                                            assert_eq!(h.modifications_l0,vec![fvid::codec::avc_slice::RefModification::Subtract(if old {3} else {1})]);
                                            let mut bits =
                                                fvid::codec::bits::BitReader::new(&h.rbsp);
                                            bits.skip(h.entropy_bit_offset).unwrap();
                                            assert_eq!(
                                                bits.unsigned_golomb().unwrap(),
                                                if mode == "skip" { 1 } else { 0 }
                                            );
                                            if mode == "coded" {
                                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                                let p =
                                                    fvid::codec::avc_inter::read_prediction_field(
                                                        &mut bits,
                                                        SliceType::P,
                                                        0,
                                                        [1, 0],
                                                        true,
                                                    )
                                                    .unwrap();
                                                assert_eq!(p[0].differences[0], [0, 0]);
                                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                            }
                                            bits.finish_rbsp().unwrap();
                                        }
                                    }
                                    let output = decoder.decode_order(&packet).unwrap();
                                    assert_eq!(output.is_some(), index % 2 == 1);
                                    if let Some(p) = output {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert!(
                                    pixels == oracle,
                                    "native {name}: {:?}",
                                    pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                                );
                                decoder.reset();
                            }
                            let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..3 {
                                    let frame = reader.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            frame.sample_index,
                                            frame.presentation_time.ticks,
                                            frame.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    frame.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert!(reader.read_frame().unwrap().is_none());
                                assert!(pixels == oracle, "playback {name}");
                                reader.rewind();
                            }
                            assert_eq!(reader.seek_to_sync(4), 0);
                            for index in 0..2 {
                                assert_eq!(
                                    reader.read_frame().unwrap().unwrap().sample_index,
                                    index * 2
                                );
                            }
                            let resumed = reader.read_frame().unwrap().unwrap();
                            let mut pixels = Vec::new();
                            resumed.picture.write_planar(&mut pixels).unwrap();
                            assert_eq!(resumed.presentation_time.ticks, 4);
                            assert_eq!(pixels, &oracle[2 * frame_bytes..]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn opposite_field_slice_references_match_jm_chroma_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for swap in [false, true] {
                for mode in ["skip", "coded"] {
                    for filter in [0, 1, 2] {
                        for aso in [false, true] {
                            let prefix = format!(
                                "avc-field-opposite-{depth}bit-{order}-{}-{mode}",
                                if swap { "swap" } else { "normal" }
                            );
                            let suffix = if aso { "-aso" } else { "" };
                            let name = format!("{prefix}-filter{filter}{suffix}");
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            let frame_bytes = oracle.len() / 3;
                            let ordered =
                                std::fs::read(root.join(format!("{prefix}-filter{filter}.yuv")))
                                    .unwrap();
                            assert_eq!(oracle, ordered);
                            let switched=std::fs::read(root.join(format!("avc-field-opposite-{depth}bit-{order}-{}-{mode}-filter{filter}{suffix}.yuv",if swap {"normal"} else {"swap"}))).unwrap();
                            assert_ne!(
                                &oracle[2 * frame_bytes..],
                                &switched[2 * frame_bytes..],
                                "reference modification must change pixels"
                            );
                            if filter == 0 {
                                let off = std::fs::read(
                                    root.join(format!("{prefix}-filter1{suffix}.yuv")),
                                )
                                .unwrap();
                                assert_ne!(
                                    &oracle[2 * frame_bytes..],
                                    &off[2 * frame_bytes..],
                                    "different reference identities must produce boundary strength"
                                );
                            }
                            if mode == "coded" {
                                let skipped=std::fs::read(root.join(format!("avc-field-opposite-{depth}bit-{order}-{}-skip-filter{filter}{suffix}.yuv",if swap {"swap"} else {"normal"}))).unwrap();
                                assert_eq!(oracle, skipped);
                            }
                            let mut input =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = input.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            let mut packet = Vec::new();
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..6 {
                                    input.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 4 {
                                        let nals = NalUnits::new(&packet, avc.length_size)
                                            .unwrap()
                                            .map(|n| n.unwrap())
                                            .collect::<Vec<_>>();
                                        assert_eq!(nals.len(), 2);
                                        for (wire, nal) in nals.iter().enumerate() {
                                            let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                            assert!(h.field_pic);
                                            assert_eq!(h.frame_num, 2);
                                            assert_eq!(
                                                h.first_mb,
                                                if aso { 1 - wire as u32 } else { wire as u32 }
                                            );
                                            let old = (h.first_mb == 0) ^ swap;
                                            assert_eq!(h.modifications_l0,vec![fvid::codec::avc_slice::RefModification::Subtract(if old {4} else {2})]);
                                            let mut bits =
                                                fvid::codec::bits::BitReader::new(&h.rbsp);
                                            bits.skip(h.entropy_bit_offset).unwrap();
                                            assert_eq!(
                                                bits.unsigned_golomb().unwrap(),
                                                if mode == "skip" { 1 } else { 0 }
                                            );
                                            if mode == "coded" {
                                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                                let p =
                                                    fvid::codec::avc_inter::read_prediction_field(
                                                        &mut bits,
                                                        SliceType::P,
                                                        0,
                                                        [1, 0],
                                                        true,
                                                    )
                                                    .unwrap();
                                                assert_eq!(p[0].differences[0], [0, 0]);
                                                assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                            }
                                            bits.finish_rbsp().unwrap();
                                        }
                                    }
                                    let output = decoder.decode_order(&packet).unwrap();
                                    assert_eq!(output.is_some(), index % 2 == 1);
                                    if let Some(p) = output {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert!(
                                    pixels == oracle,
                                    "native {name}: {:?}",
                                    pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                                );
                                decoder.reset();
                            }
                            let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..3 {
                                    let frame = reader.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            frame.sample_index,
                                            frame.presentation_time.ticks,
                                            frame.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    frame.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert!(reader.read_frame().unwrap().is_none());
                                assert!(pixels == oracle, "playback {name}");
                                reader.rewind();
                            }
                            assert_eq!(reader.seek_to_sync(4), 0);
                            for index in 0..2 {
                                assert_eq!(
                                    reader.read_frame().unwrap().unwrap().sample_index,
                                    index * 2
                                );
                            }
                            let resumed = reader.read_frame().unwrap().unwrap();
                            let mut pixels = Vec::new();
                            resumed.picture.write_planar(&mut pixels).unwrap();
                            assert_eq!(resumed.presentation_time.ticks, 4);
                            assert_eq!(pixels, &oracle[2 * frame_bytes..]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn active_field_references_per_partition_match_jm_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for rotate in [false, true] {
                for (code, sub) in [(1, 0), (2, 0), (3, 0), (3, 1), (3, 2), (3, 3)] {
                    for filter in [0, 1, 2] {
                        for aso in [false, true] {
                            let prefix = format!(
                                "avc-field-multiref-{depth}bit-{order}-type{code}-sub{sub}"
                            );
                            let suffix = if aso { "-aso" } else { "" };
                            let name =
                                format!("{prefix}-r{}-filter{filter}{suffix}", u8::from(rotate));
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            let frame_bytes = oracle.len() / 3;
                            let ordered = std::fs::read(root.join(format!(
                                "{prefix}-r{}-filter{filter}.yuv",
                                u8::from(rotate)
                            )))
                            .unwrap();
                            assert_eq!(oracle, ordered);
                            let rotated = std::fs::read(root.join(format!(
                                "{prefix}-r{}-filter{filter}{suffix}.yuv",
                                u8::from(!rotate)
                            )))
                            .unwrap();
                            assert_ne!(&oracle[2 * frame_bytes..], &rotated[2 * frame_bytes..]);
                            if filter == 0 {
                                let off = std::fs::read(root.join(format!(
                                    "{prefix}-r{}-filter1{suffix}.yuv",
                                    u8::from(rotate)
                                )))
                                .unwrap();
                                assert_ne!(
                                    &oracle[2 * frame_bytes..],
                                    &off[2 * frame_bytes..],
                                    "reference identity/parity must affect filtering"
                                );
                            }
                            let mut input =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = input.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            let mut packet = Vec::new();
                            let budget = if filter == 1 { 33984 } else { 36032 };
                            let mut decoder = AvcDecoder::new(&config, budget).unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..6 {
                                    input.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 4 {
                                        let nals = NalUnits::new(&packet, avc.length_size)
                                            .unwrap()
                                            .map(|n| n.unwrap())
                                            .collect::<Vec<_>>();
                                        assert_eq!(nals.len(), 2);
                                        for (wire, nal) in nals.iter().enumerate() {
                                            let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                            assert!(h.field_pic);
                                            assert_eq!(h.refs_l0, 4);
                                            assert_eq!(
                                                h.first_mb,
                                                if aso { 1 - wire as u32 } else { wire as u32 }
                                            );
                                            use fvid::codec::avc_slice::RefModification as R;
                                            assert_eq!(
                                                h.modifications_l0,
                                                vec![
                                                    R::Subtract(3),
                                                    R::Subtract(0),
                                                    R::Add(2),
                                                    R::Subtract(0)
                                                ]
                                            );
                                            let mut bits =
                                                fvid::codec::bits::BitReader::new(&h.rbsp);
                                            bits.skip(h.entropy_bit_offset).unwrap();
                                            assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                            assert_eq!(bits.unsigned_golomb().unwrap(), code);
                                            let parts =
                                                fvid::codec::avc_inter::read_prediction_field(
                                                    &mut bits,
                                                    SliceType::P,
                                                    code,
                                                    [4, 0],
                                                    true,
                                                )
                                                .unwrap();
                                            assert_eq!(
                                                parts.len(),
                                                if code < 3 {
                                                    2
                                                } else {
                                                    4 * [1, 2, 2, 4][sub as usize]
                                                }
                                            );
                                            for p in parts {
                                                assert_eq!(
                                                    p.references[0],
                                                    Some(
                                                        ((h.first_mb as usize * 2
                                                            + p.group
                                                            + usize::from(rotate))
                                                            % 4)
                                                            as u8
                                                    )
                                                );
                                                assert_eq!(p.differences[0], [0, 0]);
                                            }
                                            assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                            bits.finish_rbsp().unwrap();
                                        }
                                    }
                                    let output = decoder.decode_order(&packet).unwrap();
                                    assert_eq!(output.is_some(), index % 2 == 1);
                                    if let Some(p) = output {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert!(
                                    pixels == oracle,
                                    "native {name}: {:?}",
                                    pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                                );
                                decoder.reset();
                            }
                            let mut tight = AvcDecoder::new(&config, budget - 1).unwrap();
                            for index in 0..4 {
                                input.read_packet(0, index, &mut packet).unwrap();
                                tight.decode_order(&packet).unwrap();
                            }
                            input.read_packet(0, 4, &mut packet).unwrap();
                            assert!(
                                tight
                                    .decode_order(&packet)
                                    .err()
                                    .unwrap()
                                    .to_string()
                                    .contains("AVC P field exceeds memory budget")
                            );
                            let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..3 {
                                    let frame = reader.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            frame.sample_index,
                                            frame.presentation_time.ticks,
                                            frame.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    frame.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert!(reader.read_frame().unwrap().is_none());
                                assert!(pixels == oracle, "playback {name}");
                                reader.rewind();
                            }
                            assert_eq!(reader.seek_to_sync(4), 0);
                            for _ in 0..2 {
                                reader.read_frame().unwrap().unwrap();
                            }
                            let frame = reader.read_frame().unwrap().unwrap();
                            let mut pixels = Vec::new();
                            frame.picture.write_planar(&mut pixels).unwrap();
                            assert_eq!(pixels, &oracle[2 * frame_bytes..]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn weighted_field_prediction_matches_jm_clipping_residual_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for identity in [false, true] {
                for mode in ["whole-skip", "16x8", "8x8", "residual"] {
                    for filter in [0, 1, 2] {
                        for aso in [false, true] {
                            if mode == "whole-skip" && aso {
                                continue;
                            }
                            let prefix = format!("avc-field-weight-{depth}bit-{order}-{mode}");
                            let suffix = if aso { "-aso" } else { "" };
                            let name = format!(
                                "{prefix}-{}-filter{filter}{suffix}",
                                if identity { "identity" } else { "weighted" }
                            );
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            let frame_bytes = oracle.len() / 3;
                            let ordered = std::fs::read(root.join(format!(
                                "{prefix}-{}-filter{filter}.yuv",
                                if identity { "identity" } else { "weighted" }
                            )))
                            .unwrap();
                            assert_eq!(oracle, ordered);
                            if !identity {
                                let control =
                                    std::fs::read(root.join(format!(
                                        "{prefix}-identity-filter{filter}{suffix}.yuv"
                                    )))
                                    .unwrap();
                                assert_ne!(
                                    &oracle[2 * frame_bytes..],
                                    &control[2 * frame_bytes..],
                                    "weights must affect pixels"
                                );
                            }
                            if !identity && mode == "8x8" && filter == 1 {
                                let values = if depth == 8 {
                                    oracle[2 * frame_bytes..]
                                        .iter()
                                        .map(|&v| u16::from(v))
                                        .collect::<Vec<_>>()
                                } else {
                                    oracle[2 * frame_bytes..]
                                        .chunks_exact(2)
                                        .map(|v| u16::from_le_bytes([v[0], v[1]]))
                                        .collect::<Vec<_>>()
                                };
                                assert!(values.contains(&0));
                                assert!(values.contains(&((1 << depth) - 1)));
                            }
                            if mode == "residual" && filter == 1 {
                                let clean=std::fs::read(root.join(format!("avc-field-weight-{depth}bit-{order}-8x8-{}-filter1{suffix}.yuv",if identity {"identity"} else {"weighted"}))).unwrap();
                                assert_ne!(
                                    &oracle[2 * frame_bytes..],
                                    &clean[2 * frame_bytes..],
                                    "residual must change weighted pixels before filtering"
                                );
                            }
                            let mut input =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = input.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            assert!(pps.weighted_pred);
                            let mut packet = Vec::new();
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..6 {
                                    input.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 4 {
                                        let nals = NalUnits::new(&packet, avc.length_size)
                                            .unwrap()
                                            .map(|n| n.unwrap())
                                            .collect::<Vec<_>>();
                                        assert_eq!(
                                            nals.len(),
                                            if mode == "whole-skip" { 1 } else { 2 }
                                        );
                                        for nal in nals {
                                            let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                            assert!(h.field_pic);
                                            assert_eq!(h.refs_l0, 4);
                                            let w = h.weights.as_ref().unwrap();
                                            assert_eq!((w.luma_denom, w.chroma_denom), (2, 1));
                                            assert_eq!(w.l0.len(), 4);
                                            let expected = [
                                                ((3, 5), [(1, -7), (3, 8)]),
                                                ((-2, 90), [(0, 24), (-1, 48)]),
                                                ((4, 0), [(2, 0); 2]),
                                                ((12, -24), [(5, -50), (-2, 80)]),
                                            ];
                                            for (i, entry) in w.l0.iter().enumerate() {
                                                assert_eq!(
                                                    entry.luma,
                                                    if identity { (4, 0) } else { expected[i].0 }
                                                );
                                                assert_eq!(
                                                    entry.chroma,
                                                    if identity {
                                                        [(2, 0); 2]
                                                    } else {
                                                        expected[i].1
                                                    }
                                                );
                                            }
                                            let mut bits =
                                                fvid::codec::bits::BitReader::new(&h.rbsp);
                                            bits.skip(h.entropy_bit_offset).unwrap();
                                            assert_eq!(
                                                bits.unsigned_golomb().unwrap(),
                                                if mode == "whole-skip" { 2 } else { 0 }
                                            );
                                            if mode != "whole-skip" {
                                                let syntax = fvid::codec::avc_inter::InterSyntax {
                                                    slice: SliceType::P,
                                                    active_references: [4, 0],
                                                    previous_qp: h.slice_qp,
                                                    bit_depth: sps.bit_depth_luma,
                                                    chroma_array_type: 1,
                                                    transform8_enabled: false,
                                                    direct8_inference: sps.direct_8x8_inference,
                                                };
                                                let mb=fvid::codec::avc_inter::read_inter_header_field(&mut bits,&syntax,true).unwrap();
                                                assert_eq!(
                                                    mb.residual.pattern,
                                                    if mode == "residual" { 47 } else { 0 }
                                                );
                                                let c=fvid::codec::avc_inter_coefficients::read_inter_coefficients_field(&mut bits,mb.residual.pattern,false,Default::default(),true).unwrap();
                                                assert_eq!(
                                                    c.luma_counts,
                                                    if mode == "residual" {
                                                        [1; 16]
                                                    } else {
                                                        [0; 16]
                                                    }
                                                );
                                            }
                                            bits.finish_rbsp().unwrap();
                                        }
                                    }
                                    let output = decoder.decode_order(&packet).unwrap();
                                    assert_eq!(output.is_some(), index % 2 == 1);
                                    if let Some(p) = output {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert!(
                                    pixels == oracle,
                                    "native {name}: {:?}",
                                    pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                                );
                                decoder.reset();
                            }
                            let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..3 {
                                    let frame = reader.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            frame.sample_index,
                                            frame.presentation_time.ticks,
                                            frame.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    frame.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert!(reader.read_frame().unwrap().is_none());
                                assert!(pixels == oracle, "playback {name}");
                                reader.rewind();
                            }
                            assert_eq!(reader.seek_to_sync(4), 0);
                            for _ in 0..2 {
                                reader.read_frame().unwrap().unwrap();
                            }
                            let frame = reader.read_frame().unwrap().unwrap();
                            let mut pixels = Vec::new();
                            frame.picture.write_planar(&mut pixels).unwrap();
                            assert_eq!(pixels, &oracle[2 * frame_bytes..]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn mixed_pcm_inter_fields_match_jm_and_preserve_rewind_seek() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for placement in ["pcm-first", "pcm-last"] {
                for mode in ["skip", "coded"] {
                    for filter in 0..3 {
                        let name = format!(
                            "avc-field-mixed-pcm-{depth}bit-{order}-{placement}-{mode}-filter{filter}"
                        );
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        let mut container =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = container.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            let mut packet = Vec::new();
                            for index in 0..4 {
                                container.read_packet(0, index, &mut packet).unwrap();
                                if index >= 2 {
                                    let nal = NalUnits::new(&packet, avc.length_size)
                                        .unwrap()
                                        .next()
                                        .unwrap()
                                        .unwrap();
                                    let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                    assert_eq!(h.slice_type, SliceType::P);
                                    assert!(h.field_pic);
                                    let mut bits = fvid::codec::bits::BitReader::new(&h.rbsp);
                                    bits.skip(h.entropy_bit_offset).unwrap();
                                    if placement == "pcm-last" {
                                        assert_eq!(
                                            bits.unsigned_golomb().unwrap(),
                                            if mode == "skip" { 1 } else { 0 }
                                        );
                                        if mode == "coded" {
                                            assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                            assert_eq!(bits.signed_golomb().unwrap(), 1);
                                            assert_eq!(bits.signed_golomb().unwrap(), -1);
                                            assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                        }
                                    }
                                    if placement != "pcm-last" || mode != "skip" {
                                        assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                    }
                                    assert_eq!(bits.unsigned_golomb().unwrap(), 30);
                                }
                                if let Some(picture) = decoder
                                    .decode_order(&packet)
                                    .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"))
                                {
                                    picture.write_planar(&mut pixels).unwrap();
                                }
                            }
                            assert!(
                                pixels == oracle,
                                "native {name}: {:?}",
                                pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                            );
                            decoder.reset();
                        }
                        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            for index in 0..2 {
                                let frame = reader.read_frame().unwrap().unwrap();
                                assert_eq!(
                                    (
                                        frame.sample_index,
                                        frame.presentation_time.ticks,
                                        frame.duration.ticks
                                    ),
                                    (index * 2, index as i64 * 2, 2)
                                );
                                frame.picture.write_planar(&mut pixels).unwrap();
                            }
                            assert!(reader.read_frame().unwrap().is_none());
                            assert!(pixels == oracle, "playback {name}");
                            reader.rewind();
                        }
                        assert_eq!(reader.seek_to_sync(2), 0);
                        reader.read_frame().unwrap().unwrap();
                        let mut pixels = Vec::new();
                        reader
                            .read_frame()
                            .unwrap()
                            .unwrap()
                            .picture
                            .write_planar(&mut pixels)
                            .unwrap();
                        assert_eq!(pixels, &oracle[oracle.len() / 2..], "seek {name}");
                    }
                }
            }
        }
    }
}
#[test]
fn mixed_intra4_inter_fields_match_jm_and_preserve_rewind_seek() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for placement in ["intra-first", "intra-last"] {
                for mode in ["skip", "coded"] {
                    for filter in 0..3 {
                        let name = format!(
                            "avc-field-mixed-intra4-{depth}bit-{order}-{placement}-{mode}-filter{filter}"
                        );
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        let mut container =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = container.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            let mut packet = Vec::new();
                            for index in 0..4 {
                                container.read_packet(0, index, &mut packet).unwrap();
                                if index >= 2 {
                                    let nal = NalUnits::new(&packet, avc.length_size)
                                        .unwrap()
                                        .next()
                                        .unwrap()
                                        .unwrap();
                                    let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                    assert_eq!(h.slice_type, SliceType::P);
                                    assert!(h.field_pic);
                                    let mut bits = fvid::codec::bits::BitReader::new(&h.rbsp);
                                    bits.skip(h.entropy_bit_offset).unwrap();
                                    if placement == "intra-last" {
                                        assert_eq!(
                                            bits.unsigned_golomb().unwrap(),
                                            if mode == "skip" { 1 } else { 0 }
                                        );
                                        if mode == "coded" {
                                            assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                            assert_eq!(bits.signed_golomb().unwrap(), 1);
                                            assert_eq!(bits.signed_golomb().unwrap(), -1);
                                            assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                        }
                                    }
                                    if placement != "intra-last" || mode != "skip" {
                                        assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                    }
                                    assert_eq!(bits.unsigned_golomb().unwrap(), 5);
                                    for _ in 0..16 {
                                        assert!(bits.bit().unwrap());
                                    }
                                    assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                    assert_eq!(bits.unsigned_golomb().unwrap(), 3);
                                }
                                if let Some(picture) = decoder
                                    .decode_order(&packet)
                                    .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"))
                                {
                                    picture.write_planar(&mut pixels).unwrap();
                                }
                            }
                            assert!(
                                pixels == oracle,
                                "native {name}: {:?}",
                                pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                            );
                            decoder.reset();
                        }
                        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            for index in 0..2 {
                                let frame = reader.read_frame().unwrap().unwrap();
                                assert_eq!(
                                    (
                                        frame.sample_index,
                                        frame.presentation_time.ticks,
                                        frame.duration.ticks
                                    ),
                                    (index * 2, index as i64 * 2, 2)
                                );
                                frame.picture.write_planar(&mut pixels).unwrap();
                            }
                            assert!(reader.read_frame().unwrap().is_none());
                            assert!(pixels == oracle, "playback {name}");
                            reader.rewind();
                        }
                        assert_eq!(reader.seek_to_sync(2), 0);
                        reader.read_frame().unwrap().unwrap();
                        let mut pixels = Vec::new();
                        reader
                            .read_frame()
                            .unwrap()
                            .unwrap()
                            .picture
                            .write_planar(&mut pixels)
                            .unwrap();
                        assert_eq!(pixels, &oracle[oracle.len() / 2..], "seek {name}");
                    }
                }
            }
        }
    }
}
#[test]
fn field_intra_residual_matches_jm_and_preserve_rewind_seek() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for placement in ["intra-first", "intra-last"] {
                for mode in ["skip", "coded"] {
                    for (filter, residual) in (0..3).flat_map(|f| {
                        [
                            "i4-zero",
                            "i4-ac",
                            "i16-zero",
                            "i16-positive",
                            "i16-negative",
                        ]
                        .map(|r| (f, r))
                    }) {
                        let name = format!(
                            "avc-field-intra-residual-{depth}bit-{order}-{placement}-{mode}-{residual}-filter{filter}"
                        );
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        if filter == 1 && !residual.ends_with("zero") {
                            let control = name.replace(
                                residual,
                                if residual.starts_with("i4") {
                                    "i4-zero"
                                } else {
                                    "i16-zero"
                                },
                            );
                            let zero = std::fs::read(root.join(format!("{control}.yuv"))).unwrap();
                            assert_ne!(
                                &oracle[oracle.len() / 2..],
                                &zero[zero.len() / 2..],
                                "residual has no effect {name}"
                            );
                        }
                        let mut container =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = container.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            let mut packet = Vec::new();
                            for index in 0..4 {
                                container.read_packet(0, index, &mut packet).unwrap();
                                if index >= 2 {
                                    let nal = NalUnits::new(&packet, avc.length_size)
                                        .unwrap()
                                        .next()
                                        .unwrap()
                                        .unwrap();
                                    let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                                    assert_eq!(h.slice_type, SliceType::P);
                                    assert!(h.field_pic);
                                    let mut bits = fvid::codec::bits::BitReader::new(&h.rbsp);
                                    bits.skip(h.entropy_bit_offset).unwrap();
                                    if placement == "intra-last" {
                                        assert_eq!(
                                            bits.unsigned_golomb().unwrap(),
                                            if mode == "skip" { 1 } else { 0 }
                                        );
                                        if mode == "coded" {
                                            assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                            assert_eq!(bits.signed_golomb().unwrap(), 1);
                                            assert_eq!(bits.signed_golomb().unwrap(), -1);
                                            assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                        }
                                    }
                                    if placement != "intra-last" || mode != "skip" {
                                        assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                                    }
                                    let code = bits.unsigned_golomb().unwrap();
                                    assert_eq!(
                                        code,
                                        if residual.starts_with("i4") { 5 } else { 8 }
                                    );
                                    let mut syntax =
                                        fvid::codec::avc_macroblock::IntraCavlcReader::new_context(
                                            &h, &sps, &pps, 2,
                                        )
                                        .unwrap();
                                    let address = if placement == "intra-first" { 0 } else { 1 };
                                    if address == 1 {
                                        syntax.record_inter(0, [0; 16], [[0; 4]; 2]).unwrap();
                                    }
                                    let block = syntax
                                        .read_embedded(&mut bits, address, h.slice_qp, code - 5)
                                        .unwrap();
                                    assert!(syntax.field_decoding());
                                    let (counts, chroma) = syntax.counts(address as usize).unwrap();
                                    assert_eq!(
                                        counts,
                                        if residual == "i4-ac" {
                                            [1; 16]
                                        } else {
                                            [0; 16]
                                        }
                                    );
                                    assert_eq!(chroma, [[0; 4]; 2]);
                                    if residual.starts_with("i4") {
                                        for (i, levels) in block.luma_levels.iter().enumerate() {
                                            assert_eq!(levels[1], 0);
                                            if residual == "i4-zero" {
                                                assert_eq!(*levels, [0; 16]);
                                            } else {
                                                assert_eq!(
                                                    levels[4],
                                                    if i % 2 == 0 { 1 } else { -1 }
                                                );
                                            }
                                        }
                                    } else {
                                        assert_eq!(
                                            block.luma_dc[0],
                                            match residual {
                                                "i16-positive" => 1,
                                                "i16-negative" => -1,
                                                _ => 0,
                                            }
                                        );
                                        assert_eq!(block.luma_dc[1..], [0; 15]);
                                    }
                                }
                                if let Some(picture) = decoder
                                    .decode_order(&packet)
                                    .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"))
                                {
                                    picture.write_planar(&mut pixels).unwrap();
                                }
                            }
                            assert!(
                                pixels == oracle,
                                "native {name}: {:?}",
                                pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                            );
                            decoder.reset();
                        }
                        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let mut pixels = Vec::new();
                            for index in 0..2 {
                                let frame = reader.read_frame().unwrap().unwrap();
                                assert_eq!(
                                    (
                                        frame.sample_index,
                                        frame.presentation_time.ticks,
                                        frame.duration.ticks
                                    ),
                                    (index * 2, index as i64 * 2, 2)
                                );
                                frame.picture.write_planar(&mut pixels).unwrap();
                            }
                            assert!(reader.read_frame().unwrap().is_none());
                            assert!(pixels == oracle, "playback {name}");
                            reader.rewind();
                        }
                        assert_eq!(reader.seek_to_sync(2), 0);
                        reader.read_frame().unwrap().unwrap();
                        let mut pixels = Vec::new();
                        reader
                            .read_frame()
                            .unwrap()
                            .unwrap()
                            .picture
                            .write_planar(&mut pixels)
                            .unwrap();
                        assert_eq!(pixels, &oracle[oracle.len() / 2..], "seek {name}");
                    }
                }
            }
        }
    }
}

#[test]
fn non_pcm_i_fields_match_jm_with_aso_and_pair_timing() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in [
                "i4-zero",
                "i4-ac",
                "i16-zero",
                "i16-positive",
                "i16-negative",
            ] {
                for filter in 0..3 {
                    for aso in [false, true] {
                        let name = format!(
                            "avc-field-intra-{depth}bit-{order}-{mode}-filter{filter}{}",
                            if aso { "-aso" } else { "" }
                        );
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        if aso {
                            assert_eq!(
                                oracle,
                                std::fs::read(
                                    root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                )
                                .unwrap()
                            );
                        }
                        if filter == 1 && !mode.ends_with("zero") {
                            let zero = name.replace(
                                mode,
                                if mode.starts_with("i4") {
                                    "i4-zero"
                                } else {
                                    "i16-zero"
                                },
                            );
                            assert_ne!(
                                oracle,
                                std::fs::read(root.join(format!("{zero}.yuv"))).unwrap()
                            );
                        }
                        let mut container =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = container.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut output = Vec::new();
                            let mut packet = Vec::new();
                            for index in 0..2 {
                                container.read_packet(0, index, &mut packet).unwrap();
                                for (wire, nal) in
                                    NalUnits::new(&packet, avc.length_size).unwrap().enumerate()
                                {
                                    let h = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                                    assert!(h.field_pic);
                                    assert_eq!(h.slice_type, SliceType::I);
                                    assert_eq!(
                                        h.first_mb,
                                        if aso { 1 - wire as u32 } else { wire as u32 }
                                    );
                                    let mut syntax =
                                        fvid::codec::avc_macroblock::IntraCavlcReader::new(
                                            &h, &sps, &pps, 2,
                                        )
                                        .unwrap();
                                    let block = syntax.read_macroblock().unwrap().unwrap();
                                    assert!(syntax.field_decoding());
                                    if mode == "i4-ac" {
                                        for levels in block.luma_levels {
                                            assert_eq!(levels[1], 0);
                                            assert_eq!(levels[4].abs(), 1);
                                        }
                                    } else if mode.starts_with("i16") {
                                        assert_eq!(
                                            block.luma_dc[0],
                                            match mode {
                                                "i16-positive" => 1,
                                                "i16-negative" => -1,
                                                _ => 0,
                                            }
                                        );
                                    }
                                    assert!(syntax.read_macroblock().unwrap().is_none());
                                }
                                let picture = decoder
                                    .decode_order(&packet)
                                    .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
                                assert_eq!(picture.is_some(), index == 1);
                                if let Some(p) = picture {
                                    assert_eq!(p.dimensions(), (32, 32));
                                    p.write_planar(&mut output).unwrap();
                                }
                            }
                            assert_eq!(output, oracle, "native {name}");
                            decoder.reset();
                        }
                        let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let frame = player.read_frame().unwrap().unwrap();
                            assert_eq!(
                                (
                                    frame.sample_index,
                                    frame.presentation_time.ticks,
                                    frame.duration.ticks
                                ),
                                (0, 0, 2)
                            );
                            let mut pixels = Vec::new();
                            frame.picture.write_planar(&mut pixels).unwrap();
                            assert_eq!(pixels, oracle, "player {name}");
                            assert!(player.read_frame().unwrap().is_none());
                            player.rewind();
                        }
                        assert_eq!(player.seek_to_sync(1), 0);
                        let mut pixels = Vec::new();
                        player
                            .read_frame()
                            .unwrap()
                            .unwrap()
                            .picture
                            .write_planar(&mut pixels)
                            .unwrap();
                        assert_eq!(pixels, oracle);
                    }
                }
            }
        }
    }
}
#[test]
fn i_field_chroma_residual_matches_jm_with_aso_and_pair_timing() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in [
                "i4-zero",
                "i4-ac",
                "i16-zero",
                "i16-positive",
                "i16-negative",
            ] {
                for filter in 0..3 {
                    for aso in [false, true] {
                        let name = format!(
                            "avc-field-intra-chroma-{depth}bit-{order}-{mode}-filter{filter}{}",
                            if aso { "-aso" } else { "" }
                        );
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        if aso {
                            assert_eq!(
                                oracle,
                                std::fs::read(
                                    root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                )
                                .unwrap()
                            );
                        }
                        if filter == 1 && !mode.ends_with("zero") {
                            let zero = name.replace(
                                mode,
                                if mode.starts_with("i4") {
                                    "i4-zero"
                                } else {
                                    "i16-zero"
                                },
                            );
                            assert_ne!(
                                oracle,
                                std::fs::read(root.join(format!("{zero}.yuv"))).unwrap()
                            );
                        }
                        if filter == 1 {
                            let control = name.replace("intra-chroma-", "intra-");
                            let luma_only =
                                std::fs::read(root.join(format!("{control}.yuv"))).unwrap();
                            let y_bytes = 1024 * if depth == 10 { 2 } else { 1 };
                            assert_eq!(
                                &oracle[..y_bytes],
                                &luma_only[..y_bytes],
                                "chroma changed luma {name}"
                            );
                            if mode.ends_with("zero") {
                                assert_eq!(oracle, luma_only);
                            } else {
                                assert_ne!(
                                    &oracle[y_bytes..],
                                    &luma_only[y_bytes..],
                                    "chroma residual has no effect {name}"
                                );
                            }
                        }
                        let mut container =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = container.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut output = Vec::new();
                            let mut packet = Vec::new();
                            for index in 0..2 {
                                container.read_packet(0, index, &mut packet).unwrap();
                                for (wire, nal) in
                                    NalUnits::new(&packet, avc.length_size).unwrap().enumerate()
                                {
                                    let h = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                                    assert!(h.field_pic);
                                    assert_eq!(h.slice_type, SliceType::I);
                                    assert_eq!(
                                        h.first_mb,
                                        if aso { 1 - wire as u32 } else { wire as u32 }
                                    );
                                    let mut syntax =
                                        fvid::codec::avc_macroblock::IntraCavlcReader::new(
                                            &h, &sps, &pps, 2,
                                        )
                                        .unwrap();
                                    let block = syntax.read_macroblock().unwrap().unwrap();
                                    assert!(syntax.field_decoding());
                                    if mode == "i4-ac" {
                                        for levels in block.luma_levels {
                                            assert_eq!(levels[1], 0);
                                            assert_eq!(levels[4].abs(), 1);
                                        }
                                    } else if mode.starts_with("i16") {
                                        assert_eq!(
                                            block.luma_dc[0],
                                            match mode {
                                                "i16-positive" => 1,
                                                "i16-negative" => -1,
                                                _ => 0,
                                            }
                                        );
                                    }
                                    for c in 0..2 {
                                        assert_eq!(
                                            block.chroma_dc[c][0],
                                            if mode.ends_with("zero") {
                                                0
                                            } else if c == 0 {
                                                1
                                            } else {
                                                -1
                                            }
                                        );
                                        assert_eq!(block.chroma_dc[c][1..], [0; 3]);
                                        for i in 0..4 {
                                            let levels = &block.chroma_ac[c][i];
                                            assert_eq!(levels[1], 0);
                                            assert_eq!(
                                                levels[4],
                                                if mode.ends_with("zero") {
                                                    0
                                                } else if (c + i) % 2 == 0 {
                                                    1
                                                } else {
                                                    -1
                                                }
                                            );
                                        }
                                    }
                                    assert!(syntax.read_macroblock().unwrap().is_none());
                                }
                                let picture = decoder
                                    .decode_order(&packet)
                                    .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
                                assert_eq!(picture.is_some(), index == 1);
                                if let Some(p) = picture {
                                    assert_eq!(p.dimensions(), (32, 32));
                                    p.write_planar(&mut output).unwrap();
                                }
                            }
                            assert_eq!(output, oracle, "native {name}");
                            decoder.reset();
                        }
                        let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let frame = player.read_frame().unwrap().unwrap();
                            assert_eq!(
                                (
                                    frame.sample_index,
                                    frame.presentation_time.ticks,
                                    frame.duration.ticks
                                ),
                                (0, 0, 2)
                            );
                            let mut pixels = Vec::new();
                            frame.picture.write_planar(&mut pixels).unwrap();
                            assert_eq!(pixels, oracle, "player {name}");
                            assert!(player.read_frame().unwrap().is_none());
                            player.rewind();
                        }
                        assert_eq!(player.seek_to_sync(1), 0);
                        let mut pixels = Vec::new();
                        player
                            .read_frame()
                            .unwrap()
                            .unwrap()
                            .picture
                            .write_planar(&mut pixels)
                            .unwrap();
                        assert_eq!(pixels, oracle);
                    }
                }
            }
        }
    }
}
#[test]
fn intra8_fields_match_jm_with_aso_and_pair_timing() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["i8-zero", "i8-ac"] {
                for filter in 0..3 {
                    for (aso, scaled) in
                        [(false, false), (false, true), (true, false), (true, true)]
                    {
                        let name = format!(
                            "avc-field-intra8-{depth}bit-{order}-{mode}-filter{filter}-scale{}{}",
                            if scaled { 8 } else { 16 },
                            if aso { "-aso" } else { "" }
                        );
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        if aso {
                            assert_eq!(
                                oracle,
                                std::fs::read(
                                    root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                )
                                .unwrap()
                            );
                        }
                        if filter == 1 && !mode.ends_with("zero") {
                            let zero = name.replace(
                                mode,
                                if mode.starts_with("i8") {
                                    "i8-zero"
                                } else {
                                    "i16-zero"
                                },
                            );
                            assert_ne!(
                                oracle,
                                std::fs::read(root.join(format!("{zero}.yuv"))).unwrap()
                            );
                        }
                        if filter == 1 && scaled {
                            let flat = name.replace("scale8", "scale16");
                            let flat = std::fs::read(root.join(format!("{flat}.yuv"))).unwrap();
                            if mode == "i8-zero" {
                                assert_eq!(oracle, flat);
                            } else {
                                assert_ne!(oracle, flat, "intra scaling has no effect {name}");
                            }
                        }
                        let mut container =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = container.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut output = Vec::new();
                            let mut packet = Vec::new();
                            for index in 0..2 {
                                container.read_packet(0, index, &mut packet).unwrap();
                                for (wire, nal) in
                                    NalUnits::new(&packet, avc.length_size).unwrap().enumerate()
                                {
                                    let h = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                                    assert!(h.field_pic);
                                    assert_eq!(h.slice_type, SliceType::I);
                                    assert_eq!(
                                        h.first_mb,
                                        if aso { 1 - wire as u32 } else { wire as u32 }
                                    );
                                    let mut syntax =
                                        fvid::codec::avc_macroblock::IntraCavlcReader::new(
                                            &h, &sps, &pps, 2,
                                        )
                                        .unwrap();
                                    assert!(pps.transform_8x8);
                                    assert_eq!(
                                        fvid::codec::avc_scaling::ScalingMatrices::new(&sps, &pps)
                                            .unwrap()
                                            .eight[0],
                                        [if scaled { 8 } else { 16 }; 64]
                                    );
                                    let block = syntax.read_macroblock().unwrap().unwrap();
                                    assert!(syntax.field_decoding());
                                    match &block.luma {
                                        fvid::codec::avc_macroblock::IntraLuma::Blocks8 {
                                            levels,
                                            ..
                                        } => {
                                            for level in levels {
                                                let mut expected = [0; 64];
                                                if mode == "i8-ac" {
                                                    for (sub, position) in
                                                        [9, 24, 32, 17].into_iter().enumerate()
                                                    {
                                                        expected[position] =
                                                            if (sub + h.first_mb as usize + index)
                                                                % 2
                                                                == 0
                                                            {
                                                                1
                                                            } else {
                                                                -1
                                                            };
                                                    }
                                                }
                                                assert_eq!(*level, expected, "field scan {name}");
                                                assert_eq!(
                                                    level.iter().map(|v| v.abs()).sum::<i32>(),
                                                    if mode == "i8-ac" { 4 } else { 0 }
                                                );
                                            }
                                        }
                                        _ => panic!("missing Intra8x8 {name}"),
                                    }
                                    assert!(syntax.read_macroblock().unwrap().is_none());
                                }
                                let picture = decoder
                                    .decode_order(&packet)
                                    .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
                                assert_eq!(picture.is_some(), index == 1);
                                if let Some(p) = picture {
                                    assert_eq!(p.dimensions(), (32, 32));
                                    p.write_planar(&mut output).unwrap();
                                }
                            }
                            assert_eq!(output, oracle, "native {name}");
                            decoder.reset();
                        }
                        let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let frame = player.read_frame().unwrap().unwrap();
                            assert_eq!(
                                (
                                    frame.sample_index,
                                    frame.presentation_time.ticks,
                                    frame.duration.ticks
                                ),
                                (0, 0, 2)
                            );
                            let mut pixels = Vec::new();
                            frame.picture.write_planar(&mut pixels).unwrap();
                            assert_eq!(pixels, oracle, "player {name}");
                            assert!(player.read_frame().unwrap().is_none());
                            player.rewind();
                        }
                        assert_eq!(player.seek_to_sync(1), 0);
                        let mut pixels = Vec::new();
                        player
                            .read_frame()
                            .unwrap()
                            .unwrap()
                            .picture
                            .write_planar(&mut pixels)
                            .unwrap();
                        assert_eq!(pixels, oracle);
                    }
                }
            }
        }
    }
}

#[test]
fn i_field_bypass_matches_jm_and_ignores_scaling() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in [
                "i4-zero",
                "i4-ac",
                "i8-zero",
                "i8-ac",
                "i16-zero",
                "i16-positive",
                "i16-negative",
            ] {
                for scaled in [false, true] {
                    for bypass in [false, true] {
                        for aso in [false, true] {
                            let name = format!(
                                "avc-field-intra-bypass-{depth}bit-{order}-{mode}-filter1-scale{}-{}{}",
                                if scaled { 8 } else { 16 },
                                if bypass { "enabled" } else { "control" },
                                if aso { "-aso" } else { "" }
                            );
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            if aso {
                                assert_eq!(
                                    oracle,
                                    std::fs::read(
                                        root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                    )
                                    .unwrap()
                                );
                            }
                            if bypass {
                                let other = name.replace(
                                    if scaled { "scale8" } else { "scale16" },
                                    if scaled { "scale16" } else { "scale8" },
                                );
                                assert_eq!(
                                    oracle,
                                    std::fs::read(root.join(format!("{other}.yuv"))).unwrap(),
                                    "bypass depends on matrix {name}"
                                );
                                if !mode.ends_with("zero") {
                                    let control = name.replace("enabled", "control");
                                    assert_ne!(
                                        oracle,
                                        std::fs::read(root.join(format!("{control}.yuv"))).unwrap(),
                                        "bypass not exercised {name}"
                                    );
                                }
                            }
                            let mut container =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = container.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            assert_eq!(sps.profile, 244);
                            assert_eq!(sps.chroma_format, 1);
                            assert_eq!(sps.transform_bypass, bypass);
                            assert!(pps.transform_8x8);
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut packet = Vec::new();
                                let mut pixels = Vec::new();
                                for index in 0..2 {
                                    container.read_packet(0, index, &mut packet).unwrap();
                                    for nal in NalUnits::new(&packet, avc.length_size).unwrap() {
                                        let h =
                                            SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                                        assert!(h.field_pic);
                                        assert_eq!(h.slice_type, SliceType::I);
                                        assert_eq!(h.slice_qp + 6 * (depth - 8), 0);
                                        let mut reader =
                                            fvid::codec::avc_macroblock::IntraCavlcReader::new(
                                                &h, &sps, &pps, 2,
                                            )
                                            .unwrap();
                                        let mb = reader.read_macroblock().unwrap().unwrap();
                                        assert_eq!(mb.qp, h.slice_qp);
                                        match (&mb.luma, mode) {
                                            (
                                                fvid::codec::avc_macroblock::IntraLuma::Blocks4(_),
                                                m,
                                            ) if m.starts_with("i4") => {}
                                            (
                                                fvid::codec::avc_macroblock::IntraLuma::Blocks8 {
                                                    ..
                                                },
                                                m,
                                            ) if m.starts_with("i8") => {}
                                            (
                                                fvid::codec::avc_macroblock::IntraLuma::Block16(2),
                                                m,
                                            ) if m.starts_with("i16") => {}
                                            _ => panic!("wrong intra type {name}"),
                                        }
                                        for c in 0..2 {
                                            assert_eq!(
                                                mb.chroma_dc[c][0],
                                                if mode.ends_with("zero") {
                                                    0
                                                } else if c == 0 {
                                                    1
                                                } else {
                                                    -1
                                                }
                                            );
                                        }
                                        assert!(reader.read_macroblock().unwrap().is_none());
                                    }
                                    if let Some(p) = decoder
                                        .decode_order(&packet)
                                        .unwrap_or_else(|e| panic!("{name}: {e:?}"))
                                    {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert_eq!(pixels, oracle, "native {name}");
                                decoder.reset();
                            }
                            let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let f = player.read_frame().unwrap().unwrap();
                                assert_eq!(
                                    (f.sample_index, f.presentation_time.ticks, f.duration.ticks),
                                    (0, 0, 2)
                                );
                                let mut pixels = Vec::new();
                                f.picture.write_planar(&mut pixels).unwrap();
                                assert_eq!(pixels, oracle, "player {name}");
                                assert!(player.read_frame().unwrap().is_none());
                                player.rewind();
                            }
                            assert_eq!(player.seek_to_sync(1), 0);
                            let mut pixels = Vec::new();
                            player
                                .read_frame()
                                .unwrap()
                                .unwrap()
                                .picture
                                .write_planar(&mut pixels)
                                .unwrap();
                            assert_eq!(pixels, oracle);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn fmo_i_fields_match_jm_group_addresses_filtering_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for kind in 0..7 {
            for direction in 0..if (3..=5).contains(&kind) { 2 } else { 1 } {
                for order in ["top-first", "bottom-first"] {
                    for residual in [0, 1] {
                        for filter in 0..3 {
                            for aso in [false, true] {
                                let name = format!(
                                    "avc-field-fmo-{depth}bit-type{kind}-dir{direction}-{order}-dc{residual}-filter{filter}{}",
                                    if aso { "-aso" } else { "" }
                                );
                                let video =
                                    std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                                let oracle =
                                    std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                                if aso {
                                    assert_eq!(
                                        oracle,
                                        std::fs::read(root.join(format!(
                                            "{}.yuv",
                                            name.trim_end_matches("-aso")
                                        )))
                                        .unwrap()
                                    );
                                }
                                if filter == 1 && residual == 1 {
                                    let zero = name.replace("dc1", "dc0");
                                    assert_ne!(
                                        oracle,
                                        std::fs::read(root.join(format!("{zero}.yuv"))).unwrap()
                                    );
                                }
                                if matches!(kind, 0 | 6) && filter == 0 {
                                    for control in [1, 2] {
                                        let control_name =
                                            name.replace("filter0", &format!("filter{control}"));
                                        assert_ne!(
                                            oracle,
                                            std::fs::read(root.join(format!("{control_name}.yuv")))
                                                .unwrap(),
                                            "field filtering not exercised {name}"
                                        );
                                    }
                                }
                                let mapping = match kind {
                                    0 | 6 => [0, 1, 0, 1],
                                    1 => [0, 1, 1, 0],
                                    2 => [0, 1, 1, 1],
                                    3 => {
                                        if direction == 1 {
                                            [0, 1, 0, 1]
                                        } else {
                                            [1, 1, 0, 0]
                                        }
                                    }
                                    4 => {
                                        if direction == 1 {
                                            [1, 1, 0, 0]
                                        } else {
                                            [0, 0, 1, 1]
                                        }
                                    }
                                    5 => {
                                        if direction == 1 {
                                            [1, 0, 1, 0]
                                        } else {
                                            [0, 1, 0, 1]
                                        }
                                    }
                                    _ => unreachable!(),
                                };
                                let mut container =
                                    Mp4Reader::open(Cursor::new(&video), Default::default())
                                        .unwrap();
                                let config = container.tracks()[0].configuration.clone();
                                let avc = AvcConfig::parse(&config).unwrap();
                                let sps = Sps::parse(avc.sps[0]).unwrap();
                                let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                                let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                                for _ in 0..2 {
                                    let mut packet = Vec::new();
                                    let mut pixels = Vec::new();
                                    for index in 0..2 {
                                        container.read_packet(0, index, &mut packet).unwrap();
                                        for (wire, nal) in NalUnits::new(&packet, avc.length_size)
                                            .unwrap()
                                            .enumerate()
                                        {
                                            let h = SliceHeader::parse(nal.unwrap(), &sps, &pps)
                                                .unwrap();
                                            assert!(h.field_pic);
                                            assert_eq!(h.slice_type, SliceType::I);
                                            let group = if aso { 1 - wire } else { wire };
                                            let expected: Vec<_> = mapping
                                                .iter()
                                                .enumerate()
                                                .filter(|(_, g)| **g == group)
                                                .map(|(a, _)| a as u32)
                                                .collect();
                                            let mut reader=fvid::codec::avc_macroblock::IntraCavlcReader::new_fmo(&h,&sps,&pps,4).unwrap();
                                            let mut addresses = Vec::new();
                                            while let Some(mb) = reader.read_macroblock().unwrap() {
                                                assert!(reader.field_decoding());
                                                addresses.push(mb.address);
                                                if addresses.len() == 1 {
                                                    assert!(matches!(mb.luma,fvid::codec::avc_macroblock::IntraLuma::Pcm{..}));
                                                } else {
                                                    assert!(matches!(mb.luma,fvid::codec::avc_macroblock::IntraLuma::Block16(2)));
                                                    assert_eq!(
                                                        mb.luma_dc[0],
                                                        if residual == 0 {
                                                            0
                                                        } else if h.bottom_field {
                                                            -1
                                                        } else {
                                                            1
                                                        }
                                                    );
                                                }
                                            }
                                            assert_eq!(addresses, expected, "group {name}");
                                        }
                                        let picture =
                                            decoder.decode_order(&packet).unwrap_or_else(|e| {
                                                panic!("{name} sample {index}: {e:?}")
                                            });
                                        assert_eq!(picture.is_some(), index == 1);
                                        if let Some(p) = picture {
                                            assert_eq!(p.dimensions(), (32, 64));
                                            p.write_planar(&mut pixels).unwrap();
                                        }
                                    }
                                    assert!(
                                        pixels == oracle,
                                        "native {name}, first mismatch {:?}",
                                        pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                                    );
                                    decoder.reset();
                                }
                                let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                                    Cursor::new(&video),
                                    Default::default(),
                                    16 << 20,
                                )
                                .unwrap();
                                for _ in 0..2 {
                                    let f = player.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            f.sample_index,
                                            f.presentation_time.ticks,
                                            f.duration.ticks
                                        ),
                                        (0, 0, 2)
                                    );
                                    let mut pixels = Vec::new();
                                    f.picture.write_planar(&mut pixels).unwrap();
                                    assert_eq!(pixels, oracle, "player {name}");
                                    assert!(player.read_frame().unwrap().is_none());
                                    player.rewind();
                                }
                                assert_eq!(player.seek_to_sync(1), 0);
                                let mut pixels = Vec::new();
                                player
                                    .read_frame()
                                    .unwrap()
                                    .unwrap()
                                    .picture
                                    .write_planar(&mut pixels)
                                    .unwrap();
                                assert_eq!(pixels, oracle);
                            }
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn cabac_i_fields_match_jm_with_aso_and_pair_timing() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["zero", "positive", "negative"] {
                for filter in 0..3 {
                    for aso in [false, true] {
                        let name = format!(
                            "avc-field-cabac-{depth}bit-{order}-{mode}-filter{filter}{}",
                            if aso { "-aso" } else { "" }
                        );
                        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                        let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                        if aso {
                            assert_eq!(
                                oracle,
                                std::fs::read(
                                    root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                )
                                .unwrap()
                            );
                        }
                        if filter == 1 && !mode.ends_with("zero") {
                            let zero = name.replace(mode, "zero");
                            assert_ne!(
                                oracle,
                                std::fs::read(root.join(format!("{zero}.yuv"))).unwrap()
                            );
                        }
                        let mut container =
                            Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                        let config = container.tracks()[0].configuration.clone();
                        let avc = AvcConfig::parse(&config).unwrap();
                        let sps = Sps::parse(avc.sps[0]).unwrap();
                        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                        for _ in 0..2 {
                            let mut output = Vec::new();
                            let mut packet = Vec::new();
                            for index in 0..2 {
                                container.read_packet(0, index, &mut packet).unwrap();
                                for (wire, nal) in
                                    NalUnits::new(&packet, avc.length_size).unwrap().enumerate()
                                {
                                    let h = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                                    assert!(h.field_pic);
                                    assert_eq!(h.slice_type, SliceType::I);
                                    assert_eq!(
                                        h.first_mb,
                                        if aso { 1 - wire as u32 } else { wire as u32 }
                                    );
                                    let mut syntax =
                                        fvid::codec::avc_cabac_macroblock::IntraCabacReader::new(
                                            &h, &sps, &pps, 2,
                                        )
                                        .unwrap();
                                    let block = syntax.read_macroblock().unwrap().unwrap();
                                    assert!(syntax.field_decoding());
                                    assert!(pps.cabac);
                                    assert!(matches!(
                                        block.luma,
                                        fvid::codec::avc_macroblock::IntraLuma::Block16(2)
                                    ));
                                    assert_eq!(block.luma_dc[1], 0);
                                    assert_eq!(
                                        block.luma_dc[4],
                                        match mode {
                                            "positive" => 1,
                                            "negative" => -1,
                                            _ => 0,
                                        }
                                    );
                                    assert!(syntax.read_macroblock().unwrap().is_none());
                                }
                                let picture = decoder
                                    .decode_order(&packet)
                                    .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
                                assert_eq!(picture.is_some(), index == 1);
                                if let Some(p) = picture {
                                    assert_eq!(p.dimensions(), (32, 32));
                                    p.write_planar(&mut output).unwrap();
                                }
                            }
                            assert_eq!(output, oracle, "native {name}");
                            decoder.reset();
                        }
                        let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                            Cursor::new(&video),
                            Default::default(),
                            16 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let frame = player.read_frame().unwrap().unwrap();
                            assert_eq!(
                                (
                                    frame.sample_index,
                                    frame.presentation_time.ticks,
                                    frame.duration.ticks
                                ),
                                (0, 0, 2)
                            );
                            let mut pixels = Vec::new();
                            frame.picture.write_planar(&mut pixels).unwrap();
                            assert_eq!(pixels, oracle, "player {name}");
                            assert!(player.read_frame().unwrap().is_none());
                            player.rewind();
                        }
                        assert_eq!(player.seek_to_sync(1), 0);
                        let mut pixels = Vec::new();
                        player
                            .read_frame()
                            .unwrap()
                            .unwrap()
                            .picture
                            .write_planar(&mut pixels)
                            .unwrap();
                        assert_eq!(pixels, oracle);
                    }
                }
            }
        }
    }
}

#[test]
fn cabac_p_fields_match_jm_skip_fractional_motion_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["skip", "coded"] {
                for init in 0..3 {
                    for filter in 0..3 {
                        for aso in [false, true] {
                            let name = format!(
                                "avc-field-cabac-p-{depth}bit-{order}-{mode}-init{init}-filter{filter}{}",
                                if aso { "-aso" } else { "" }
                            );
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            if aso {
                                assert_eq!(
                                    oracle,
                                    std::fs::read(
                                        root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                    )
                                    .unwrap()
                                );
                            }
                            if init != 0 {
                                let control = name.replace(&format!("init{init}"), "init0");
                                assert_eq!(
                                    oracle,
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap()
                                );
                            }
                            if mode == "coded" && filter == 1 {
                                let control = name.replace("coded", "skip");
                                let skip =
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap();
                                assert_ne!(
                                    &oracle[oracle.len() / 2..],
                                    &skip[skip.len() / 2..],
                                    "motion not exercised {name}"
                                );
                            }
                            let mut container =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = container.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut packet = Vec::new();
                                let mut pixels = Vec::new();
                                for index in 0..4 {
                                    container.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 2 {
                                        for nal in NalUnits::new(&packet, avc.length_size).unwrap()
                                        {
                                            let h = SliceHeader::parse(nal.unwrap(), &sps, &pps)
                                                .unwrap();
                                            assert_eq!(h.slice_type, SliceType::P);
                                            assert!(h.field_pic);
                                            assert_eq!(h.refs_l0, 1);
                                            assert_eq!(h.cabac_init_idc, init);
                                            let mut syntax =
                                                fvid::codec::avc_cabac_slice::InterCabacSlice::new(
                                                    &h, &sps, &pps, 8192,
                                                )
                                                .unwrap();
                                            let mb = syntax.read_macroblock().unwrap().unwrap();
                                            assert!(syntax.field_decoding());
                                            match mb {
                            fvid::codec::avc_inter_slice::InterMacroblock::Skip{..}=>assert_eq!(mode,"skip"),
                            fvid::codec::avc_inter_slice::InterMacroblock::Coded{header,..}=>{assert_eq!(mode,"coded");assert_eq!(header.residual.pattern,0);assert_eq!(header.partitions.len(),1);assert_eq!(header.partitions[0].differences,[[1,-1],[0,0]]);},
                            _=>panic!("wrong CABAC P block {name}"),
                        }
                                            assert!(syntax.read_macroblock().unwrap().is_none());
                                        }
                                    }
                                    let p = decoder
                                        .decode_order(&packet)
                                        .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
                                    assert_eq!(p.is_some(), index % 2 == 1);
                                    if let Some(p) = p {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert_eq!(pixels, oracle, "native {name}");
                                decoder.reset();
                            }
                            let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..2 {
                                    let f = player.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            f.sample_index,
                                            f.presentation_time.ticks,
                                            f.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    f.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert_eq!(pixels, oracle, "player {name}");
                                assert!(player.read_frame().unwrap().is_none());
                                player.rewind();
                            }
                            assert_eq!(player.seek_to_sync(2), 0);
                            player.read_frame().unwrap().unwrap();
                            let mut pixels = Vec::new();
                            player
                                .read_frame()
                                .unwrap()
                                .unwrap()
                                .picture
                                .write_planar(&mut pixels)
                                .unwrap();
                            assert_eq!(pixels, &oracle[oracle.len() / 2..]);
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn cabac_p_residual_fields_match_jm_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["zero", "ac"] {
                for init in 0..3 {
                    for filter in 0..3 {
                        for aso in [false, true] {
                            let name = format!(
                                "avc-field-cabac-p-residual-{depth}bit-{order}-{mode}-init{init}-filter{filter}{}",
                                if aso { "-aso" } else { "" }
                            );
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            if aso {
                                assert_eq!(
                                    oracle,
                                    std::fs::read(
                                        root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                    )
                                    .unwrap()
                                );
                            }
                            if init != 0 {
                                let control = name.replace(&format!("init{init}"), "init0");
                                assert_eq!(
                                    oracle,
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap()
                                );
                            }
                            if mode == "ac" && filter == 1 {
                                let control = name.replace("ac-init", "zero-init");
                                let skip =
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap();
                                assert_ne!(
                                    &oracle[oracle.len() / 2..],
                                    &skip[skip.len() / 2..],
                                    "residual not exercised {name}"
                                );
                            }
                            let mut container =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = container.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut packet = Vec::new();
                                let mut pixels = Vec::new();
                                for index in 0..4 {
                                    container.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 2 {
                                        for nal in NalUnits::new(&packet, avc.length_size).unwrap()
                                        {
                                            let h = SliceHeader::parse(nal.unwrap(), &sps, &pps)
                                                .unwrap();
                                            assert_eq!(h.slice_type, SliceType::P);
                                            assert!(h.field_pic);
                                            assert_eq!(h.refs_l0, 1);
                                            assert_eq!(h.cabac_init_idc, init);
                                            let mut syntax =
                                                fvid::codec::avc_cabac_slice::InterCabacSlice::new(
                                                    &h, &sps, &pps, 8192,
                                                )
                                                .unwrap();
                                            let mb = syntax.read_macroblock().unwrap().unwrap();
                                            assert!(syntax.field_decoding());
                                            match mb {
                                        fvid::codec::avc_inter_slice::InterMacroblock::Coded{header,coefficients,..}=>{
                                            assert_eq!(header.residual.pattern,15);
                                            assert_eq!(header.partitions[0].differences,[[1,-1],[0,0]]);
                                            assert_eq!(coefficients.luma_counts,if mode=="ac" {[1;16]} else {[0;16]});
                                            for (i,levels) in coefficients.luma4.iter().enumerate() {
                                                assert_eq!(levels[1],0);
                                                assert_eq!(levels[4],if mode=="zero" {0} else if i%2==0 {1} else {-1});
                                            }
                                        },_=>panic!("wrong CABAC residual block {name}"),
                                    }
                                            assert!(syntax.read_macroblock().unwrap().is_none());
                                        }
                                    }
                                    let p = decoder
                                        .decode_order(&packet)
                                        .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
                                    assert_eq!(p.is_some(), index % 2 == 1);
                                    if let Some(p) = p {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert_eq!(pixels, oracle, "native {name}");
                                decoder.reset();
                            }
                            let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..2 {
                                    let f = player.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            f.sample_index,
                                            f.presentation_time.ticks,
                                            f.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    f.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert_eq!(pixels, oracle, "player {name}");
                                assert!(player.read_frame().unwrap().is_none());
                                player.rewind();
                            }
                            assert_eq!(player.seek_to_sync(2), 0);
                            player.read_frame().unwrap().unwrap();
                            let mut pixels = Vec::new();
                            player
                                .read_frame()
                                .unwrap()
                                .unwrap()
                                .picture
                                .write_planar(&mut pixels)
                                .unwrap();
                            assert_eq!(pixels, &oracle[oracle.len() / 2..]);
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn cabac_p_chroma_fields_match_jm_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["zero", "ac"] {
                for init in 0..3 {
                    for filter in 0..3 {
                        for aso in [false, true] {
                            let name = format!(
                                "avc-field-cabac-p-chroma-{depth}bit-{order}-{mode}-init{init}-filter{filter}{}",
                                if aso { "-aso" } else { "" }
                            );
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            if aso {
                                assert_eq!(
                                    oracle,
                                    std::fs::read(
                                        root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                    )
                                    .unwrap()
                                );
                            }
                            if init != 0 {
                                let control = name.replace(&format!("init{init}"), "init0");
                                assert_eq!(
                                    oracle,
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap()
                                );
                            }
                            if mode == "ac" && filter == 1 {
                                let control = name.replace("ac-init", "zero-init");
                                let skip =
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap();
                                assert_ne!(
                                    &oracle[oracle.len() / 2..],
                                    &skip[skip.len() / 2..],
                                    "residual not exercised {name}"
                                );
                            }
                            if filter == 1 {
                                let luma_name = name.replace("p-chroma-", "p-residual-");
                                let luma =
                                    std::fs::read(root.join(format!("{luma_name}.yuv"))).unwrap();
                                let frame_bytes = 1536 * if depth == 10 { 2 } else { 1 };
                                let y_bytes = 1024 * if depth == 10 { 2 } else { 1 };
                                assert_eq!(
                                    &oracle[..frame_bytes + y_bytes],
                                    &luma[..frame_bytes + y_bytes]
                                );
                                if mode == "zero" {
                                    assert_eq!(oracle, luma);
                                } else {
                                    assert_ne!(
                                        &oracle[frame_bytes + y_bytes..],
                                        &luma[frame_bytes + y_bytes..]
                                    );
                                }
                            }
                            let mut container =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = container.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut packet = Vec::new();
                                let mut pixels = Vec::new();
                                for index in 0..4 {
                                    container.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 2 {
                                        for nal in NalUnits::new(&packet, avc.length_size).unwrap()
                                        {
                                            let h = SliceHeader::parse(nal.unwrap(), &sps, &pps)
                                                .unwrap();
                                            assert_eq!(h.slice_type, SliceType::P);
                                            assert!(h.field_pic);
                                            assert_eq!(h.refs_l0, 1);
                                            assert_eq!(h.cabac_init_idc, init);
                                            let mut syntax =
                                                fvid::codec::avc_cabac_slice::InterCabacSlice::new(
                                                    &h, &sps, &pps, 8192,
                                                )
                                                .unwrap();
                                            let mb = syntax.read_macroblock().unwrap().unwrap();
                                            assert!(syntax.field_decoding());
                                            match mb {
                                        fvid::codec::avc_inter_slice::InterMacroblock::Coded{header,coefficients,..}=>{
                                            assert_eq!(header.residual.pattern,47);
                                            assert_eq!(header.partitions[0].differences,[[1,-1],[0,0]]);
                                            assert_eq!(coefficients.luma_counts,if mode=="ac" {[1;16]} else {[0;16]});
                                            for c in 0..2 {
    assert_eq!(coefficients.chroma_dc[c][0],if mode=="zero" {0} else if c==0 {1} else {-1});
    assert_eq!(coefficients.chroma_counts[c],if mode=="zero" {[0;4]} else {[1;4]});
    for i in 0..4 {
        assert_eq!(coefficients.chroma_ac[c][i][1],0);
        assert_eq!(coefficients.chroma_ac[c][i][4],if mode=="zero" {0} else if (i+c)%2==0 {1} else {-1});
    }
}
for (i,levels) in coefficients.luma4.iter().enumerate() {
                                                assert_eq!(levels[1],0);
                                                assert_eq!(levels[4],if mode=="zero" {0} else if i%2==0 {1} else {-1});
                                            }
                                        },_=>panic!("wrong CABAC residual block {name}"),
                                    }
                                            assert!(syntax.read_macroblock().unwrap().is_none());
                                        }
                                    }
                                    let p = decoder
                                        .decode_order(&packet)
                                        .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
                                    assert_eq!(p.is_some(), index % 2 == 1);
                                    if let Some(p) = p {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert_eq!(pixels, oracle, "native {name}");
                                decoder.reset();
                            }
                            let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..2 {
                                    let f = player.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            f.sample_index,
                                            f.presentation_time.ticks,
                                            f.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    f.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert_eq!(pixels, oracle, "player {name}");
                                assert!(player.read_frame().unwrap().is_none());
                                player.rewind();
                            }
                            assert_eq!(player.seek_to_sync(2), 0);
                            player.read_frame().unwrap().unwrap();
                            let mut pixels = Vec::new();
                            player
                                .read_frame()
                                .unwrap()
                                .unwrap()
                                .picture
                                .write_planar(&mut pixels)
                                .unwrap();
                            assert_eq!(pixels, &oracle[oracle.len() / 2..]);
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn cabac_p_transform8_fields_match_jm_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["zero", "ac"] {
                for init in 0..3 {
                    for filter in 0..3 {
                        for (aso, scaled) in
                            [(false, false), (false, true), (true, false), (true, true)]
                        {
                            let name = format!(
                                "avc-field-cabac-p-transform8-{depth}bit-{order}-{mode}-init{init}-filter{filter}-scale{}{}",
                                if scaled { 24 } else { 16 },
                                if aso { "-aso" } else { "" }
                            );
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            if aso {
                                assert_eq!(
                                    oracle,
                                    std::fs::read(
                                        root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                    )
                                    .unwrap()
                                );
                            }
                            if init != 0 {
                                let control = name.replace(&format!("init{init}"), "init0");
                                assert_eq!(
                                    oracle,
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap()
                                );
                            }
                            if mode == "ac" && filter == 1 {
                                let control = name.replace("ac-init", "zero-init");
                                let skip =
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap();
                                assert_ne!(
                                    &oracle[oracle.len() / 2..],
                                    &skip[skip.len() / 2..],
                                    "residual not exercised {name}"
                                );
                            }
                            if filter == 1 && scaled {
                                let flat = name.replace("scale24", "scale16");
                                let flat = std::fs::read(root.join(format!("{flat}.yuv"))).unwrap();
                                assert_eq!(
                                    &oracle[..oracle.len() / 2],
                                    &flat[..flat.len() / 2],
                                    "changed reference {name}"
                                );
                                if mode == "zero" {
                                    assert_eq!(oracle, flat);
                                } else {
                                    assert_ne!(
                                        &oracle[oracle.len() / 2..],
                                        &flat[flat.len() / 2..],
                                        "matrix has no effect {name}"
                                    );
                                }
                            }
                            let mut container =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = container.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut packet = Vec::new();
                                let mut pixels = Vec::new();
                                for index in 0..4 {
                                    container.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 2 {
                                        for nal in NalUnits::new(&packet, avc.length_size).unwrap()
                                        {
                                            let h = SliceHeader::parse(nal.unwrap(), &sps, &pps)
                                                .unwrap();
                                            assert_eq!(h.slice_type, SliceType::P);
                                            assert!(h.field_pic);
                                            assert_eq!(h.refs_l0, 1);
                                            assert_eq!(h.cabac_init_idc, init);
                                            let mut syntax =
                                                fvid::codec::avc_cabac_slice::InterCabacSlice::new(
                                                    &h, &sps, &pps, 8192,
                                                )
                                                .unwrap();
                                            let mb = syntax.read_macroblock().unwrap().unwrap();
                                            assert!(syntax.field_decoding());
                                            match mb {
                                        fvid::codec::avc_inter_slice::InterMacroblock::Coded{header,coefficients,..}=>{
                                            assert_eq!(header.residual.pattern,if mode=="ac" {15} else {0});
                                            assert_eq!(header.partitions[0].differences,[[1,-1],[0,0]]);
                                            if mode=="zero" {assert_eq!(coefficients.luma_counts,[0;16]);}
                                            assert_eq!(header.residual.transform8,mode=="ac");
                                            for (i,levels) in coefficients.luma8.iter().enumerate() {
                                                let mut expected=[0;64];
                                                if mode=="ac" {expected[8]=if i%2==0 {1} else {-1};}
                                                assert_eq!(*levels,expected,"8x8 field scan {name}");
                                            }
                                        },_=>panic!("wrong CABAC residual block {name}"),
                                    }
                                            assert!(syntax.read_macroblock().unwrap().is_none());
                                        }
                                    }
                                    let p = decoder
                                        .decode_order(&packet)
                                        .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
                                    assert_eq!(p.is_some(), index % 2 == 1);
                                    if let Some(p) = p {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert!(
                                    pixels == oracle,
                                    "native {name}: first mismatch {:?}",
                                    pixels.iter().zip(&oracle).position(|(a, b)| a != b)
                                );
                                decoder.reset();
                            }
                            let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..2 {
                                    let f = player.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            f.sample_index,
                                            f.presentation_time.ticks,
                                            f.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    f.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert_eq!(pixels, oracle, "player {name}");
                                assert!(player.read_frame().unwrap().is_none());
                                player.rewind();
                            }
                            assert_eq!(player.seek_to_sync(2), 0);
                            player.read_frame().unwrap().unwrap();
                            let mut pixels = Vec::new();
                            player
                                .read_frame()
                                .unwrap()
                                .unwrap()
                                .picture
                                .write_planar(&mut pixels)
                                .unwrap();
                            assert_eq!(pixels, &oracle[oracle.len() / 2..]);
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn cabac_p_partition_fields_match_jm_and_rewind() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for order in ["top-first", "bottom-first"] {
            for mode in ["16x8", "8x16", "8x8", "8x4", "4x8", "4x4"] {
                for init in 0..3 {
                    for filter in 0..3 {
                        for aso in [false, true] {
                            let name = format!(
                                "avc-field-cabac-p-partition-{depth}bit-{order}-{mode}-init{init}-filter{filter}{}",
                                if aso { "-aso" } else { "" }
                            );
                            let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
                            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
                            if aso {
                                assert_eq!(
                                    oracle,
                                    std::fs::read(
                                        root.join(format!("{}.yuv", name.trim_end_matches("-aso")))
                                    )
                                    .unwrap()
                                );
                            }
                            if init != 0 {
                                let control = name.replace(&format!("init{init}"), "init0");
                                assert_eq!(
                                    oracle,
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap()
                                );
                            }
                            if filter == 1 {
                                let control = format!(
                                    "avc-field-cabac-p-{depth}bit-{order}-coded-init{init}-filter1{}",
                                    if aso { "-aso" } else { "" }
                                );
                                let control =
                                    std::fs::read(root.join(format!("{control}.yuv"))).unwrap();
                                assert_ne!(
                                    &oracle[oracle.len() / 2..],
                                    &control[control.len() / 2..],
                                    "partition has no effect {name}"
                                );
                            }
                            let mut container =
                                Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
                            let config = container.tracks()[0].configuration.clone();
                            let avc = AvcConfig::parse(&config).unwrap();
                            let sps = Sps::parse(avc.sps[0]).unwrap();
                            let pps = Pps::parse(avc.pps[0], &sps).unwrap();
                            let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
                            for _ in 0..2 {
                                let mut packet = Vec::new();
                                let mut pixels = Vec::new();
                                for index in 0..4 {
                                    container.read_packet(0, index, &mut packet).unwrap();
                                    if index >= 2 {
                                        for nal in NalUnits::new(&packet, avc.length_size).unwrap()
                                        {
                                            let h = SliceHeader::parse(nal.unwrap(), &sps, &pps)
                                                .unwrap();
                                            assert_eq!(h.slice_type, SliceType::P);
                                            assert!(h.field_pic);
                                            assert_eq!(h.refs_l0, 1);
                                            assert_eq!(h.cabac_init_idc, init);
                                            let mut syntax =
                                                fvid::codec::avc_cabac_slice::InterCabacSlice::new(
                                                    &h, &sps, &pps, 8192,
                                                )
                                                .unwrap();
                                            let mb = syntax.read_macroblock().unwrap().unwrap();
                                            assert!(syntax.field_decoding());
                                            match mb {
                                                fvid::codec::avc_inter_slice::InterMacroblock::Coded{header,..}=>{
                                                    assert_eq!(header.residual.pattern,0);
                                                    let (count,size)=match mode {"16x8"=>(2,[16,8]),"8x16"=>(2,[8,16]),"8x8"=>(4,[8,8]),"8x4"=>(8,[8,4]),"4x8"=>(8,[4,8]),_ =>(16,[4,4])};
                                                    assert_eq!(header.partitions.len(),count);
                                                    for (i,part) in header.partitions.iter().enumerate() {
                                                        assert_eq!(part.size,size);
                                                        assert_eq!(part.differences[0],if i%2==0 {[1,-1]} else {[-1,1]});
                                                        assert_eq!(part.references[0],Some(0));
                                                    }
                                                },_=>panic!("wrong CABAC partition block {name}"),
                                            }
                                            assert!(syntax.read_macroblock().unwrap().is_none());
                                        }
                                    }
                                    let p = decoder
                                        .decode_order(&packet)
                                        .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
                                    assert_eq!(p.is_some(), index % 2 == 1);
                                    if let Some(p) = p {
                                        p.write_planar(&mut pixels).unwrap();
                                    }
                                }
                                assert_eq!(pixels, oracle, "native {name}");
                                decoder.reset();
                            }
                            let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
                                Cursor::new(&video),
                                Default::default(),
                                16 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut pixels = Vec::new();
                                for index in 0..2 {
                                    let f = player.read_frame().unwrap().unwrap();
                                    assert_eq!(
                                        (
                                            f.sample_index,
                                            f.presentation_time.ticks,
                                            f.duration.ticks
                                        ),
                                        (index * 2, index as i64 * 2, 2)
                                    );
                                    f.picture.write_planar(&mut pixels).unwrap();
                                }
                                assert_eq!(pixels, oracle, "player {name}");
                                assert!(player.read_frame().unwrap().is_none());
                                player.rewind();
                            }
                            assert_eq!(player.seek_to_sync(2), 0);
                            player.read_frame().unwrap().unwrap();
                            let mut pixels = Vec::new();
                            player
                                .read_frame()
                                .unwrap()
                                .unwrap()
                                .picture
                                .write_planar(&mut pixels)
                                .unwrap();
                            assert_eq!(pixels, &oracle[oracle.len() / 2..]);
                        }
                    }
                }
            }
        }
    }
}
