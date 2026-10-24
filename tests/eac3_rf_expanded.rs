#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{CompressionMode, Eac3Decoder, transient::apply_window},
};
use std::{io::Cursor, path::Path, time::Duration};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-rf-expanded")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str) -> fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>> {
    fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(name)), Default::default())
        .unwrap()
}
fn packets(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut d = Eac3Decoder::for_container_with_compression(
        s.extra_data(),
        s.sample_rate(),
        s.channels(),
        CompressionMode::Rf,
    )
    .unwrap();
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
    packets(s).into_iter().flat_map(|(_, p)| p).collect()
}
fn expected(c: &serde_json::Value, common_only: bool, correct: bool) -> Vec<u8> {
    expected_for(c, common_only, correct, true)
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
                mixed[n * width + output] = plane[1536 + n];
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
    if c["clip"] == true {
        assert!(
            mixed.iter().any(|v| v.abs() == 1.0),
            "{} must actually clip",
            c["name"]
        );
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
#[test]
fn rf_final_dependent_words_preserve_ownership_fallback_transient_and_clipping() {
    for c in cases() {
        let name = c["name"].as_str().unwrap();
        let want = expected(&c, false, true);
        assert_ne!(
            want,
            expected(&c, true, true),
            "{name}: unequal DRC must matter"
        );
        if c["transient"] == true {
            assert_ne!(
                want,
                expected(&c, false, false),
                "{name}: correction must matter"
            );
        }
        let width = c["channels"].as_u64().unwrap() as usize;
        let rate = c["rate"].as_u64().unwrap() as u32;
        let raw = read(&format!("{name}.ec3"));
        let mut d = Eac3Decoder::new(&[], rate, width as u16)
            .unwrap()
            .with_compression(CompressionMode::Rf);
        let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        assert!(got == want[256 * width * 4..], "{name}: raw PCM");
        d.reset();
        assert_eq!(
            d.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
            got,
            "{name}: reset"
        );
        let mut s = open(&format!("{name}.mkv"));
        let got = packets(&mut s);
        assert_eq!(got.len(), 3);
        assert!(
            got.iter().flat_map(|(_, b)| b.iter().copied()).eq(want),
            "{name}: Matroska PCM"
        );
        s.rewind();
        assert_eq!(packets(&mut s), got);
        let samples = c["blocks"].as_u64().unwrap() * 256;
        for (i, (pts, b)) in got.iter().enumerate() {
            assert_eq!(
                *pts,
                Duration::from_nanos(i as u64 * samples * 1_000_000_000 / u64::from(rate))
            );
            assert_eq!(b.len(), samples as usize * width * 4);
        }
        let seek = Duration::from_millis(20);
        let sample = u64::from(rate) / 50;
        let mut wanted = got[(sample / samples) as usize..].to_vec();
        wanted[0].0 = seek;
        wanted[0]
            .1
            .drain(..((sample % samples) as usize * width * 4));
        let mut seeking = open(&format!("{name}.mkv"));
        assert_eq!(
            seeking.seek_to(i64::from(seeking.timescale()) / 50),
            i64::from(seeking.timescale()) / 50
        );
        assert_eq!(packets(&mut seeking), wanted, "{name}: RF Matroska seek");
    }
}
#[test]
fn rf_mp4_preserves_pcm_pts_rewind_and_seek() {
    let cases: Vec<_> = cases()
        .into_iter()
        .filter(|c| c["mp4"].is_string())
        .collect();
    assert_eq!(cases.len(), 32);
    for c in cases {
        let open_mp4 = || -> Box<dyn AudioStream> {
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(read(c["mp4"].as_str().unwrap())),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        let mut s = open_mp4();
        let got = packets(&mut *s);
        assert_eq!(got.len(), 3);
        assert!(
            got.iter()
                .flat_map(|(_, b)| b.iter().copied())
                .eq(expected(&c, false, true)),
            "{}: MP4 PCM",
            c["name"]
        );
        s.rewind();
        assert_eq!(packets(&mut *s), got);
        let rate = c["rate"].as_u64().unwrap();
        for (i, (pts, b)) in got.iter().enumerate() {
            assert_eq!(
                *pts,
                Duration::from_nanos(i as u64 * 1536 * 1_000_000_000 / rate)
            );
            assert_eq!(b.len(), 1536 * 8 * 4);
        }
        let seek = Duration::from_millis(40);
        let sample = rate / 25;
        let mut wanted = got[(sample / 1536) as usize..].to_vec();
        wanted[0].0 = seek;
        wanted[0].1.drain(..((sample % 1536) as usize * 8 * 4));
        let mut seeking = open_mp4();
        let at = i64::from(seeking.timescale()) / 25;
        assert_eq!(seeking.seek_to(at), at);
        assert_eq!(packets(&mut *seeking), wanted, "{}: RF MP4 seek", c["name"]);
    }
}

#[test]
fn rf_compatibility_output_uses_independent_compression_and_skips_dependent_words() {
    for c in cases() {
        let base = &c["components"][0];
        let width = base["roles"].as_array().unwrap().len();
        let control = pcm(&mut open(base["control"].as_str().unwrap()));
        let mut want = Vec::new();
        for (n, b) in control.chunks_exact(4).enumerate() {
            let gain = if base["dual"] == true && n % width == 1 {
                128.0
            } else {
                64.0
            };
            let value = (f32::from_le_bytes(b.try_into().unwrap())
                * c["scale"].as_f64().unwrap() as f32
                * gain)
                .clamp(-1.0, 1.0);
            want.extend(value.to_le_bytes());
        }
        let raw = read(&format!("{}.ec3", c["name"].as_str().unwrap()));
        let full_width = c["channels"].as_u64().unwrap() as usize;
        let line = Eac3Decoder::new(&[], c["rate"].as_u64().unwrap() as u32, full_width as u16)
            .unwrap()
            .decode_complete(&raw, 0, 0)
            .unwrap()
            .unwrap()
            .data;
        assert!(
            line == expected_for(&c, false, true, false)[256 * full_width * 4..],
            "{}: default expanded Line must ignore RF words",
            c["name"]
        );

        let mut d = Eac3Decoder::new(&[], c["rate"].as_u64().unwrap() as u32, width as u16)
            .unwrap()
            .with_compression(CompressionMode::Rf);
        let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        assert!(
            got == want[256 * width * 4..],
            "{}: compatibility RF must use independent metadata",
            c["name"]
        );
    }
}
