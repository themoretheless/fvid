//! MBAFF I/P/B pixel acceptance with independent saved JM references.
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
fn owned_mbaff_cabac_inter_reconstruction_succeeds_and_resets() {
    for (bytes, cabac) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.mp4").as_slice(),
            true,
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.mp4").as_slice(),
            true,
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
        for _ in 0..2 {
            for index in 0..3 {
                input.read_packet(0, index, &mut packet).unwrap();
                assert!(decoder.decode_order(&packet).unwrap().is_some());
            }
            decoder.reset();
        }
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
fn mbaff_playback_matches_every_jm_sample_and_rewinds() {
    for (video, oracle) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-cabac.yuv").as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-cabac.yuv").as_slice(),
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

#[test]
fn cavlc_mbaff_p_pictures_match_jm_and_restart() {
    use fvid::codec::{
        avc_mbaff_picture::{decode_intra_slices, decode_p_slices, decode_p_slices_unfiltered},
        avc_slice::{SliceHeader, SliceType},
        config::NalUnits,
    };
    for (case, (video, oracle)) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-unfiltered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-unfiltered-cavlc.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-unfiltered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-unfiltered-cavlc.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-filtered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-filtered-cavlc.yuv")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-filtered-cavlc.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-filtered-cavlc.yuv")
                .as_slice(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut playback = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            16 << 20,
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
            assert_eq!(actual, oracle, "native playback case {case}");
            playback.rewind();
        }
        for _ in 0..2 {
            let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
            let configuration = input.tracks()[0].configuration.clone();
            let config = AvcConfig::parse(&configuration).unwrap();
            let sps = Sps::parse(config.sps[0]).unwrap();
            let pps = Pps::parse(config.pps[0], &sps).unwrap();
            let mut previous = None;
            for index in 0..3 {
                let mut packet = Vec::new();
                input.read_packet(0, index, &mut packet).unwrap();
                let headers: Vec<_> = NalUnits::new(&packet, config.length_size)
                    .unwrap()
                    .map(|n| n.unwrap())
                    .filter(|n| matches!(n[0] & 31, 1 | 5))
                    .map(|n| SliceHeader::parse(n, &sps, &pps).unwrap())
                    .collect();
                assert_eq!(headers.len(), 1);
                assert_eq!(
                    headers[0].disable_deblocking_filter_idc,
                    if case < 2 { 1 } else { 0 }
                );
                let borrowed: Vec<_> = headers.iter().collect();
                let picture = if index == 0 {
                    assert_eq!(headers[0].slice_type, SliceType::I);
                    decode_intra_slices(&borrowed, &sps, &pps, 1 << 20).unwrap()
                } else {
                    assert_eq!(headers[0].slice_type, SliceType::P);
                    let mut syntax = fvid::codec::avc_inter_slice::InterCavlcSlice::new_mbaff(
                        &headers[0],
                        &sps,
                        &pps,
                        65536,
                    )
                    .unwrap();
                    let mut fields = 0;
                    let mut coded = 0;
                    let mut total = 0;
                    while let Some(block) = syntax.read_macroblock().unwrap() {
                        fields += usize::from(syntax.field_decoding());
                        coded += usize::from(matches!(
                            block,
                            fvid::codec::avc_inter_slice::InterMacroblock::Coded { .. }
                        ));
                        total += 1;
                    }
                    assert_eq!(total, 16);
                    assert_eq!(fields, if case % 2 == 0 { 0 } else { 16 });
                    assert!(coded > 0);
                    let refs = [previous.as_ref().unwrap()];
                    for changed in 0..5 {
                        let nal = NalUnits::new(&packet, config.length_size)
                            .unwrap()
                            .map(|n| n.unwrap())
                            .find(|n| n[0] & 31 == 1)
                            .unwrap();
                        let mut other = SliceHeader::parse(nal, &sps, &pps).unwrap();
                        other.first_mb = 1;
                        match changed {
                            0 => other.frame_num += 1,
                            1 => other.pps_id += 1,
                            2 => other.poc_lsb = Some(other.poc_lsb.unwrap_or(0) + 1),
                            3 => other.delta_poc[0] += 1,
                            _ => other.nal_ref_idc = if other.nal_ref_idc == 0 { 1 } else { 0 },
                        }
                        let error = decode_p_slices_unfiltered(
                            &[&headers[0], &other],
                            &sps,
                            &pps,
                            &[[&refs, &[]], [&refs, &[]]],
                            1 << 20,
                        )
                        .err()
                        .unwrap();
                        assert!(error.to_string().contains("different pictures"));
                    }
                    let unfiltered =
                        decode_p_slices_unfiltered(&borrowed, &sps, &pps, &[[&refs, &[]]], 1 << 20)
                            .unwrap()
                            .0;
                    if case >= 2 {
                        let filtered =
                            decode_p_slices(&borrowed, &sps, &pps, &[[&refs, &[]]], 1 << 20)
                                .unwrap()
                                .0;
                        assert!(
                            filtered.y != unfiltered.y
                                || filtered.cb != unfiltered.cb
                                || filtered.cr != unfiltered.cr,
                            "P fixture must exercise filtering"
                        );
                        filtered
                    } else {
                        unfiltered
                    }
                };
                let actual: Vec<_> = picture
                    .y
                    .iter()
                    .chain(&picture.cb)
                    .chain(&picture.cr)
                    .copied()
                    .collect();
                let expected = &oracle[index * 6144..(index + 1) * 6144];
                for (sample, (&a, &b)) in actual.iter().zip(expected).enumerate() {
                    assert_eq!(a, u16::from(b), "case {case} frame {index} sample {sample}");
                }
                previous = Some(picture);
            }
        }
    }
}

#[test]
fn original_cavlc_mbaff_playback_matches_every_jm_sample_and_rewinds() {
    for (video, oracle) in [
        (
            include_bytes!("fixtures/playback-errors/avc-mbaff-cavlc.mp4").as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-cavlc.yuv").as_slice(),
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
fn cavlc_mbaff_inter_mixed_high10_and_multislice_match_jm_and_rewind() {
    use fvid::codec::{
        avc_inter_slice::{InterCavlcSlice, InterMacroblock},
        avc_macroblock::{IntraCavlcReader, IntraLuma},
        avc_slice::{SliceHeader, SliceType},
        config::NalUnits,
    };
    let mut high10_eight = 0;
    for (name,video,oracle) in [
        ("mixed-inter-filtered-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-inter-filtered-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-inter-filtered-cavlc.yuv").as_slice()),
        ("mixed-reverse-inter-filtered-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-inter-filtered-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-inter-filtered-cavlc.yuv").as_slice()),
        ("mixed-vertical-inter-filtered-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-inter-filtered-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-inter-filtered-cavlc.yuv").as_slice()),
        ("mixed-vertical-reverse-inter-filtered-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-inter-filtered-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-inter-filtered-cavlc.yuv").as_slice()),
        ("field-multislice-inter-filtered-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-inter-filtered-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-inter-filtered-cavlc.yuv").as_slice()),
        ("frame-inter-filtered-high10-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-filtered-high10-cavlc.yuv").as_slice()),
        ("field-inter-filtered-high10-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-filtered-high10-cavlc.yuv").as_slice()),
        ("mixed-inter-filtered-high10-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-inter-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-inter-filtered-high10-cavlc.yuv").as_slice()),
        ("mixed-reverse-inter-filtered-high10-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-inter-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-inter-filtered-high10-cavlc.yuv").as_slice()),
        ("mixed-vertical-inter-filtered-high10-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-inter-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-inter-filtered-high10-cavlc.yuv").as_slice()),
        ("mixed-vertical-reverse-inter-filtered-high10-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-inter-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-inter-filtered-high10-cavlc.yuv").as_slice()),
        ("field-multislice-inter-filtered-high10-cavlc", include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-inter-filtered-high10-cavlc.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-inter-filtered-high10-cavlc.yuv").as_slice()),
    ] {
        let mut input=Mp4Reader::open(Cursor::new(video),Default::default()).unwrap();
        let configuration=input.tracks()[0].configuration.clone();
        let config=AvcConfig::parse(&configuration).unwrap();
        let sps=Sps::parse(config.sps[0]).unwrap();
        let pps=Pps::parse(config.pps[0],&sps).unwrap();
        let depth=if name.contains("high10") {10} else {8};
        assert_eq!(sps.bit_depth_luma,depth);assert_eq!(sps.bit_depth_chroma,depth);
        assert!(!pps.cabac && sps.mb_adaptive_frame_field);
        let mut coded_inter=0;
        for index in 0..3 {
            let mut packet=Vec::new();input.read_packet(0,index,&mut packet).unwrap();
            let headers:Vec<_>=NalUnits::new(&packet,config.length_size).unwrap().map(|n|n.unwrap())
                .filter(|n|matches!(n[0]&31,1|5)).map(|n|SliceHeader::parse(n,&sps,&pps).unwrap()).collect();
            assert_eq!(headers.len(),if name.contains("multislice") {2} else {1});
            let mut count=0;let mut fields=0;
            for header in &headers {
                assert_eq!(header.disable_deblocking_filter_idc,0);
                assert_eq!(header.first_mb as usize*2,count);
                let mut intra=if index==0 {Some(IntraCavlcReader::new_mbaff(header,&sps,&pps,16).unwrap())} else {None};
                let mut inter=if index!=0 {Some(InterCavlcSlice::new_mbaff(header,&sps,&pps,65536).unwrap())} else {None};
                loop {
                    let (address,field)=if let Some(reader)=&mut intra {
                        let Some(mb)=reader.read_macroblock().unwrap() else {break};
                        if depth==10 {high10_eight+=usize::from(matches!(mb.luma,IntraLuma::Blocks8{..}));}
                        (mb.address as usize,reader.field_decoding())
                    } else {
                        assert_eq!(header.slice_type,SliceType::P);
                        let reader=inter.as_mut().unwrap();
                        let Some(block)=reader.read_macroblock().unwrap() else {break};
                        let address=match block {
                            InterMacroblock::Skip{address,..}=>address,
                            InterMacroblock::Intra(mb)=>mb.address as usize,
                            InterMacroblock::Coded{address,header,..}=>{coded_inter+=1;if depth==10 {high10_eight+=usize::from(header.residual.transform8);}address}
                        };
                        (address,reader.field_decoding())
                    };
                    assert_eq!(address,count);
                    let expected=if name.starts_with("field") {true} else if name.starts_with("frame") {false}
                        else if name.contains("vertical-reverse") {address<8}
                        else if name.contains("vertical") {address>=8}
                        else if name.starts_with("mixed-reverse") {address/2%4<2}
                        else {address/2%4>=2};
                    assert_eq!(field,expected,"{name} frame {index} address {address}");
                    fields+=usize::from(field);count+=1;
                }
            }
            assert_eq!(count,16);
            assert_eq!(fields,if name.starts_with("field") {16} else if name.starts_with("frame") {0} else {8});
        }
        assert!(coded_inter>0);
        let mut playback=fvid::playback_mp4::Mp4VideoReader::open_software(Cursor::new(video),Default::default(),16<<20).unwrap();
        for _ in 0..2 {
            let mut actual=Vec::new();let mut frames=0;
            while let Some(frame)=playback.read_frame().unwrap() {frame.picture.write_planar(&mut actual).unwrap();frames+=1;}
            assert_eq!(frames,3);
            assert!(actual==oracle,"{name} mismatch at {:?}",actual.iter().zip(oracle).position(|(a,b)|a!=b));
            if depth==10 {assert!(actual.chunks_exact(2).any(|v|u16::from_le_bytes([v[0],v[1]])%4!=0));}
            playback.rewind();
        }
    }
    assert!(
        high10_eight > 0,
        "High10 inter corpus must exercise 8x8 transforms"
    );
}

