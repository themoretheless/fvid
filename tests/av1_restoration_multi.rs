use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
// Owned acceptance for native restoration pixels and playback lifecycle.
#[test]
fn owned_multiunit_tiled_restoration_matches_independent_pixels() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-restoration-multi-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 18);
    let mut active = 0;
    let mut inactive = 0;
    for record in records {
        let name = record["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let obus: Vec<_> = Obus::new(&data).map(Result::unwrap).collect();
        let sequence = Sequence::parse(obus.iter().find(|o| o.kind == 1).unwrap().payload).unwrap();
        assert!(sequence.restoration, "{name}: sequence tool enabled");
        let frame = obus.iter().find(|o| o.kind == 6).unwrap();
        let header = Header::parse_intra(
            &sequence,
            frame.payload,
            frame.temporal_id,
            frame.spatial_id,
        )
        .unwrap();
        eprintln!(
            "{name}: restoration {:?} units {:?}",
            header.restoration_types, header.restoration_sizes
        );
        assert!(sequence.superblock128, "{name}: 128x128 superblocks");
        assert_eq!(header.tiles.count(), 2, "{name}: two entropy tiles");
        assert_eq!(header.size, [384, 640], "{name}: multiunit frame extent");
        assert!(header.quant.matrix.is_none(), "{name}: isolate restoration");
        if header.restoration_types == [0; 3] {
            inactive += 1;
        } else {
            active += 1;
            let unit_size = if record["depth"].as_u64() == Some(8)
                && record["quality"].as_u64() == Some(48)
                && record["orientation"].as_u64() == Some(1)
            {
                128
            } else {
                256
            };
            assert_eq!(
                header.restoration_sizes, [unit_size; 3],
                "{name}: actual multiunit size"
            );
        }
        let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 1);
            let picture = &frames[0].picture;
            let mut pixels = Vec::new();
            for (i, plane) in picture.planes.iter().enumerate() {
                let divisor = if i == 0 { 1 } else { 2 };
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
            assert_eq!(pixels, expected, "{name}: restored independent pixels");
            decoder.finish().unwrap();
            decoder.reset();
        }
        let webm = std::fs::read(root.join(record["webm"].as_str().unwrap())).unwrap();
        if header.restoration_types != [0; 3] {
            let mut constrained = fvid::playback_webm::WebmVideoReader::open(
                std::io::Cursor::new(webm.clone()),
                16 << 20,
            )
            .unwrap();
            let error = match constrained.read_frame_raw() {
                Err(e) => e,
                Ok(_) => panic!("{name}: constrained budget must account for restoration buffers"),
            };
            assert!(
                error
                    .to_string()
                    .contains("AV1 image exceeds memory budget"),
                "{name}: {error}"
            );
        }
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(webm), 32 << 20)
                .unwrap();
        for _ in 0..2 {
            assert_eq!(
                raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
                expected,
                "{name}: restored WebM pixels"
            );
            assert_eq!(
                reader.frame_interval(),
                Some((0, 20_000_000, 1_000_000_000))
            );
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
        assert_eq!(
            raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
            expected,
            "{name}: seek pixels"
        );
    }
    eprintln!("active {active}, inactive {inactive}");
    assert_eq!((active, inactive), (17, 1));
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
