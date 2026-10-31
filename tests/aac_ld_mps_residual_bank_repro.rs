use fvid_media::owned_aac::{
    aac_ld_mps_config::{Config, ExtensionValue},
    aac_ld_mps_residual,
    bits::BitReader,
};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-residual-bank-repro")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn hex(v: &Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn owned_cores_and_paired_ordinary_movies_accept_the_intended_bank_geometries() {
    let m = manifest();
    assert_eq!(m["controls"].as_array().unwrap().len(), 12);
    for c in m["controls"].as_array().unwrap() {
        let mut pcm = Vec::new();
        let s = fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut pcm,
        )
        .unwrap();
        assert_eq!(s.sample_rate as usize, n(&c["rate"]));
        assert_eq!(s.channels as usize, n(&c["channels"]));
        let gold = std::fs::read(root().join(c["gold"].as_str().unwrap())).unwrap();
        assert_eq!(pcm.len(), gold.len());
        for (a, b) in pcm.chunks_exact(4).zip(gold.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
    }
    assert_eq!(m["cases"].as_array().unwrap().len(), 44);
    for c in m["cases"].as_array().unwrap() {
        let mut pcm = Vec::new();
        let s = fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap(),
            &mut pcm,
        )
        .unwrap_or_else(|e| panic!("{} ordinary control: {e}", c["name"]));
        assert_eq!(s.sample_frames as usize, n(&c["samples"]));
        assert_eq!(s.channels as usize, n(&c["output_channels"]));
        assert_eq!(s.sample_rate as usize, n(&c["rate"]));
        assert!(pcm.iter().any(|&v| v != 0));
    }
}
#[test]
fn residual_syntax_parses_but_native_bank_admission_is_still_a_specific_refusal() {
    let m = manifest();
    let mut blocks = 0;
    for c in m["cases"].as_array().unwrap() {
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        let ext = &cfg
            .extensions
            .iter()
            .find(|x| matches!(x.value, ExtensionValue::Residual { .. }))
            .unwrap()
            .value;
        for wire in c["reference"].as_array().unwrap() {
            let wire = hex(wire);
            let frame = aac_ld_mps_residual::read(&mut BitReader::new(&wire), &cfg, ext, 2)
                .unwrap_or_else(|e| panic!("{} residual syntax: {e}", c["name"]));
            for b in frame.boxes.iter().flatten() {
                assert_eq!(b.blocks.len(), 1);
                assert_eq!(b.blocks[0].first.coefficients.len(), 1024);
                blocks += 1;
            }
        }
        let mut pcm = Vec::new();
        let e = fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut pcm,
        )
        .unwrap_err();
        assert!(
            e.to_string()
                .contains("requires the qualified full-complex64-bin QMF domain"),
            "{} wrong failure: {e}",
            c["name"]
        );
        assert!(pcm.is_empty());
    }
    assert!(blocks > 1000);
    eprintln!("residual32/128 parsed spectral blocks: {blocks};44 precise native bank refusals");
}
#[test]
#[ignore = "requires independently qualified residual32/128 bank/phase mapping; refusal coverage is not acceptance"]
fn residual32_and128_movies_decode_complete_pcm() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut pcm = Vec::new();
        let s = fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut pcm,
        )
        .unwrap_or_else(|e| panic!("{} residual playback: {e}", c["name"]));
        assert_eq!(s.sample_frames as usize, n(&c["samples"]));
        assert_eq!(s.channels as usize, n(&c["output_channels"]));
        assert_eq!(pcm.len(), n(&c["samples"]) * n(&c["output_channels"]) * 4);
    }
}
