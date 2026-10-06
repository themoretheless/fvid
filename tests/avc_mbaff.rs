//! MBAFF intra pixel acceptance and distinct remaining inter/CABAC refusals.
use fvid::{
    codec::{
        avc::{Pps, Sps},
        avc_decoder::AvcDecoder,
        config::AvcConfig,
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
#[test]
fn cavlc_mbaff_inter_dispatch_consumes_owned_ip_slices() {
    use fvid::codec::{
        avc_inter_slice::{InterCavlcSlice, InterMacroblock},
        avc_slice::{SliceHeader, SliceType},
        config::NalUnits,
    };
    let mut coded = 0;
    let mut fields = 0;
    for video in [
        include_bytes!("fixtures/playback-errors/avc-mbaff-cavlc.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-mbaff-field-cavlc.mp4").as_slice(),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        for packet_index in 1..3 {
            let mut packet = Vec::new();
            input.read_packet(0, packet_index, &mut packet).unwrap();
            let nal = NalUnits::new(&packet, config.length_size)
                .unwrap()
                .map(|n| n.unwrap())
                .find(|n| n[0] & 31 == 1)
                .unwrap();
            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
            assert_eq!(header.slice_type, SliceType::P);
            for _ in 0..2 {
                let mut syntax = InterCavlcSlice::new_mbaff(&header, &sps, &pps, 65536).unwrap();
                for address in 0..16 {
                    let block = syntax.read_macroblock().unwrap().unwrap();
                    let actual = match block {
                        InterMacroblock::Skip { address, .. } => address,
                        InterMacroblock::Intra(mb) => mb.address as usize,
                        InterMacroblock::Coded { address, .. } => {
                            coded += 1;
                            address
                        }
                    };
                    assert_eq!(actual, address);
                    if syntax.field_decoding() {
                        fields += 1;
                    }
                    assert!(syntax.pair_field(address / 2).is_some());
                }
                assert!(syntax.read_macroblock().unwrap().is_none());
                assert_eq!(syntax.bit_position(), header.rbsp.len() * 8);
            }
        }
    }
    assert!(coded > 0);
    assert!(fields > 0);
}
#[test]
fn cabac_mbaff_intra_reader_consumes_complete_pairs_and_slice_end() {
    use fvid::codec::{
        avc_cabac_macroblock::IntraCabacReader, avc_slice::SliceHeader, config::NalUnits,
    };
    for (video, field) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.mp4").as_slice(),
            false,
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.mp4").as_slice(),
            true,
        ),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let nal = NalUnits::new(&packet, config.length_size)
            .unwrap()
            .map(|n| n.unwrap())
            .find(|n| n[0] & 31 == 5)
            .unwrap();
        let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
        let mut syntax = IntraCabacReader::new_mbaff(&header, &sps, &pps, 16).unwrap();
        for address in 0..16 {
            let mb = syntax.read_macroblock().unwrap().unwrap();
            assert_eq!(mb.address as usize, address);
            assert_eq!(syntax.field_decoding(), field);
            assert_eq!(syntax.is_finished(), address == 15);
        }
        assert!(syntax.read_macroblock().unwrap().is_none());
    }
}
#[test]
fn cabac_first_pair_flag_matches_owned_frame_and_field_syntax() {
    use fvid::codec::{
        avc_cabac::AvcCabac, avc_cabac_inter::field_decoding_flag, avc_mbaff::PairMode,
        avc_slice::SliceHeader, config::NalUnits,
    };
    for (video, expected) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.mp4").as_slice(),
            false,
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.mp4").as_slice(),
            true,
        ),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert!(pps.cabac && sps.mb_adaptive_frame_field);
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let nal = NalUnits::new(&packet, config.length_size)
            .unwrap()
            .map(|n| n.unwrap())
            .find(|n| n[0] & 31 == 5)
            .unwrap();
        let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
        let mut arithmetic = AvcCabac::new(
            &header.rbsp,
            header.entropy_bit_offset,
            header.slice_type,
            header.cabac_init_idc as u8,
            header.slice_qp,
        )
        .unwrap();
        let mut pair = PairMode::default();
        assert_eq!(
            pair.read(0, false, || field_decoding_flag(
                &mut arithmetic,
                [false; 2]
            ))
            .unwrap(),
            expected
        );
        let position = arithmetic.bit_position();
        assert_eq!(
            pair.read(1, false, || panic!(
                "bottom inherits without an arithmetic bin"
            ))
            .unwrap(),
            expected
        );
        assert_eq!(arithmetic.bit_position(), position);
    }
}
#[test]
fn filtered_high10_mbaff_keeps_precision_and_matches_jm() {
    use fvid::codec::{
        avc_macroblock::{IntraCavlcReader, IntraLuma},
        avc_slice::SliceHeader,
        config::NalUnits,
    };
    let mut eight_blocks = [0; 2];
    for (kind, (video, oracle)) in [
        (include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-filtered-high10-cavlc.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-filtered-high10-cavlc.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-high10-cavlc.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-filtered-high10-cavlc.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-intra-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-intra-filtered-high10-cavlc.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-high10-cavlc.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-intra-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-intra-filtered-high10-cavlc.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-filtered-high10-cabac.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-filtered-high10-cabac.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-high10-cabac.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-filtered-high10-cabac.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-intra-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-intra-filtered-high10-cabac.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-high10-cabac.yuv").as_slice()),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-intra-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-intra-filtered-high10-cabac.yuv").as_slice()),
    ].into_iter().enumerate() {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        assert_eq!(sps.bit_depth_luma, 10);
        assert_eq!(sps.bit_depth_chroma, 10);
        assert!(sps.mb_adaptive_frame_field);
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(pps.cabac, kind >= 7);
        for frame in 0..3 {
            let mut packet = Vec::new();
            input.read_packet(0, frame, &mut packet).unwrap();
            let mut fields = 0;
            let mut blocks = 0;
            for nal in NalUnits::new(&packet, config.length_size).unwrap().map(|n| n.unwrap()).filter(|n| n[0] & 31 == 5) {
                let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
                assert_eq!(header.disable_deblocking_filter_idc, 0);
                let mut cavlc = if pps.cabac { None } else { Some(IntraCavlcReader::new_mbaff(&header, &sps, &pps, 16).unwrap()) };
                let mut cabac = if pps.cabac { Some(fvid::codec::avc_cabac_macroblock::IntraCabacReader::new_mbaff(&header, &sps, &pps, 16).unwrap()) } else { None };
                loop {
                    let mb = match (&mut cavlc, &mut cabac) { (Some(r),_)=>r.read_macroblock().unwrap(), (_,Some(r))=>r.read_macroblock().unwrap(), _=>unreachable!() };
                    let Some(mb) = mb else { break };
                    let field = match (&cavlc,&cabac) { (Some(r),_)=>r.field_decoding(), (_,Some(r))=>r.field_decoding(), _=>unreachable!() };
                    let address = mb.address as usize;
                    let expected = match kind % 7 {
                        0 | 6 => true, 1 => false, 2 => address / 2 % 4 >= 2,
                        3 => address / 2 % 4 < 2, 4 => address >= 8, 5 => address < 8,
                        _ => unreachable!(),
                    };
                    assert_eq!(field, expected, "High10 topology at block {address}");
                    fields += usize::from(field);
                    eight_blocks[usize::from(pps.cabac)] += usize::from(matches!(mb.luma, IntraLuma::Blocks8 { .. }));
                    blocks += 1;
                }
            }
            assert_eq!(blocks, 16);
            assert_eq!(fields, [16, 0, 8, 8, 8, 8, 16][kind % 7]);
        }
        let mut playback = fvid::playback_mp4::Mp4VideoReader::open_software(Cursor::new(video), Default::default(), 1 << 20).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            let mut count = 0;
            while let Some(frame) = playback.read_frame().unwrap() {
                assert_eq!(frame.picture.bit_depth, 10);
                frame.picture.write_planar(&mut actual).unwrap();
                count += 1;
            }
            assert_eq!(count, 3);
            assert!(actual == oracle, "High10 MBAFF differs at {:?}", actual.iter().zip(oracle).position(|(a,b)| a != b));
            assert!(actual.chunks_exact(2).any(|s| u16::from_le_bytes([s[0],s[1]]) & 3 != 0), "must preserve real ten-bit precision");
            playback.rewind();
        }
    }
    assert!(
        eight_blocks.iter().all(|n| *n > 0),
        "High10 fixtures must exercise 8x8 reconstruction"
    );
}
#[test]
fn filtered_first_intra_matches_jm_and_reset() {
    for (video, oracle) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cavlc.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-cavlc.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cavlc.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cavlc.yuv").as_slice(),
        ),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        for _ in 0..2 {
            let picture = decoder.decode_order(&packet).unwrap().unwrap();
            let mut actual = Vec::new();
            picture.write_planar(&mut actual).unwrap();
            assert!(
                actual == oracle[..actual.len()],
                "filtered intra differs at {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
}
#[test]
fn owned_mbaff_streams_reach_the_specific_entropy_reconstruction_limit() {
    for (bytes, cabac, expected) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.mp4").as_slice(),
            true,
            "unsupported inter-picture reconstruction tools",
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cavlc.mp4").as_slice(),
            false,
            "unsupported inter-picture reconstruction tools",
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.mp4").as_slice(),
            true,
            "unsupported inter-picture reconstruction tools",
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cavlc.mp4").as_slice(),
            false,
            "unsupported inter-picture reconstruction tools",
        ),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        assert_eq!(input.tracks()[0].samples.len(), 3);
        let configuration = input.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        assert!(!sps.frame_mbs_only);
        assert!(sps.mb_adaptive_frame_field);
        assert_eq!(pps.cabac, cabac);
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        assert!(decoder.decode_order(&packet).unwrap().is_some());
        input.read_packet(0, 1, &mut packet).unwrap();
        let error = decoder.decode_order(&packet).unwrap_err().to_string();
        assert!(error.contains(expected), "unrelated error: {error}");
    }
}

