use fvid::codec::avc_decoder::AvcDecoder;
#[test]
fn b_cabac_residual_422_prediction_and_order() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-422-b-residual-cabac");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 405);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
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
            for i in 0..3 {
                let display = case["decode_order"][i].as_u64().unwrap() as usize;
                let at = display * 1024;
                let picture = decoder
                    .decode_order(&std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
                    .unwrap_or_else(|e| panic!("{name} decode{i}: {e}"))
                    .unwrap();
                assert_eq!(picture.chroma_format().unwrap(), 2);
                assert_eq!([picture.bit_depth, picture.chroma_bit_depth], [yd, cd]);
                assert_eq!(picture.y, samples[at..at + 512], "Y {name} decode{i}");
                assert_eq!(
                    picture.cb,
                    samples[at + 512..at + 768],
                    "Cb {name} decode{i}"
                );
                assert_eq!(
                    picture.cr,
                    samples[at + 768..at + 1024],
                    "Cr {name} decode{i}"
                );
                let mut planar = Vec::new();
                picture.write_planar(&mut planar).unwrap();
                let mut expected = Vec::new();
                for (p, &v) in samples[at..at + 1024].iter().enumerate() {
                    if (if p < 512 { yd } else { cd }) == 8 {
                        expected.push(v as u8);
                    } else {
                        expected.extend_from_slice(&v.to_le_bytes());
                    }
                }
                assert_eq!(planar, expected, "planar {name} decode{i}");
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
            for display in 0..3 {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                    .unwrap();
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    display as i128 * 40_000_000,
                    "PTS {name} display{display}"
                );
                assert_eq!(
                    frame.duration.nanoseconds().unwrap(),
                    40_000_000,
                    "duration {name}"
                );
                assert_eq!(
                    frame.sample_index,
                    [0, 2, 1][display],
                    "decode index {name}"
                );
                let packed = frame.packed.unwrap();
                let at = display * 1024;
                assert_eq!(packed.depth, depth);
                assert_eq!(packed.frame.subsampling, Some([2, 1]));
                let mut expected = Vec::new();
                for (p, &v) in samples[at..at + 1024].iter().enumerate() {
                    let v = v << (depth - if p < 512 { yd } else { cd });
                    if depth == 8 {
                        expected.push(v as u8);
                    } else {
                        expected.extend_from_slice(&v.to_le_bytes());
                    }
                }
                assert_eq!(packed.frame.data, expected, "MP4 {name} display{display}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
