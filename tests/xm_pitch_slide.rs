#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-pitch-slide")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
// This reference knows no effect numbers, parameters or effect memories. The
// fixture supplies the entire period/reset trace. Its sample is the authored
// signed16 64-frame ramp, rendered with the documented native linear mixer.
fn reference(c: &serde_json::Value, rate: u32, channels: u16) -> Vec<u8> {
    let periods = c["periods"].as_array().unwrap();
    let resets = c["resets"].as_array().unwrap();
    let release = c["release"].as_u64().unwrap() as usize;
    let mut out = Vec::new();
    let mut position = 0.0f64;
    let mut gain = 0.0f32;
    for (tick, p) in periods.iter().enumerate() {
        if resets.iter().any(|x| x.as_u64() == Some(tick as u64)) {
            position = 0.0;
        }
        let p = p.as_f64().unwrap();
        let hz = if c["linear"].as_bool().unwrap() {
            8363.0 * f64::exp2((4608.0 - p) / 768.0)
        } else {
            4.0 * 7_093_789.2 / (p * 2.0)
        };
        let first = tick * rate as usize / 50;
        let end = (tick + 1) * rate as usize / 50;
        for _ in first..end {
            gain = if tick < release {
                (gain + 1.0 / 256.0).min(0.25)
            } else {
                (gain - 1.0 / 256.0).max(0.0)
            };
            let index = position.floor() as usize;
            let frac = (position - position.floor()) as f32;
            let ramp = |i: usize| ((i as i32 - 32) * 512) as f32 / 32768.0;
            let a = ramp(index);
            let b = ramp((index + 1) % 64);
            let sample = (a + (b - a) * frac) * gain;
            position += hz / f64::from(rate);
            if position >= 64.0 {
                position = position.rem_euclid(64.0);
            }
            let v = 0.0f32
                + if channels == 1 {
                    sample
                } else {
                    sample * 0.5f64.sqrt() as f32
                };
            for _ in 0..channels {
                out.extend(v.to_le_bytes());
            }
        }
    }
    out
}
#[test]
fn pitch_slides_match_literal_period_pcm_without_phase_restarts() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 42);
    for c in m["cases"].as_array().unwrap() {
        for rate in [11025, 44100] {
            for channels in [1, 2] {
                let mut r = fvid::playback_xm::XmAudioReader::open(
                    Cursor::new(std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap()),
                    Default::default(),
                )
                .unwrap();
                let mut d = fvid::codec::xm_decoder::XmDecoder::new(r.extra_data(), rate, channels)
                    .unwrap();
                let mut actual = Vec::new();
                while let Some(p) = r.next_packet().unwrap() {
                    actual.extend(
                        d.decode_packet(&p.data, p.pts, p.duration as u64)
                            .unwrap()
                            .unwrap()
                            .packet
                            .data,
                    );
                }
                let expected = reference(c, rate, channels);
                assert_eq!(actual.len(), expected.len(), "{}", c["name"]);
                assert!(
                    actual == expected,
                    "{}: PCM differs at {rate}/{channels}",
                    c["name"]
                );
                assert_eq!(d.ignored(), Default::default(), "{}", c["name"]);
            }
        }
    }
}
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn pitch_slide_worker_matches_literal_periods_after_rewind_seek_and_eof() {
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(root().join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for c in manifest()["cases"].as_array().unwrap() {
        let pcm = reference(c, 44100, 2);
        let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let mut r =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes.clone()), Default::default())
                .unwrap();
        let total = r.duration().unwrap();
        let mut expected = Vec::new();
        while let Some(p) = r.next_packet().unwrap() {
            let first = p.pts as usize * 44100 / 1000;
            let end = (p.pts + p.duration) as usize * 44100 / 1000;
            expected.push((
                Duration::from_nanos(first as u64 * 1_000_000_000 / 44100),
                pcm[first * 8..end * 8].to_vec(),
            ));
        }
        for target in [expected[1].0, total] {
            let suffix: Vec<_> = expected
                .iter()
                .filter(|(at, _)| *at >= target)
                .cloned()
                .collect();
            let reader = fvid::playback_xm::XmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            capture::worker_check_with_seek_expected(Box::new(reader), &expected, target, &suffix);
        }
    }
}

#[test]
fn omitted_slide_controls_reproduce_equal_length_pitch_failure() {
    for linear in [true, false] {
        let m = manifest();
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["name"]
                    == if linear {
                        "ordinary-up-linear"
                    } else {
                        "ordinary-up-amiga"
                    }
            })
            .unwrap();
        let file = c["file"].as_str().unwrap().replace(".xm", "-no-slide.xm");
        let mut r = fvid::playback_xm::XmAudioReader::open(
            Cursor::new(std::fs::read(root().join(file)).unwrap()),
            Default::default(),
        )
        .unwrap();
        let mut d = fvid::codec::xm_decoder::XmDecoder::new(r.extra_data(), 44100, 2).unwrap();
        let mut actual = Vec::new();
        while let Some(p) = r.next_packet().unwrap() {
            actual.extend(
                d.decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap()
                    .packet
                    .data,
            );
        }
        let wanted = reference(c, 44100, 2);
        assert_eq!(actual.len(), wanted.len());
        assert_ne!(actual, wanted);
        let mut flat = c.clone();
        flat["periods"] = serde_json::json!(vec![
            if linear { 4608 } else { 1712 };
            c["periods"].as_array().unwrap().len()
        ]);
        assert_eq!(actual, reference(&flat, 44100, 2));
        assert_eq!(d.ignored(), Default::default());
    }
}
