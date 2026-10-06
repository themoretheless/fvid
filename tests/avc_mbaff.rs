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
                            _ => other.nal_ref_idc ^= 1,
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
        ($name:literal) => {
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
    for (name, video, oracle) in [
        case!("field-to-frame"),
        case!("frame-to-field"),
        case!("field-to-frame-high10"),
        case!("frame-to-field-high10"),
        case!("field-to-frame-explicit"),
        case!("frame-to-field-explicit"),
        case!("field-to-frame-explicit-high10"),
        case!("frame-to-field-explicit-high10"),
    ] {
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
        assert_eq!(
            sps.bit_depth_luma,
            if name.contains("high10") { 10 } else { 8 }
        );
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
            if name.contains("high10") {
                assert!(
                    oracle
                        .chunks_exact(2)
                        .any(|v| u16::from_le_bytes([v[0], v[1]]) == 1023)
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
