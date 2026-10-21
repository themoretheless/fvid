#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::Eac3Decoder,
};
use std::{io::Cursor, path::Path};
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-dynrng")
            .join(n),
    )
    .unwrap()
}
fn pcm(raw: &[u8]) -> Vec<u8> {
    Eac3Decoder::new(&[], 48000, 2)
        .unwrap()
        .decode_complete(raw, 0, 0)
        .unwrap()
        .unwrap()
        .data
}
fn verify(got: &[u8], control: &[u8], c: &serde_json::Value) {
    assert_eq!(got.len(), control.len());
    let gain = c["gain"].as_f64().unwrap() as f32;
    let mut peak = 0.0f32;
    for (g, e) in got.chunks_exact(4).zip(control.chunks_exact(4)) {
        let g = f32::from_le_bytes(g.try_into().unwrap());
        let e = f32::from_le_bytes(e.try_into().unwrap()) * gain;
        assert!(e.abs() < 1.0, "oracle must not need clipping");
        peak = peak.max(g.abs());
        assert_eq!(
            g, e,
            "{} signed exponent/mantissa or block reuse",
            c["name"]
        );
    }
    assert!(peak > 1e-6);
}
fn video(name: &str) -> Vec<u8> {
    let mut s = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(read(name)),
        Default::default(),
    )
    .unwrap();
    let mut d = s.make_decoder().unwrap();
    let mut out = vec![];
    while let Some(step) = s.next_step().unwrap() {
        if let AudioStep::Encoded(p) = step {
            if let Some(p) = d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .and_then(|d| s.present_decoded(d.packet, d.source_pts).unwrap())
            {
                out.extend(p.data);
            }
        }
    }
    out.extend(d.drain_presented(&mut s).into_iter().flat_map(|p| p.data));
    s.validate_eof().unwrap();
    out
}
#[test]
fn all_256_dynrng_codes_obey_signed_exponents_and_reuse() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 256);
    let raw_control = pcm(&read("control.ec3"));
    let video_control = video("control.mkv");
    for c in cases {
        verify(&pcm(&read(c["file"].as_str().unwrap())), &raw_control, c);
        verify(&video(c["video"].as_str().unwrap()), &video_control, c);
    }
}
