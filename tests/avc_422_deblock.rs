use fvid::codec::avc_decoder::AvcDecoder;
#[test]
fn intra_422_chroma_deblocking() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-422-deblock");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 54);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut decoder = AvcDecoder::new(
            &std::fs::read(root.join(format!("{name}.avcc"))).unwrap(),
            1 << 20,
        )
        .unwrap();
        let packet = std::fs::read(root.join(format!("{name}.packet"))).unwrap();
        let raw = std::fs::read(root.join(format!("{name}.y16"))).unwrap();
        let samples: Vec<u16> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let depth = yd.max(cd);
        for _ in 0..2 {
            let picture = decoder
                .decode_order(&packet)
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .unwrap();
            assert_eq!(picture.chroma_format().unwrap(), 2);
            assert_eq!(picture.bit_depth, yd);
            assert_eq!(picture.chroma_bit_depth, cd);
            assert_eq!(picture.y, samples[..1024], "Y {name}");
            assert_eq!(picture.cb, samples[1024..1536], "Cb {name}");
            assert_eq!(picture.cr, samples[1536..], "Cr {name}");
            let mut planar = Vec::new();
            picture.write_planar(&mut planar).unwrap();
            let mut native = Vec::new();
            for (i, &v) in samples.iter().enumerate() {
                if (if i < 1024 { yd } else { cd }) == 8 {
                    native.push(v as u8);
                } else {
                    native.extend_from_slice(&v.to_le_bytes());
                }
            }
            assert_eq!(planar, native, "planar {name}");
            decoder.reset();
        }
        let mut expected = Vec::new();
        for (i, &v) in samples.iter().enumerate() {
            let v = v << (depth - if i < 1024 { yd } else { cd });
            if depth == 8 {
                expected.push(v as u8);
            } else {
                expected.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open(
            std::io::Cursor::new(std::fs::read(root.join(format!("{name}.mp4"))).unwrap()),
            fvid::container::mp4::Limits::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let frame = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                .unwrap();
            let packed = frame.packed.unwrap();
            assert_eq!(packed.depth, depth);
            assert_eq!(packed.frame.subsampling, Some([2, 1]));
            assert_eq!(packed.frame.data, expected, "MP4 {name}");
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
