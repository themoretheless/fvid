use fvid_media::owned_aac::{aac_ld_mps_stp::Decoder, aac_sbr_qmf::Complex};
use serde_json::Value;
const EXPECTED_SCALARS: usize = 1_384_448;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-mps128-stp")
}
fn manifest(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
include!("support/aac_ld_mps_stp_component_suite.rs");
