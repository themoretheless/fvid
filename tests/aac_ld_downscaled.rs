const CORPUS: &str = "aac-ld-downscaled";
const CASES: usize = 310;
const EXPORTS: usize = 310;
include!("support/aac_eld_downscaled.rs");

#[test]
fn ld_nominal_prediction_matches_full_source_oracle_and_mode_guards_stay_strict() {
    use fvid_media::owned_aac::{
        NativeAacDecoder, aac_ld_channel::LdChannel, config::AudioSpecificConfig,
    };
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let mut checked = std::collections::HashSet::new();
    for c in m["cases"].as_array().unwrap() {
        let reference = c["nominal"].as_str().unwrap();
        if !checked.insert(reference.to_owned()) {
            continue;
        }
        let asc = hex(c["asc"].as_str().unwrap());
        let mut full = NativeAacDecoder::new(&asc).unwrap();
        let mut actual = Vec::new();
        for r in c["frames"].as_array().unwrap() {
            let at = r["offset"].as_u64().unwrap() as usize;
            actual.extend(
                full.decode(&blob[at..at + r["bytes"].as_u64().unwrap() as usize])
                    .unwrap(),
            );
        }
        let expected = std::fs::read(root().join(reference)).unwrap();
        assert_eq!(actual.len() * 4, expected.len());
        let error = actual
            .iter()
            .zip(expected.chunks_exact(4))
            .map(|(a, b)| (*a - f32::from_le_bytes(b.try_into().unwrap())).abs())
            .fold(0f32, f32::max);
        assert!(error < 1e-7, "{reference}: {error}");
    }
    assert_eq!(checked.len(), 120);
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["n"] == 512 && c["source_rate"] == 24000)
        .unwrap();
    let asc = hex(c["asc"].as_str().unwrap());
    let parsed = AudioSpecificConfig::parse(&asc).unwrap();
    assert!(parsed.ld_downscale_factor(8000).is_none());
    assert!(NativeAacDecoder::new_with_output_rate(&asc, 8000).is_err());
    assert!(NativeAacDecoder::new_with_output_rate(&asc, 0).is_err());
    assert!(NativeAacDecoder::new_with_output_rate(&asc, 7000).is_err());
    let mut unsupported = parsed.clone();
    unsupported.sbr_present = Some(true);
    assert!(unsupported.ld_downscale_factor(12000).is_none());
    unsupported.sbr_present = Some(false);
    unsupported.ps_present = Some(true);
    assert!(unsupported.ld_downscale_factor(12000).is_none());
    assert!(LdChannel::new_downscaled(512, 3).is_err());
    assert!(LdChannel::new_downscaled(480, 0).is_err());
    let mut first = LdChannel::new_downscaled(480, 2).unwrap();
    let other = LdChannel::new_downscaled(480, 3).unwrap();
    assert!(first.restore(&other.checkpoint()).is_err());
}