#[test]
fn cabac_mbaff_first_p_pair_flag_and_motion_prefix_use_field_contexts() {
    use fvid::codec::{
        avc_cabac::AvcCabac,
        avc_cabac_inter as syntax,
        avc_cabac_motion::CabacMotionContexts,
        avc_slice::{SliceHeader, SliceType},
        config::NalUnits,
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
        for index in 1..3 {
            let mut packet = Vec::new();
            input.read_packet(0, index, &mut packet).unwrap();
            let nal = NalUnits::new(&packet, config.length_size)
                .unwrap()
                .map(|n| n.unwrap())
                .find(|n| n[0] & 31 == 1)
                .unwrap();
            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
            assert_eq!(header.slice_type, SliceType::P);
            for _ in 0..2 {
                let mut bins = AvcCabac::new(
                    &header.rbsp,
                    header.entropy_bit_offset,
                    header.slice_type,
                    header.cabac_init_idc as u8,
                    header.slice_qp,
                )
                .unwrap();
                assert!(
                    !syntax::skip(&mut bins, SliceType::P, [false; 2]).unwrap(),
                    "owned first P block must be coded"
                );
                assert_eq!(
                    syntax::field_decoding_flag(&mut bins, [false; 2]).unwrap(),
                    field
                );
                let code = syntax::macroblock_type(&mut bins, SliceType::P, [false; 2]).unwrap();
                assert!(code < 5, "owned prefix must use inter motion syntax");
                let mut motion = CabacMotionContexts::new_mbaff(4, 4, 65536).unwrap();
                let parts = motion
                    .read_prediction_mbaff(
                        &mut bins,
                        0,
                        0,
                        code,
                        [header.refs_l0 * if field { 2 } else { 1 }, 0],
                        field,
                        |pair| if pair == 0 { Some(field) } else { None },
                    )
                    .unwrap();
                assert!(!parts.is_empty());
                assert!(parts.iter().all(|p| p.references[0].is_some()));
            }
        }
    }
}

#[test]
fn cabac_mbaff_inter_dispatch_consumes_complete_owned_p_slices() {
    use fvid::codec::{
        avc_cabac_slice::InterCabacSlice, avc_inter_slice::InterMacroblock, avc_slice::SliceHeader,
        config::NalUnits,
    };
    let mut coded = 0;
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
        for index in 1..3 {
            let mut packet = Vec::new();
            input.read_packet(0, index, &mut packet).unwrap();
            let nal = NalUnits::new(&packet, config.length_size)
                .unwrap()
                .map(|n| n.unwrap())
                .find(|n| n[0] & 31 == 1)
                .unwrap();
            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
            for _ in 0..2 {
                let mut reader = InterCabacSlice::new_mbaff(&header, &sps, &pps, 65536).unwrap();
                for address in 0..16 {
                    let block = reader.read_macroblock().unwrap().unwrap();
                    let actual = match block {
                        InterMacroblock::Skip { address, .. } => address,
                        InterMacroblock::Intra(mb) => mb.address as usize,
                        InterMacroblock::Coded { address, .. } => {
                            coded += 1;
                            address
                        }
                    };
                    assert_eq!(actual, address);
                    assert_eq!(reader.field_decoding(), field);
                }
                assert!(reader.read_macroblock().unwrap().is_none());
            }
        }
    }
    assert!(coded > 0);
}

#[test]
fn cabac_mbaff_skipped_pairs_and_skipped_top_use_correct_mode_and_pixels() {
    use fvid::codec::{
        avc_cabac_slice::InterCabacSlice, avc_inter_slice::InterMacroblock, avc_slice::SliceHeader,
        config::NalUnits,
    };
    for (topskip, video, oracle) in [
        (
            false,
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-skipped-cabac.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-skipped-cabac.yuv")
                .as_slice(),
        ),
        (
            false,
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-skipped-cabac.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-skipped-cabac.yuv")
                .as_slice(),
        ),
        (
            true,
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-topskip-cabac.mp4")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-topskip-cabac.yuv")
                .as_slice(),
        ),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        for index in 1..3 {
            let mut packet = Vec::new();
            input.read_packet(0, index, &mut packet).unwrap();
            let nal = NalUnits::new(&packet, config.length_size)
                .unwrap()
                .map(|n| n.unwrap())
                .find(|n| n[0] & 31 == 1)
                .unwrap();
            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
            let mut reader = InterCabacSlice::new_mbaff(&header, &sps, &pps, 65536).unwrap();
            let mut skipped = [false; 16];
            let mut fields = [false; 16];
            for address in 0..16 {
                let block = reader.read_macroblock().unwrap().unwrap();
                let actual = match &block {
                    InterMacroblock::Skip { address, .. }
                    | InterMacroblock::Coded { address, .. } => *address,
                    InterMacroblock::Intra(mb) => mb.address as usize,
                };
                assert_eq!(actual, address);
                skipped[address] = matches!(block, InterMacroblock::Skip { .. });
                fields[address] = reader.field_decoding();
            }
            assert!(reader.read_macroblock().unwrap().is_none());
            if topskip {
                assert!(
                    (0..8).any(|pair| skipped[pair * 2]
                        && !skipped[pair * 2 + 1]
                        && fields[pair * 2]
                        && fields[pair * 2 + 1]),
                    "fixture must exercise skipped top before coded field bottom"
                );
            } else {
                assert!(
                    (0..8).any(|pair| skipped[pair * 2] && skipped[pair * 2 + 1]),
                    "fixture must contain a fully skipped pair"
                );
                for pair in 0..8 {
                    if skipped[pair * 2] && skipped[pair * 2 + 1] {
                        let expected = if pair % 4 != 0 {
                            fields[(pair - 1) * 2]
                        } else if pair >= 4 {
                            fields[(pair - 4) * 2]
                        } else {
                            false
                        };
                        assert_eq!(fields[pair * 2], expected);
                        assert_eq!(fields[pair * 2 + 1], expected);
                    }
                }
            }
        }
        let mut playback = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            16 << 20,
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
                "skip pixels differ at {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            playback.rewind();
        }
    }
}

#[test]
fn truncated_cabac_mbaff_dispatch_cannot_resume_after_entropy_failure() {
    use fvid::codec::{avc_cabac_slice::InterCabacSlice, avc_slice::SliceHeader, config::NalUnits};
    let video = include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-topskip-cabac.mp4");
    let mut input = Mp4Reader::open(Cursor::new(video.as_slice()), Default::default()).unwrap();
    let configuration = input.tracks()[0].configuration.clone();
    let config = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let pps = Pps::parse(config.pps[0], &sps).unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 1, &mut packet).unwrap();
    let nal = NalUnits::new(&packet, config.length_size)
        .unwrap()
        .map(|n| n.unwrap())
        .find(|n| n[0] & 31 == 1)
        .unwrap();
    let original = SliceHeader::parse(nal, &sps, &pps).unwrap();
    let minimum = original.entropy_bit_offset.div_ceil(8) + 2;
    let mut failures = 0;
    for cut in (minimum..original.rbsp.len()).step_by(((original.rbsp.len() - minimum) / 16).max(1))
    {
        let mut header = SliceHeader::parse(nal, &sps, &pps).unwrap();
        header.rbsp.truncate(cut);
        let mut reader = InterCabacSlice::new_mbaff(&header, &sps, &pps, 65536).unwrap();
        for _ in 0..17 {
            match reader.read_macroblock() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {
                    failures += 1;
                    assert!(
                        reader
                            .read_macroblock()
                            .err()
                            .unwrap()
                            .to_string()
                            .contains("previously failed")
                    );
                    break;
                }
            }
        }
    }
    assert!(failures > 0);
}

#[test]
fn cabac_mbaff_inter_mixed_high10_and_multislice_match_jm_and_rewind() {
    use fvid::codec::{
        avc_cabac_macroblock::IntraCabacReader,
        avc_cabac_slice::InterCabacSlice,
        avc_inter_slice::InterMacroblock,
        avc_macroblock::IntraLuma,
        avc_slice::{SliceHeader, SliceType},
        config::NalUnits,
    };
    let mut high10_eight = 0;
    for (name,video,oracle) in [
        ("frame-inter-filtered-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-filtered-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-filtered-cabac.yuv").as_slice()),
        ("field-inter-filtered-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-filtered-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-filtered-cabac.yuv").as_slice()),
        ("mixed-inter-filtered-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-inter-filtered-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-inter-filtered-cabac.yuv").as_slice()),
        ("mixed-reverse-inter-filtered-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-inter-filtered-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-inter-filtered-cabac.yuv").as_slice()),
        ("mixed-vertical-inter-filtered-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-inter-filtered-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-inter-filtered-cabac.yuv").as_slice()),
        ("mixed-vertical-reverse-inter-filtered-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-inter-filtered-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-inter-filtered-cabac.yuv").as_slice()),
        ("field-multislice-inter-filtered-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-inter-filtered-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-inter-filtered-cabac.yuv").as_slice()),
        ("frame-inter-filtered-high10-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-frame-inter-filtered-high10-cabac.yuv").as_slice()),
        ("field-inter-filtered-high10-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-inter-filtered-high10-cabac.yuv").as_slice()),
        ("mixed-inter-filtered-high10-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-inter-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-inter-filtered-high10-cabac.yuv").as_slice()),
        ("mixed-reverse-inter-filtered-high10-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-inter-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-reverse-inter-filtered-high10-cabac.yuv").as_slice()),
        ("mixed-vertical-inter-filtered-high10-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-inter-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-inter-filtered-high10-cabac.yuv").as_slice()),
        ("mixed-vertical-reverse-inter-filtered-high10-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-inter-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-mixed-vertical-reverse-inter-filtered-high10-cabac.yuv").as_slice()),
        ("field-multislice-inter-filtered-high10-cabac", include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-inter-filtered-high10-cabac.mp4").as_slice(), include_bytes!("fixtures/playback-errors/avc-mbaff-field-multislice-inter-filtered-high10-cabac.yuv").as_slice()),
    ] {
        let mut input=Mp4Reader::open(Cursor::new(video),Default::default()).unwrap();
        let configuration=input.tracks()[0].configuration.clone();
        let config=AvcConfig::parse(&configuration).unwrap();
        let sps=Sps::parse(config.sps[0]).unwrap();
        let pps=Pps::parse(config.pps[0],&sps).unwrap();
        let depth=if name.contains("high10") {10} else {8};
        assert_eq!(sps.bit_depth_luma,depth);assert_eq!(sps.bit_depth_chroma,depth);
        assert!(pps.cabac && sps.mb_adaptive_frame_field);
        let mut coded_inter=0;
        for index in 0..3 {
            let mut packet=Vec::new();input.read_packet(0,index,&mut packet).unwrap();
            let headers:Vec<_>=NalUnits::new(&packet,config.length_size).unwrap().map(|n|n.unwrap())
                .filter(|n|matches!(n[0]&31,1|5)).map(|n|SliceHeader::parse(n,&sps,&pps).unwrap()).collect();
            assert_eq!(headers.len(),if name.contains("multislice") {2} else {1});
            let mut count=0;let mut fields=0;
            for header in &headers {
                assert_eq!(header.disable_deblocking_filter_idc,0);
                assert_eq!(header.first_mb as usize*2,count);
                let mut intra=if index==0 {Some(IntraCabacReader::new_mbaff(header,&sps,&pps,16).unwrap())} else {None};
                let mut inter=if index!=0 {Some(InterCabacSlice::new_mbaff(header,&sps,&pps,65536).unwrap())} else {None};
                loop {
                    let (address,field)=if let Some(reader)=&mut intra {
                        let Some(mb)=reader.read_macroblock().unwrap_or_else(|e|panic!("{name} frame {index} address {count}: {e}")) else {break};
                        if depth==10 {high10_eight+=usize::from(matches!(mb.luma,IntraLuma::Blocks8{..}));}
                        (mb.address as usize,reader.field_decoding())
                    } else {
                        assert_eq!(header.slice_type,SliceType::P);
                        let reader=inter.as_mut().unwrap();
                        let Some(block)=reader.read_macroblock().unwrap_or_else(|e|panic!("{name} frame {index} address {count}: {e}")) else {break};
                        let address=match block {
                            InterMacroblock::Skip{address,..}=>address,
                            InterMacroblock::Intra(mb)=>mb.address as usize,
                            InterMacroblock::Coded{address,header,..}=>{coded_inter+=1;if depth==10 {high10_eight+=usize::from(header.residual.transform8);}address}
                        };
                        (address,reader.field_decoding())
                    };
                    assert_eq!(address,count);
                    let expected=if name.starts_with("field") {true} else if name.starts_with("frame") {false}
                        else if name.contains("vertical-reverse") {address<8}
                        else if name.contains("vertical") {address>=8}
                        else if name.starts_with("mixed-reverse") {address/2%4<2}
                        else {address/2%4>=2};
                    assert_eq!(field,expected,"{name} frame {index} address {address}");
                    fields+=usize::from(field);count+=1;
                }
            }
            assert_eq!(count,16);
            assert_eq!(fields,if name.starts_with("field") {16} else if name.starts_with("frame") {0} else {8});
        }
        assert!(coded_inter>0);
        let mut playback=fvid::playback_mp4::Mp4VideoReader::open_software(Cursor::new(video),Default::default(),16<<20).unwrap();
        for _ in 0..2 {
            let mut actual=Vec::new();let mut frames=0;
            while let Some(frame)=playback.read_frame().unwrap() {frame.picture.write_planar(&mut actual).unwrap();frames+=1;}
            assert_eq!(frames,3);
            assert!(actual==oracle,"{name} mismatch at {:?}",actual.iter().zip(oracle).position(|(a,b)|a!=b));
            if depth==10 {assert!(actual.chunks_exact(2).any(|v|u16::from_le_bytes([v[0],v[1]])%4!=0));}
            playback.rewind();
        }
    }
    assert!(
        high10_eight > 0,
        "High10 inter corpus must exercise 8x8 transforms"
    );
}

