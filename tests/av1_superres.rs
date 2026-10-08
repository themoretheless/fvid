use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
#[test]
fn owned_superres_pixels_acceptance() {
    check("av1-superres-generated.json", false);
}
#[test]
fn owned_superres_restoration_pixels_acceptance() {
    check("av1-superres-restoration-generated.json", true);
}
fn check(manifest_name: &str, restoration: bool) {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join(manifest_name)).unwrap()).unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 18);
    let mut restored = 0;
    for record in records {
        let name = record["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let obus: Vec<_> = Obus::new(&data).map(Result::unwrap).collect();
        let seq = Sequence::parse(obus.iter().find(|o| o.kind == 1).unwrap().payload).unwrap();
        assert!(seq.superres);
        let o = obus.iter().find(|o| o.kind == 6).unwrap();
        let h = Header::parse_intra(&seq, o.payload, o.temporal_id, o.spatial_id).unwrap();
        let denom = record["denominator"].as_u64().unwrap() as u8;
        assert_eq!(h.superres_denom, denom);
        assert_eq!(h.upscaled_width, 192);
        assert_eq!(
            h.size,
            [(192 * 8 + u32::from(denom) / 2) / u32::from(denom), 128]
        );
        assert!(!h.intrabc);
        if restoration {
            if h.restoration_types != [0; 3] {
                restored += 1;
            }
        } else {
            assert_eq!(h.restoration_types, [0; 3]);
        }
        if !restoration
            && denom == 16
            && record["depth"].as_u64() == Some(8)
            && record["orientation"].as_u64() == Some(0)
        {
            let error = match Decoder::new(1 << 20).decode_packet(&data) {
                Ok(_) => panic!("{name}: missing upscale memory accounting"),
                Err(e) => e,
            };
            assert_eq!(error.to_string(), "AV1 image exceeds memory budget");
        }
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let result = decoder.decode_packet(&data);
            let frames = result.unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 1);
            let p = &frames[0].picture;
            assert_eq!(p.size, [192, 128]);
            let mut pixels = Vec::new();
            for (i, plane) in p.planes.iter().enumerate() {
                let sub = if i == 0 { 1 } else { 2 };
                for y in 0..128 / sub {
                    for x in 0..192 / sub {
                        let value = plane.samples[y * plane.width + x];
                        if p.depth == 8 {
                            pixels.push(value as u8)
                        } else {
                            pixels.extend(value.to_le_bytes())
                        }
                    }
                }
            }
            let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
            assert_eq!(pixels.len(), expected.len());
            assert_eq!(
                pixels.iter().zip(expected).position(|(a, b)| *a != b),
                None,
                "{name}: pixels"
            );
            decoder.finish().unwrap();
            decoder.reset();
        }
        {
            let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
            let webm = std::fs::read(root.join(record["webm"].as_str().unwrap())).unwrap();
            let mut reader =
                fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(webm), 16 << 20)
                    .unwrap();
            for _ in 0..2 {
                assert_eq!(
                    raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
                    expected,
                    "{name}: WebM pixels"
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
    }
    assert!(!restoration || restored > 0, "actual restoration required");
}

fn raw_bytes(frame: fvid::playback_native::RawFrame) -> Vec<u8> {
    match frame {
        fvid::playback_native::RawFrame::Planar8(p) => {
            p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect()
        }
        fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
        _ => panic!("unexpected AV1 frame"),
    }
}
