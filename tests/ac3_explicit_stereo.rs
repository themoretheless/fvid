#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::AudioDecode,
    codec::ac3_decoder::{Ac3Decoder, StereoDownmixMode as Mode, StereoDownmixPolicy as Policy},
};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/ac3-explicit-stereo")
}
fn floats(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
#[test]
fn explicit_stereo_folds_centre_instead_of_passing_c_lfe_as_lr() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let rate = c["rate"].as_u64().unwrap() as u32;
        let name = c["name"].as_str().unwrap();
        let raw = std::fs::read(root().join(format!("{name}.ac3"))).unwrap();
        let mut native = Ac3Decoder::new(&[], rate, 2).unwrap();
        let n = floats(&native.decode_complete(&raw, 0, 4608).unwrap().unwrap().data);
        assert_eq!(n.len(), 4352 * 2);
        assert!(n.chunks_exact(2).any(|p| (p[0] - p[1]).abs() > 1e-5));
        for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
            for headroom in [false, true] {
                let policy = Policy { mode, headroom };
                let mut d = Ac3Decoder::new_stereo(&[], rate)
                    .unwrap()
                    .with_stereo_downmix(policy);
                let first = d.decode_complete(&raw, 0, 4608).unwrap().unwrap().data;
                let actual = floats(&first);
                assert_eq!(actual.len(), n.len());
                for (a, b) in actual.chunks_exact(2).zip(n.chunks_exact(2)) {
                    let expected = b[0] * 0.707;
                    assert!(
                        (a[0] - expected).abs() < 1e-7 && (a[1] - expected).abs() < 1e-7,
                        "{name} {mode:?} {headroom}"
                    );
                }
                d.reset();
                assert_eq!(
                    d.decode_complete(&raw, 0, 4608).unwrap().unwrap().data,
                    first
                );
                let mut container = Ac3Decoder::for_container_stereo(&[], rate)
                    .unwrap()
                    .with_stereo_downmix(policy);
                let full = container
                    .decode_complete(&raw, 0, 4608)
                    .unwrap()
                    .unwrap()
                    .data;
                assert_eq!(full.len(), 4608 * 8);
                assert_eq!(&full[256 * 8..], &first);
                container.reset();
                assert_eq!(
                    container
                        .decode_complete(&raw, 0, 4608)
                        .unwrap()
                        .unwrap()
                        .data,
                    full
                );
            }
        }
    }
}