#[test]
fn mbaff_b_spatial_temporal_match_every_jm_sample_and_rewind() {
    macro_rules! case {
        ($name:literal) => {
            (
                $name,
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-",
                    $name,
                    ".mp4"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-",
                    $name,
                    ".yuv"
                ))
                .as_slice(),
            )
        };
    }
    let mut high10_eight = 0;
    let mut expected_direct_cases = std::collections::HashSet::new();
    let mut actual_direct_cases = std::collections::HashSet::new();
    let mut expected_co_direct_cases = std::collections::HashSet::new();
    let mut actual_co_direct_cases = std::collections::HashSet::new();
    for (name, video, oracle) in [
        case!("frame-b-spatial-cavlc"),
        case!("field-b-spatial-cavlc"),
        case!("mixed-b-spatial-cavlc"),
        case!("mixed-reverse-b-spatial-cavlc"),
        case!("mixed-vertical-b-spatial-cavlc"),
        case!("mixed-vertical-reverse-b-spatial-cavlc"),
        case!("field-multislice-b-spatial-cavlc"),
        case!("frame-b-spatial-high10-cavlc"),
        case!("field-b-spatial-high10-cavlc"),
        case!("mixed-b-spatial-high10-cavlc"),
        case!("mixed-reverse-b-spatial-high10-cavlc"),
        case!("mixed-vertical-b-spatial-high10-cavlc"),
        case!("mixed-vertical-reverse-b-spatial-high10-cavlc"),
        case!("field-multislice-b-spatial-high10-cavlc"),
        case!("frame-b-temporal-cavlc"),
        case!("field-b-temporal-cavlc"),
        case!("mixed-b-temporal-cavlc"),
        case!("mixed-reverse-b-temporal-cavlc"),
        case!("mixed-vertical-b-temporal-cavlc"),
        case!("mixed-vertical-reverse-b-temporal-cavlc"),
        case!("field-multislice-b-temporal-cavlc"),
        case!("frame-b-temporal-high10-cavlc"),
        case!("field-b-temporal-high10-cavlc"),
        case!("mixed-b-temporal-high10-cavlc"),
        case!("mixed-reverse-b-temporal-high10-cavlc"),
        case!("mixed-vertical-b-temporal-high10-cavlc"),
        case!("mixed-vertical-reverse-b-temporal-high10-cavlc"),
        case!("field-multislice-b-temporal-high10-cavlc"),
        case!("frame-b-spatial-cabac"),
        case!("field-b-spatial-cabac"),
        case!("mixed-b-spatial-cabac"),
        case!("mixed-reverse-b-spatial-cabac"),
        case!("mixed-vertical-b-spatial-cabac"),
        case!("mixed-vertical-reverse-b-spatial-cabac"),
        case!("field-multislice-b-spatial-cabac"),
        case!("frame-b-spatial-high10-cabac"),
        case!("field-b-spatial-high10-cabac"),
        case!("mixed-b-spatial-high10-cabac"),
        case!("mixed-reverse-b-spatial-high10-cabac"),
        case!("mixed-vertical-b-spatial-high10-cabac"),
        case!("mixed-vertical-reverse-b-spatial-high10-cabac"),
        case!("field-multislice-b-spatial-high10-cabac"),
        case!("frame-b-temporal-cabac"),
        case!("field-b-temporal-cabac"),
        case!("mixed-b-temporal-cabac"),
        case!("mixed-reverse-b-temporal-cabac"),
        case!("mixed-vertical-b-temporal-cabac"),
        case!("mixed-vertical-reverse-b-temporal-cabac"),
        case!("field-multislice-b-temporal-cabac"),
        case!("frame-b-temporal-high10-cabac"),
        case!("field-b-temporal-high10-cabac"),
        case!("mixed-b-temporal-high10-cabac"),
        case!("mixed-reverse-b-temporal-high10-cabac"),
        case!("mixed-vertical-b-temporal-high10-cabac"),
        case!("mixed-vertical-reverse-b-temporal-high10-cabac"),
        case!("field-multislice-b-temporal-high10-cabac"),
        case!("field-multislice-b-temporal-skipped-cavlc"),
        case!("field-multislice-b-temporal-skipped-high10-cavlc"),
        case!("field-multislice-b-temporal-skipped-cabac"),
        case!("field-multislice-b-temporal-skipped-high10-cabac"),
        case!("frame-b-pyramid-spatial-cavlc"),
        case!("frame-b-pyramid-spatial-high10-cavlc"),
        case!("frame-b-pyramid-temporal-cavlc"),
        case!("frame-b-pyramid-temporal-high10-cavlc"),
        case!("frame-b-pyramid-spatial-cabac"),
        case!("frame-b-pyramid-spatial-high10-cabac"),
        case!("frame-b-pyramid-temporal-cabac"),
        case!("frame-b-pyramid-temporal-high10-cabac"),
        case!("field-b-pyramid-spatial-cavlc"),
        case!("field-b-pyramid-spatial-high10-cavlc"),
        case!("field-b-pyramid-temporal-cavlc"),
        case!("field-b-pyramid-temporal-high10-cavlc"),
        case!("field-b-pyramid-spatial-cabac"),
        case!("field-b-pyramid-spatial-high10-cabac"),
        case!("field-b-pyramid-temporal-cabac"),
        case!("field-b-pyramid-temporal-high10-cabac"),
        case!("mixed-b-pyramid-spatial-cavlc"),
        case!("mixed-b-pyramid-spatial-high10-cavlc"),
        case!("mixed-b-pyramid-temporal-cavlc"),
        case!("mixed-b-pyramid-temporal-high10-cavlc"),
        case!("mixed-b-pyramid-spatial-cabac"),
        case!("mixed-b-pyramid-spatial-high10-cabac"),
        case!("mixed-b-pyramid-temporal-cabac"),
        case!("mixed-b-pyramid-temporal-high10-cabac"),
        case!("field-b-pyramid-temporal-skipped-high10-cabac"),
        case!("mixed-b-pyramid-temporal-skipped-cavlc"),
        case!("mixed-b-pyramid-temporal-skipped-high10-cavlc"),
        case!("changing-b-pyramid-spatial-cavlc"),
        case!("changing-b-pyramid-spatial-high10-cavlc"),
        case!("changing-b-pyramid-temporal-cavlc"),
        case!("changing-b-pyramid-temporal-high10-cavlc"),
        case!("changing-b-pyramid-spatial-cabac"),
        case!("changing-b-pyramid-spatial-high10-cabac"),
        case!("changing-b-pyramid-temporal-cabac"),
        case!("changing-b-pyramid-temporal-high10-cabac"),
    ] {
        use fvid::codec::{
            avc_inter_slice::{InterCavlcSlice, InterMacroblock},
            avc_slice::{SliceHeader, SliceType},
            config::NalUnits,
        };
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let timescale = input.tracks()[0].timescale;
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(
            pps.weighted_bipred, 2,
            "{name} must exercise implicit B weighting"
        );
        assert!(sps.mb_adaptive_frame_field && !sps.frame_mbs_only);
        assert_eq!(
            sps.bit_depth_luma,
            if name.contains("high10") { 10 } else { 8 }
        );
        let frame_count = if name.contains("pyramid") { 9 } else { 3 };
        let expected_b = if name.contains("pyramid") { 6 } else { 1 };
        let mut reference_b = 0;
        let mut nonzero_frame_indices = 0;
        let mut b_nonzero_frame_indices = 0;
        let mut active_multiple = false;
        let mut picture_types = [0usize; 3];
        let mut b_count = 0;
        let mut b_slices = 0;
        let mut direct_blocks = 0;
        let mut direct_by_mode = [0usize; 2];
        let mut colocated_b = 0;
        let mut co_direct_by_mode = [0usize; 2];
        let mut reference_types = std::collections::HashMap::new();
        let mut dpb =
            fvid::codec::avc_dpb::ReferenceBuffer::new(sps.frame_num_bits, sps.max_num_ref_frames)
                .unwrap();
        let mut poc = fvid::codec::avc_poc::PocDecoder::default();
        for packet_index in 0..frame_count {
            let mut packet = Vec::new();
            input.read_packet(0, packet_index, &mut packet).unwrap();
            let mut expected_address = 0;
            let mut headers_seen = 0;
            let mut first_header = None;
            let mut packet_order = None;
            for nal in NalUnits::new(&packet, config.length_size)
                .unwrap()
                .map(|n| n.unwrap())
                .filter(|n| matches!(n[0] & 31, 1 | 5))
            {
                let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
                let is_b = header.slice_type == SliceType::B;
                if header.first_mb == 0 {
                    picture_types[match header.slice_type {
                        SliceType::I => 0,
                        SliceType::P => 1,
                        SliceType::B => 2,
                        _ => panic!("unexpected fixture picture type"),
                    }] += 1;
                }
                let spatial_mode = header.direct_spatial_mv_pred;
                let before_direct = direct_blocks;
                let order = *packet_order.get_or_insert_with(|| poc.decode(&sps, &header).unwrap());
                let mut uses_colocated_b = false;
                if is_b {
                    let lists = dpb.lists(&header, order.before_marking.picture()).unwrap();
                    if lists
                        .l1
                        .first()
                        .is_some_and(|id| reference_types.get(id) == Some(&SliceType::B))
                    {
                        colocated_b += 1;
                        uses_colocated_b = true;
                    }
                }
                assert_eq!(header.first_mb as usize * 2, expected_address);
                assert_eq!(header.disable_deblocking_filter_idc, 0);
                headers_seen += 1;
                if is_b {
                    if header.first_mb == 0 {
                        b_count += 1;
                    }
                    b_slices += 1;
                    if header.first_mb == 0 && header.nal_ref_idc != 0 {
                        reference_b += 1;
                    }
                    active_multiple |= header.refs_l0 > 1 || header.refs_l1 > 1;
                    if !name.contains("pyramid") || name.contains("spatial") {
                        assert_eq!(
                            spatial_mode,
                            name.contains("spatial"),
                            "{name} packet {packet_index}"
                        );
                    }
                }
                let mut decoded = Vec::new();
                if header.slice_type == SliceType::I {
                    if pps.cabac {
                        let mut syntax =
                            fvid::codec::avc_cabac_macroblock::IntraCabacReader::new_mbaff(
                                &header, &sps, &pps, 16,
                            )
                            .unwrap();
                        while let Some(block) = syntax.read_macroblock().unwrap() {
                            decoded.push((
                                InterMacroblock::Intra(Box::new(block)),
                                syntax.field_decoding(),
                            ));
                        }
                    } else {
                        let mut syntax = fvid::codec::avc_macroblock::IntraCavlcReader::new_mbaff(
                            &header, &sps, &pps, 16,
                        )
                        .unwrap();
                        while let Some(block) = syntax.read_macroblock().unwrap() {
                            decoded.push((
                                InterMacroblock::Intra(Box::new(block)),
                                syntax.field_decoding(),
                            ));
                        }
                    }
                } else if pps.cabac {
                    let mut syntax = fvid::codec::avc_cabac_slice::InterCabacSlice::new_mbaff(
                        &header, &sps, &pps, 65536,
                    )
                    .unwrap();
                    while let Some(block) = syntax.read_macroblock().unwrap() {
                        decoded.push((block, syntax.field_decoding()));
                    }
                } else {
                    let mut syntax =
                        InterCavlcSlice::new_mbaff(&header, &sps, &pps, 65536).unwrap();
                    while let Some(block) = syntax.read_macroblock().unwrap() {
                        decoded.push((block, syntax.field_decoding()));
                    }
                }
                let mut count = header.first_mb as usize * 2;
                let start = count;
                let mut fields = 0;
                for (block, field) in decoded {
                    let address = match block {
                        InterMacroblock::Skip { address, .. } => {
                            direct_blocks += usize::from(is_b);
                            address
                        }
                        InterMacroblock::Coded {
                            address, header, ..
                        } => {
                            let used_nonzero = header
                                .partitions
                                .iter()
                                .flat_map(|p| p.references)
                                .flatten()
                                .filter(|&r| usize::from(r) / if field { 2 } else { 1 } > 0)
                                .count();
                            nonzero_frame_indices += used_nonzero;
                            if is_b {
                                b_nonzero_frame_indices += used_nonzero;
                            }
                            high10_eight += usize::from(
                                is_b && name.contains("high10") && header.residual.transform8,
                            );
                            direct_blocks += usize::from(
                                is_b && header.partitions.iter().any(|p| {
                                    p.prediction == fvid::codec::avc_inter::Prediction::Direct
                                }),
                            );
                            address
                        }
                        InterMacroblock::Intra(mb) => mb.address as usize,
                    };
                    assert_eq!(address, count);
                    count += 1;
                    let expected = if name.starts_with("changing") {
                        [false, true, true, false, false, false, false, true, true][packet_index]
                    } else if name.starts_with("field") {
                        true
                    } else if name.starts_with("frame") {
                        false
                    } else if name.contains("vertical-reverse") {
                        address < 8
                    } else if name.contains("vertical") {
                        address >= 8
                    } else if name.starts_with("mixed-reverse") {
                        address / 2 % 4 < 2
                    } else {
                        address / 2 % 4 >= 2
                    };
                    assert_eq!(field, expected, "{name} B address {address}");
                    fields += usize::from(field);
                }
                expected_address = count;
                assert_eq!(
                    count - start,
                    if name.contains("multislice") { 8 } else { 16 }
                );

                assert_eq!(
                    fields,
                    if name.starts_with("changing") {
                        if [false, true, true, false, false, false, false, true, true][packet_index]
                        {
                            count - start
                        } else {
                            0
                        }
                    } else if name.starts_with("field") {
                        count - start
                    } else if name.starts_with("frame") {
                        0
                    } else {
                        8
                    }
                );
                if is_b {
                    direct_by_mode[usize::from(!spatial_mode)] += direct_blocks - before_direct;
                    if uses_colocated_b {
                        co_direct_by_mode[usize::from(!spatial_mode)] +=
                            direct_blocks - before_direct;
                    }
                }
                if first_header.is_none() {
                    first_header = Some(header);
                }
            }
            assert_eq!(expected_address, 16);
            assert_eq!(
                headers_seen,
                if name.contains("multislice") { 2 } else { 1 }
            );
            let first = first_header.unwrap();
            let order = packet_order.unwrap();
            dpb.finish(
                &first,
                order.after_marking.picture(),
                packet_index as u64,
                std::sync::Arc::new(()),
            )
            .unwrap();
            if first.nal_ref_idc != 0 {
                reference_types.insert(packet_index as u64, first.slice_type);
            }
        }
        let direct_key = name.replace("-skipped", "");
        let qualified_direct = !name.starts_with("changing") || name.contains("spatial");
        if qualified_direct {
            expected_direct_cases.insert(direct_key.clone());
        }
        if qualified_direct && direct_by_mode[usize::from(name.contains("temporal"))] > 0 {
            actual_direct_cases.insert(direct_key.clone());
        }
        if name.contains("pyramid") && qualified_direct {
            expected_co_direct_cases.insert(direct_key.clone());
            if co_direct_by_mode[usize::from(name.contains("temporal"))] > 0 {
                actual_co_direct_cases.insert(direct_key);
            }
        }
        assert_eq!(
            b_count, expected_b,
            "{name} must contain the expected B pictures"
        );
        assert_eq!(
            picture_types,
            if name.contains("pyramid") {
                [1, 2, 6]
            } else {
                [1, 1, 1]
            }
        );
        if name.contains("pyramid") {
            assert_eq!(reference_b, 2, "{name}");
            assert!(
                colocated_b > 0,
                "{name} must actually use a retained B co-located picture"
            );
            assert!(
                active_multiple,
                "{name} must declare multiple active references"
            );
            assert!(
                name.contains("skipped")
                    || (nonzero_frame_indices > 0 && b_nonzero_frame_indices > 0),
                "{name} must actually use a nonzero frame reference"
            );
        }
        assert_eq!(
            b_slices,
            expected_b * if name.contains("multislice") { 2 } else { 1 }
        );
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..2 {
            let mut actual = Vec::new();
            let mut count = 0;
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name} pass {pass} frame {count}: {e}"))
            {
                assert_eq!(
                    frame.presentation_time.ticks, count as i64,
                    "{name} output timestamp"
                );
                assert_eq!(frame.presentation_time.timescale, timescale);
                assert_eq!(frame.duration.ticks, 1);
                assert_eq!(frame.duration.timescale, timescale);
                frame.picture.write_planar(&mut actual).unwrap();
                count += 1;
            }
            assert_eq!(count, frame_count, "{name}");
            if actual != oracle {
                let first = actual.iter().zip(oracle).position(|(a, b)| a != b).unwrap();
                panic!(
                    "{name} pass {pass} mismatch byte {first}: native {} JM {}",
                    actual[first], oracle[first]
                );
            }
            if name.contains("high10") {
                assert!(
                    actual
                        .chunks_exact(2)
                        .any(|v| u16::from_le_bytes([v[0], v[1]]) % 4 != 0)
                );
            }
            reader.rewind();
        }
    }
    assert!(
        high10_eight > 0,
        "High10 B corpus must exercise 8x8 transforms"
    );
    let missing: Vec<_> = expected_direct_cases
        .difference(&actual_direct_cases)
        .collect();
    assert!(
        missing.is_empty(),
        "cases missing actual direct: {missing:?}"
    );
    let missing: Vec<_> = expected_co_direct_cases
        .difference(&actual_co_direct_cases)
        .collect();
    assert!(
        missing.is_empty(),
        "pyramid cases missing actual direct from B motion: {missing:?}"
    );
}

