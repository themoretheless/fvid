//! Parameter syntax acceptance and explicit synthesis refusal are separate checks.
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
fn owned_film_grain_playback_refuses_pending_synthesis() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("av1-film-grain-generated.json")).unwrap())
            .unwrap();
    for fixture in manifest["fixtures"].as_array().unwrap() {
        let name = fixture["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let error = decoder.decode_packet(&bytes).err().unwrap().to_string();
            assert!(
                error.contains("AV1 film grain synthesis not implemented"),
                "{name}: {error}"
            );
            decoder.reset();
        }
    }
}
