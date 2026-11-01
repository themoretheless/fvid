const CORPUS: &str = "aac-ld-mps-arbitrary-permutations";
include!("support/aac_ld_mps_arbitrary_playback.rs");

#[test]
fn changing_headers_keep_topology_and_parse_all_three_complete_frames() {
    use fvid_media::owned_aac::{aac_ld_mps_config::ExtensionValue, aac_ld_mps_frame};
    for c in manifest()["cases"].as_array().unwrap() {
        let initial = Config::parse(&hex(&c["configs"][0])).unwrap();
        assert_eq!(c["positions"].as_array().unwrap().len(), 3);
        for i in 0..3 {
            let cfg = Config::parse(&hex(&c["configs"][i])).unwrap();
            cfg.validate_low_delay_profile().unwrap();
            assert_eq!(cfg.tree, initial.tree);
            assert_eq!(cfg.output_channels(), initial.output_channels());
            let ExtensionValue::ArbitraryTree {
                nodes,
                ott,
                positions,
            } = &cfg.extensions[0].value
            else {
                panic!("missing additional tree")
            };
            let ExtensionValue::ArbitraryTree {
                nodes: old,
                ott: old_ott,
                ..
            } = &initial.extensions[0].value
            else {
                unreachable!()
            };
            assert_eq!(nodes, old);
            assert_eq!(ott, old_ott);
            assert_eq!(
                positions,
                &serde_json::from_value::<Vec<u8>>(c["positions"][i].clone()).unwrap()
            );
            let wire = hex(&c["reference"][i]);
            let mut bits = BitReader::new(&wire);
            aac_ld_mps_frame::Decoder::new(&cfg)
                .unwrap()
                .decode(&mut bits)
                .unwrap();
            assert_eq!(bits.remaining(), 0);
        }
    }
}
