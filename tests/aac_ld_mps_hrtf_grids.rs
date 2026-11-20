use fvid_media::owned_aac::{aac_ld_mps_config::Config, aac_ld_mps_frame, bits::BitReader};
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-hrtf-grids")
}

fn hex(value: &Value) -> Vec<u8> {
    value
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn unequal_grid_movies_reproduce_mapping_refusal_after_valid_spatial_syntax() {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let mut refusals = 0;
    let mut controls = 0;
    for c in manifest["cases"].as_array().unwrap() {
        let config = Config::parse(&hex(&c["ssc"])).unwrap();
        config.validate_low_delay_profile().unwrap();
        assert_eq!(
            u64::from(config.parameter_bands),
            c["spatial_bands"].as_u64().unwrap()
        );
        assert_eq!(
            u64::from(config.hrtf.as_ref().unwrap().bands),
            c["hrtf_bands"].as_u64().unwrap()
        );
        let spatial = hex(&c["spatial_frame"]);
        let mut syntax = aac_ld_mps_frame::Decoder::new(&config).unwrap();
        let mut bits = BitReader::new(&spatial);
        let frame = syntax.decode(&mut bits).unwrap();
        assert_eq!(frame.slots.len(), 2);
        assert!(bits.remaining() < 8);
        let video = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut output = Vec::new();
        let decoded = fvid::native_media::decode_mp4_aac_pcm(&video, &mut output);
        if c["acceptance"] == true {
            controls += 1;
            let stats = decoded.unwrap();
            assert_eq!(stats.sample_frames, c["samples"].as_u64().unwrap());
            assert_eq!(u64::from(stats.sample_rate), c["rate"].as_u64().unwrap());
            assert_eq!(output.len() as u64, c["samples"].as_u64().unwrap() * 6 * 4);
            let parent = root().parent().unwrap().join("aac-ld-mps-3d");
            let original = std::fs::read(parent.join(c["source_video"].as_str().unwrap())).unwrap();
            let mut complete = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm(&original, &mut complete).unwrap();
            assert_eq!(output, complete[..output.len()]);
        } else {
            refusals += 1;
            assert_ne!(config.parameter_bands, config.hrtf.as_ref().unwrap().bands);
            let error = decoded.unwrap_err().to_string();
            assert!(
                error.contains(c["error"].as_str().unwrap()),
                "{}: {error}",
                c["name"]
            );
            // This is a passing refusal reproducer, not playback acceptance.
            assert!(output.is_empty());
        }
    }
    assert_eq!((controls, refusals), (3, 18));
}
