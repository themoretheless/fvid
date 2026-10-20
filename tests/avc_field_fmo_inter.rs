use fvid::{
    codec::{
        avc::{Pps, SliceGroups, Sps},
        avc_decoder::AvcDecoder,
        avc_field_picture::{decode_inter_field_lists_with_motion, decode_intra_slices},
        avc_inter_slice::InterCavlcSlice,
        avc_macroblock::IntraCavlcReader,
        avc_slice::{SliceHeader, SliceType},
        avc_slice_group_map,
        config::{AvcConfig, NalUnits},
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::Mp4VideoReader,
};
use std::{io::Cursor, path::Path};

fn fixture_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-field-fmo-inter")
}

fn plane_samples(raw: &[u8], bit_depth: u8) -> Vec<u16> {
    if bit_depth > 8 {
        raw.chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    } else {
        raw.iter().map(|&v| u16::from(v)).collect()
    }
}

#[test]
fn avc_field_fmo_inter_reconstructs_all_640_synthetic_movies_and_playback() {
    let root = fixture_root();
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let list = cases.as_array().unwrap();
    assert_eq!(list.len(), 640);

    for case in list {
        let stem = case["name"].as_str().unwrap();
        let depth = case["depth"].as_u64().unwrap() as u8;
        let kind = case["kind"].as_u64().unwrap() as usize;
        let direction = case["direction"].as_bool().unwrap();
        let bottom_first = case["bottom_first"].as_bool().unwrap();
        let spatial = case["spatial"].as_bool().unwrap();
        let skip_mode = case["skip_mode"].as_bool().unwrap();
        let residual = case["residual"].as_bool().unwrap();
        let deblock = case["deblock"].as_u64().unwrap() as usize;
        let aso = case["aso"].as_bool().unwrap();

        let video = std::fs::read(root.join(format!("{stem}.mp4"))).unwrap();
        let raw = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
        let oracle = plane_samples(&raw, depth);
        let frame_samples = 32 * 64 + 2 * 16 * 32;
        assert_eq!(oracle.len(), 3 * frame_samples, "{stem}");

        let mut input = Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
        let config = input.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&config).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        assert!(!sps.frame_mbs_only, "{stem}");
        assert!(!sps.mb_adaptive_frame_field, "{stem}");
        assert!(!pps.cabac, "{stem}");
        assert!(!matches!(pps.slice_groups, SliceGroups::Single), "{stem}");

        let mut decoder = AvcDecoder::new(&config, 32 << 20).unwrap();
        let mut packet = Vec::new();
        let mut decoded_frames = Vec::new();
        for pkt_idx in 0..6 {
            input.read_packet(0, pkt_idx, &mut packet).unwrap();
            let headers: Vec<SliceHeader> = NalUnits::new(&packet, avc.length_size)
                .unwrap()
                .map(|nal| SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap())
                .collect();
            assert_eq!(headers.len(), 2, "{stem} pkt {pkt_idx}");
            for h in &headers {
                assert!(h.field_pic, "{stem}");
                let map = avc_slice_group_map::map_units(
                    &pps.slice_groups,
                    2,
                    2,
                    h.slice_group_change_cycle,
                    4,
                )
                .unwrap();
                let mb_map =
                    avc_slice_group_map::macroblocks(&map, 2, false, true, false, 4).unwrap();
                assert_eq!(mb_map.len(), 4, "{stem}");
                if matches!(h.slice_type, SliceType::P | SliceType::B) {
                    let _ctx = IntraCavlcReader::new_context_fmo(h, &sps, &pps, 4).unwrap();
                    let mut slice_reader =
                        InterCavlcSlice::new_fmo(h, &sps, &pps, 65536).unwrap();
                    assert!(slice_reader.field_decoding(), "{stem}");
                    let mut mbs = Vec::new();
                    while let Some(mb) = slice_reader.read_macroblock().unwrap() {
                        mbs.push(mb);
                    }
                    let expected_mbs = mb_map
                        .iter()
                        .filter(|&&g| g == mb_map[h.first_mb as usize])
                        .count();
                    assert_eq!(mbs.len(), expected_mbs, "{stem} pkt {pkt_idx}");
                }
            }
            if let Some(pic) = decoder
                .decode_order(&packet)
                .unwrap_or_else(|e| panic!("{stem} pkt {pkt_idx}: {e}"))
            {
                decoded_frames.push(pic);
            }
        }
        assert_eq!(decoded_frames.len(), 3, "{stem}");

        // Direct API check on representative subset
        if !spatial && skip_mode && (!residual || aso) && deblock != 2 {
            let mut p0 = Vec::new();
            let mut p1 = Vec::new();
            input.read_packet(0, 0, &mut p0).unwrap();
            input.read_packet(0, 1, &mut p1).unwrap();
            let h0: Vec<SliceHeader> = NalUnits::new(&p0, avc.length_size)
                .unwrap()
                .map(|nal| SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap())
                .collect();
            let h1: Vec<SliceHeader> = NalUnits::new(&p1, avc.length_size)
                .unwrap()
                .map(|nal| SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap())
                .collect();
            let r0: Vec<&SliceHeader> = h0.iter().collect();
            let r1: Vec<&SliceHeader> = h1.iter().collect();
            let f0 = decode_intra_slices(&r0, &sps, &pps, 1 << 20).unwrap();
            let f1 = decode_intra_slices(&r1, &sps, &pps, 1 << 20).unwrap();

            input.read_packet(0, 2, &mut packet).unwrap();
            let headers: Vec<SliceHeader> = NalUnits::new(&packet, avc.length_size)
                .unwrap()
                .map(|nal| SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap())
                .collect();
            let refs: Vec<&SliceHeader> = headers.iter().collect();
            let ref_list = if headers[0].bottom_field == f0.bottom {
                [(&f0, 0u64)]
            } else {
                [(&f1, 1u64)]
            };
            let slice_lists = [[&ref_list[..], &[][..]], [&ref_list[..], &[][..]]];
            let (p_field, _motion) = decode_inter_field_lists_with_motion(
                &refs,
                &sps,
                &pps,
                &slice_lists,
                None,
                1 << 20,
            )
            .unwrap();
            let parity = usize::from(headers[0].bottom_field);
            for row in 0..32 {
                let frame_row = row * 2 + parity;
                assert_eq!(
                    &p_field.picture.y[row * 32..(row + 1) * 32],
                    &decoded_frames[1].y[frame_row * 32..(frame_row + 1) * 32],
                    "{stem} direct field row {row}"
                );
            }

            // Verify reset + re-decode
            decoder.reset();
            let mut redecoded = Vec::new();
            for pkt_idx in 0..6 {
                input.read_packet(0, pkt_idx, &mut packet).unwrap();
                if let Some(pic) = decoder.decode_order(&packet).unwrap() {
                    redecoded.push(pic);
                }
            }
            for (a, b) in redecoded.iter().zip(decoded_frames.iter()) {
                assert_eq!(a.y, b.y, "{stem}");
                assert_eq!(a.cb, b.cb, "{stem}");
                assert_eq!(a.cr, b.cr, "{stem}");
            }
        }

        // Decode-order vs display-order mapping:
        // decode order: [I (disp 0), P (disp 2), B (disp 1)]
        for (disp_idx, decode_idx) in [(0usize, 0usize), (2, 1), (1, 2)] {
            let pic = &decoded_frames[decode_idx];
            let base = disp_idx * frame_samples;
            for (idx, (&a, &b)) in pic.y.iter().zip(&oracle[base..base + 2048]).enumerate() {
                assert_eq!(a, b, "{stem} disp {disp_idx} Y idx {idx} (row {} col {})", idx / 32, idx % 32);
            }
            assert_eq!(
                &pic.cb[..],
                &oracle[base + 2048..base + 2560],
                "{stem} disp {disp_idx} Cb"
            );
            assert_eq!(
                &pic.cr[..],
                &oracle[base + 2560..base + 3072],
                "{stem} disp {disp_idx} Cr"
            );
        }

        if !aso {
            let aso_stem = format!("{stem}-aso");
            let aso_raw = std::fs::read(root.join(format!("{aso_stem}.yuv"))).unwrap();
            assert_eq!(raw, aso_raw, "{stem} vs {aso_stem}");
        }
        if skip_mode {
            let direct_stem = format!(
                "avc-field-fmo-inter-{depth}bit-type{kind}-dir{}-{}-{}-direct-res{}-filter{deblock}{}",
                usize::from(direction),
                if bottom_first {
                    "bottom-first"
                } else {
                    "top-first"
                },
                if spatial { "spatial" } else { "temporal" },
                usize::from(residual),
                if aso { "-aso" } else { "" }
            );
            let direct_raw = std::fs::read(root.join(format!("{direct_stem}.yuv"))).unwrap();
            assert_eq!(raw, direct_raw, "{stem} vs {direct_stem}");
        }

        if (!spatial && skip_mode && !residual && deblock == 1 && !aso)
            || (residual && deblock == 0 && aso)
        {
            let mut player =
                Mp4VideoReader::open_software(Cursor::new(&video), Limits::default(), 16 << 20)
                    .unwrap();
            let first = player.read_frame().unwrap().unwrap();
            let second = player.read_frame().unwrap().unwrap();
            let third = player.read_frame().unwrap().unwrap();
            assert!(player.read_frame().unwrap().is_none(), "{stem}");
            assert_eq!(first.picture.dimensions(), (32, 64));
            assert_eq!(second.picture.dimensions(), (32, 64));
            assert_eq!(third.picture.dimensions(), (32, 64));
            assert_eq!(&first.picture.y[..], &oracle[0..2048], "{stem} player disp 0");
            assert_eq!(&second.picture.y[..], &oracle[frame_samples..frame_samples + 2048], "{stem} player disp 1");
            assert_eq!(&third.picture.y[..], &oracle[2 * frame_samples..2 * frame_samples + 2048], "{stem} player disp 2");

            player.rewind();
            let re_first = player.read_frame().unwrap().unwrap();
            assert_eq!(re_first.picture.y, first.picture.y, "{stem} rewind");
            let re_second = player.read_frame().unwrap().unwrap();
            assert_eq!(re_second.picture.y, second.picture.y, "{stem} second");
            let re_third = player.read_frame().unwrap().unwrap();
            assert_eq!(re_third.picture.y, third.picture.y, "{stem} third");
        }
    }
}

