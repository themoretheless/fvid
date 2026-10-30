use super::*;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp4-audio-middle-gap")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn number(c: &serde_json::Value, key: &str) -> u64 {
    c[key].as_u64().unwrap()
}
#[test]
fn rational_gap_destinations_preserve_native_history_and_strict_silence_at_two_rates() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mode"] == "none" && c["stamp"] == "v0")
    {
        let n = c["file"].as_str().unwrap();
        let source = std::fs::read(root().join(n)).unwrap();
        let rate = number(c, "rate") as u32 * 2;
        let channels = number(c, "channels") as u16;
        let mask = crate::owned_pcm_channels::standard_mask(channels).unwrap() as u32;
        let mut out = Vec::new();
        let stats = decode_mp4_audio_pcm_targeted(
            Cursor::new(&source),
            &mut out,
            None,
            &Default::default(),
            Some((rate, channels, mask)),
        )
        .unwrap();
        let expected = std::fs::read(root().join(format!(
            "{}-rational-2x.f32le",
            c["codec"].as_str().unwrap()
        )))
        .unwrap();
        assert_eq!(out.len(), expected.len(), "{n}");
        assert_eq!(stats.sample_frames, number(c, "frames") * 2);
        for (i, (a, b)) in out
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
        {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!((a - b).abs() <= 2e-6, "{n} rational sample{i}: {a} != {b}");
        }
        let block = number(c, "packet_frames") as usize * 2 * usize::from(channels) * 4;
        assert!(
            out[block..2 * block].iter().all(|&b| b == 0),
            "{n}: rational gap is not literal silence"
        );
    }
}
#[test]
fn precise_rational_pcm_keeps_f64_silence_and_refuses_non_pcm() {
    for c in manifest()["cases"].as_array().unwrap().iter().filter(|c| {
        (c["codec"] == "pcm" || c["codec"] == "alac") && c["mode"] == "none" && c["stamp"] == "v0"
    }) {
        let n = c["file"].as_str().unwrap();
        let source = std::fs::read(root().join(n)).unwrap();
        let rate = number(c, "rate") as u32 * 2;
        let channels = number(c, "channels") as u16;
        let mask = crate::owned_pcm_channels::standard_mask(channels).unwrap() as u32;
        let mut out = Vec::new();
        let result = decode_mp4_pcm_f64_targeted(
            Cursor::new(&source),
            &mut out,
            None,
            &Default::default(),
            Some((rate, channels, mask)),
        );
        if c["codec"] == "alac" {
            assert_eq!(
                result.unwrap_err().to_string(),
                "selected MP4 audio stream is not supported PCM"
            );
            assert!(out.is_empty());
            continue;
        }
        let stats = result.unwrap();
        let expected = std::fs::read(root().join(format!(
            "{}-rational-2x.f64le",
            c["codec"].as_str().unwrap()
        )))
        .unwrap();
        assert_eq!(out.len(), expected.len(), "{n}");
        assert_eq!(stats.sample_frames, number(c, "frames") * 2);
        for (i, (a, b)) in out
            .chunks_exact(8)
            .zip(expected.chunks_exact(8))
            .enumerate()
        {
            let a = f64::from_le_bytes(a.try_into().unwrap());
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!(
                (a - b).abs() <= 1e-12,
                "{n} f64 rational sample{i}: {a} != {b}"
            );
        }
        let block = number(c, "packet_frames") as usize * 2 * usize::from(channels) * 8;
        assert!(out[block..2 * block].iter().all(|&b| b == 0));
    }
}
