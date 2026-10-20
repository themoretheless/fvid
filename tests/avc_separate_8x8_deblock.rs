use fvid::{
    codec::{
        avc::{Pps, ScalingList, Sps},
        avc_access_unit,
        avc_decoder::AvcDecoder,
        avc_field_picture::{self, PcmField},
        avc_picture::decode_intra_slices,
        avc_scaling::ScalingMatrices,
        config::AvcConfig,
    },
    container::mp4::Limits,
    playback::video::mp4::Mp4VideoReader,
};
use std::path::Path;

#[test]
fn separate_plane_8x8_scaling_aso_and_deblocking_decode_frames_fields_and_mp4() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-8x8-deblock");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let list = cases.as_array().unwrap();
    assert_eq!(list.len(), 216);

    for case in list {
        let name = case["name"].as_str().unwrap();
        let depth = case["depth"].as_u64().unwrap() as u8;
        let paff = case["paff"].as_bool().unwrap();
        let cfg_bytes = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let parsed_cfg = AvcConfig::parse(&cfg_bytes).unwrap();
        let sps = Sps::parse(parsed_cfg.sps[0]).unwrap();
        let pps = Pps::parse(parsed_cfg.pps[0], &sps).unwrap();
        assert!(sps.separate_colour_plane);
        assert!(pps.transform_8x8);

        let golden: Vec<u16> = std::fs::read(root.join(format!("{name}.y16")))
            .unwrap()
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let plane_samples = if paff { 32 * 32 } else { 32 * 16 };
        let frame_samples = plane_samples * 3;
        assert_eq!(golden.len(), frame_samples * 2);

        let packet_count = if paff { 4 } else { 2 };
        let packets: Vec<Vec<u8>> = (0..packet_count)
            .map(|i| std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
            .collect();

        // 1. Direct intra slice / field slice entrypoint checks (verifying raw unsorted ASO slice order too!)
        if !paff {
            let mut raw_slices = Vec::new();
            for nal in fvid::codec::config::NalUnits::new(&packets[0], 4).unwrap() {
                let nal = nal.unwrap();
                raw_slices.push(fvid::codec::avc_slice::SliceHeader::parse(nal, &sps, &pps).unwrap());
            }
            let raw_refs: Vec<_> = raw_slices.iter().collect();
            let direct_pic = decode_intra_slices(&raw_refs, &sps, &pps, 1 << 20)
                .unwrap_or_else(|e| panic!("direct decode_intra_slices {name}: {e}"));
            assert_eq!(direct_pic.y, &golden[0..plane_samples], "{name} direct Y");
            assert_eq!(
                direct_pic.cb,
                &golden[plane_samples..2 * plane_samples],
                "{name} direct Cb"
            );
            assert_eq!(
                direct_pic.cr,
                &golden[2 * plane_samples..3 * plane_samples],
                "{name} direct Cr"
            );
        } else {
            // Direct PAFF field entrypoints: I fields (packets 0, 1) and P fields (packets 2, 3)
            let mut decoded_i_fields: Vec<PcmField> = Vec::new();
            for au in 0..2 {
                let mut raw_slices = Vec::new();
                for nal in fvid::codec::config::NalUnits::new(&packets[au], 4).unwrap() {
                    let nal = nal.unwrap();
                    raw_slices
                        .push(fvid::codec::avc_slice::SliceHeader::parse(nal, &sps, &pps).unwrap());
                }
                let raw_refs: Vec<_> = raw_slices.iter().collect();
                let field = avc_field_picture::decode_intra_slices(&raw_refs, &sps, &pps, 1 << 20)
                    .unwrap_or_else(|e| panic!("direct PAFF I field {name} au {au}: {e}"));
                decoded_i_fields.push(field);
            }
            let woven_i =
                avc_field_picture::weave_pair(&decoded_i_fields[0], &decoded_i_fields[1], 1 << 20)
                    .unwrap();
            assert_eq!(woven_i.y, &golden[0..plane_samples], "{name} woven I Y");
            assert_eq!(
                woven_i.cb,
                &golden[plane_samples..2 * plane_samples],
                "{name} woven I Cb"
            );
            assert_eq!(
                woven_i.cr,
                &golden[2 * plane_samples..3 * plane_samples],
                "{name} woven I Cr"
            );

            let mut decoded_p_fields: Vec<PcmField> = Vec::new();
            for (parity_idx, au) in (2..4).enumerate() {
                let slices = avc_access_unit::prepare(&packets[au], 4, &sps, &pps, 1 << 20).unwrap();
                let headers: Vec<_> = slices.iter().map(|s| &s.header).collect();
                let ref_field = &decoded_i_fields[parity_idx];
                let single_ref = [(ref_field, parity_idx as u64 + 1)];
                let empty_ref: [(&PcmField, u64); 0] = [];
                let refs_by_slice: Vec<[&[(&PcmField, u64)]; 2]> = headers
                    .iter()
                    .map(|_| [&single_ref[..], &empty_ref[..]])
                    .collect();
                let p_field = avc_field_picture::decode_inter_field_lists(
                    &headers,
                    &sps,
                    &pps,
                    &refs_by_slice,
                    1 << 20,
                )
                .unwrap_or_else(|e| panic!("direct PAFF P field {name} au {au}: {e}"));
                decoded_p_fields.push(p_field);
            }
            let woven_p =
                avc_field_picture::weave_pair(&decoded_p_fields[0], &decoded_p_fields[1], 1 << 20)
                    .unwrap();
            assert_eq!(
                woven_p.y,
                &golden[frame_samples..frame_samples + plane_samples],
                "{name} woven P Y"
            );
            assert_eq!(
                woven_p.cb,
                &golden[frame_samples + plane_samples..frame_samples + 2 * plane_samples],
                "{name} woven P Cb"
            );
            assert_eq!(
                woven_p.cr,
                &golden[frame_samples + 2 * plane_samples..frame_samples + 3 * plane_samples],
                "{name} woven P Cr"
            );
        }

        // 2. AvcDecoder verification
        let mut decoder = AvcDecoder::new(&cfg_bytes, 2 << 20).unwrap();
        for _ in 0..2 {
            let mut emitted = Vec::new();
            for (au, pkt) in packets.iter().enumerate() {
                if let Some(pic) = decoder
                    .decode_order(pkt)
                    .unwrap_or_else(|e| panic!("{name} au {au}: {e}"))
                {
                    emitted.push(pic);
                }
            }
            assert_eq!(emitted.len(), 2, "{name} emitted count");
            for (frame_idx, pic) in emitted.iter().enumerate() {
                assert_eq!(pic.chroma_format().unwrap(), 3);
                assert_eq!(pic.component_depths(), [depth; 3]);
                for (c, plane) in [&pic.y, &pic.cb, &pic.cr].into_iter().enumerate() {
                    let offset = frame_idx * frame_samples + c * plane_samples;
                    assert_eq!(
                        plane.as_slice(),
                        &golden[offset..offset + plane_samples],
                        "{name} frame {frame_idx} plane {c}"
                    );
                }
            }
            decoder.reset();
        }

        // 3. Mp4VideoReader verification
        let mp4_bytes = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
        let mut reader = Mp4VideoReader::open_software(
            std::io::Cursor::new(mp4_bytes),
            Limits::default(),
            16 << 20,
        )
        .unwrap();
        let crop_y = if paff { (2, 30) } else { (1, 15) };
        for _ in 0..2 {
            for frame_idx in 0..2 {
                let decoded = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name} frame {frame_idx}: {e}"))
                    .unwrap();
                let packed = decoded.packed.unwrap();
                assert_eq!(packed.depth, depth);
                assert_eq!(packed.frame.subsampling, Some([1, 1]));
                let mut expected = Vec::new();
                for c in 0..3 {
                    let base = frame_idx * frame_samples + c * plane_samples;
                    for y in crop_y.0..crop_y.1 {
                        for x in 1..31 {
                            let v = golden[base + y * 32 + x];
                            if depth == 8 {
                                expected.push(v as u8);
                            } else {
                                expected.extend_from_slice(&v.to_le_bytes());
                            }
                        }
                    }
                }
                assert_eq!(packed.frame.data, expected, "MP4 {name} frame {frame_idx}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}

#[test]
fn six_list_pps_preserves_sps_8x8_scaling_matrices_for_standard_and_separate_planes() {
    let bytes = include_bytes!(
        "fixtures/playback-errors/avc-separate-8x8-deblock/sep-8x8-8-e0-sps-prog-d0-a0.avcc"
    );
    let config = AvcConfig::parse(bytes).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let mut pps = Pps::parse(config.pps[0], &sps).unwrap();
    // Simulate a 6-list PPS (pic_scaling_matrix_present_flag = 1 with only 4x4 lists present):
    pps.scaling_lists = Some(vec![None; 6]);
    for (plane, expected) in [(0, [8, 12]), (1, [16, 20]), (2, [24, 28])] {
        let m = ScalingMatrices::for_colour_plane(&sps, &pps, plane).unwrap();
        assert_eq!(m.eight[0], [expected[0]; 64], "plane {plane} intra 8x8");
        assert_eq!(m.eight[1], [expected[1]; 64], "plane {plane} inter 8x8");
    }

    // Also verify non-separated 8-list SPS with a 6-list PPS:
    let mut std_sps = sps.clone();
    std_sps.chroma_format = 1;
    std_sps.separate_colour_plane = false;
    std_sps.scaling_lists = Some(vec![
        None,
        None,
        None,
        None,
        None,
        None,
        Some(ScalingList::Explicit(vec![19; 64])),
        Some(ScalingList::Explicit(vec![29; 64])),
    ]);
    let mut std_pps = pps.clone();
    std_pps.scaling_lists = Some(vec![None; 6]);
    let m = ScalingMatrices::new(&std_sps, &std_pps).unwrap();
    assert_eq!(m.eight[0], [19; 64]);
    assert_eq!(m.eight[1], [29; 64]);
}
