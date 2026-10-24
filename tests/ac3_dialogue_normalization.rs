#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod support;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::ac3_decoder::{Ac3Decoder, CompressionMode},
};
use std::{io::Cursor, path::Path, time::Duration};
use support::DecodeComplete;
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/ac3-dialogue-normalization")
            .join(n),
    )
    .unwrap()
}
fn open(n: &str) -> Box<dyn AudioStream> {
    if n.ends_with("mp4") {
        Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(read(n)),
                Default::default(),
            )
            .unwrap(),
        )
    } else {
        Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(read(n)),
                Default::default(),
            )
            .unwrap(),
        )
    }
}
fn decode(
    s: &mut dyn AudioStream,
    enabled: bool,
    mode: CompressionMode,
    channels: u16,
) -> Vec<(Duration, Vec<f32>)> {
    let mut d =
        Ac3Decoder::for_container_with_compression(s.extra_data(), s.sample_rate(), channels, mode)
            .unwrap()
            .with_dialogue_normalization(enabled);
    let mut out: Vec<(Duration, Vec<f32>)> = vec![];
    let mut encoded = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        if let AudioStep::Encoded(p) = step {
            encoded.extend_from_slice(&p.data);
            if let Some(p) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                    out.push((
                        p.presentation_time(),
                        p.data
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                            .collect(),
                    ));
                }
            }
        }
    }
    assert!(d.drain_presented(s).is_empty());
    s.validate_eof().unwrap();
    d.reset();
    let replay = d.decode_complete(&encoded, 0, 0).unwrap().unwrap().data;
    let expected: Vec<u8> = out
        .iter()
        .flat_map(|(_, pcm)| pcm.iter().flat_map(|v| v.to_le_bytes()))
        .collect();
    assert_eq!(
        replay, expected,
        "reset must preserve normalization and compression settings"
    );
    out
}
fn gain(c: u64) -> f32 {
    10f64.powf((if c == 0 { 31 } else { c } as f64 - 31.) / 20.) as f32
}
#[test]
fn dialogue_levels_apply_per_programme_before_folding_and_survive_container_navigation() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(m["accepted"], true);
    assert_eq!(cases.len(), 216);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize;
        let dual = c["layout"] == 0;
        let reference = decode(
            &mut *open(c["control"].as_str().unwrap()),
            false,
            CompressionMode::Line,
            width as u16,
        );
        assert_eq!(reference.len(), 3);
        assert!(reference
            .iter()
            .flat_map(|(_, p)| p)
            .any(|v| v.abs() > 1e-7));
        for ext in if c["mp4"] == true {
            vec!["mkv", "mp4"]
        } else {
            vec!["mkv"]
        } {
            for mode in [CompressionMode::Line, CompressionMode::Rf] {
                let mut reference = reference.clone();
                if c["loud"] == true {
                    for (_, pcm) in &mut reference {
                        for (i, value) in pcm.iter_mut().enumerate() {
                            let second = dual && i % width == 1;
                            let block = i / width / 256;
                            let line = if second {
                                [0.25, 2., 2., 4., 4., 0.5][block]
                            } else {
                                [4., 4., 0.5, 0.5, 2., 2.][block]
                            };
                            *value *= 1024.0
                                * if mode == CompressionMode::Rf {
                                    248.0
                                } else {
                                    line
                                };
                        }
                    }
                    if mode == CompressionMode::Rf {
                        assert!(reference.iter().flat_map(|(_, p)| p).any(|v| v.abs() > 1.));
                    }
                }
                let filename = format!("{name}.{ext}");
                let mut stream = open(&filename);
                let plain = decode(&mut *stream, false, mode, width as u16);
                for ((p, a), (q, b)) in plain.iter().zip(&reference) {
                    assert_eq!(p, q);
                    assert_eq!(a.len(), b.len());
                    assert!(
                        a.iter()
                            .zip(b)
                            .all(|(a, b)| (*a - b.clamp(-1., 1.)).abs() < 2e-6),
                        "{filename}: unchanged waveform"
                    );
                }
                stream.rewind();
                for attempt in 0..2 {
                    let got = decode(&mut *stream, true, mode, width as u16);
                    assert_eq!(got.len(), 3);
                    for (frame, ((pts, pcm), (rp, base))) in got.iter().zip(&reference).enumerate()
                    {
                        assert_eq!(pts, rp);
                        assert_eq!(pcm.len(), base.len());
                        for (i, (a, b)) in pcm.iter().zip(base).enumerate() {
                            let ch = usize::from(dual && i % width == 1);
                            let expected =
                                (b * gain(c["levels"][frame][ch].as_u64().unwrap())).clamp(-1., 1.);
                            assert!((a-expected).abs()<2e-6,"{filename}: attempt{attempt} frame{frame} sample{i} got{a} expected{expected}");
                        }
                    }
                    stream.rewind();
                }
                if dual {
                    let mono = decode(&mut *open(&filename), true, mode, 1);
                    for (frame, ((_, samples), (_, base))) in
                        mono.iter().zip(&reference).enumerate()
                    {
                        for (a, b) in samples.iter().zip(base.chunks_exact(width)) {
                            let expected =
                                (b[0] * gain(c["levels"][frame][0].as_u64().unwrap()) * 0.5
                                    + b[1] * gain(c["levels"][frame][1].as_u64().unwrap()) * 0.5)
                                    .clamp(-1., 1.);
                            assert!((a - expected).abs() < 2e-6, "{filename}: dual fold");
                        }
                    }
                }
            }
        }
    }
}