#[test]
fn owned_cross_mode_temporal_direct_from_b_motion_matches_jm_and_rewind() {
    use fvid::codec::{
        avc_dpb::ReferenceBuffer,
        avc_inter::Prediction,
        avc_inter_slice::{InterCavlcSlice, InterMacroblock},
        avc_poc::PocDecoder,
        avc_slice::{SliceHeader, SliceType},
        config::NalUnits,
    };
    macro_rules! case {
        ($name:expr) => {
            (
                $name,
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-colocated-",
                    $name,
                    "-cavlc.mp4"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-colocated-",
                    $name,
                    "-cavlc.yuv"
                ))
                .as_slice(),
            )
        };
    }
    macro_rules! cases_for_depth {
        ($depth:literal) => {
            [
                case!(concat!("field-to-frame", $depth)),
                case!(concat!("frame-to-field", $depth)),
                case!(concat!("field-to-frame-explicit", $depth)),
                case!(concat!("frame-to-field-explicit", $depth)),
            ]
        };
    }
    for (name, video, oracle) in [
        cases_for_depth!(""),
        cases_for_depth!("-high10"),
        cases_for_depth!("-high12"),
        cases_for_depth!("-high14"),
    ]
    .into_iter()
    .flatten()
    {
        let depth = if name.contains("high14") {
            14
        } else if name.contains("high12") {
            12
        } else if name.contains("high10") {
            10
        } else {
            8
        };
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert!(sps.mb_adaptive_frame_field && !pps.cabac);
        assert_eq!(
            pps.weighted_bipred,
            if name.contains("explicit") { 1 } else { 2 }
        );
        assert_eq!(sps.coded_dimensions(), (16, 32));
        assert_eq!(sps.bit_depth_luma, depth);
        assert_eq!(sps.bit_depth_chroma, depth);
        assert_eq!(sps.chroma_format, 1);
        assert_eq!(
            sps.profile,
            if depth == 8 {
                77
            } else if depth == 10 {
                110
            } else {
                244
            }
        );
        if depth > 8 {
            let samples: Vec<_> = oracle
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect();
            assert!(samples.iter().all(|v| *v < (1 << depth)));
            assert!(
                samples.iter().any(|v| *v & ((1 << (depth - 8)) - 1) != 0),
                "oracle must retain precision below 8-bit for {name}"
            );
        }
        let source_field = name.starts_with("field");
        let mut poc = PocDecoder::default();
        let mut dpb = ReferenceBuffer::new(sps.frame_num_bits, sps.max_num_ref_frames).unwrap();
        for index in 0..5 {
            let mut packet = Vec::new();
            input.read_packet(0, index, &mut packet).unwrap();
            let nal = NalUnits::new(&packet, config.length_size)
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
            let order = poc.decode(&sps, &header).unwrap();
            assert_eq!(order.before_marking.picture(), [0, 8, 4, 2, 6][index]);
            if index == 2 || index == 3 {
                assert_eq!(header.slice_type, SliceType::B);
                if name.contains("explicit") {
                    let weights = header.weights.as_ref().unwrap();
                    assert_eq!((weights.luma_denom, weights.chroma_denom), (1, 1));
                    assert_eq!(weights.l0[0].luma, (3, 1));
                    assert_eq!(weights.l1[0].luma, (1, -3));
                    assert_eq!(weights.l0[0].chroma, [(3, 2), (3, -1)]);
                    assert_eq!(weights.l1[0].chroma, [(1, -2), (1, 3)]);
                } else {
                    assert!(header.weights.is_none());
                }
                assert!(!header.direct_spatial_mv_pred);
                assert_eq!(header.nal_ref_idc != 0, index == 2);
                let lists = dpb.lists(&header, order.before_marking.picture()).unwrap();
                if index == 3 {
                    assert_eq!(lists.l1[0], 2, "co-located picture must be retained B");
                    assert_eq!(lists.l0[0], 0);
                }
                let mut reader = InterCavlcSlice::new_mbaff(&header, &sps, &pps, 65536).unwrap();
                for address in 0..2 {
                    let InterMacroblock::Coded {
                        address: actual,
                        header: mb,
                        ..
                    } = reader.read_macroblock().unwrap().unwrap()
                    else {
                        panic!("expected owned coded block")
                    };
                    assert_eq!(actual, address);
                    assert_eq!(
                        reader.field_decoding(),
                        if index == 2 {
                            source_field
                        } else {
                            !source_field
                        }
                    );
                    if index == 2 {
                        assert_eq!(mb.partitions.len(), 1);
                        assert_eq!(mb.partitions[0].prediction, Prediction::L0);
                        assert_eq!(mb.partitions[0].references[0], Some(u8::from(source_field)));
                        assert_eq!(
                            mb.partitions[0].differences[0],
                            if address == 0 || source_field {
                                [8, 4]
                            } else {
                                [0, 0]
                            }
                        );
                    } else {
                        assert_eq!(mb.partitions.len(), 16);
                        assert!(
                            mb.partitions
                                .iter()
                                .all(|p| p.prediction == Prediction::Direct)
                        );
                    }
                }
                assert!(reader.read_macroblock().unwrap().is_none());
            }
            dpb.finish(
                &header,
                order.after_marking.picture(),
                index as u64,
                std::sync::Arc::new(()),
            )
            .unwrap();
        }
        if name.contains("explicit") {
            if depth > 8 {
                assert!(
                    oracle
                        .chunks_exact(2)
                        .any(|v| u16::from_le_bytes([v[0], v[1]]) == (1 << depth) - 1)
                );
            } else {
                assert!(oracle.contains(&255));
            }
        }
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..2 {
            let mut actual = Vec::new();
            let mut count = 0;
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name} pass {pass} frame {count}: {e}"))
            {
                assert_eq!(frame.picture.bit_depth, depth);
                assert_eq!(frame.presentation_time.ticks, count);
                assert_eq!(frame.presentation_time.timescale, 25);
                frame.picture.write_planar(&mut actual).unwrap();
                count += 1;
            }
            assert_eq!(count, 5);
            assert_eq!(actual.len(), oracle.len());
            assert!(
                actual == oracle,
                "{name} pass {pass} first byte mismatch {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            reader.rewind();
        }
    }
}

