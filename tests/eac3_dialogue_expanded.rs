#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{transient::apply_window, CompressionMode, Eac3Decoder},
};
use std::{io::Cursor, path::Path, time::Duration};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-dialogue-expanded")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str) -> fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>> {
    fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(name)), Default::default())
        .unwrap()
}
fn packets(s: &mut dyn AudioStream, mode: CompressionMode) -> Vec<(Duration, Vec<u8>)> {
    let mut d = Eac3Decoder::for_container_with_compression(
        s.extra_data(),
        s.sample_rate(),
        s.channels(),
        mode,
    )
    .unwrap()
    .with_dialogue_normalization(true);
    let mut out = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        if let AudioStep::Encoded(p) = step {
            if let Some(p) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                    out.push((p.presentation_time(), p.data));
                }
            }
        }
    }
    out.extend(
        d.drain_presented(s)
            .into_iter()
            .map(|p| (p.presentation_time(), p.data)),
    );
    s.validate_eof().unwrap();
    out
}
fn pcm(s: &mut dyn AudioStream) -> Vec<u8> {
    packets(s, CompressionMode::Rf)
        .into_iter()
        .flat_map(|(_, p)| p)
        .collect()
}
fn expected_for(c: &serde_json::Value, common_only: bool, correct: bool, rf: bool) -> Vec<u8> {
    let width = c["channels"].as_u64().unwrap() as usize;
    let frame = c["blocks"].as_u64().unwrap() as usize * 256;
    let count = frame * 3;
    let mut mixed = vec![0.0f32; count * width];
    let mut programme = vec![0; width * 3];
    for part in c["components"].as_array().unwrap() {
        let bytes = pcm(&mut open(part["control"].as_str().unwrap()));
        let control: Vec<_> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let roles = part["roles"].as_array().unwrap();
        assert_eq!(control.len(), count * roles.len());
        assert!(control.iter().all(|v| v.abs() < 0.9));
        for (coded, role) in roles.iter().enumerate() {
            let native = part["coded_to_native"][coded].as_u64().unwrap() as usize;
            let mut plane = vec![0.0; 1536 + count.max(frame * 2 + 960)];
            for n in 0..count {
                plane[1536 + n] =
                    control[n * roles.len() + native] * c["scale"].as_f64().unwrap() as f32;
            }
            let w = &part["windows"][coded];
            if correct && !w.is_null() {
                for f in 0..3 {
                    apply_window(
                        &mut plane,
                        -1536,
                        (f * frame) as i64 + w["location"].as_i64().unwrap(),
                        w["length"].as_u64().unwrap() as u8,
                    )
                    .unwrap();
                }
            }
            let output = c["speakers"]
                .as_array()
                .unwrap()
                .iter()
                .position(|s| s == role)
                .unwrap();
            for f in 0..3 {
                programme[f * width + output] =
                    usize::from(!common_only && part["dual_frames"][f] == true && coded == 1);
            }
            for n in 0..count {
                let second = part["dual_frames"][n / frame] == true && coded == 1;
                let code = c["levels"][n / frame][usize::from(second)]
                    .as_u64()
                    .unwrap();
                mixed[n * width + output] = plane[1536 + n] * norm(code);
            }
        }
    }
    for n in 0..count {
        for ch in 0..width {
            let selector = if c["switching"] == true && n / frame == 1 {
                0
            } else {
                programme[n / frame * width + ch]
            };
            let word = if rf {
                c["rf_words"][selector][n / frame].as_u64()
            } else {
                None
            };
            let factor = word
                .map(|w| {
                    let e = (w >> 4) as i32 - if w & 128 != 0 { 16 } else { 0 };
                    (2f64.powi(e) * (16 + (w & 15)) as f64 / 16.0) as f32
                })
                .unwrap_or(
                    c["gains"][selector][n / 256 % (frame / 256)]
                        .as_f64()
                        .unwrap() as f32,
                );
            mixed[n * width + ch] = (mixed[n * width + ch] * factor).clamp(-1.0, 1.0);
        }
    }
    mixed.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();

    assert_eq!(m["accepted"], true);
    let c = m["cases"].as_array().unwrap().clone();
    assert_eq!(c.len(), 80);
    c
}
fn norm(c: u64) -> f32 {
    10f64.powf(((if c == 0 { 31 } else { c }) as f64 - 31.) / 20.) as f32
}
fn bytes_equal(a: &[u8], b: &[u8], name: &str) {
    assert_eq!(a.len(), b.len(), "{name}: length");
    for (i, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 2e-6, "{name}: sample{i}, {a} != {b}");
    }
}
fn check_seek(
    s: &mut dyn AudioStream,
    mode: CompressionMode,
    expected: &[u8],
    rate: u32,
    width: usize,
    name: &str,
) {
    let at = i64::from(s.timescale()) / 25;
    assert_eq!(s.seek_to(at), at);
    let got = packets(s, mode);
    assert_eq!(got.first().unwrap().0, Duration::from_millis(40));
    let bytes: Vec<u8> = got.into_iter().flat_map(|(_, b)| b).collect();
    bytes_equal(&bytes, &expected[(rate as usize / 25) * width * 4..], name);
}
#[test]
fn expanded_dialogue_levels_follow_owning_stream_after_correction_before_programme_drc() {
    for c in cases() {
        let name = c["name"].as_str().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize;
        let rate = c["rate"].as_u64().unwrap() as u32;
        let raw = read(&format!("{name}.ec3"));
        for mode in [CompressionMode::Line, CompressionMode::Rf] {
            let expected = expected_for(&c, false, true, mode == CompressionMode::Rf);
            let mut d = Eac3Decoder::new(&[], rate, width as u16)
                .unwrap()
                .with_compression(mode)
                .with_dialogue_normalization(true);
            for _ in 0..2 {
                let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
                bytes_equal(&got, &expected[256 * width * 4..], name);
                d.reset();
            }
            let mut direct =
                fvid::codec::eac3_decoder::presentation::PresentationDecoder::for_container(
                    rate,
                    width as u16,
                )
                .unwrap()
                .with_dialogue_normalization(true);
            direct.set_dolby_compression(mode);
            let got = direct.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
            bytes_equal(&got, &expected, name);
            let mut stream = open(&format!("{name}.mkv"));
            for _ in 0..2 {
                let got: Vec<u8> = packets(&mut stream, mode)
                    .into_iter()
                    .flat_map(|(_, b)| b)
                    .collect();
                bytes_equal(&got, &expected, name);
                stream.rewind();
            }
            check_seek(&mut stream, mode, &expected, rate, width, name);
            if let Some(n) = c["mp4"].as_str() {
                let mut s = fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(read(n)),
                    Default::default(),
                )
                .unwrap();
                let got: Vec<u8> = packets(&mut s, mode)
                    .into_iter()
                    .flat_map(|(_, b)| b)
                    .collect();
                bytes_equal(&got, &expected, n);
                check_seek(&mut s, mode, &expected, rate, width, n);
            }
        }
    }
}