#[test]
fn actual_cavlc_mbaff_slice_reads_field_flag_before_macroblock_type() {
    use fvid::codec::{
        avc_mbaff::PairMode,
        avc_slice::{SliceHeader, SliceType},
        bits::BitReader,
        config::NalUnits,
    };
    let mut input = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/avc-mbaff-cavlc.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let avc = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 0, &mut packet).unwrap();
    let nal = NalUnits::new(&packet, avc.length_size)
        .unwrap()
        .map(|n| n.unwrap())
        .find(|n| n[0] & 31 == 5)
        .unwrap();
    let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
    assert_eq!(header.slice_type, SliceType::I);
    assert!(!header.field_pic);
    let mut bits = BitReader::new(&header.rbsp);
    bits.skip(header.entropy_bit_offset).unwrap();
    let mut modes = PairMode::default();
    let position = bits.position();
    modes
        .read(header.first_mb as usize * 2, false, || bits.bit())
        .unwrap();
    assert_eq!(bits.position(), position + 1);
    assert!(bits.unsigned_golomb().unwrap() <= 25);
}

#[test]
#[ignore = "Inter MBAFF playback is not connected yet"]
fn mbaff_playback_matches_every_jm_sample_and_rewinds() {
    for (video, oracle) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cavlc.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-cavlc.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cavlc.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cavlc.yuv").as_slice(),
        ),
    ] {
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
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
            assert_eq!(frames, 3);
            assert!(
                actual == oracle,
                "MBAFF pixels differ from independent JM decoder"
            );
            reader.rewind();
        }
    }
}