#[test]
fn owned_cabac_cross_mode_temporal_direct_matches_jm_and_rewind() {
    use fvid::codec::{
        avc_cabac_slice::InterCabacSlice,
        avc_inter::Prediction,
        avc_inter_slice::InterMacroblock,
        avc_slice::{SliceHeader, SliceType},
        config::NalUnits,
    };
    macro_rules! case {
        ($name:expr) => {
            (
                $name,
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-colocated-",
                    $name,
                    "-cabac.mp4"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-colocated-",
                    $name,
                    "-cabac.yuv"
                ))
                .as_slice(),
            )
        };
    }
    macro_rules! cases_for_depth {
        ($depth:literal) => {
            [
                case!(concat!("field-to-frame", $depth)),
                case!(concat!("frame-to-field", $depth)),
                case!(concat!("field-to-frame-explicit", $depth)),
                case!(concat!("frame-to-field-explicit", $depth)),
                case!(concat!("field-to-frame-init1", $depth)),
                case!(concat!("frame-to-field-init1", $depth)),
                case!(concat!("field-to-frame-init1-explicit", $depth)),
                case!(concat!("frame-to-field-init1-explicit", $depth)),
                case!(concat!("field-to-frame-init2", $depth)),
                case!(concat!("frame-to-field-init2", $depth)),
                case!(concat!("field-to-frame-init2-explicit", $depth)),
                case!(concat!("frame-to-field-init2-explicit", $depth)),
                case!(concat!("field-to-frame-top-skip", $depth)),
                case!(concat!("frame-to-field-top-skip", $depth)),
                case!(concat!("field-to-frame-top-skip-explicit", $depth)),
                case!(concat!("frame-to-field-top-skip-explicit", $depth)),
                case!(concat!("field-to-frame-source-cabac", $depth)),
                case!(concat!("frame-to-field-source-cabac", $depth)),
                case!(concat!("field-to-frame-source-cabac-explicit", $depth)),
                case!(concat!("frame-to-field-source-cabac-explicit", $depth)),
                case!(concat!("field-to-frame-source-cabac-init1", $depth)),
                case!(concat!("frame-to-field-source-cabac-init1", $depth)),
                case!(concat!(
                    "field-to-frame-source-cabac-init1-explicit",
                    $depth
                )),
                case!(concat!(
                    "frame-to-field-source-cabac-init1-explicit",
                    $depth
                )),
                case!(concat!("field-to-frame-source-cabac-init2", $depth)),
                case!(concat!("frame-to-field-source-cabac-init2", $depth)),
                case!(concat!(
                    "field-to-frame-source-cabac-init2-explicit",
                    $depth
                )),
                case!(concat!(
                    "frame-to-field-source-cabac-init2-explicit",
                    $depth
                )),
                case!(concat!("field-to-frame-source-cabac-top-skip", $depth)),
                case!(concat!("frame-to-field-source-cabac-top-skip", $depth)),
                case!(concat!(
                    "field-to-frame-source-cabac-top-skip-explicit",
                    $depth
                )),
                case!(concat!(
                    "frame-to-field-source-cabac-top-skip-explicit",
                    $depth
                )),
            ]
        };
    }
    for (name, video, oracle) in [
        cases_for_depth!(""),
        cases_for_depth!("-high10"),
        cases_for_depth!("-high12"),
        cases_for_depth!("-high14"),
    ]
    .into_iter()
    .flatten()
    {
        let depth = if name.contains("high14") {
            14
        } else if name.contains("high12") {
            12
        } else if name.contains("high10") {
            10
        } else {
            8
        };
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        assert!(sps.mb_adaptive_frame_field);
        assert_eq!(sps.coded_dimensions(), (16, 32));
        assert_eq!(sps.bit_depth_luma, depth);
        assert_eq!(sps.bit_depth_chroma, depth);
        assert_eq!(sps.chroma_format, 1);
        assert_eq!(
            sps.profile,
            if depth == 8 {
                77
            } else if depth == 10 {
                110
            } else {
                244
            }
        );
        if depth > 8 {
            let samples: Vec<_> = oracle
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect();
            assert!(samples.iter().all(|v| *v < (1 << depth)));
            assert!(
                samples.iter().any(|v| *v & ((1 << (depth - 8)) - 1) != 0),
                "oracle must retain precision below 8-bit for {name}"
            );
        }
        assert_eq!(config.pps.len(), 2);
        let anchor_pps = Pps::parse(config.pps[0], &sps).unwrap();
        let pps = Pps::parse(config.pps[1], &sps).unwrap();
        assert!(!anchor_pps.cabac && pps.cabac);
        assert_eq!(pps.id, 1);
        assert_eq!(
            pps.weighted_bipred,
            if name.contains("explicit") { 1 } else { 2 }
        );
        // The CABAC target uses the same owned retained B motion and timeline
        // as the separately qualified CAVLC corpus; entropy is switched by PPS.
        let mut packet = Vec::new();
        input.read_packet(0, 3, &mut packet).unwrap();
        let nal = NalUnits::new(&packet, config.length_size)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
        assert_eq!(header.slice_type, SliceType::B);
        assert!(!header.direct_spatial_mv_pred);
        assert_eq!(header.nal_ref_idc, 0);
        if name.contains("explicit") {
            let weights = header.weights.as_ref().unwrap();
            assert_eq!((weights.luma_denom, weights.chroma_denom), (1, 1));
            assert_eq!(weights.l0[0].luma, (3, 1));
            assert_eq!(weights.l1[0].luma, (1, -3));
            assert_eq!(weights.l0[0].chroma, [(3, 2), (3, -1)]);
            assert_eq!(weights.l1[0].chroma, [(1, -2), (1, 3)]);
        } else {
            assert!(header.weights.is_none());
        }
        assert_eq!(
            header.cabac_init_idc,
            if name.contains("init1") {
                1
            } else if name.contains("init2") {
                2
            } else {
                0
            }
        );
        let mut poc = fvid::codec::avc_poc::PocDecoder::default();
        let mut dpb =
            fvid::codec::avc_dpb::ReferenceBuffer::new(sps.frame_num_bits, sps.max_num_ref_frames)
                .unwrap();
        for index in 0..4 {
            let mut bytes = Vec::new();
            input.read_packet(0, index, &mut bytes).unwrap();
            let nal = NalUnits::new(&bytes, config.length_size)
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            let h = SliceHeader::parse(
                nal,
                &sps,
                if index == 3 || (index == 2 && name.contains("source-cabac")) {
                    &pps
                } else {
                    &anchor_pps
                },
            )
            .unwrap();
            let order = poc.decode(&sps, &h).unwrap();
            assert_eq!(order.before_marking.picture(), [0, 8, 4, 2][index]);
            if index == 2 {
                assert_eq!(h.slice_type, SliceType::B);
                assert_ne!(h.nal_ref_idc, 0);
                let source_field = name.starts_with("field");
                let mut source_cavlc = if name.contains("source-cabac") {
                    None
                } else {
                    Some(
                        fvid::codec::avc_inter_slice::InterCavlcSlice::new_mbaff(
                            &h,
                            &sps,
                            &anchor_pps,
                            65536,
                        )
                        .unwrap(),
                    )
                };
                let mut source_cabac = if name.contains("source-cabac") {
                    assert_eq!(h.cabac_init_idc, header.cabac_init_idc);
                    Some(InterCabacSlice::new_mbaff(&h, &sps, &pps, 65536).unwrap())
                } else {
                    None
                };
                for address in 0..2 {
                    let InterMacroblock::Coded {
                        address: actual,
                        header: mb,
                        ..
                    } = (if let Some(reader) = &mut source_cabac {
                        reader.read_macroblock().unwrap().unwrap()
                    } else {
                        source_cavlc
                            .as_mut()
                            .unwrap()
                            .read_macroblock()
                            .unwrap()
                            .unwrap()
                    })
                    else {
                        panic!("expected owned B motion source");
                    };
                    assert_eq!(actual, address);
                    assert_eq!(
                        if let Some(reader) = &source_cabac {
                            reader.field_decoding()
                        } else {
                            source_cavlc.as_ref().unwrap().field_decoding()
                        },
                        source_field
                    );
                    assert_eq!(mb.partitions.len(), 1);
                    assert_eq!(mb.partitions[0].prediction, Prediction::L0);
                    assert_eq!(mb.partitions[0].references[0], Some(u8::from(source_field)));
                    assert_eq!(
                        mb.partitions[0].differences[0],
                        if address == 0 || source_field {
                            [8, 4]
                        } else {
                            [0, 0]
                        }
                    );
                }
                assert!(if let Some(reader) = &mut source_cabac {
                    reader.read_macroblock().unwrap().is_none()
                } else {
                    source_cavlc
                        .as_mut()
                        .unwrap()
                        .read_macroblock()
                        .unwrap()
                        .is_none()
                });
            }
            if index == 3 {
                let lists = dpb.lists(&h, order.before_marking.picture()).unwrap();
                assert_eq!(lists.l0[0], 0);
                assert_eq!(
                    lists.l1[0], 2,
                    "CABAC co-located reference must be retained B"
                );
            }
            dpb.finish(
                &h,
                order.after_marking.picture(),
                index as u64,
                std::sync::Arc::new(()),
            )
            .unwrap();
        }
        let mut syntax = InterCabacSlice::new_mbaff(&header, &sps, &pps, 65536).unwrap();
        for address in 0..2 {
            let block = syntax
                .read_macroblock()
                .unwrap_or_else(|e| panic!("{name} syntax block {address}: {e}"))
                .unwrap();
            if address == 0 && name.contains("top-skip") {
                assert!(matches!(block, InterMacroblock::Skip { address: 0, .. }));
                assert_eq!(syntax.field_decoding(), name.starts_with("frame"));
                continue;
            }
            let InterMacroblock::Coded {
                address: actual,
                header: mb,
                ..
            } = block
            else {
                panic!("expected owned CABAC direct block: {name}");
            };
            assert_eq!(actual, address);
            assert_eq!(syntax.field_decoding(), name.starts_with("frame"));
            assert_eq!(mb.partitions.len(), 16);
            assert!(
                mb.partitions
                    .iter()
                    .all(|p| p.prediction == Prediction::Direct)
            );
        }
        assert!(syntax.read_macroblock().unwrap().is_none());
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..2 {
            let mut pixels = Vec::new();
            let mut count = 0;
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name} pass {pass} frame {count}: {e}"))
            {
                assert_eq!(frame.picture.bit_depth, depth);
                assert_eq!(frame.presentation_time.ticks, count);
                assert_eq!(frame.presentation_time.timescale, 25);
                frame.picture.write_planar(&mut pixels).unwrap();
                count += 1;
            }
            assert_eq!(count, 5);
            assert_eq!(pixels.len(), oracle.len());
            assert!(
                pixels == oracle,
                "{name} pass {pass} first byte mismatch {:?}",
                pixels.iter().zip(oracle).position(|(a, b)| a != b)
            );
            reader.rewind();
        }
    }
}

