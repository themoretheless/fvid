use fvid::codec::av1_decoder::Decoder;
// Owned native AV1 pixel and playback acceptance; external codecs run only at generation.
#[test]
fn owned_intrabc_tiles_odd_displacements_and_small_blocks() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-intrabc-tools-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 12);
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
        assert!(header.intrabc, "{name}: actual intra block copy permission");
        assert_eq!(
            header.lossless.iter().all(|v| *v),
            record["lossless"].as_bool().unwrap()
        );
        assert_eq!(header.tiles.columns.len(), 3, "{name}: two tiles");
        assert_eq!(sequence.superblock128, record["sb"] == 128);
        let mut decoder = Decoder::new(32 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 1);
            let picture = &frames[0].picture;
            assert!(picture.intrabc_blocks > 0, "{name}: actual copied blocks");
            assert!(
                picture.intrabc_sub8_blocks > 0,
                "{name}: actual sub-8x8 copies"
            );
            assert!(
                picture.intrabc_phases.iter().all(|v| *v > 0),
                "{name}: all chroma phases"
            );
            assert!(
                picture.intrabc_residual_blocks.iter().all(|v| *v > 0),
                "{name}: nonzero Y/U/V copy residuals"
            );
            eprintln!(
                "{name}: intrabc={} residuals={:?} phases={:?} sub8={}",
                picture.intrabc_blocks,
                picture.intrabc_residual_blocks,
                picture.intrabc_phases,
                picture.intrabc_sub8_blocks
            );
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
            assert_eq!(pixels.len(), expected.len(), "{name}: pixel extent");
            assert_eq!(
                pixels.iter().zip(&expected).position(|(a, b)| a != b),
                None,
                "{name}: first differing byte"
            );
            decoder.finish().unwrap();
            decoder.reset();
        }
        let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
        let webm = std::fs::read(root.join(record["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(webm), 32 << 20)
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
