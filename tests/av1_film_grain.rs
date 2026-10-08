//! Owned AV1 film grain header and native pixel/playback acceptance.
use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
#[test]
fn owned_film_grain_parameter_headers_parse() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("av1-film-grain-generated.json")).unwrap())
            .unwrap();
    let fixtures = manifest["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 6);
    for fixture in fixtures {
        let name = fixture["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let obus: Vec<_> = Obus::new(&bytes).map(Result::unwrap).collect();
        let sequence = Sequence::parse(obus.iter().find(|o| o.kind == 1).unwrap().payload).unwrap();
        assert!(sequence.film_grain);
        let frame = obus.iter().find(|o| o.kind == 6).unwrap();
        let header = Header::parse_intra(&sequence, frame.payload, 0, 0).unwrap();
        let grain = header.grain.as_ref().unwrap();
        assert!(grain.point_counts[0] > 0);
        assert!((8..=11).contains(&grain.scaling_shift));
        assert!((6..=9).contains(&grain.ar_shift));
        assert!(grain.ar_lag <= 3);
        assert!(header.header_bytes < frame.payload.len());
        // Every shortened header must fail rather than silently dropping grain.
        for end in 0..header.header_bytes {
            assert!(
                Header::parse_intra(&sequence, &frame.payload[..end], 0, 0).is_err(),
                "{name}: truncated {end}"
            );
        }
    }
}

#[test]
fn owned_film_grain_pixels_reset_and_webm() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("av1-film-grain-generated.json")).unwrap())
            .unwrap();
    for fixture in manifest["fixtures"].as_array().unwrap() {
        let name = fixture["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let expected = std::fs::read(root.join(fixture["reference"].as_str().unwrap())).unwrap();
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 1);
            assert!(frames[0].show);
            let picture = &frames[0].picture;
            let mut bytes = Vec::new();
            for (p, plane) in picture.planes.iter().enumerate() {
                let divisor = if p == 0 { 1 } else { 2 };
                for y in 0..(picture.size[1] as usize).div_ceil(divisor) {
                    for x in 0..(picture.size[0] as usize).div_ceil(divisor) {
                        let value = plane.samples[y * plane.width + x];
                        if picture.depth == 8 {
                            bytes.push(value as u8);
                        } else {
                            bytes.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
            assert_eq!(bytes.len(), expected.len());
            assert_eq!(
                bytes.iter().zip(&expected).position(|(a, b)| a != b),
                None,
                "{name}: first differing byte"
            );
            decoder.finish().unwrap();
            decoder.reset();
        }
        let container = std::fs::read(root.join(fixture["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(container), 16 << 20)
                .unwrap();
        for _ in 0..2 {
            let raw = reader.read_frame_raw().unwrap().unwrap();
            let bytes = match raw {
                fvid::playback_native::RawFrame::Planar8(p) => {
                    p.y.iter()
                        .chain(&p.cb)
                        .chain(&p.cr)
                        .copied()
                        .collect::<Vec<u8>>()
                }
                fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
                _ => panic!("unexpected raw frame"),
            };
            assert_eq!(bytes, expected, "{name}: WebM pixels");
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
    }
}
