use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
// Reproduction only. Replace refusal with independent pixel acceptance when fixed.
#[test]
fn owned_active_loop_restoration_reproduces_specific_refusal() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-restoration-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 18);
    let mut active = 0;
    let mut inactive = 0;
    let expected_types = [
        [0, 2, 2],
        [2, 0, 0],
        [2, 2, 2],
        [0, 0, 0],
        [2, 2, 0],
        [0, 0, 0],
        [2, 2, 1],
        [2, 0, 0],
        [2, 2, 2],
        [2, 0, 0],
        [2, 0, 0],
        [0, 0, 0],
        [2, 2, 2],
        [2, 0, 0],
        [2, 0, 0],
        [2, 2, 0],
        [0, 0, 0],
        [0, 0, 2],
    ];
    for (case, record) in records.iter().enumerate() {
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
        assert_eq!(
            header.restoration_types, expected_types[case],
            "{name}: normative restoration remapping"
        );
        assert!(header.quant.matrix.is_none(), "{name}: isolate restoration");
        if header.restoration_types == [0; 3] {
            inactive += 1;
            let mut decoder = Decoder::new(16 << 20);
            let frames = decoder.decode_packet(&data).unwrap();
            assert_eq!(frames.len(), 1);
            let picture = &frames[0].picture;
            let mut pixels = Vec::new();
            for (i, plane) in picture.planes.iter().enumerate() {
                let d = if i == 0 { 1 } else { 2 };
                for y in 0..(picture.size[1] as usize).div_ceil(d) {
                    for x in 0..(picture.size[0] as usize).div_ceil(d) {
                        let value = plane.samples[y * plane.width + x];
                        if picture.depth == 8 {
                            pixels.push(value as u8);
                        } else {
                            pixels.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
            assert_eq!(
                pixels,
                std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap(),
                "{name}: inactive control acceptance"
            );
            continue;
        }
        active += 1;
        assert!(header.restoration_sizes[0] > 0);
        let mut decoder = Decoder::new(16 << 20);
        let error = match decoder.decode_packet(&data) {
            Ok(_) => panic!("{name}: unexpectedly accepted before restoration fix"),
            Err(e) => e,
        };
        assert!(
            error
                .to_string()
                .contains("AV1 loop restoration not implemented"),
            "{name}: {error}"
        );
    }
    eprintln!("active {active}, inactive {inactive}");
    assert_eq!((active, inactive), (14, 4));
}
