use fvid::codec::av1_decoder::Decoder;
#[test]
fn scalar_multi_escape_clipping_and_adaptation_match_owned_pixels() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-delta-lf-owned-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 64);
    let distinct: std::collections::HashSet<_> = records
        .iter()
        .map(|r| r["reference_sha256"].as_str().unwrap())
        .collect();
    assert!(
        distinct.len() > 1,
        "delta-LF reference cases must change reconstructed pixels"
    );
    for record in records {
        let name = record["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
        let obus: Vec<_> = fvid::codec::av1::Obus::new(&bytes)
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
        assert_eq!(
            header.filter.delta_resolution,
            Some((
                record["resolution"].as_u64().unwrap() as u8,
                record["multi"].as_bool().unwrap()
            )),
            "{name}"
        );
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 2);
            assert!(!frames[0].show);
            assert!(frames[1].show);
            let pixels: Vec<_> = frames[1]
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().map(|v| *v as u8))
                .collect();
            assert_eq!(pixels, expected, "{name}");
            decoder.finish().unwrap();
            decoder.reset();
        }
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
            "{name}: WebM seek"
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