#[test]
fn mixed_mbaff_intra_and_p_slices_match_jm_and_rewind() {
    use fvid::codec::{avc_access_unit::prepare, avc_slice::SliceType};
    macro_rules! case {
        ($name:expr) => {
            (
                $name,
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-mixed-",
                    $name,
                    ".mp4"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-mixed-",
                    $name,
                    ".yuv"
                ))
                .as_slice(),
            )
        };
    }
    macro_rules! cases_for_depth {
        ($depth:literal, $entropy:literal) => {
            [
                case!(concat!("ip-frame-field-filter0", $depth, $entropy)),
                case!(concat!("ip-frame-field-filter1", $depth, $entropy)),
                case!(concat!("ip-frame-field-filter2", $depth, $entropy)),
                case!(concat!("ip-field-frame-filter0", $depth, $entropy)),
                case!(concat!("ip-field-frame-filter1", $depth, $entropy)),
                case!(concat!("ip-field-frame-filter2", $depth, $entropy)),
                case!(concat!("pi-frame-field-filter0", $depth, $entropy)),
                case!(concat!("pi-frame-field-filter1", $depth, $entropy)),
                case!(concat!("pi-frame-field-filter2", $depth, $entropy)),
                case!(concat!("pi-field-frame-filter0", $depth, $entropy)),
                case!(concat!("pi-field-frame-filter1", $depth, $entropy)),
                case!(concat!("pi-field-frame-filter2", $depth, $entropy)),
            ]
        };
    }
    let cases: Vec<_> = [
        cases_for_depth!("", ""),
        cases_for_depth!("-high10", ""),
        cases_for_depth!("", "-cabac"),
        cases_for_depth!("-high10", "-cabac"),
    ]
    .into_iter()
    .flatten()
    .collect();
    for (name, video, oracle) in cases.iter().copied() {
        if name.contains("filter0") {
            let other_name = name.replace("filter0", "filter2");
            let (_, _, other) = cases.iter().find(|(n, _, _)| *n == other_name).unwrap();
            let frame_bytes = 32 * 32 * 3 / 2 * if name.contains("high10") { 2 } else { 1 };
            assert_eq!(
                &oracle[..frame_bytes],
                &other[..frame_bytes],
                "PCM anchor must isolate the mixed-picture filter difference"
            );
            assert_ne!(
                &oracle[frame_bytes..frame_bytes * 2],
                &other[frame_bytes..frame_bytes * 2],
                "{name} must actually exercise cross-slice filtering"
            );
        }
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let bytes = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&bytes).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let cabac = name.ends_with("-cabac");
        let pps = Pps::parse(config.pps[usize::from(cabac)], &sps).unwrap();
        assert!(sps.mb_adaptive_frame_field);
        assert_eq!(pps.cabac, cabac);
        assert_eq!(sps.coded_dimensions(), (32, 32));
        let mut packet = Vec::new();
        input.read_packet(0, 1, &mut packet).unwrap();
        let slices = prepare(&packet, config.length_size, &sps, &pps, packet.len()).unwrap();
        assert_eq!(slices.len(), 2);
        if !cabac {
            assert_ne!(
                slices[0].header.nal_ref_idc, slices[1].header.nal_ref_idc,
                "different nonzero priorities must belong to one picture"
            );
        }
        for (pair, slice) in slices.iter().enumerate() {
            assert_eq!(slice.header.first_mb, pair as u32);
            assert_eq!(
                slice.header.slice_type,
                if (pair == 0) == name.starts_with("ip") {
                    SliceType::I
                } else {
                    SliceType::P
                }
            );
            assert!(!slice.header.idr);
            assert_eq!(
                slice.header.slice_qp,
                if slice.header.slice_type == SliceType::P {
                    50
                } else {
                    26
                }
            );
            assert_ne!(slice.header.nal_ref_idc, 0);
            let field = (pair == 0) == name.contains("field-frame");
            if slice.header.slice_type == SliceType::I {
                let mut cv = if cabac {
                    None
                } else {
                    Some(
                        fvid::codec::avc_macroblock::IntraCavlcReader::new_mbaff(
                            &slice.header,
                            &sps,
                            &pps,
                            4,
                        )
                        .unwrap(),
                    )
                };
                let mut cb = if cabac {
                    Some(
                        fvid::codec::avc_cabac_macroblock::IntraCabacReader::new_mbaff(
                            &slice.header,
                            &sps,
                            &pps,
                            4,
                        )
                        .unwrap(),
                    )
                } else {
                    None
                };
                for address in 0..2 {
                    let mb = if let Some(r) = &mut cb {
                        r.read_macroblock().unwrap().unwrap()
                    } else {
                        cv.as_mut().unwrap().read_macroblock().unwrap().unwrap()
                    };
                    assert_eq!(mb.address as usize, pair * 2 + address);
                    assert!(matches!(
                        mb.luma,
                        fvid::codec::avc_macroblock::IntraLuma::Pcm { .. }
                    ));
                    assert_eq!(
                        if let Some(r) = &cb {
                            r.field_decoding()
                        } else {
                            cv.as_ref().unwrap().field_decoding()
                        },
                        field
                    );
                }
                assert!(if let Some(r) = &mut cb {
                    r.read_macroblock().unwrap().is_none()
                } else {
                    cv.as_mut().unwrap().read_macroblock().unwrap().is_none()
                });
            } else {
                let mut cv = if cabac {
                    None
                } else {
                    Some(
                        fvid::codec::avc_inter_slice::InterCavlcSlice::new_mbaff(
                            &slice.header,
                            &sps,
                            &pps,
                            65536,
                        )
                        .unwrap(),
                    )
                };
                let mut cb = if cabac {
                    Some(
                        fvid::codec::avc_cabac_slice::InterCabacSlice::new_mbaff(
                            &slice.header,
                            &sps,
                            &pps,
                            65536,
                        )
                        .unwrap(),
                    )
                } else {
                    None
                };
                for address in 0..2 {
                    let block = if let Some(r) = &mut cb {
                        r.read_macroblock().unwrap().unwrap()
                    } else {
                        cv.as_mut().unwrap().read_macroblock().unwrap().unwrap()
                    };
                    let fvid::codec::avc_inter_slice::InterMacroblock::Coded {
                        address: actual,
                        header: mb,
                        ..
                    } = block
                    else {
                        panic!("expected mixed P motion");
                    };
                    assert_eq!(actual, pair * 2 + address);
                    assert_eq!(mb.partitions.len(), 1);
                    assert_eq!(
                        mb.partitions[0].prediction,
                        fvid::codec::avc_inter::Prediction::L0
                    );
                    assert_eq!(mb.partitions[0].references[0], Some(u8::from(field)));
                    assert_eq!(
                        mb.partitions[0].differences[0],
                        if address == 0 || field {
                            [8, 4]
                        } else {
                            [0, 0]
                        }
                    );
                    assert_eq!(
                        if let Some(r) = &cb {
                            r.field_decoding()
                        } else {
                            cv.as_ref().unwrap().field_decoding()
                        },
                        field
                    );
                }
                assert!(if let Some(r) = &mut cb {
                    r.read_macroblock().unwrap().is_none()
                } else {
                    cv.as_mut().unwrap().read_macroblock().unwrap().is_none()
                });
            }
            assert_eq!(
                slice.header.disable_deblocking_filter_idc,
                if name.contains("filter0") {
                    0
                } else if name.contains("filter2") {
                    2
                } else {
                    1
                }
            );
        }
        // Exercise the public reconstruction facade as well as native playback.
        let mut first = Vec::new();
        input.read_packet(0, 0, &mut first).unwrap();
        let mut decoder = fvid::codec::avc_decoder::AvcDecoder::new(&bytes, 16 << 20).unwrap();
        let anchor = decoder.decode_order(&first).unwrap().unwrap();
        let first_slices = prepare(&first, config.length_size, &sps, &pps, first.len()).unwrap();
        let first_headers: Vec<_> = first_slices.iter().map(|slice| &slice.header).collect();
        let (first_picture, _) =
            fvid::codec::avc_inter_picture::decode_inter_resolved_slices_with_motion(
                &first_headers,
                &sps,
                &pps,
                &[[&[], &[]], [&[], &[]]],
                &[None, None],
                16 << 20,
            )
            .unwrap();
        assert_eq!(
            (&first_picture.y, &first_picture.cb, &first_picture.cr),
            (&anchor.y, &anchor.cb, &anchor.cr),
            "{name} all-intra common reconstruction"
        );
        let frames = [anchor.as_ref()];
        let references: Vec<[&[&fvid::codec::avc_picture::IntraPicture]; 2]> = slices
            .iter()
            .map(|slice| {
                if slice.header.slice_type == SliceType::I {
                    [&[][..], &[][..]]
                } else {
                    [&frames[..], &[][..]]
                }
            })
            .collect();
        let headers: Vec<_> = slices.iter().map(|slice| &slice.header).collect();
        let (picture, _) =
            fvid::codec::avc_inter_picture::decode_inter_resolved_slices_with_motion(
                &headers,
                &sps,
                &pps,
                &references,
                &[None, None],
                16 << 20,
            )
            .unwrap();
        if name == "ip-frame-field-filter0" {
            let mut invalid =
                fvid::codec::avc_slice::SliceHeader::parse(slices[1].nal, &sps, &pps).unwrap();
            invalid.nal_ref_idc = 0;
            let error = fvid::codec::avc_inter_picture::decode_inter_resolved_slices_with_motion(
                &[headers[0], &invalid],
                &sps,
                &pps,
                &references,
                &[None, None],
                16 << 20,
            )
            .err()
            .unwrap();
            assert!(error.to_string().contains("different pictures"));
        }
        let mut single = Vec::new();
        picture.write_planar(&mut single).unwrap();
        let frame_bytes = 32 * 32 * 3 / 2 * if name.contains("high10") { 2 } else { 1 };
        assert_eq!(
            single.as_slice(),
            &oracle[frame_bytes..frame_bytes * 2],
            "{name} public reconstruction"
        );
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..2 {
            let mut pixels = Vec::new();
            let mut count = 0;
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name} pass {pass} picture {count}: {e}"))
            {
                assert_eq!(frame.presentation_time.ticks, count);
                assert_eq!(frame.presentation_time.timescale, 25);
                frame.picture.write_planar(&mut pixels).unwrap();
                count += 1;
            }
            assert_eq!(count, 3);
            assert_eq!(pixels.len(), oracle.len());
            assert!(
                pixels == oracle,
                "{name} pass {pass} first byte mismatch {:?}",
                pixels.iter().zip(oracle).position(|(a, b)| a != b)
            );
            reader.rewind();
        }
    }
}

