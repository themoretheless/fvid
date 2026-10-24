#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod support;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{CompressionMode, Eac3Decoder, checked_frame, transient::apply_window},
};
use std::{
    collections::{BTreeSet, HashMap},
    io::Cursor,
    path::Path,
    time::Duration,
};
use support::DecodeComplete;
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-rf-compression")
            .join(n),
    )
    .unwrap()
}
fn open(n: &str) -> fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>> {
    fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(n)), Default::default())
        .unwrap()
}
fn container(n: &str, mode: CompressionMode) -> Vec<u8> {
    let mut s = open(n);
    let mut d = Eac3Decoder::for_container_with_compression(
        s.extra_data(),
        s.sample_rate(),
        s.channels(),
        mode,
    )
    .unwrap();
    let mut packets = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        if let AudioStep::Encoded(p) = step {
            if let Some(p) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                    packets.push((p.presentation_time(), p.data));
                }
            }
        }
    }
    packets.extend(
        d.drain_presented(&mut s)
            .into_iter()
            .map(|p| (p.presentation_time(), p.data)),
    );
    s.validate_eof().unwrap();
    assert_eq!(packets.len(), 3);
    for (i, (pts, b)) in packets.iter().enumerate() {
        assert_eq!(*pts, Duration::from_millis(i as u64 * 16));
        assert_eq!(b.len(), 768 * usize::from(s.channels()) * 4);
    }
    packets.into_iter().flat_map(|(_, b)| b).collect()
}
fn gain(word: &serde_json::Value) -> Option<f32> {
    word.as_u64().map(|w| {
        let x = (w >> 4) as i32 - if w & 128 != 0 { 16 } else { 0 };
        (2f64.powi(x) * (16 + (w & 15)) as f64 / 16.0) as f32
    })
}
fn expected(c: &serde_json::Value, mode: CompressionMode, control: &[f32]) -> Vec<u8> {
    let width = c["channels"].as_u64().unwrap() as usize;
    assert_eq!(control.len(), 2304 * width);
    let mut pcm = vec![0.0f32; 2304 * width];
    for coded in 0..width {
        let native = c["coded_to_native"][coded].as_u64().unwrap() as usize;
        let mut plane = vec![0.0; 1536 + 2496];
        for n in 0..2304 {
            plane[1536 + n] = control[n * width + native] * c["scale"].as_f64().unwrap() as f32;
        }
        let w = &c["windows"][coded];
        if !w.is_null() {
            for f in 0..3 {
                apply_window(
                    &mut plane,
                    -1536,
                    f * 768 + w["location"].as_i64().unwrap(),
                    w["length"].as_u64().unwrap() as u8,
                )
                .unwrap();
            }
        }
        let second = c["layout"] == 0 && coded == 1;
        for n in 0..2304 {
            let line = if second {
                [0.25, 2.0, 2.0]
            } else {
                [4.0, 4.0, 0.5]
            }[n / 256 % 3];
            let factor = if mode == CompressionMode::Rf {
                gain(&c[if second { "words2" } else { "words" }][n / 768]).unwrap_or(line)
            } else {
                line
            };
            pcm[n * width + native] = (plane[1536 + n] * factor).clamp(-1.0, 1.0);
        }
    }
    if c["clip"] == true && mode == CompressionMode::Rf {
        assert!(pcm.iter().any(|v| v.abs() == 1.0));
    }
    pcm.iter().flat_map(|v| v.to_le_bytes()).collect()
}
#[test]
fn rf_words_override_line_per_channel_and_fall_back_per_frame_without_changing_pcm_history() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(manifest["accepted"], true);
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 632);
    for layout in [0, 2] {
        let words: BTreeSet<_> = cases
            .iter()
            .filter(|c| c["layout"] == layout && c["lfe"] == false)
            .map(|c| c["words"][0].as_u64().unwrap())
            .collect();
        assert_eq!(words.len(), 256);
    }
    let mut controls = HashMap::new();
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let key = c["control"].as_str().unwrap();
        let control = controls.entry(key.to_owned()).or_insert_with(|| {
            container(key, CompressionMode::Line)
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>()
        });
        let raw = read(&format!("{name}.ec3"));
        for (f, bytes) in raw.chunks_exact(4096).enumerate() {
            let frame = checked_frame(bytes).unwrap();
            assert_eq!(
                frame.compression,
                [
                    c["words"][f].as_u64().map(|v| v as u8),
                    c["words2"][f].as_u64().map(|v| v as u8)
                ]
            );
        }
        let rf = expected(c, CompressionMode::Rf, control);
        let line = expected(c, CompressionMode::Line, control);
        assert_ne!(rf, line, "{name}: RF must change PCM");
        let width = c["channels"].as_u64().unwrap() as usize;
        let mut d = Eac3Decoder::new(&[], 48000, width as u16)
            .unwrap()
            .with_compression(CompressionMode::Rf);
        let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        assert!(got == rf[256 * width * 4..], "{name}: RF PCM mismatch");
        d.reset();
        assert_eq!(
            d.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
            got,
            "{name}: reset"
        );
        let got = Eac3Decoder::new(&[], 48000, width as u16)
            .unwrap()
            .decode_complete(&raw, 0, 0)
            .unwrap()
            .unwrap()
            .data;
        assert!(
            got == line[256 * width * 4..],
            "{name}: default Line must remain unchanged"
        );
        assert!(
            container(&format!("{name}.mkv"), CompressionMode::Rf) == rf,
            "{name}: Matroska RF PCM"
        );
        if c["layout"] == 0
            && c["lfe"] == false
            && [0, 127, 128, 255].contains(&c["words"][0].as_u64().unwrap())
        {
            assert!(
                rf.chunks_exact(4)
                    .all(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() < 1.0)
            );
            let mono: Vec<_> = rf
                .chunks_exact(8)
                .flat_map(|p| {
                    (f32::from_le_bytes(p[..4].try_into().unwrap()) * 0.5
                        + f32::from_le_bytes(p[4..].try_into().unwrap()) * 0.5)
                        .clamp(-1.0, 1.0)
                        .to_le_bytes()
                })
                .collect();
            let got = Eac3Decoder::new(&[], 48000, 1)
                .unwrap()
                .with_compression(CompressionMode::Rf)
                .decode_complete(&raw, 0, 0)
                .unwrap()
                .unwrap()
                .data;
            assert!(
                got == mono[256 * 4..],
                "{name}: unequal RF must precede mono fold"
            );
        }
        if c["layout"] == 1 && c["lfe"] == true && c["transient"] == false && c["words"][0] == 127 {
            let got = Eac3Decoder::new_stereo(&[], 48000)
                .unwrap()
                .with_compression(CompressionMode::Rf)
                .decode_complete(&raw, 0, 0)
                .unwrap()
                .unwrap()
                .data;
            let expected: Vec<_> = rf
                .chunks_exact(8)
                .skip(256)
                .flat_map(|p| {
                    let centre = f32::from_le_bytes(p[..4].try_into().unwrap()) * 0.707;
                    [centre, centre]
                })
                .collect();
            let actual: Vec<_> = got
                .chunks_exact(4)
                .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
                .collect();
            assert_eq!(actual.len(), expected.len());
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 2e-8),
                "{name}: explicit RF stereo must fold C and retain disabled LFE injection"
            );
        }
    }
}
