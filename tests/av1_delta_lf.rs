use fvid::codec::av1_decoder::Decoder;

// Reproduction only: acceptance must replace this expectation with decoded pixels.
#[test]
fn owned_delta_lf_reproduces_native_refusal() {
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
        let error = match decoder.decode_packet(&data) {
            Err(error) => error,
            Ok(_) => panic!("{name}: expected current delta-LF refusal"),
        };
        assert!(
            error
                .to_string()
                .contains("quantization matrices or in-loop filtering not implemented"),
            "{name}: {error}"
        );
    }
}