fn assert_owned_mixed_mbaff_pair(
    header: &fvid::codec::avc_slice::SliceHeader,
    sps: &Sps,
    pps: &Pps,
    pair: usize,
    field: bool,
    direct: bool,
) {
    use fvid::codec::{
        avc_cabac_macroblock::IntraCabacReader,
        avc_cabac_slice::InterCabacSlice,
        avc_inter::{InterHeader, Prediction},
        avc_inter_slice::{InterCavlcSlice, InterMacroblock},
        avc_macroblock::{IntraCavlcReader, IntraLuma},
        avc_slice::SliceType,
    };
    type Parsed = (usize, bool, Option<InterHeader>);
    let mut read: Box<dyn FnMut() -> fvid::Result<Option<Parsed>> + '_> =
        match (header.slice_type == SliceType::I, pps.cabac) {
            (true, false) => {
                let mut reader = IntraCavlcReader::new_mbaff(header, sps, pps, 4).unwrap();
                Box::new(move || {
                    let block = reader.read_macroblock()?;
                    let field = reader.field_decoding();
                    Ok(block.map(|mb| {
                        assert!(matches!(mb.luma, IntraLuma::Pcm { .. }));
                        (mb.address as usize, field, None)
                    }))
                })
            }
            (true, true) => {
                let mut reader = IntraCabacReader::new_mbaff(header, sps, pps, 4).unwrap();
                Box::new(move || {
                    let block = reader.read_macroblock()?;
                    let field = reader.field_decoding();
                    Ok(block.map(|mb| {
                        assert!(matches!(mb.luma, IntraLuma::Pcm { .. }));
                        (mb.address as usize, field, None)
                    }))
                })
            }
            (false, false) => {
                let mut reader = InterCavlcSlice::new_mbaff(header, sps, pps, 65536).unwrap();
                Box::new(move || {
                    let block = reader.read_macroblock()?;
                    let field = reader.field_decoding();
                    Ok(block.map(|mb| {
                        let InterMacroblock::Coded {
                            address, header, ..
                        } = mb
                        else {
                            panic!("expected owned coded inter block")
                        };
                        (address, field, Some(header))
                    }))
                })
            }
            (false, true) => {
                let mut reader = InterCabacSlice::new_mbaff(header, sps, pps, 65536).unwrap();
                Box::new(move || {
                    let block = reader.read_macroblock()?;
                    let field = reader.field_decoding();
                    Ok(block.map(|mb| {
                        let InterMacroblock::Coded {
                            address, header, ..
                        } = mb
                        else {
                            panic!("expected owned coded inter block")
                        };
                        (address, field, Some(header))
                    }))
                })
            }
        };
    for local in 0..2 {
        let (address, mode, mb) = read().unwrap().unwrap();
        assert_eq!(address, pair * 2 + local);
        assert_eq!(mode, field);
        if let Some(mb) = mb {
            if direct {
                assert_eq!(mb.partitions.len(), 16);
                assert!(
                    mb.partitions
                        .iter()
                        .all(|p| p.prediction == Prediction::Direct)
                );
            } else {
                assert_eq!(mb.partitions.len(), 1);
                assert_eq!(mb.partitions[0].prediction, Prediction::L0);
                assert_eq!(mb.partitions[0].references[0], Some(u8::from(field)));
                assert_eq!(
                    mb.partitions[0].differences[0],
                    if local == 0 || field { [8, 4] } else { [0, 0] }
                );
            }
        } else {
            assert!(!direct);
        }
    }
    assert!(read().unwrap().is_none());
}

