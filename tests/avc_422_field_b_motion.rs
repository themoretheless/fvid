use fvid::codec::avc_decoder::AvcDecoder;
fn bytes(values: impl IntoIterator<Item = u16>, depth: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for v in values {
        if depth == 8 {
            out.push(v as u8)
        } else {
            out.extend_from_slice(&v.to_le_bytes())
        }
    }
    out
}
#[test]
fn field_422_b_motion() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-422-field-b-motion");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 432);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let depth = yd.max(cd);
        let raw = std::fs::read(root.join(format!("{name}.y16"))).unwrap();
        let samples: Vec<u16> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(samples.len(), 6144);
        let expected = |pair: usize, packed: bool| -> Vec<u8> {
            let at = pair * 2048;
            let mut out = Vec::new();
            for (p, range) in [
                (at..at + 1024),
                (at + 1024..at + 1536),
                (at + 1536..at + 2048),
            ]
            .into_iter()
            .enumerate()
            {
                let d = if p == 0 { yd } else { cd };
                out.extend(bytes(
                    samples[range]
                        .iter()
                        .map(|&v| if packed { v << (depth - d) } else { v }),
                    if packed { depth } else { d },
                ));
            }
            out
        };
        let mut decoder = AvcDecoder::new(
            &std::fs::read(root.join(format!("{name}.avcc"))).unwrap(),
            1 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for i in 0..6 {
                let frame = decoder
                    .decode_order(&std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
                    .unwrap_or_else(|e| panic!("{name} field{i}: {e}"));
                if i % 2 == 0 {
                    assert!(frame.is_none());
                    assert!(decoder.has_pending_field());
                } else {
                    let frame = frame.unwrap();
                    let pair = [0, 2, 1][i / 2];
                    let at = pair * 2048;
                    assert!(!decoder.has_pending_field());
                    assert!(decoder.output_is_field_pair());
                    assert_eq!(frame.chroma_format().unwrap(), 2);
                    assert_eq!(frame.y, samples[at..at + 1024], "Y {name}");
                    assert_eq!(frame.cb, samples[at + 1024..at + 1536], "Cb {name}");
                    assert_eq!(frame.cr, samples[at + 1536..at + 2048], "Cr {name}");
                    let mut planar = Vec::new();
                    frame.write_planar(&mut planar).unwrap();
                    assert_eq!(planar, expected(pair, false), "native {name}");
                }
            }
            decoder.reset();
        }
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(std::fs::read(root.join(format!("{name}.mp4"))).unwrap()),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for pair in 0..3 {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name} MP4: {e}"))
                    .unwrap();
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    pair as i128 * 40_000_000
                );
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
                assert_eq!(frame.sample_index, [0, 4, 2][pair]);
                let packed = frame.packed.unwrap();
                assert_eq!(packed.depth, depth);
                assert_eq!(packed.frame.subsampling, Some([2, 1]));
                assert_eq!(packed.frame.data, expected(pair, true), "packed {name}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
