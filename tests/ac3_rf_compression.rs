#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod support;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::ac3_decoder::{Ac3Decoder, CompressionMode},
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
            .join("tests/fixtures/playback-errors/ac3-rf-compression")
            .join(n),
    )
    .unwrap()
}
fn open(n: &str) -> fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>> {
    fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(n)), Default::default())
        .unwrap()
}
fn packets(s: &mut dyn AudioStream, mode: CompressionMode) -> Vec<(Duration, Vec<u8>)> {
    let mut d = Ac3Decoder::for_container_with_compression(
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
        d.drain_presented(s)
            .into_iter()
            .map(|p| (p.presentation_time(), p.data)),
    );
    s.validate_eof().unwrap();
    packets
}
fn container(n: &str, mode: CompressionMode) -> Vec<u8> {
    let mut s = open(n);
    let packets = packets(&mut s, mode);
    assert_eq!(packets.len(), 3);
    for (i, (pts, b)) in packets.iter().enumerate() {
        assert_eq!(
            *pts,
            Duration::from_nanos(i as u64 * 1536 * 1_000_000_000 / u64::from(s.sample_rate()))
        );
        assert_eq!(b.len(), 1536 * usize::from(s.channels()) * 4);
    }
    packets.into_iter().flat_map(|(_, b)| b).collect()
}
fn gain(word: &serde_json::Value) -> Option<f32> {
    word.as_u64().map(|w| {
        let e = (w >> 4) as i32 - if w & 128 != 0 { 16 } else { 0 };
        (2f64.powi(e) * (16 + (w & 15)) as f64 / 16.0) as f32
    })
}
fn expected(c: &serde_json::Value, mode: CompressionMode, control: &[f32], clip: bool) -> Vec<f32> {
    let width = c["channels"].as_u64().unwrap() as usize;
    assert_eq!(control.len(), 4608 * width);
    control
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let second = c["layout"] == 0 && i % width == 1;
            let n = i / width;
            let line = c["line"][usize::from(second)][n / 256 % 6]
                .as_f64()
                .unwrap() as f32;
            let factor = if mode == CompressionMode::Rf {
                gain(&c[if second { "words2" } else { "words" }][n / 1536]).unwrap_or(line)
            } else {
                line
            };
            let value = v * c["scale"].as_f64().unwrap() as f32 * factor;
            if clip { value.clamp(-1.0, 1.0) } else { value }
        })
        .collect()
}
fn bytes(p: &[f32]) -> Vec<u8> {
    p.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["accepted"], true);
    let cases = m["cases"].as_array().unwrap().clone();
    assert_eq!(cases.len(), 659);
    cases
}
#[test]
fn rf_ac3_words_fallback_native_reset_and_downmix_match_unclipped_controls() {
    let cases = cases();
    let all: BTreeSet<_> = cases
        .iter()
        .filter(|c| c["layout"] == 0 && c["lfe"] == false && c["rate"] == 48000)
        .map(|c| c["words"][0].as_u64().unwrap())
        .collect();
    assert_eq!(all.len(), 256);
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
        assert!(
            control.iter().any(|v| v.abs() > 1e-7),
            "{name}: fixture must decode signal, not unrelated concealment"
        );
        assert!(
            control.iter().all(|v| v.abs() < 0.9),
            "{name}: control must retain headroom"
        );
        let width = c["channels"].as_u64().unwrap() as usize;
        let rate = c["rate"].as_u64().unwrap() as u32;
        let raw = read(&format!("{name}.ac3"));
        for p in raw.chunks_exact(c["frame_bytes"].as_u64().unwrap() as usize) {
            assert!(
                fvid::codec::ac3_decoder::checked_syncframe(p).is_some(),
                "{name}: valid CRC required"
            );
        }
        if let Some(source) = c["source"].as_str() {
            let original = Ac3Decoder::new(&[], rate, width as u16)
                .unwrap()
                .decode_complete(&read(source), 0, 0)
                .unwrap()
                .unwrap()
                .data;
            assert!(
                original == bytes(&control[256 * width..]),
                "{name}: repacking BSI/CRC must not change the original waveform"
            );
        }
        let line = bytes(&expected(&c, CompressionMode::Line, control, true));
        let got = Ac3Decoder::new(&[], rate, width as u16)
            .unwrap()
            .decode_complete(&raw, 0, 0)
            .unwrap()
            .unwrap()
            .data;
        assert!(
            got == line[256 * width * 4..],
            "{name}: first validate intended line PCM, not a syntax error"
        );
        let rf_values = expected(&c, CompressionMode::Rf, control, true);
        let rf = bytes(&rf_values);
        assert_ne!(rf, line, "{name}: RF must affect PCM");
        if c["clip"] == true {
            assert!(rf_values.iter().any(|v| v.abs() == 1.0));
        }
        let mut d = Ac3Decoder::new(&[], rate, width as u16)
            .unwrap()
            .with_compression(CompressionMode::Rf);
        let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        assert!(got == rf[256 * width * 4..], "{name}: RF PCM mismatch");
        d.reset();
        assert_eq!(
            d.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
            got,
            "{name}: mode/history reset"
        );
        assert!(
            container(&format!("{name}.mkv"), CompressionMode::Rf) == rf,
            "{name}: Matroska RF"
        );
        if c["layout"] == 0 && c["clip"] == true {
            let unclipped = expected(&c, CompressionMode::Rf, control, false);
            let mono: Vec<_> = unclipped
                .chunks_exact(width)
                .map(|p| (p[0] * 0.5 + p[1] * 0.5).clamp(-1.0, 1.0))
                .collect();
            let got = Ac3Decoder::new(&[], rate, 1)
                .unwrap()
                .with_compression(CompressionMode::Rf)
                .decode_complete(&raw, 0, 0)
                .unwrap()
                .unwrap()
                .data;
            assert!(
                got == bytes(&mono[256..]),
                "{name}: RF must precede folding and clip"
            );
        }
    }
}

