#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/eac3-reduced-allocation")
}
fn decode(name: &str, rate: u32) -> Vec<f32> {
    let data = std::fs::read(root().join(name)).unwrap();
    let mut decoder = fvid::codec::eac3_decoder::Eac3Decoder::new(&[], rate, 1).unwrap();
    let pcm = decoder.decode_complete(&data, 0, 0).unwrap().unwrap();
    assert_eq!(pcm.timebase_den, rate);
    pcm.data
        .chunks_exact(4)
        .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
        .collect()
}
#[test]
fn reduced_rate_quiet_allocation_matches_independent_high_precision_pcm_control() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let expected = decode(&format!("{name}-control.ec3"), rate * 2);
        let actual = decode(&format!("{name}.ec3"), rate);
        assert_eq!(actual.len(), c["frames"].as_u64().unwrap() as usize);
        assert_eq!(actual.len(), expected.len());
        let energy: f64 = expected.iter().map(|x| f64::from(*x).powi(2)).sum();
        assert!(energy > 1e-18, "{name}: silent control");
        let peak = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| f64::from((a - b).abs()))
            .fold(0., f64::max);
        let bound = c["error_bound"].as_f64().unwrap() + 1e-11;
        assert!(
            peak <= bound,
            "{name}: PCM error {peak} exceeds independently authored quantization bound {bound}"
        );
    }
}
#[test]
fn authored_controls_have_valid_headers_and_container_video() {
    for rate in [24000, 22050, 16000] {
        let data = std::fs::read(root().join(format!("quiet-{rate}.ec3"))).unwrap();
        let h = fvid::codec::eac3_decoder::checked_frame(&data).unwrap();
        assert_eq!((h.sample_rate, h.blocks, h.channels), (rate, 6, 1));
        fvid::container::webm::WebmReader::open(
            Cursor::new(std::fs::read(root().join(format!("quiet-{rate}.mkv"))).unwrap()),
            Default::default(),
        )
        .unwrap();
    }
}

#[test]
fn reduced_rate_matroska_pcm_and_rewind_preserve_the_elementary_signal() {
    for rate in [24000, 22050, 16000] {
        let expected = decode(&format!("quiet-{rate}.ec3"), rate);
        let bytes = std::fs::read(root().join(format!("quiet-{rate}.mkv"))).unwrap();
        let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        assert_eq!((stream.sample_rate(), stream.channels()), (rate, 1));
        for _ in 0..2 {
            let frames = transitions::collect(&mut stream);
            let pcm: Vec<f32> = frames
                .iter()
                .flat_map(|f| {
                    f.3.chunks_exact(4)
                        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                })
                .collect();
            assert_eq!(pcm.len(), 4608);
            assert_eq!(&pcm[256..], expected.as_slice());
            stream.rewind();
        }
        let mut video = fvid::playback_webm::WebmVideoReader::open_software(
            Cursor::new(bytes),
            16 * 1024 * 1024,
        )
        .unwrap();
        let mut count = 0;
        while video.read_frame().unwrap() {
            count += 1;
        }
        assert!(count > 0);
    }
}
