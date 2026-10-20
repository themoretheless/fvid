use fvid::{
    codec::{
        avc::{Pps, Sps},
        avc_decoder::AvcDecoder,
        avc_mbaff_picture,
        avc_picture::decode_intra_slices,
        config::AvcConfig,
    },
    container::mp4::Limits,
    playback::video::mp4::Mp4VideoReader,
};
use std::path::Path;

#[test]
fn separate_plane_mbaff_8x8_scaling_aso_and_deblocking_decode_frames_and_mp4() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-mbaff-8x8-deblock");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let list = cases.as_array().unwrap();
    assert_eq!(list.len(), 432);

    for case in list {
        let name = case["name"].as_str().unwrap();
        let depth = case["depth"].as_u64().unwrap() as u8;
        let cfg_bytes = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let parsed_cfg = AvcConfig::parse(&cfg_bytes).unwrap();
        let sps = Sps::parse(parsed_cfg.sps[0]).unwrap();
        let pps = Pps::parse(parsed_cfg.pps[0], &sps).unwrap();
        assert!(sps.separate_colour_plane);
        assert!(!sps.frame_mbs_only);
        assert!(sps.mb_adaptive_frame_field);
        assert!(pps.transform_8x8);

        let golden: Vec<u16> = std::fs::read(root.join(format!("{name}.y16")))
            .unwrap()
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let plane_samples = 32 * 64;
        let frame_samples = plane_samples * 3;
        assert_eq!(golden.len(), frame_samples * 2);

        let packets: Vec<Vec<u8>> = (0..2)
            .map(|i| std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
            .collect();

        // 1. Direct intra slice / MBAFF intra slice entrypoint checks (including raw unsorted ASO slice order)
        let mut raw_slices = Vec::new();
        for nal in fvid::codec::config::NalUnits::new(&packets[0], 4).unwrap() {
            let nal = nal.unwrap();
            raw_slices.push(fvid::codec::avc_slice::SliceHeader::parse(nal, &sps, &pps).unwrap());
        }
        let raw_refs: Vec<_> = raw_slices.iter().collect();

        let direct_pic = decode_intra_slices(&raw_refs, &sps, &pps, 1 << 20)
            .unwrap_or_else(|e| panic!("direct avc_picture::decode_intra_slices {name}: {e}"));
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

        let mbaff_direct = avc_mbaff_picture::decode_intra_slices(&raw_refs, &sps, &pps, 1 << 20)
            .unwrap_or_else(|e| panic!("direct avc_mbaff_picture::decode_intra_slices {name}: {e}"));
        assert_eq!(mbaff_direct.y, &golden[0..plane_samples], "{name} mbaff direct Y");
        assert_eq!(
            mbaff_direct.cb,
            &golden[plane_samples..2 * plane_samples],
            "{name} mbaff direct Cb"
        );
        assert_eq!(
            mbaff_direct.cr,
            &golden[2 * plane_samples..3 * plane_samples],
            "{name} mbaff direct Cr"
        );

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
                    for y in 2..62 {
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
