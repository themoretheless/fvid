use fvid::codec::avc_decoder::AvcDecoder;
#[test]
fn inter_422_mixed_pcm_dc_ac_reconstruction() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-422-inter-residual");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 29);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let w = case["width"].as_u64().unwrap_or(32) as usize;
        let h = case["height"].as_u64().unwrap_or(16) as usize;
        let luma_count = w * h;
        let chroma_count = luma_count / 2;
        let frame_samples = luma_count + 2 * chroma_count;
        let mut decoder = AvcDecoder::new(
            &std::fs::read(root.join(format!("{name}.avcc"))).unwrap(),
            1 << 20,
        )
        .unwrap();
        let raw = std::fs::read(root.join(format!("{name}.y16"))).unwrap();
        let samples: Vec<u16> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let depth = yd.max(cd);
        for _ in 0..2 {
            for i in 0..2 {
                let picture = decoder
                    .decode_order(&std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
                    .unwrap_or_else(|e| panic!("{name} frame{i}: {e}"))
                    .unwrap();
                let at = i * frame_samples;
                assert_eq!(
                    picture.y,
                    samples[at..at + luma_count],
                    "Y {name} frame{i}"
                );
                assert_eq!(
                    picture.cb,
                    samples[at + luma_count..at + luma_count + chroma_count],
                    "Cb {name} frame{i}"
                );
                assert_eq!(
                    picture.cr,
                    samples[at + luma_count + chroma_count..at + frame_samples],
                    "Cr {name} frame{i}"
                );
                assert_eq!(picture.chroma_format().unwrap(), 2);
                assert_eq!(picture.bit_depth, yd);
                assert_eq!(picture.chroma_bit_depth, cd);
                let mut planar = Vec::new();
                picture.write_planar(&mut planar).unwrap();
                let mut expected = Vec::new();
                for (p, &v) in samples[at..at + frame_samples].iter().enumerate() {
                    if (if p < luma_count { yd } else { cd }) == 8 {
                        expected.push(v as u8);
                    } else {
                        expected.extend_from_slice(&v.to_le_bytes());
                    }
                }
                assert_eq!(planar, expected, "planar {name} frame{i}");
            }
            decoder.reset();
        }
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open(
            std::io::Cursor::new(std::fs::read(root.join(format!("{name}.mp4"))).unwrap()),
            fvid::container::mp4::Limits::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for i in 0..2 {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                    .unwrap();
                let packed = frame.packed.unwrap();
                assert_eq!(packed.depth, depth);
                assert_eq!(packed.frame.subsampling, Some([2, 1]));
                let mut expected = Vec::new();
                for (p, &v) in samples[i * frame_samples..(i + 1) * frame_samples]
                    .iter()
                    .enumerate()
                {
                    let v = v << (depth - if p < luma_count { yd } else { cd });
                    if depth == 8 {
                        expected.push(v as u8);
                    } else {
                        expected.extend_from_slice(&v.to_le_bytes());
                    }
                }
                assert_eq!(packed.frame.data, expected, "MP4 {name} frame{i}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}

#[test]
fn cabac_422_eight_block_inter_neighbour_context() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_cabac_macroblock::{InterNeighbourContext, IntraCabacReader},
        avc_slice::SliceHeader,
        config::AvcConfig,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-422-inter-residual-cabac");
    let raw = std::fs::read(root.join("cabac-422-residual-y8-c8-dc1-mask.avcc")).unwrap();
    let cfg = AvcConfig::parse(&raw).unwrap();
    let sps = Sps::parse(cfg.sps[0]).unwrap();
    let pps = Pps::parse(cfg.pps[0], &sps).unwrap();
    let packet = std::fs::read(root.join("cabac-422-residual-y8-c8-dc1-mask-1.packet")).unwrap();
    let header = SliceHeader::parse(&packet[4..], &sps, &pps).unwrap();
    let mut reader = IntraCabacReader::new_context(&header, &sps, &pps, 2).unwrap();
    let ctx = InterNeighbourContext {
        qp: header.slice_qp,
        qp_delta_nonzero: false,
        pattern: 32,
        transform8: false,
        luma_coded: [false; 16],
        chroma_dc: [true, true],
        chroma_ac: [[false; 4]; 2],
    };
    let err = reader.record_inter(&ctx).unwrap_err();
    assert!(!matches!(err, fvid::Error::Unsupported(_)), "{err}");
    let mut reader = IntraCabacReader::new_context(&header, &sps, &pps, 2).unwrap();
    reader
        .record_inter_422(
            &ctx,
            [
                [true, false, false, true, true, false, false, true],
                [false, true, true, false, false, true, true, false],
            ],
        )
        .unwrap();
    assert_eq!(reader.address(), 1);
}