#[test]
fn mixed_mbaff_b_slices_and_retained_intra_motion_match_jm_and_rewind() {
    use fvid::codec::{
        avc_access_unit::prepare,
        avc_direct::MbaffDirectPrediction,
        avc_dpb::ReferenceBuffer,
        avc_mbaff_picture::decode_inter_slices,
        avc_mv::Neighbour,
        avc_poc::{FieldOrder, PocDecoder},
        avc_references::FrameReference,
        avc_slice::SliceType,
    };
    macro_rules! case {
        ($name:expr) => {
            (
                $name,
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-mixed-",
                    $name,
                    ".mp4"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "fixtures/playback-errors/avc-mbaff-mixed-",
                    $name,
                    ".yuv"
                ))
                .as_slice(),
            )
        };
    }
    macro_rules! cases_for_mode {
        ($depth:literal,$entropy:literal,$weight:literal) => {
            [
                case!(concat!("ib-frame-field-filter0", $weight, $depth, $entropy)),
                case!(concat!("ib-frame-field-filter1", $weight, $depth, $entropy)),
                case!(concat!("ib-frame-field-filter2", $weight, $depth, $entropy)),
                case!(concat!("ib-field-frame-filter0", $weight, $depth, $entropy)),
                case!(concat!("ib-field-frame-filter1", $weight, $depth, $entropy)),
                case!(concat!("ib-field-frame-filter2", $weight, $depth, $entropy)),
                case!(concat!("bi-frame-field-filter0", $weight, $depth, $entropy)),
                case!(concat!("bi-frame-field-filter1", $weight, $depth, $entropy)),
                case!(concat!("bi-frame-field-filter2", $weight, $depth, $entropy)),
                case!(concat!("bi-field-frame-filter0", $weight, $depth, $entropy)),
                case!(concat!("bi-field-frame-filter1", $weight, $depth, $entropy)),
                case!(concat!("bi-field-frame-filter2", $weight, $depth, $entropy)),
                case!(concat!("pb-frame-field-filter0", $weight, $depth, $entropy)),
                case!(concat!("pb-frame-field-filter1", $weight, $depth, $entropy)),
                case!(concat!("pb-frame-field-filter2", $weight, $depth, $entropy)),
                case!(concat!("pb-field-frame-filter0", $weight, $depth, $entropy)),
                case!(concat!("pb-field-frame-filter1", $weight, $depth, $entropy)),
                case!(concat!("pb-field-frame-filter2", $weight, $depth, $entropy)),
                case!(concat!("bp-frame-field-filter0", $weight, $depth, $entropy)),
                case!(concat!("bp-frame-field-filter1", $weight, $depth, $entropy)),
                case!(concat!("bp-frame-field-filter2", $weight, $depth, $entropy)),
                case!(concat!("bp-field-frame-filter0", $weight, $depth, $entropy)),
                case!(concat!("bp-field-frame-filter1", $weight, $depth, $entropy)),
                case!(concat!("bp-field-frame-filter2", $weight, $depth, $entropy)),
            ]
        };
    }
    let cases: Vec<_> = [
        cases_for_mode!("", "", ""),
        cases_for_mode!("", "", "-explicit"),
        cases_for_mode!("", "-cabac", ""),
        cases_for_mode!("", "-cabac", "-explicit"),
        cases_for_mode!("-high10", "", ""),
        cases_for_mode!("-high10", "", "-explicit"),
        cases_for_mode!("-high10", "-cabac", ""),
        cases_for_mode!("-high10", "-cabac", "-explicit"),
    ]
    .into_iter()
    .flatten()
    .collect();
    for (name, video, oracle) in cases.iter().copied() {
        let depth = if name.contains("high10") { 10 } else { 8 };
        let frame_bytes = 1536 * if depth == 10 { 2 } else { 1 };
        if name.contains("filter0") {
            let other_name = name.replace("filter0", "filter2");
            let (_, _, other) = cases.iter().find(|(n, _, _)| *n == other_name).unwrap();
            assert!(&oracle[..frame_bytes] == &other[..frame_bytes]);
            assert!(
                &oracle[2 * frame_bytes..3 * frame_bytes]
                    != &other[2 * frame_bytes..3 * frame_bytes],
                "{name} must filter the mixed picture itself"
            );
        }
        let cabac = name.ends_with("-cabac");
        let explicit = name.contains("explicit");
        let left_field = name.contains("field-frame");
        let kinds = match &name[..2] {
            "ib" => [SliceType::I, SliceType::B],
            "bi" => [SliceType::B, SliceType::I],
            "pb" => [SliceType::P, SliceType::B],
            "bp" => [SliceType::B, SliceType::P],
            _ => unreachable!(),
        };
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[usize::from(cabac)], &sps).unwrap();
        assert!(sps.mb_adaptive_frame_field);
        assert_eq!(sps.coded_dimensions(), (32, 32));
        assert_eq!(sps.bit_depth_luma, depth);
        assert_eq!(pps.cabac, cabac);
        assert_eq!(pps.weighted_bipred, if explicit { 1 } else { 2 });
        let mut packets = Vec::new();
        for index in 0..5 {
            let mut packet = Vec::new();
            input.read_packet(0, index, &mut packet).unwrap();
            packets.push(packet);
        }
        let mut poc = PocDecoder::default();
        let mut dpb = ReferenceBuffer::new(sps.frame_num_bits, sps.max_num_ref_frames).unwrap();
        for (index, packet) in packets.iter().enumerate() {
            let slices = prepare(packet, config.length_size, &sps, &pps, packet.len()).unwrap();
            assert_eq!(slices.len(), 2);
            let order = poc.decode(&sps, &slices[0].header).unwrap();
            assert_eq!(order.before_marking.picture(), [0, 8, 4, 2, 6][index]);
            for (pair, slice) in slices.iter().enumerate() {
                let h = &slice.header;
                let kind = if index == 0 {
                    SliceType::I
                } else if index == 1 {
                    SliceType::P
                } else if index == 2 {
                    kinds[pair]
                } else {
                    SliceType::B
                };
                assert_eq!(h.slice_type, kind);
                assert_eq!(h.first_mb, pair as u32);
                assert_eq!(h.frame_num, [0, 1, 2, 3, 3][index]);
                assert_eq!(h.nal_ref_idc != 0, index < 3);
                assert_eq!(h.idr, index == 0);
                assert_eq!(
                    h.slice_qp,
                    if index == 2 && kind != SliceType::I {
                        50
                    } else {
                        26
                    }
                );
                if kind == SliceType::B {
                    assert!(!h.direct_spatial_mv_pred);
                    if explicit {
                        let weights = h.weights.as_ref().unwrap();
                        assert_eq!((weights.luma_denom, weights.chroma_denom), (1, 1));
                        assert_eq!(weights.l0[0].luma, (3, 1));
                        assert_eq!(weights.l1[0].luma, (1, -3));
                        assert_eq!(weights.l0[0].chroma, [(3, 2), (3, -1)]);
                        assert_eq!(weights.l1[0].chroma, [(1, -2), (1, 3)]);
                    } else {
                        assert!(h.weights.is_none());
                    }
                }
                let field = ((pair == 0) == left_field) ^ (index >= 3);
                assert_owned_mixed_mbaff_pair(h, &sps, &pps, pair, field, index == 3);
                let lists = dpb.lists(h, order.before_marking.picture()).unwrap();
                if index == 2 && kind != SliceType::I {
                    assert_eq!(lists.l0, [0], "source motion must refer to IDR");
                }
                if index == 3 {
                    assert_eq!(lists.l0, [0]);
                    assert_eq!(
                        lists.l1,
                        [2],
                        "temporal direct must use the retained mixed picture"
                    );
                }
            }
            dpb.finish(
                &slices[0].header,
                order.after_marking.picture(),
                index as u64,
                std::sync::Arc::new(()),
            )
            .unwrap();
        }
        // Independently inspect the assembled retained motion, including its
        // intra cells, stable picture identity and expanded field parity.
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        let anchor = decoder.decode_order(&packets[0]).unwrap().unwrap();
        let future = decoder.decode_order(&packets[1]).unwrap().unwrap();
        let slices = prepare(
            &packets[2],
            config.length_size,
            &sps,
            &pps,
            packets[2].len(),
        )
        .unwrap();
        let headers: Vec<_> = slices.iter().map(|s| &s.header).collect();
        let l0 = [anchor.as_ref()];
        let l1 = [future.as_ref()];
        let empty: &[&fvid::codec::avc_picture::IntraPicture] = &[];
        let references: Vec<_> = kinds
            .iter()
            .map(|kind| match kind {
                SliceType::I => [empty, empty],
                SliceType::P => [&l0[..], empty],
                SliceType::B => [&l0[..], &l1[..]],
                _ => unreachable!(),
            })
            .collect();
        let idr_meta = [FrameReference {
            id: 0,
            frame_num: 0,
            poc: 0,
            long_term_index: None,
        }];
        let future_meta = [FrameReference {
            id: 1,
            frame_num: 1,
            poc: 8,
            long_term_index: None,
        }];
        let idr_order = [FieldOrder {
            top: Some(0),
            bottom: Some(0),
        }];
        let future_order = [FieldOrder {
            top: Some(8),
            bottom: Some(8),
        }];
        let source_direct = MbaffDirectPrediction {
            spatial: false,
            inference8: sps.direct_8x8_inference,
            current_order: FieldOrder {
                top: Some(4),
                bottom: Some(4),
            },
            list0: &idr_meta,
            list1: &future_meta,
            list0_orders: &idr_order,
            list1_orders: &future_order,
            colocated: None,
        };
        let direct: Vec<_> = kinds
            .iter()
            .map(|kind| (*kind == SliceType::B).then_some(&source_direct))
            .collect();
        let (source, motion) =
            decode_inter_slices(&headers, &sps, &pps, &references, &direct, 16 << 20).unwrap();
        let mut pixels = Vec::new();
        source.write_planar(&mut pixels).unwrap();
        assert!(
            pixels.as_slice() == &oracle[2 * frame_bytes..3 * frame_bytes],
            "{name} source assembly first mismatch {:?}",
            pixels
                .iter()
                .zip(&oracle[2 * frame_bytes..3 * frame_bytes])
                .position(|(a, b)| a != b)
        );
        let ids0 = [0u64];
        let ids1 = [1u64];
        let mappings: Vec<(u32, [&[u64]; 2])> = kinds
            .iter()
            .enumerate()
            .map(|(pair, kind)| {
                (
                    pair as u32,
                    match kind {
                        SliceType::I => [&[][..], &[][..]],
                        SliceType::P => [&ids0[..], &[][..]],
                        SliceType::B => [&ids0[..], &ids1[..]],
                        _ => unreachable!(),
                    },
                )
            })
            .collect();
        let saved = motion.snapshot_mbaff_slices(&mappings, 65536).unwrap();
        for y in 0..32 {
            for x in (0..32).step_by(4) {
                let pair = x / 16;
                let field = (pair == 0) == left_field;
                let (cell, mode) = saved.at_mbaff([x, y]).unwrap();
                assert_eq!(mode, field);
                if kinds[pair] == SliceType::I {
                    assert_eq!(cell, [None; 2], "intra cells must not retain stale motion");
                } else {
                    let selected = cell[0].unwrap();
                    assert_eq!(selected.picture_id, 0);
                    assert_eq!(selected.vector, [8, 4]);
                    assert_eq!(selected.reference_index, u8::from(field));
                    assert_eq!(selected.reference_bottom_field, field.then_some(y % 2 == 0));
                    assert!(cell[1].is_none());
                }
            }
        }
        let mixed_meta = [FrameReference {
            id: 2,
            frame_num: 2,
            poc: 4,
            long_term_index: None,
        }];
        let mixed_order = [FieldOrder {
            top: Some(4),
            bottom: Some(4),
        }];
        let target_direct = MbaffDirectPrediction {
            spatial: false,
            inference8: sps.direct_8x8_inference,
            current_order: FieldOrder {
                top: Some(2),
                bottom: Some(2),
            },
            list0: &idr_meta,
            list1: &mixed_meta,
            list0_orders: &idr_order,
            list1_orders: &mixed_order,
            colocated: Some(&saved),
        };
        for address in 0..4 {
            let pair = address / 2;
            let source_field = (pair == 0) == left_field;
            for y in (0..16).step_by(4) {
                for x in (0..16).step_by(4) {
                    let vectors = if kinds[pair] == SliceType::I {
                        [[0, 0], [0, 0]]
                    } else if source_field {
                        [[4, 4], [-4, -4]]
                    } else {
                        [[4, 1], [-4, -1]]
                    };
                    assert_eq!(
                        target_direct
                            .derive(
                                address,
                                [x, y],
                                !source_field,
                                [fvid::codec::avc_mv::Neighbours {
                                    left: Neighbour::Unavailable,
                                    top: Neighbour::Unavailable,
                                    top_right: Neighbour::Unavailable,
                                    top_left: Neighbour::Unavailable
                                }; 2]
                            )
                            .unwrap(),
                        [
                            Neighbour::Inter {
                                reference: 0,
                                vector: vectors[0]
                            },
                            Neighbour::Inter {
                                reference: 0,
                                vector: vectors[1]
                            }
                        ]
                    );
                }
            }
        }
        let target = prepare(
            &packets[3],
            config.length_size,
            &sps,
            &pps,
            packets[3].len(),
        )
        .unwrap();
        let target_headers: Vec<_> = target.iter().map(|s| &s.header).collect();
        let l1 = [&source];
        let refs = [&l0[..], &l1[..]];
        let (picture, _) = decode_inter_slices(
            &target_headers,
            &sps,
            &pps,
            &[refs, refs],
            &[Some(&target_direct), Some(&target_direct)],
            16 << 20,
        )
        .unwrap();
        let mut pixels = Vec::new();
        picture.write_planar(&mut pixels).unwrap();
        assert!(
            pixels.as_slice() == &oracle[frame_bytes..2 * frame_bytes],
            "{name} direct target first mismatch {:?}",
            pixels
                .iter()
                .zip(&oracle[frame_bytes..2 * frame_bytes])
                .position(|(a, b)| a != b)
        );
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(video),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..2 {
            let mut actual = Vec::new();
            let mut count = 0;
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name} pass {pass} frame {count}: {e}"))
            {
                assert_eq!(frame.presentation_time.ticks, count);
                assert_eq!(frame.presentation_time.timescale, 25);
                assert_eq!(frame.picture.bit_depth, depth);
                frame.picture.write_planar(&mut actual).unwrap();
                count += 1;
            }
            assert_eq!(count, 5);
            assert_eq!(actual.len(), oracle.len());
            assert!(
                actual == oracle,
                "{name} pass {pass} first mismatch {:?}",
                actual.iter().zip(oracle).position(|(a, b)| a != b)
            );
            reader.rewind();
        }
    }
}

#[test]
fn owned_frame_number_gaps_reproduce_precise_native_refusal() {
    for video in [
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc0-field-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc0-field-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc0-field-high10-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc0-field-high10-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc0-frame-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc0-frame-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc0-frame-high10-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc0-frame-high10-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc1-field-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc1-field-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc1-field-high10-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc1-field-high10-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc1-frame-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc1-frame-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc1-frame-high10-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc1-frame-high10-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc2-field-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc2-field-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc2-field-high10-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc2-field-high10-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc2-frame-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc2-frame-cavlc.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc2-frame-high10-cabac.mp4")
            .as_slice(),
        include_bytes!("fixtures/playback-errors/avc-frame-num-gap-poc2-frame-high10-cavlc.mp4")
            .as_slice(),
    ] {
        let mut input = Mp4Reader::open(Cursor::new(video), Default::default()).unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        assert!(sps.gaps_allowed);
        let mut decoder = AvcDecoder::new(&configuration, 16 << 20).unwrap();
        for _ in 0..2 {
            let mut packet = Vec::new();
            input.read_packet(0, 0, &mut packet).unwrap();
            assert!(decoder.decode_order(&packet).unwrap().is_some());
            input.read_packet(0, 1, &mut packet).unwrap();
            let error = decoder
                .decode_order(&packet)
                .err()
                .expect("gap must remain refused");
            assert!(error.to_string().contains("frame-number gaps"), "{error}");
            decoder.reset();
        }
    }
}
