use fvid::codec::av1_decoder::Decoder;
// Owned acceptance: all coded levels and depths match saved independent pixels.
#[test]
fn owned_inter_quant_matrix_pixels_reset_timestamps_and_seek() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-quant-matrix-inter-generated.json")).unwrap(),
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
        let mut references: [Option<fvid::codec::av1_frame::Header>; 8] =
            std::array::from_fn(|_| None);
        let mut kinds = Vec::new();
        for frame in obus.iter().filter(|o| o.kind == 6) {
            let header = fvid::codec::av1_frame::Header::parse(
                &sequence,
                frame.payload,
                frame.temporal_id,
                frame.spatial_id,
                &std::array::from_fn(|i| references[i].as_ref()),
            )
            .unwrap();
            assert_eq!(
                header.quant.matrix,
                Some([record["matrix"].as_u64().unwrap() as u8; 3]),
                "{name}: coded matrix"
            );
            kinds.push(header.frame_type);
            for i in 0..8 {
                if header.refresh_flags & (1 << i) != 0 {
                    references[i] = Some(header.clone());
                }
            }
        }
        assert_eq!(kinds, [0, 1, 1], "{name}: actual inter frames");
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 3);
            let mut pixels = Vec::new();
            for frame in &frames {
                let picture = &frame.picture;
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
            for (i, expected_frame) in expected.chunks_exact(expected.len() / 3).enumerate() {
                assert_eq!(
                    raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
                    expected_frame,
                    "{name}: WebM {i}"
                );
                assert_eq!(
                    reader.frame_interval(),
                    Some((
                        i as u128 * 20_000_000,
                        (i as u128 + 1) * 20_000_000,
                        1_000_000_000
                    ))
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(40_000_000).unwrap(), 0);
        for expected_frame in expected.chunks_exact(expected.len() / 3) {
            assert_eq!(
                raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
                expected_frame,
                "{name}: seek replay"
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
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