#[test]
fn cavlc_mbaff_intra_reader_consumes_all_pairs_and_rbsp_trailer() {
    use fvid::codec::{
        avc_macroblock::{IntraCavlcReader, IntraLuma},
        avc_slice::SliceHeader,
        bits::BitReader,
        config::NalUnits,
    };
    for (video, field) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cavlc.mp4").as_slice(),
            Some(false),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cavlc.mp4").as_slice(),
            Some(true),
        ),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-cavlc.mp4").as_slice(), None),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-filtered-cavlc.mp4").as_slice(), None),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-intra-filtered-cavlc.mp4").as_slice(), None),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-cavlc.mp4").as_slice(), None),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-high10-cavlc.mp4").as_slice(), None),
        (include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-high10-cavlc.mp4").as_slice(), None),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let nal = NalUnits::new(&packet, avc.length_size)
            .unwrap()
            .map(|n| n.unwrap())
            .find(|n| n[0] & 31 == 5)
            .unwrap();
        let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
        let mut reader = IntraCavlcReader::new_mbaff(&header, &sps, &pps, 16).unwrap();
        let mut embedded = IntraCavlcReader::new_context_mbaff(&header, &sps, &pps, 16).unwrap();
        let mut bits = BitReader::new(&header.rbsp);
        bits.skip(header.entropy_bit_offset).unwrap();
        let mut qp = header.slice_qp;
        let mut count = 0;
        let mut mode = false;
        let mut first_mode = None;
        while let Some(mb) = reader.read_macroblock().unwrap() {
            assert_eq!(mb.address as usize, count);
            if count % 2 == 0 { mode = bits.bit().unwrap(); }
            first_mode.get_or_insert(mode);
            assert_eq!(reader.field_decoding(), mode);
            if let Some(expected) = field { assert_eq!(mode, expected); }
            let kind = bits.unsigned_golomb().unwrap();
            let other = embedded
                .read_embedded_mbaff(&mut bits, count as u32, qp, kind, mode)
                .unwrap();
            assert_eq!(other.address, mb.address);
            assert_eq!(other.qp, mb.qp);
            assert_eq!(format!("{:?}", other.luma), format!("{:?}", mb.luma));
            assert_eq!(
                format!("{:?}", other.chroma_mode),
                format!("{:?}", mb.chroma_mode)
            );
            assert_eq!(other.coded_block_pattern, mb.coded_block_pattern);
            assert_eq!(other.luma_dc, mb.luma_dc);
            assert_eq!(other.luma_levels, mb.luma_levels);
            assert_eq!(other.chroma_dc, mb.chroma_dc);
            assert_eq!(other.chroma_ac, mb.chroma_ac);
            assert_eq!(
                embedded.counts(count).unwrap(),
                reader.counts(count).unwrap()
            );
            assert_eq!(bits.position(), reader.bit_position());
            if !matches!(other.luma, IntraLuma::Pcm { .. }) {
                qp = other.qp;
            }
            count += 1;
        }
        assert_eq!(count, 16);
        bits.finish_rbsp().unwrap();
        assert_eq!(bits.position(), reader.bit_position());
        assert!(embedded.record_pair_mode(0, !first_mode.unwrap()).is_err());
        assert!(embedded.record_pair_mode(16, mode).is_err());
        assert!(reader.read_macroblock().unwrap().is_none());
    }
}