#[test]
fn avc_field_fmo_inter_still_rejects_cabac_with_fmo_slice_groups() {
    let root = fixture_root();
    let video = std::fs::read(
        root.join(
            "avc-field-fmo-inter-8bit-type0-dir0-top-first-temporal-direct-res0-filter1.mp4",
        ),
    )
    .unwrap();
    let mut input = Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
    let config = input.tracks()[0].configuration.clone();
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps_orig = Pps::parse(avc.pps[0], &sps).unwrap();

    let mut p0 = Vec::new();
    input.read_packet(0, 0, &mut p0).unwrap();
    let h0: Vec<SliceHeader> = NalUnits::new(&p0, avc.length_size)
        .unwrap()
        .map(|nal| SliceHeader::parse(nal.unwrap(), &sps, &pps_orig).unwrap())
        .collect();
    let r0: Vec<&SliceHeader> = h0.iter().collect();
    let top_ref = decode_intra_slices(&r0, &sps, &pps_orig, 1 << 20).unwrap();

    let mut packet = Vec::new();
    input.read_packet(0, 2, &mut packet).unwrap();
    let headers: Vec<SliceHeader> = NalUnits::new(&packet, avc.length_size)
        .unwrap()
        .map(|nal| SliceHeader::parse(nal.unwrap(), &sps, &pps_orig).unwrap())
        .collect();
    let refs: Vec<&SliceHeader> = headers.iter().collect();
    let mut pps = pps_orig;
    pps.cabac = true;

    let ref_list = [(&top_ref, 0u64)];
    let slice_lists = [[&ref_list[..], &[][..]], [&ref_list[..], &[][..]]];
    let err = decode_inter_field_lists_with_motion(
        &refs,
        &sps,
        &pps,
        &slice_lists,
        None,
        1 << 20,
    )
    .err()
    .expect("CABAC + FMO field slices must remain rejected");
    assert!(
        err.to_string()
            .contains("general AVC P field reconstruction is not connected"),
        "unexpected error: {err}"
    );
}