#[test]
fn compatibility_output_normalizes_independent_programme_without_dependent_gain() {
    for c in cases() {
        let base = &c["components"][0];
        let width = base["roles"].as_array().unwrap().len();
        let control = pcm(&mut open(base["control"].as_str().unwrap()));
        let frame = c["blocks"].as_u64().unwrap() as usize * 256;
        let raw = read(&format!("{}.ec3", c["name"].as_str().unwrap()));
        for mode in [CompressionMode::Line, CompressionMode::Rf] {
            let expected: Vec<u8> = control
                .chunks_exact(4)
                .enumerate()
                .flat_map(|(i, b)| {
                    let second = base["dual"] == true && i % width == 1;
                    let compression = match (mode, second) {
                        (CompressionMode::Rf, false) => 64.,
                        (CompressionMode::Rf, true) => 128.,
                        (CompressionMode::Line, false) => 8.,
                        (CompressionMode::Line, true) => 0.125,
                    };
                    let code = c["levels"][i / width / frame][usize::from(second)]
                        .as_u64()
                        .unwrap();
                    let sample = f32::from_le_bytes(b.try_into().unwrap())
                        * c["scale"].as_f64().unwrap() as f32;
                    (sample * (compression * norm(code)))
                        .clamp(-1., 1.)
                        .to_le_bytes()
                })
                .collect();
            let mut d = Eac3Decoder::new(&[], c["rate"].as_u64().unwrap() as u32, width as u16)
                .unwrap()
                .with_compression(mode)
                .with_dialogue_normalization(true);
            let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
            bytes_equal(
                &got,
                &expected[256 * width * 4..],
                c["name"].as_str().unwrap(),
            );
        }
    }
}