#[test]
fn rf_mp4_transport_and_stream_seeks_preserve_pcm_clocks_and_native_geometry() {
    let cases: Vec<_> = cases()
        .into_iter()
        .filter(|c| c["mp4"].is_string())
        .collect();
    assert_eq!(cases.len(), 163);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize;
        let rate = c["rate"].as_u64().unwrap();
        let control = container(c["control"].as_str().unwrap(), CompressionMode::Line);
        let control: Vec<_> = control
            .chunks_exact(4)
            .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
            .collect();
        let want = bytes(&expected(&c, CompressionMode::Rf, &control, true));
        let mut mp4 = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(read(c["mp4"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            mp4.channels() as usize,
            width,
            "{name}: placeholder geometry must resolve from CRC-valid AC-3"
        );
        assert_eq!(u64::from(mp4.sample_rate()), rate);
        let got = packets(&mut mp4, CompressionMode::Rf);
        assert_eq!(got.len(), 3);
        assert!(
            got.iter().flat_map(|(_, b)| b.iter().copied()).eq(want),
            "{name}: MP4 RF PCM"
        );
        for (i, (pts, b)) in got.iter().enumerate() {
            assert_eq!(
                *pts,
                Duration::from_nanos(i as u64 * 1536 * 1_000_000_000 / rate)
            );
            assert_eq!(b.len(), 1536 * width * 4);
        }
        mp4.rewind();
        assert_eq!(
            packets(&mut mp4, CompressionMode::Rf),
            got,
            "{name}: rewind"
        );
        let seek = Duration::from_millis(20);
        let mut wanted = got.clone();
        wanted[0].0 = seek;
        wanted[0].1.drain(..(rate / 50) as usize * width * 4);
        let at = i64::from(mp4.timescale()) / 50;
        assert_eq!(mp4.seek_to(at), at);
        assert_eq!(
            packets(&mut mp4, CompressionMode::Rf),
            wanted,
            "{name}: RF MP4 seek"
        );
        let mut mkv = open(&format!("{name}.mkv"));
        let at = i64::from(mkv.timescale()) / 50;
        assert_eq!(mkv.seek_to(at), at);
        assert_eq!(
            packets(&mut mkv, CompressionMode::Rf),
            wanted,
            "{name}: RF Matroska seek"
        );
    }
}
