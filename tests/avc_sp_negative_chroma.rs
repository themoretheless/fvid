use fvid::codec::avc_decoder::AvcDecoder;
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn signed_chroma_prediction_uses_arithmetic_shift_in_primary_and_secondary_sp() {
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/avc-sp-negative-chroma.json"
    ))
    .unwrap();
    for c in m["cases"].as_array().unwrap() {
        let expected = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(c["reference"].as_str().unwrap()),
        )
        .unwrap();
        let mut decoder =
            AvcDecoder::new(&hex(c["configuration"].as_str().unwrap()), 1 << 20).unwrap();
        for _ in 0..2 {
            let mut actual = vec![];
            for packet in c["packets"].as_array().unwrap() {
                decoder
                    .decode(&hex(packet.as_str().unwrap()))
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut actual)
                    .unwrap();
            }
            assert_eq!(actual.len(), 768);
            assert_eq!(
                &actual[..640],
                &expected[..640],
                "I frame and SP luma"
            );
            let difference = actual.iter().zip(&expected).position(|(a, b)| a != b);
            assert!(
                difference.is_none(),
                "{} first differing YUV byte {:?}",
                c["file"],
                difference
            );
            decoder.reset();
        }
    }
}
