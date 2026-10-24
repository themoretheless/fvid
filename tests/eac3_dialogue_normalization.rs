#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod support;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{checked_frame, transient::apply_window, CompressionMode, Eac3Decoder},
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
            .join("tests/fixtures/playback-errors/eac3-dialogue-normalization")
            .join(n),
    )
    .unwrap()
}
fn open(n: &str) -> fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>> {
    fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(n)), Default::default())
        .unwrap()
}
fn packets(
    s: &mut dyn AudioStream,
    mode: CompressionMode,
    enabled: bool,
) -> Vec<(Duration, Vec<u8>)> {
    let mut d = Eac3Decoder::for_container_with_compression(
        s.extra_data(),
        s.sample_rate(),
        s.channels(),
        mode,
    )
    .unwrap()
    .with_dialogue_normalization(enabled);
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
        d.drain_presented(s)
            .into_iter()
            .map(|p| (p.presentation_time(), p.data)),
    );
    s.validate_eof().unwrap();
    packets
}
fn container(n: &str, mode: CompressionMode, enabled: bool) -> Vec<u8> {
    let mut stream = open(n);
    let packets = packets(&mut stream, mode, enabled);
    assert_eq!(packets.len(), 3);
    for (i, (pts, b)) in packets.iter().enumerate() {
        assert_eq!(*pts, Duration::from_millis(i as u64 * 16));
        assert_eq!(b.len(), 768 * usize::from(stream.channels()) * 4);
    }
    packets.into_iter().flat_map(|(_, b)| b).collect()
}
fn gain(word: &serde_json::Value) -> Option<f32> {
    word.as_u64().map(|w| {
        let x = (w >> 4) as i32 - if w & 128 != 0 { 16 } else { 0 };
        (2f64.powi(x) * (16 + (w & 15)) as f64 / 16.0) as f32
    })
}
fn expected(
    c: &serde_json::Value,
    mode: CompressionMode,
    control: &[f32],
    enabled: bool,
) -> Vec<u8> {
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
            pcm[n * width + native] = (plane[1536 + n]
                * (factor
                    * if enabled {
                        norm(c["levels"][n / 768][usize::from(second)].as_u64().unwrap())
                    } else {
                        1.
                    }))
            .clamp(-1.0, 1.0);
        }
    }
    if !enabled && c["clip"] == true && mode == CompressionMode::Rf {
        assert!(pcm.iter().any(|v| v.abs() == 1.0));
    }
    pcm.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn norm(c: u64) -> f32 {
    10f64.powf(((if c == 0 { 31 } else { c }) as f64 - 31.) / 20.) as f32
}
#[test]
fn dialogue_metadata_controls_native_pcm_after_transient_before_clip_and_reset() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(manifest["accepted"], true);
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 152);
    let mut controls = HashMap::new();
    let mut words = BTreeSet::new();
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let key = c["control"].as_str().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize;
        if c["layout"] == 0 && c["lfe"] == false {
            words.insert(c["levels"][0][0].as_u64().unwrap());
        }
        let control = controls.entry(key.to_owned()).or_insert_with(|| {
            container(key, CompressionMode::Line, false)
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>()
        });
        assert!(control.iter().any(|v| v.abs() > 1e-6));
        let raw = read(&format!("{name}.ec3"));
        for (f, bytes) in raw.chunks_exact(4096).enumerate() {
            let frame = checked_frame(bytes).unwrap();
            let first = c["levels"][f][0].as_u64().unwrap() as u8;
            let second = if frame.layout == 0 {
                c["levels"][f][1].as_u64().unwrap() as u8
            } else {
                first
            };
            assert_eq!(frame.dialogue, [first, second]);
        }
        for mode in [CompressionMode::Line, CompressionMode::Rf] {
            let plain = expected(c, mode, control, false);
            assert_eq!(
                container(&format!("{name}.mkv"), mode, false),
                plain,
                "{name}: unchanged valid waveform"
            );
            let normalized = expected(c, mode, control, true);
            let mut seeking = open(&format!("{name}.mkv"));
            let at = i64::from(seeking.timescale()) / 50;
            assert_eq!(seeking.seek_to(at), at);
            let got = packets(&mut seeking, mode, true);
            assert_eq!(got.first().unwrap().0, Duration::from_millis(20));
            let got: Vec<u8> = got.into_iter().flat_map(|(_, b)| b).collect();
            assert_eq!(
                got,
                normalized[960 * width * 4..],
                "{name}: exact normalized seek"
            );
            let mut d = Eac3Decoder::new(&[], 48000, width as u16)
                .unwrap()
                .with_compression(mode)
                .with_dialogue_normalization(true);
            for _ in 0..2 {
                let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
                assert!(
                    got == normalized[256 * width * 4..],
                    "{name}: normalized native PCM"
                );
                d.reset();
            }
            assert_eq!(
                container(&format!("{name}.mkv"), mode, true),
                normalized,
                "{name}: normalized container PCM"
            );
        }
    }
    assert_eq!(words.len(), 32);
}
