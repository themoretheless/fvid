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
fn mbaff_422_b_partitions() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-422-mbaff-b-partitions");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 840);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let fmt = case["format"].as_u64().unwrap() as u8;
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let depth = if fmt == 0 { yd } else { yd.max(cd) };
        let crop: [usize; 4] = std::array::from_fn(|i| case["crop"][i].as_u64().unwrap() as usize);
        let w = 32 - crop[0] - crop[1];
        let h = 64 - crop[2] - crop[3];
        let n = if fmt == 0 { 2048 } else { 4096 };
        let raw = std::fs::read(root.join(format!("{name}.y16"))).unwrap();
        let samples: Vec<u16> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(samples.len(), 4 * n);
        let expected = |frame: usize, packed: bool| -> Vec<u8> {
            let mut out = Vec::new();
            let at = frame * n;
            for p in 0..3 {
                if p > 0 && fmt == 0 {
                    if packed {
                        out.extend(bytes(
                            std::iter::repeat_n(1u16 << (depth - 1), w.div_ceil(2) * h.div_ceil(2)),
                            depth,
                        ));
                    }
                    continue;
                }
                let sx = if p == 0 { 1 } else { 2 };
                let stride = 32 / sx;
                let offset = at + if p == 0 { 0 } else { 2048 + (p - 1) * 1024 };
                let d = if p == 0 { yd } else { cd };
                for y in crop[2]..64 - crop[3] {
                    let start = offset + y * stride + crop[0] / sx;
                    out.extend(bytes(
                        samples[start..start + w / sx]
                            .iter()
                            .map(|&v| if packed { v << (depth - d) } else { v }),
                        if packed { depth } else { d },
                    ));
                }
            }
            out
        };
        let mut decoder = AvcDecoder::new(
            &std::fs::read(root.join(format!("{name}.avcc"))).unwrap(),
            1 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for i in 0..4 {
                let display = [0, 2, 1, 3][i];
                let picture = decoder
                    .decode_order(&std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
                    .unwrap_or_else(|e| panic!("{name} frame{i}: {e}"))
                    .unwrap();
                assert!(!decoder.has_pending_field());
                assert!(!decoder.output_is_field_pair());
                assert_eq!(picture.chroma_format().unwrap(), fmt);
                assert_eq!(picture.crop, crop);
                assert_eq!(picture.dimensions(), (w, h));
                let at = display * n;
                assert_eq!(picture.y, samples[at..at + 2048], "Y {name}");
                if fmt == 0 {
                    assert!(picture.cb.is_empty() && picture.cr.is_empty());
                } else {
                    assert_eq!(picture.cb, samples[at + 2048..at + 3072], "Cb {name}");
                    assert_eq!(picture.cr, samples[at + 3072..at + 4096], "Cr {name}");
                }
                let mut planar = Vec::new();
                picture.write_planar(&mut planar).unwrap();
                assert_eq!(planar, expected(display, false), "native {name}");
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
            for i in 0..4 {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                    .unwrap();
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    i as i128 * 40_000_000
                );
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
                assert_eq!(frame.sample_index, [0, 2, 1, 3][i]);
                let packed = frame.packed.unwrap();
                assert_eq!(packed.depth, depth);
                assert_eq!(
                    packed.frame.subsampling,
                    Some(if fmt == 0 { [2, 2] } else { [2, 1] })
                );
                assert_eq!(packed.frame.data, expected(i, true), "packed {name}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
