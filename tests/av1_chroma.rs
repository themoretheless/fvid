//! Current explicit reconstruction refusal for owned 4:2:2/4:4:4 streams.
use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
#[test]
fn owned_422_444_headers_and_specific_reconstruction_refusal() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("av1-chroma-generated.json")).unwrap())
            .unwrap();
    let records = m["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 12);
    for r in records {
        let name = r["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let obus: Vec<_> = Obus::new(&bytes).map(Result::unwrap).collect();
        let s = Sequence::parse(obus.iter().find(|o| o.kind == 1).unwrap().payload).unwrap();
        let layout = r["layout"].as_str().unwrap();
        let sub = if layout == "422" {
            [true, false]
        } else {
            [false, false]
        };
        assert_eq!(s.color.subsampling, sub, "{name}: actual layout");
        assert_eq!(s.color.depth, r["depth"].as_u64().unwrap() as u8);
        let h = Header::parse_intra(&s, obus.iter().find(|o| o.kind == 6).unwrap().payload, 0, 0)
            .unwrap();
        assert_eq!(h.size, [64, 48]);
        assert_eq!(
            h.lossless.iter().all(|v| *v),
            r["lossless"].as_bool().unwrap()
        );
        let factor = if layout == "422" { 2 } else { 3 };
        let word = if s.color.depth == 8 { 1 } else { 2 };
        assert_eq!(
            std::fs::read(root.join(r["reference"].as_str().unwrap()))
                .unwrap()
                .len(),
            64 * 48 * factor * word * 2
        );
        let error = Decoder::new(16 << 20)
            .decode_packet(&bytes)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("AV1 native reconstruction requires 4:2:0"),
            "{name}: {error}"
        );
    }
}
