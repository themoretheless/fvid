#![cfg(feature = "player")]
use fvid::{
    audio::{AudioStep, AudioStream},
    codec::eac3_decoder::Eac3Decoder,
};
use std::{io::Cursor, path::Path};
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-lfe-downmix")
            .join(n),
    )
    .unwrap()
}
fn native_video(n: &str) -> Vec<u8> {
    let mut s =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(n)), Default::default())
            .unwrap();
    let mut d = s.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        if let AudioStep::Encoded(p) = step {
            if let Some(p) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                    out.extend(p.data);
                }
            }
        }
    }
    out.extend(d.drain_presented(&mut s).into_iter().flat_map(|p| p.data));
    s.validate_eof().unwrap();
    out
}
#[test]
fn all_lfe_codes_mix_only_when_requested_and_preserve_native_pcm() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["lfe_downmix_accepted"], true);
    assert_eq!(m["cases"].as_array().unwrap().len(), 272);
    let controls: Vec<_> = (0..8)
        .into_iter()
        .map(|l| (l, native_video(&format!("l{l}-control.mkv"))))
        .collect();
    for c in m["cases"].as_array().unwrap() {
        let layout = c["layout"].as_u64().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let raw = read(&format!("{}.ec3", c["name"].as_str().unwrap()));
        let codes: Vec<Option<u8>> = if let Some(v) = c["codes"].as_array() {
            v.iter().map(|n| n.as_u64().map(|v| v as u8)).collect()
        } else {
            vec![c["code"].as_u64().map(|v| v as u8); 3]
        };
        let parsed = fvid::codec::eac3_decoder::checked_frame(&raw).unwrap();
        assert_eq!(parsed.downmix.unwrap().lfe, codes[0]);
        let control = &controls.iter().find(|(l, _)| *l == layout).unwrap().1;
        assert_eq!(
            native_video(&format!("{}.mkv", c["name"].as_str().unwrap())),
            *control,
            "native output must retain its separate LFE"
        );
        for output in [1, 2] {
            let mut d = if output == 2 {
                Eac3Decoder::new_stereo(&[], 48_000)
            } else {
                Eac3Decoder::new(&[], 48_000, output)
            }
            .unwrap();
            let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
            let mut expected = Vec::new();
            for (n, p) in control.chunks_exact(channels * 4).enumerate().skip(256) {
                let gain =
                    codes[n / 768].map(|code| 10.0f64.powf((5.5 - f64::from(code)) / 20.0) as f32);
                let p: Vec<f32> = p
                    .chunks_exact(4)
                    .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                    .collect();
                let (mut left, mut right, lfe) = match layout {
                    1 => (p[0] * 0.707, p[0] * 0.707, p[1]),
                    0 | 2 => (p[0], p[1], p[2]),
                    3 => (p[0] + p[2] * 0.707, p[2] * 0.707 + p[1], p[3]),
                    4 => (
                        p[0] + p[3] * (0.707 * 0.7),
                        p[1] + p[3] * (0.707 * 0.7),
                        p[2],
                    ),
                    5 => (
                        p[0] + p[2] * 0.707 + p[4] * (0.707 * 0.7),
                        p[2] * 0.707 + p[1] + p[4] * (0.707 * 0.7),
                        p[3],
                    ),
                    6 => (p[0] + p[3] * 0.707, p[1] + p[4] * 0.707, p[2]),
                    7 => (
                        p[0] + p[2] * 0.707 + p[4] * 0.707,
                        p[2] * 0.707 + p[1] + p[5] * 0.707,
                        p[3],
                    ),
                    _ => unreachable!(),
                };
                if let Some(g) = gain {
                    left += lfe * g;
                    right += lfe * g;
                }
                if output == 1 {
                    let mono = if gain.is_some() {
                        10.0f64.powf(-6.0 / 20.0) as f32
                    } else {
                        0.5
                    };
                    expected.push((left + right) * mono);
                } else {
                    expected.extend([left, right]);
                }
            }
            let actual: Vec<f32> = got
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            assert_eq!(actual.len(), expected.len());
            use fvid::audio::AudioDecode;
            d.reset();
            assert_eq!(
                d.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
                got,
                "reset must preserve the requested output mode"
            );
            let error = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(
                error < 3e-8,
                "{} output {output}: LFE downmix error {error}",
                c["name"]
            );
        }
    }
}
