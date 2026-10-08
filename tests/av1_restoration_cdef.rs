use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
// Owned acceptance for native restoration pixels and playback lifecycle.
#[test]
fn owned_cdef_restoration_stripe_pixels_match_independent_references() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-restoration-cdef-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 18);
    let mut active = 0;
    let mut inactive = 0;
    let mut combined = 0;
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
        assert!(sequence.cdef, "{name}: sequence CDEF enabled");
        eprintln!(
            "{name}: CDEF {:?}, LF {:?}",
            header.cdef.strengths, header.filter.levels
        );
        assert!(header.quant.matrix.is_none(), "{name}: isolate restoration");
        if header.restoration_types == [0; 3] {
            inactive += 1;
        } else {
            active += 1;
            if header.filter.levels.iter().any(|&v| v > 0)
                && header.cdef.strengths.iter().flatten().any(|&v| v > 0)
            {
                combined += 1;
            }
            assert!(header.restoration_sizes[0] > 0);
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
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(webm), 16 << 20)
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
    assert!(
        combined > 0,
        "must exercise active restoration with CDEF and deblocking"
    );
    assert!(active > 0);
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
