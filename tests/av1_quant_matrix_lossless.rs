use fvid::codec::av1_decoder::Decoder;
// Owned acceptance: all coded levels and depths match saved independent pixels.
#[test]
fn owned_lossless_quant_matrix_bypass_matches_source_and_oracles() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-quant-matrix-lossless-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 9);
    for record in records {
        let name = record["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let obus: Vec<_> = fvid::codec::av1::Obus::new(&data)
            .map(Result::unwrap)
            .collect();
        let sequence = fvid::codec::av1_sequence::Sequence::parse(
            obus.iter().find(|o| o.kind == 1).unwrap().payload,
        )
        .unwrap();
        assert!(
            sequence.color.separate_uv_delta_q,
            "{name}: independently coded U/V"
        );
        let frame = obus.iter().find(|o| o.kind == 6).unwrap();
        let header = fvid::codec::av1_frame::Header::parse_intra(
            &sequence,
            frame.payload,
            frame.temporal_id,
            frame.spatial_id,
        )
        .unwrap();
        assert_eq!(
            header.quant.matrix,
            Some(std::array::from_fn(
                |i| record["matrix"][i].as_u64().unwrap() as u8
            )),
            "{name}: coded matrix"
        );
        assert!(
            header.lossless.iter().all(|&v| v),
            "{name}: actual lossless segments"
        );
        assert_eq!(header.quant.base, 0);
        let mut source = Vec::new();
        for plane in 0..3 {
            let width = if plane == 0 { 64 } else { 32 };
            for y in 0..width {
                for x in 0..width {
                    let value =
                        (((x * 7 + y * 3 + plane * 59) % 256) as u16) << (sequence.color.depth - 8);
                    if sequence.color.depth == 8 {
                        source.push(value as u8);
                    } else {
                        source.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        assert_eq!(
            std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap(),
            source,
            "{name}: owned source is exactly lossless"
        );
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 1);
            let picture = &frames[0].picture;
            let mut pixels = Vec::new();
            for (index, plane) in picture.planes.iter().enumerate() {
                let divisor = if index == 0 { 1 } else { 2 };
                for y in 0..(picture.size[1] as usize).div_ceil(divisor) {
                    for x in 0..(picture.size[0] as usize).div_ceil(divisor) {
                        let value = plane.samples[y * plane.width + x];
                        if picture.depth == 8 {
                            pixels.push(value as u8);
                        } else {
                            pixels.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
            let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
            assert_eq!(pixels, expected, "{name}");
            decoder.finish().unwrap();
            decoder.reset();
        }
        let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
        let webm = std::fs::read(root.join(record["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(webm), 16 << 20)
                .unwrap();
        for _ in 0..2 {
            assert_eq!(
                raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
                expected,
                "{name}: WebM replay"
            );
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
        assert_eq!(
            raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
            expected,
            "{name}: seek"
        );
    }
}

fn raw_bytes(frame: fvid::playback_native::RawFrame) -> Vec<u8> {
    match frame {
        fvid::playback_native::RawFrame::Planar8(p) => {
            p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect()
        }
        fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
        _ => panic!("unexpected AV1 raw representation"),
    }
}