#[test]
fn cavlc_frame_field_and_mixed_intra_match_jm_samples() {
    use fvid::codec::{avc_mbaff_picture, avc_slice::SliceHeader, config::NalUnits};
    for (kind, (video, oracle)) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-unfiltered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-unfiltered-cavlc.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-unfiltered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-unfiltered-cavlc.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-unfiltered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-unfiltered-cavlc.yuv")
                .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-unfiltered-cavlc.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-unfiltered-cavlc.yuv"
            )
            .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-filtered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-filtered-cavlc.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-filtered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-filtered-cavlc.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-cavlc.yuv")
                .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-filtered-cavlc.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-filtered-cavlc.yuv"
            )
            .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-vertical-intra-filtered-cavlc.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-vertical-intra-filtered-cavlc.yuv"
            )
            .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-cavlc.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-cavlc.yuv"
            )
            .as_slice(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        let mut actual = Vec::new();
        let mut packet = Vec::new();
        for index in 0..3 {
            input.read_packet(0, index, &mut packet).unwrap();
            let nal = NalUnits::new(&packet, avc.length_size)
                .unwrap()
                .map(|n| n.unwrap())
                .find(|n| n[0] & 31 == 5)
                .unwrap();
            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
            assert_eq!(
                header.disable_deblocking_filter_idc,
                if kind < 4 { 1 } else { 0 }
            );
            let mut syntax =
                fvid::codec::avc_macroblock::IntraCavlcReader::new_mbaff(&header, &sps, &pps, 16)
                    .unwrap();
            let mut fields = 0;
            let mut blocks = 0;
            while syntax.read_macroblock().unwrap().is_some() {
                if kind >= 8 {
                    assert_eq!(
                        syntax.field_decoding(),
                        if kind == 8 { blocks >= 8 } else { blocks < 8 },
                        "vertical topology must be present in actual syntax"
                    );
                }
                blocks += 1;
                fields += usize::from(syntax.field_decoding());
            }
            assert_eq!(blocks, 16);
            assert_eq!(fields, [16, 0, 8, 8, 16, 0, 8, 8, 8, 8][kind]);
            let picture =
                avc_mbaff_picture::decode_intra_picture(&header, &sps, &pps, 1 << 20).unwrap();
            picture.write_planar(&mut actual).unwrap();
            if kind >= 4 && index == 0 {
                let mut disabled = SliceHeader::parse(nal, &sps, &pps).unwrap();
                disabled.disable_deblocking_filter_idc = 1;
                let unfiltered =
                    avc_mbaff_picture::decode_intra_picture(&disabled, &sps, &pps, 1 << 20)
                        .unwrap();
                let mut pixels = Vec::new();
                unfiltered.write_planar(&mut pixels).unwrap();
                assert!(
                    pixels != actual,
                    "filtered fixture must exercise deblocking"
                );
            }
        }
        let mut playback = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            1 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut pixels = Vec::new();
            let mut count = 0;
            while let Some(frame) = playback.read_frame().unwrap() {
                frame.picture.write_planar(&mut pixels).unwrap();
                count += 1;
            }
            assert_eq!(count, 3);
            assert!(pixels == oracle, "native MBAFF playback differs from JM");
            playback.rewind();
        }
        assert_eq!(actual.len(), oracle.len());
        assert!(
            actual == oracle,
            "first MBAFF sample mismatch: {:?}",
            actual.iter().zip(oracle.iter()).position(|(a, b)| a != b)
        );
    }
}

