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
fn component_field_intra() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-component-field-intra");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 600);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let format = case["format"].as_u64().unwrap() as u8;
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let crop: [usize; 4] = std::array::from_fn(|i| case["crop"][i].as_u64().unwrap() as usize);
        let sx = if format == 2 { 2 } else { 1 };
        let c = if format == 0 { 0 } else { 2048 / sx };
        let raw = std::fs::read(root.join(format!("{name}.y16"))).unwrap();
        let words: Vec<u16> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(words.len(), 2048 + 2 * c);
        let planes = [&words[..2048], &words[2048..2048 + c], &words[2048 + c..]];
        let w = 32 - crop[0] - crop[1];
        let h = 64 - crop[2] - crop[3];
        let mut visible: [Vec<u16>; 3] = std::array::from_fn(|_| Vec::new());
        for p in 0..3 {
            if p > 0 && format == 0 {
                continue;
            }
            let sub = if p == 0 { 1 } else { sx };
            let stride = 32 / sub;
            for row in crop[2]..64 - crop[3] {
                visible[p].extend_from_slice(
                    &planes[p][row * stride + crop[0] / sub..row * stride + (32 - crop[1]) / sub],
                );
            }
        }
        let expected_native: Vec<u8> = visible
            .iter()
            .enumerate()
            .flat_map(|(p, v)| bytes(v.iter().copied(), if p == 0 { yd } else { cd }))
            .collect();
        let depth = if format == 0 { yd } else { yd.max(cd) };
        let mut expected_packed = Vec::new();
        for p in 0..3 {
            if format == 0 && p > 0 {
                expected_packed.extend(bytes(
                    std::iter::repeat_n(1u16 << (depth - 1), w.div_ceil(2) * h.div_ceil(2)),
                    depth,
                ));
            } else {
                let d = if p == 0 { yd } else { cd };
                expected_packed.extend(bytes(visible[p].iter().map(|&v| v << (depth - d)), depth));
            }
        }
        let mut decoder = AvcDecoder::new(
            &std::fs::read(root.join(format!("{name}.avcc"))).unwrap(),
            1 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            assert!(decoder
                .decode_order(&std::fs::read(root.join(format!("{name}-0.packet"))).unwrap())
                .unwrap_or_else(|e| panic!("{name} first: {e}"))
                .is_none());
            assert!(decoder.has_pending_field());
            let picture = decoder
                .decode_order(&std::fs::read(root.join(format!("{name}-1.packet"))).unwrap())
                .unwrap_or_else(|e| panic!("{name} second: {e}"))
                .unwrap();
            assert!(!decoder.has_pending_field());
            assert!(decoder.output_is_field_pair());
            assert_eq!(picture.chroma_format().unwrap(), format);
            assert_eq!(picture.crop, crop);
            assert_eq!(picture.dimensions(), (w, h));
            for (got, want) in [&picture.y, &picture.cb, &picture.cr]
                .into_iter()
                .zip(planes)
            {
                assert_eq!(got.as_slice(), want, "{name} coded");
            }
            let mut planar = Vec::new();
            picture.write_planar(&mut planar).unwrap();
            assert_eq!(planar, expected_native, "{name} native");
            decoder.reset();
        }
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(std::fs::read(root.join(format!("{name}.mp4"))).unwrap()),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let frame = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name} MP4: {e}"))
                .unwrap();
            assert_eq!(frame.presentation_time.nanoseconds().unwrap(), 0);
            assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
            assert_eq!(frame.sample_index, 0);
            let packed = frame.packed.unwrap();
            assert_eq!(packed.depth, depth);
            assert_eq!(packed.frame.data, expected_packed, "{name} packed");
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
