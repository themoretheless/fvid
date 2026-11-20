const CORPUS: &str = "aac-ld-mps-arbdmx";
const CONTROLS: usize = 5;
const PROGRAMMES: usize = 60;
include!("support/aac_ld_mps_short_pcm.rs");

#[test]
fn mono_gain_matrices_match_original_reference_before_temporal_interpolation() {
    use fvid_media::owned_aac::aac_ld_mps_assemble;
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(root().join("oracle.json")).unwrap()).unwrap();
    let golden = std::fs::read(root().join("expected.bin")).unwrap();
    let mut count = 0;
    let mut maximum = 0.0f64;
    for (case, control) in m()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        assert_eq!(case["name"], control["name"]);
        let cfg = Config::parse(&hex(&case["ssc"])).unwrap();
        assert!(cfg.arbitrary_downmix);
        let mut decoder = aac_ld_mps_assemble::Decoder::new(&cfg).unwrap();
        for (wire, expected) in case["reference"]
            .as_array()
            .unwrap()
            .iter()
            .zip(control["frames"].as_array().unwrap())
        {
            let packet = hex(wire);
            let saved = decoder.clone();
            let mut short = BitReader::new(&packet[..packet.len() - 3]);
            assert!(decoder.decode(&mut short).is_err());
            assert_eq!(short.position(), 0);
            assert_eq!(decoder, saved);
            let actual = decoder.decode(&mut BitReader::new(&packet)).unwrap();
            for (i, matrix) in [&actual.m1, &actual.dry, &actual.wet]
                .into_iter()
                .enumerate()
            {
                let mut at = n(&expected["offsets"][i]);
                assert_eq!(
                    matrix.endpoints.len(),
                    expected["slots"].as_array().unwrap().len()
                );
                for endpoint in &matrix.endpoints {
                    assert_eq!(endpoint.len(), n(&expected["widths"][i]));
                    for &value in endpoint {
                        let want =
                            f64::from(f32::from_le_bytes(golden[at..at + 4].try_into().unwrap()));
                        assert!(value.is_finite());
                        let error = (value - want).abs();
                        assert!(error < 2e-6, "{} plane{i}: {value} != {want}", case["name"]);
                        maximum = maximum.max(error);
                        count += 1;
                        at += 4;
                    }
                }
            }
        }
    }
    assert_eq!(count * 4, golden.len());
    eprintln!(
        "Mono arbitrary-downmix C/Decimal oracle: {count} matrix endpoints;max error {maximum}"
    );
}
