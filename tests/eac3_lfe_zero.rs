#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{Eac3Decoder, frame},
};
use std::{io::Cursor, path::Path, time::Duration};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-lfe-zero")
            .join(name),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let c = m["cases"].as_array().unwrap().clone();
    assert_eq!(c.len(), 96);
    c
}
fn direct(c: &serde_json::Value) -> Vec<u8> {
    let b = read(c["file"].as_str().unwrap());
    let h = frame(&b).unwrap();
    assert_eq!(
        (h.sample_rate, h.channels, h.blocks, h.frame_bytes),
        (48000, 3, 6, 4096)
    );
    for (bit, plane) in [0, 1, 6].into_iter().enumerate() {
        assert_eq!(h.aht[plane], c["mask"].as_u64().unwrap() & (1 << bit) != 0);
    }
    Eac3Decoder::new(&[], 48000, 3)
        .unwrap()
        .decode_complete(&b, 0, 0)
        .unwrap()
        .unwrap()
        .data
}
fn verify(c: &serde_json::Value, pcm: &[u8]) {
    assert!(!pcm.is_empty() && pcm.len() % 12 == 0);
    let mut peak = [0.0f32; 3];
    for sample in pcm.chunks_exact(12) {
        for ch in 0..3 {
            let v = f32::from_le_bytes(sample[ch * 4..ch * 4 + 4].try_into().unwrap());
            assert!(v.is_finite());
            peak[ch] = peak[ch].max(v.abs());
        }
    }
    if c["allocated"] == true {
        assert!(
            peak.iter().all(|&x| x > 1e-6),
            "{} coded planes muted {peak:?}",
            c["name"]
        );
    } else {
        assert_eq!(
            peak[2], 0.0,
            "{} zero-bit LFE must not invent dither",
            c["name"]
        );
        for ch in 0..2 {
            if c["enabled"] == true {
                assert!(peak[ch] > 1e-6);
            } else {
                assert_eq!(peak[ch], 0.0, "{} ch{ch}", c["name"]);
            }
        }
    }
}
fn open(c: &serde_json::Value, kind: &str) -> Box<dyn AudioStream> {
    let key = match kind {
        "raw" => "file",
        "mkv" => "video",
        "mp4" => "mp4",
        _ => unreachable!(),
    };
    let b = Cursor::new(read(c[key].as_str().unwrap()));
    match kind {
        "raw" => Box::new(fvid::playback_ac3::Ac3AudioReader::open(b, Default::default()).unwrap()),
        "mkv" => Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(b, Default::default()).unwrap(),
        ),
        _ => {
            Box::new(fvid::playback_mp4_audio::Mp4AudioReader::open(b, Default::default()).unwrap())
        }
    }
}
fn packets(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut d = s.make_decoder().unwrap();
    let mut out = vec![];
    while let Some(step) = s.next_step().unwrap() {
        let p = match step {
            AudioStep::Encoded(p) => d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .and_then(|d| s.present_decoded(d.packet, d.source_pts).unwrap()),
            AudioStep::Pcm(p) => Some(p),
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => {
                s.reset_decoder(&mut d).unwrap();
                None
            }
        };
        if let Some(p) = p {
            out.push((p.presentation_time(), p.data));
        }
    }
    out.extend(d.drain_presented(s).into_iter().map(|p| (p.presentation_time(), p.data)));
    s.validate_eof().unwrap();
    out
}
#[test]
fn zero_bit_lfe_is_silent_with_conventional_and_aht_coding() {
    for c in cases().iter().filter(|c| c["allocated"] == false) {
        verify(c, &direct(c));
    }
}
#[test]
fn coded_lfe_is_not_suppressed_and_ignores_full_band_dither_flags() {
    for c in cases()
        .iter()
        .filter(|c| c["allocated"] == true && c["enabled"] == false)
    {
        let got = direct(c);
        verify(c, &got);
        let name = c["file"].as_str().unwrap().replace("-d0.ec3", "-d1.ec3");
        let other = Eac3Decoder::new(&[], 48000, 3)
            .unwrap()
            .decode_complete(&read(&name), 0, 0)
            .unwrap()
            .unwrap()
            .data;
        assert_eq!(got, other, "{} coded dither flag changed PCM", c["name"]);
    }
}
#[test]
fn all_readers_preserve_lfe_silence_signal_and_rewind() {
    for c in cases() {
        for kind in ["raw", "mkv", "mp4"] {
            let mut s = open(&c, kind);
            let expected = packets(&mut *s);
            assert_eq!(expected.len(), 3);
            let pcm = expected
                .iter()
                .flat_map(|(_, b)| b.iter().copied())
                .collect::<Vec<_>>();
            verify(&c, &pcm);
            let lead = if kind == "raw" { 256 } else { 0 };
            assert_eq!(pcm.len(), (4608 - lead) * 12);
            let duration = s.duration();
            s.rewind();
            assert_eq!(packets(&mut *s), expected, "{} {kind} rewind", c["name"]);
            assert_eq!(s.duration(), duration);
        }
    }
}
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn worker_preserves_lfe_assignment_and_pcm_on_rewind_and_partial_seek() {
    for c in cases() {
        for kind in ["raw", "mkv", "mp4"] {
            let expected = packets(&mut *open(&c, kind));
            assert_eq!(expected.len(), 3);
            assert_eq!(expected[0].0, Duration::ZERO);
            let pcm = expected
                .iter()
                .flat_map(|(_, b)| b.iter().copied())
                .collect::<Vec<_>>();
            verify(&c, &pcm);
            let seek = Duration::from_millis(20);
            let mut wanted = expected.clone();
            wanted[0].0 = seek;
            wanted[0].1.drain(..960 * 12);
            capture::worker_check_with_layout_and_seek_expected(
                open(&c, kind),
                &expected,
                seek,
                &wanted,
                0xb,
            );
        }
    }
}
