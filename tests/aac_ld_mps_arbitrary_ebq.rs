const PROGRAMMES: usize = 18;
const CORPUS: &str = "aac-ld-mps-arbitrary-ebq";
include!("support/aac_ld_mps_arbitrary_playback.rs");

#[test]
fn ebq_with_active_tree_has_complete_syntax_and_specific_playback_refusal() {
    use fvid_media::owned_aac::{aac_ld_mps_frame, aac_ld_mps_prepare, AacDecodeOptions, NativeAacDecoder};
    let m = manifest();
    assert_eq!(m["errors"].as_array().unwrap().len(), 36);
    for c in m["cases"].as_array().unwrap().iter().chain(m["errors"].as_array().unwrap()) {
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        cfg.validate_low_delay_profile().unwrap();
        assert!(cfg.has_arbitrary_tree());
        assert_eq!(usize::from(cfg.quantization_mode), n(&c["quantization"]));
        let mut syntax = aac_ld_mps_frame::Decoder::new(&cfg).unwrap();
        for wire in c["reference"].as_array().unwrap() {
            let bytes = hex(wire);
            let mut bits = BitReader::new(&bytes);
            let frame = syntax.decode(&mut bits).unwrap();
            assert_eq!(bits.remaining(), 0);
            assert!(!frame.extensions[0].parameters.is_empty());
        }
        if cfg.quantization_mode == 0 { continue; }
        let marker = "energy quantization for this tree/TTT/arbitrary-tree configuration is not yet qualified";
        let error = aac_ld_mps_prepare::Decoder::new(&cfg).err().unwrap().to_string();
        assert!(error.contains(marker), "{error}");
        let error = NativeAacDecoder::new_with_options(&hex(&c["asc"]), AacDecodeOptions { output_sample_rate: Some(n(&c["rate"]) as u32), ..Default::default() }).err().unwrap().to_string();
        assert!(error.contains(marker), "{error}");
        for mkv in [false, true] {
            let mut out = Vec::new();
            let error = if mkv {
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(Cursor::new(read(c, "file")), &mut out, None, &Default::default()).unwrap_err().to_string()
            } else {
                fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut out).unwrap_err().to_string()
            };
            assert!(error.contains(marker), "{error}");
            assert!(out.is_empty());
        }
    }
}