#[test]
fn mbaff_two_slices_match_jm_and_rewind() {
    for (video, oracle) in [
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-field-multislice-intra-unfiltered-cavlc.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-field-multislice-intra-unfiltered-cavlc.yuv"
            )
            .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-field-multislice-intra-filtered-cavlc.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-field-multislice-intra-filtered-cavlc.yuv"
            )
            .as_slice(),
        ),
    ] {
        use fvid::codec::{avc_slice::SliceHeader, config::NalUnits};
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let headers: Vec<_> = NalUnits::new(&packet, avc.length_size)
            .unwrap()
            .map(|n| n.unwrap())
            .filter(|n| n[0] & 31 == 5)
            .map(|n| SliceHeader::parse(n, &sps, &pps).unwrap())
            .collect();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].first_mb, 0);
        assert!(headers[1].first_mb > 0);
        let mut playback = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            1 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            let mut count = 0;
            while let Some(frame) = playback.read_frame().unwrap() {
                frame.picture.write_planar(&mut actual).unwrap();
                count += 1;
            }
            assert_eq!(count, 3);
            assert!(
                actual == oracle,
                "MBAFF multi-slice differs at {:?}",
                actual.iter().zip(oracle.iter()).position(|(a, b)| a != b)
            );
            playback.rewind();
        }
    }
}

