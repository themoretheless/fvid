#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::playback_mp4_audio::Mp4AudioReader;
use std::io::Cursor;
use transitions::collect;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/mp4-aac-transitions/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn open(name: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(
        Cursor::new(bytes(&format!("{name}.mp4"))),
        Default::default(),
    )
    .unwrap()
}
#[test]
fn owned_aac_controls_match_independent_nonzero_pulse_pcm() {
    for i in 0..3 {
        let frames = collect(&mut open(&format!("control-{i}")));
        assert_eq!(frames.len(), 3);
        let actual: Vec<u8> = frames.into_iter().flat_map(|f| f.3).collect();
        let expected = bytes(&format!("control-{i}.f32le"));
        assert_eq!(actual.len(), expected.len());
        let mut peak = 0f32;
        for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            peak = peak.max(b.abs());
            assert!((a - b).abs() <= 1e-6, "control{i}: {a} != {b}");
        }
        assert!(peak > 1e-4, "oracle must exercise overlap/history");
    }
}
fn document() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn expected(case: &serde_json::Value, target_ms: u64) -> transitions::Frames {
    let mut controls = std::collections::HashMap::new();
    let mut result = Vec::new();
    for p in case["parts"].as_array().unwrap() {
        let rate = p["rate"].as_u64().unwrap();
        let ch = p["channels"].as_u64().unwrap();
        let stride = ch as usize * 4;
        let first = p["first"].as_u64().unwrap() as usize;
        let last = p["last"].as_u64().unwrap() as usize;
        let pcm = if let Some(id) = p["control"].as_u64() {
            let data = controls
                .entry(id)
                .or_insert_with(|| collect(&mut open(&format!("control-{id}"))));
            data[p["index"].as_u64().unwrap() as usize].3[first * stride..last * stride].to_vec()
        } else {
            vec![0; (last - first) * stride]
        };
        let n = u128::from(p["pts"][0].as_u64().unwrap());
        let d = u128::from(p["pts"][1].as_u64().unwrap());
        let target = u128::from(target_ms) * d;
        let skip = if target <= n * 1000 {
            0
        } else {
            ((target - n * 1000) * u128::from(rate)).div_ceil(d * 1000) as usize
        };
        if skip >= last - first {
            continue;
        }
        let at = (n * u128::from(rate) + skip as u128 * d) * 1_000_000_000 / (d * u128::from(rate));
        result.push((
            rate as u32,
            ch as u16,
            std::time::Duration::from_nanos(at as u64),
            pcm[skip * stride..].to_vec(),
        ));
    }
    result
}
#[test]
fn aac_profile_frame_geometry_and_rate_changes_match_exact_control_history() {
    use fvid::audio::AudioStream;
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut source = open(name);
        transitions::compare(&collect(&mut source), &expected(case, 0));
        source.rewind();
        transitions::compare(&collect(&mut source), &expected(case, 0));
        assert_eq!(
            source.duration(),
            Some(std::time::Duration::from_nanos(
                case["duration"][0].as_u64().unwrap() * 1_000_000_000
                    / case["duration"][1].as_u64().unwrap()
            ))
        );
    }
}

#[test]
fn aac_direct_seek_matches_exact_continuous_suffix_across_960_1024_boundaries() {
    use fvid::audio::AudioStream;
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut source = open(name);
        for ms in [
            0, 5, 20, 21, 63, 64, 93, 94, 123, 124, 153, 154, 165, 166, 172, 199, 250, 331, 500,
        ] {
            source.seek_to(ms * 14112);
            let mut decoder = source.make_decoder().unwrap();
            source.reset_decoder(&mut decoder).unwrap();
            let actual = collect(&mut source);
            let wanted = expected(case, ms as u64);
            assert_eq!(actual.len(), wanted.len(), "{name}/seek{ms}: packet count");
            transitions::compare(&actual, &wanted);
        }
    }
}
#[test]
fn aac_workers_recreate_devices_and_keep_preroll_after_seek_and_rewind() {
    use fvid::audio_thread::AudioPlayback;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use transitions::{Capture, Trace, ended};
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let mut audio =
            AudioPlayback::start(Box::new(open(name)), move || Box::new(Capture::new(shared)));
        audio.play();
        ended(&audio, 0);
        transitions::compare(&trace.lock().unwrap().packets, &expected(case, 0));
        trace.lock().unwrap().packets.clear();
        audio.rewind();
        audio.play();
        ended(&audio, audio.generation());
        transitions::compare(&trace.lock().unwrap().packets, &expected(case, 0));
        for ms in [20, 64, 94, 124, 154, 165, 172, 250, 500] {
            trace.lock().unwrap().packets.clear();
            audio.seek(Duration::from_millis(ms));
            ended(&audio, audio.generation());
            let actual = trace.lock().unwrap().packets.clone();
            let wanted = expected(case, ms);
            assert_eq!(
                actual.len(),
                wanted.len(),
                "{name}/worker seek{ms}: packet count"
            );
            transitions::compare(&actual, &wanted);
        }
    }
}

#[test]
fn aac_seek_preroll_starts_at_the_last_real_decoder_setup_boundary() {
    use fvid::audio::{AudioStep, AudioStream};
    for name in ["changes", "third", "fragmented", "equivalent"] {
        let mut source = open(name);
        for (ms, first_pts) in [
            (94, 903168),
            (199, 2173248),
            (250, 3156288),
            (42, 0),
            (21, 0),
        ] {
            source.seek_to(ms * 14112);
            let mut decoder = source.make_decoder().unwrap();
            source.reset_decoder(&mut decoder).unwrap();
            match source.next_step().unwrap().unwrap() {
                AudioStep::Encoded(p) => assert_eq!(
                    p.pts, first_pts,
                    "{name}/seek{ms}: replay only the selected setup history"
                ),
                _ => panic!("seek must negotiate the selected setup before reading its preroll"),
            }
        }
    }
}

#[test]
fn aac_owned_video_companions_keep_exact_planes_and_clocks() {
    use fvid::playback_native::{NativeReader, RawFrame};
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut video =
            NativeReader::software(Cursor::new(bytes(&format!("{name}.mp4"))), 32 << 20).unwrap();
        for i in 0..16 {
            match video.read_frame_raw().unwrap().unwrap() {
                RawFrame::Planar(p) => {
                    assert_eq!((p.depth, p.frame.width, p.frame.height), (8, 16, 16));
                    assert_eq!(p.frame.data, vec![128; 384]);
                }
                _ => panic!("owned VP9 must retain exact planes"),
            }
            assert_eq!(video.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(video.read_frame_raw().unwrap().is_none());
    }
}
