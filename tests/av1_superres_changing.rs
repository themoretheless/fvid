use fvid::codec::av1_decoder::Decoder;
// Owned acceptance: changing coded width across reference frames.
#[test]
fn owned_changing_superres_pixels_reset_timestamps_and_seek() {
    check();
}
fn check() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-superres-changing-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 18);
    let mut active_inter = 0;
    let mut accepted = 0;
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
        let mut coded_widths = Vec::new();
        for (index, frame) in obus.iter().filter(|o| o.kind == 6).enumerate() {
            let header = fvid::codec::av1_frame::Header::parse(
                &sequence,
                frame.payload,
                frame.temporal_id,
                frame.spatial_id,
                &std::array::from_fn(|i| references[i].as_ref()),
            )
            .unwrap();
            assert_eq!(
                header.superres_denom,
                record[if index == 0 {
                    "key_denominator"
                } else {
                    "denominator"
                }]
                .as_u64()
                .unwrap() as u8,
                "{name}: actual super-resolution"
            );
            assert_eq!(header.upscaled_width, 192);
            assert!(header.quant.matrix.is_none(), "{name}: isolate restoration");
            eprintln!(
                "{name}: type {} restoration {:?}",
                header.frame_type, header.restoration_types
            );
            if header.frame_type == 1 && header.restoration_types != [0; 3] {
                active_inter += 1;
            }
            kinds.push(header.frame_type);
            coded_widths.push(header.size[0]);
            for i in 0..8 {
                if header.refresh_flags & (1 << i) != 0 {
                    references[i] = Some(header.clone());
                }
            }
        }
        assert_eq!(kinds, [0, 1, 1], "{name}: actual inter frames");
        assert_ne!(
            coded_widths[0], coded_widths[1],
            "{name}: coded width changes"
        );
        assert_eq!(coded_widths[1], coded_widths[2]);
        accepted += 1;
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 3);
            assert_ne!(
                frames[0].picture.segment_grid, frames[1].picture.segment_grid,
                "{name}: reference grid changes independently of display extent"
            );
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
            let mismatch = pixels.iter().zip(&expected).position(|(a, b)| a != b);
            assert_eq!(pixels.len(), expected.len(), "{name}: pixel extent");
            assert_eq!(mismatch, None, "{name}: first differing byte");
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
    assert_eq!(accepted, 18);
    assert!(active_inter > 0, "actual active inter restoration frames");
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