#[test]
fn cabac_filtered_intra_topologies_match_jm_and_rewind() {
    use fvid::codec::{
        avc_cabac_macroblock::IntraCabacReader, avc_slice::SliceHeader, config::NalUnits,
    };
    for (kind, (video, oracle)) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-filtered-cabac.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-intra-filtered-cabac.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-filtered-cabac.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-intra-filtered-cabac.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-cabac.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-intra-filtered-cabac.yuv")
                .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-filtered-cabac.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-reverse-intra-filtered-cabac.yuv"
            )
            .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-vertical-intra-filtered-cabac.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-vertical-intra-filtered-cabac.yuv"
            )
            .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-cabac.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-intra-filtered-cabac.yuv"
            )
            .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-field-multislice-intra-filtered-cabac.mp4"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/avc-mbaff-field-multislice-intra-filtered-cabac.yuv"
            )
            .as_slice(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert!(pps.cabac && sps.mb_adaptive_frame_field);
        for frame in 0..3 {
            let mut packet = Vec::new();
            input.read_packet(0, frame, &mut packet).unwrap();
            let mut blocks = 0;
            for nal in NalUnits::new(&packet, config.length_size)
                .unwrap()
                .map(|n| n.unwrap())
                .filter(|n| n[0] & 31 == 5)
            {
                let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
                let mut syntax = IntraCabacReader::new_mbaff(&header, &sps, &pps, 16).unwrap();
                while let Some(mb) = syntax.read_macroblock().unwrap() {
                    let address = mb.address as usize;
                    let expected = match kind {
                        0 | 6 => true,
                        1 => false,
                        2 => address / 2 % 4 >= 2,
                        3 => address / 2 % 4 < 2,
                        4 => address >= 8,
                        5 => address < 8,
                        _ => unreachable!(),
                    };
                    assert_eq!(
                        syntax.field_decoding(),
                        expected,
                        "CABAC topology {kind} block {address}"
                    );
                    blocks += 1;
                }
            }
            assert_eq!(blocks, 16);
            if frame == 0 {
                let mut headers: Vec<_> = NalUnits::new(&packet, config.length_size)
                    .unwrap()
                    .map(|n| n.unwrap())
                    .filter(|n| n[0] & 31 == 5)
                    .map(|n| SliceHeader::parse(n, &sps, &pps).unwrap())
                    .collect();
                let picture = fvid::codec::avc_mbaff_picture::decode_intra_slices(
                    &headers.iter().collect::<Vec<_>>(),
                    &sps,
                    &pps,
                    1 << 20,
                )
                .unwrap();
                let mut filtered = Vec::new();
                picture.write_planar(&mut filtered).unwrap();
                assert!(filtered == oracle[..filtered.len()]);
                for header in &mut headers {
                    header.disable_deblocking_filter_idc = 1;
                }
                let picture = fvid::codec::avc_mbaff_picture::decode_intra_slices(
                    &headers.iter().collect::<Vec<_>>(),
                    &sps,
                    &pps,
                    1 << 20,
                )
                .unwrap();
                let mut unfiltered = Vec::new();
                picture.write_planar(&mut unfiltered).unwrap();
                assert!(
                    unfiltered != filtered,
                    "CABAC fixture must exercise deblocking"
                );
            }
        }
        let mut playback = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            1 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            let mut frames = 0;
            while let Some(frame) = playback.read_frame().unwrap() {
                frame.picture.write_planar(&mut actual).unwrap();
                frames += 1;
            }
            assert_eq!(frames, 3);
            assert!(
                actual == oracle,
                "CABAC topology {kind} differs at {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            playback.rewind();
        }
    }
}
