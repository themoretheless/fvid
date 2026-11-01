const PROGRAMMES: usize = 48;
const CORPUS: &str = "aac-ld-mps-arbitrary-reindex";
include!("support/aac_ld_mps_arbitrary_playback.rs");

#[test]
fn moved_extension_preserves_tree_and_has_complete_valid_frame_syntax() {
    use fvid_media::owned_aac::{aac_ld_mps_config::ExtensionValue, aac_ld_mps_frame};
    for c in manifest()["cases"].as_array().unwrap() {
        let original = Config::parse(&hex(&c["configs"][0])).unwrap();
        for (i, index) in [0, 1, 0].into_iter().enumerate() {
            let cfg = Config::parse(&hex(&c["configs"][i])).unwrap();
            cfg.validate_low_delay_profile().unwrap();
            assert_eq!(cfg.tree, original.tree);
            assert_eq!(cfg.output_channels(), original.output_channels());
            assert!(matches!(cfg.extensions[index].value, ExtensionValue::ArbitraryTree { .. }));
            assert_eq!(cfg.extensions[index].value, original.extensions[0].value);
            let wire = hex(&c["reference"][i]);
            let mut bits = BitReader::new(&wire);
            let frame = aac_ld_mps_frame::Decoder::new(&cfg).unwrap().decode(&mut bits).unwrap();
            assert_eq!(bits.remaining(), 0);
            assert!(!frame.extensions[index].parameters.is_empty());
        }
    }
}
