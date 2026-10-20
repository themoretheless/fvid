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
fn mono_field_p() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-mono-field-p");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 96);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let depth = case["luma_depth"].as_u64().unwrap() as u8;
        let raw = std::fs::read(root.join(format!("{name}.y16"))).unwrap();
        let samples: Vec<u16> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(samples.len(), 2048);
        let mut decoder = AvcDecoder::new(
            &std::fs::read(root.join(format!("{name}.avcc"))).unwrap(),
            1 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for i in 0..4 {
                let frame = decoder
                    .decode_order(&std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
                    .unwrap_or_else(|e| panic!("{name} field{i}: {e}"));
                if i % 2 == 0 {
                    assert!(frame.is_none());
                    assert!(decoder.has_pending_field());
                } else {
                    let frame = frame.unwrap();
                    assert!(!decoder.has_pending_field());
                    assert!(decoder.output_is_field_pair());
                    assert_eq!(frame.chroma_format().unwrap(), 0);
                    assert!(frame.cb.is_empty() && frame.cr.is_empty());
                    let at = i / 2 * 1024;
                    assert_eq!(frame.y, samples[at..at + 1024], "{name} pair{}", i / 2);
                    let mut planar = Vec::new();
                    frame.write_planar(&mut planar).unwrap();
                    assert_eq!(planar, bytes(samples[at..at + 1024].iter().copied(), depth));
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
            for pair in 0..2 {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name} MP4: {e}"))
                    .unwrap();
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    pair as i128 * 40_000_000
                );
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
                assert_eq!(frame.sample_index, pair * 2);
                let packed = frame.packed.unwrap();
                assert_eq!(packed.depth, depth);
                assert_eq!(packed.frame.subsampling, Some([2, 2]));
                let at = pair * 1024;
                let mut expected = bytes(samples[at..at + 1024].iter().copied(), depth);
                expected.extend(bytes(std::iter::repeat_n(1u16 << (depth - 1), 512), depth));
                assert_eq!(packed.frame.data, expected, "{name} packed pair{pair}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
