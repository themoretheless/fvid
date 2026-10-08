use fvid::codec::av1_decoder::Decoder;

// Owned acceptance: coded delta-LF must reconstruct saved independent pixels.
#[test]
fn owned_delta_lf_matches_independent_pixels() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("av1-delta-lf-generated.json")).unwrap())
            .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 6);
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
        let frame = obus.iter().find(|o| o.kind == 6).unwrap();
        let header = fvid::codec::av1_frame::Header::parse_intra(
            &sequence,
            frame.payload,
            frame.temporal_id,
            frame.spatial_id,
        )
        .unwrap();
        assert!(
            header.filter.delta_resolution.is_some(),
            "{name}: no coded delta LF"
        );
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 1);
            let picture = &frames[0].picture;
            let mut pixels = Vec::new();
            for (plane_index, plane) in picture.planes.iter().enumerate() {
                let divisor = if plane_index == 0 { 1 } else { 2 };
                let width = (picture.size[0] as usize).div_ceil(divisor);
                let height = (picture.size[1] as usize).div_ceil(divisor);
                for y in 0..height {
                    for x in 0..width {
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
            let frame = reader.read_frame_raw().unwrap().unwrap();
            let pixels = match frame {
                fvid::playback_native::RawFrame::Planar8(p) => {
                    p.y.iter()
                        .chain(&p.cb)
                        .chain(&p.cr)
                        .copied()
                        .collect::<Vec<_>>()
                }
                fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
                _ => panic!("unexpected AV1 representation"),
            };
            assert_eq!(pixels, expected, "{name}: WebM replay");
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
        assert!(reader.read_frame_raw().unwrap().is_some());
    }
}
