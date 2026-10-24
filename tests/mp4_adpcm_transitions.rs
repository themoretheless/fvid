#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::playback_mp4_audio::Mp4AudioReader;
use std::io::Cursor;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/mp4-adpcm-transitions/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn document() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn open(name: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(
        Cursor::new(bytes(&format!("{name}.mp4"))),
        Default::default(),
    )
    .unwrap()
}
fn expected(case: &serde_json::Value, target_ms: u64) -> transitions::Frames {
    let mut result = Vec::new();
    for p in case["parts"].as_array().unwrap() {
        let rate = p["rate"].as_u64().unwrap();
        let ch = p["channels"].as_u64().unwrap();
        let stride = ch as usize * 4;
        let first = p["first"].as_u64().unwrap() as usize;
        let last = p["last"].as_u64().unwrap() as usize;
        let data = if let Some(name) = p["oracle"].as_str() {
            bytes(name)[first * stride..last * stride].to_vec()
        } else {
            vec![0; (last - first) * stride]
        };
        let n = u128::from(p["pts"][0].as_u64().unwrap());
        let d = u128::from(p["pts"][1].as_u64().unwrap());
        let skip = if u128::from(target_ms) * d <= n * 1000 {
            0
        } else {
            ((u128::from(target_ms) * d - n * 1000) * u128::from(rate)).div_ceil(d * 1000) as usize
        };
        if skip >= last - first {
            continue;
        }
        let ns = (n * u128::from(rate) + skip as u128 * d) * 1_000_000_000 / (d * u128::from(rate));
        result.push((
            rate as u32,
            ch as u16,
            std::time::Duration::from_nanos(ns as u64),
            data[skip * stride..].to_vec(),
        ));
    }
    result
}
#[test]
fn adpcm_duration_clocks_keep_the_exact_grouped_pcm_and_intentional_trim() {
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut source = open(name);
        let actual = transitions::collect(&mut source);
        let wanted = expected(case, 0);
        assert_eq!(actual.len(), wanted.len(), "{name}");
        transitions::compare(&actual, &wanted);
    }
}

#[test]
fn adpcm_seek_rewind_and_worker_output_preserve_clock_conversion_after_reconfiguration() {
    use fvid::{audio::AudioStream, audio_thread::AudioPlayback};
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use transitions::{Capture, Trace, collect, compare, ended};
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let scale = case["scale"].as_i64().unwrap();
        let mut source = open(name);
        let duration = Duration::from_nanos(
            case["duration"][0].as_u64().unwrap() * 1_000_000_000
                / case["duration"][1].as_u64().unwrap(),
        );
        assert_eq!(source.duration(), Some(duration));
        for ms in [0, 3, 8, 24, 31, 32, 34, 49, 50, 64, 65, 75, 99, 200] {
            source.seek_to(ms * scale / 1000);
            let mut decoder = source.make_decoder().unwrap();
            source.reset_decoder(&mut decoder).unwrap();
            let actual = collect(&mut source);
            let wanted = expected(case, ms as u64);
            assert_eq!(actual.len(), wanted.len(), "{name}/seek{ms}");
            compare(&actual, &wanted);
        }
        source.rewind();
        compare(&collect(&mut source), &expected(case, 0));
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let mut audio =
            AudioPlayback::start(Box::new(open(name)), move || Box::new(Capture::new(shared)));
        audio.play();
        ended(&audio, 0);
        compare(&trace.lock().unwrap().packets, &expected(case, 0));
        trace.lock().unwrap().packets.clear();
        audio.rewind();
        audio.play();
        ended(&audio, audio.generation());
        compare(&trace.lock().unwrap().packets, &expected(case, 0));
        for ms in [8, 32, 50, 64, 200] {
            trace.lock().unwrap().packets.clear();
            audio.seek(Duration::from_millis(ms));
            ended(&audio, audio.generation());
            let actual = trace.lock().unwrap().packets.clone();
            let wanted = expected(case, ms);
            assert_eq!(actual.len(), wanted.len(), "{name}/worker seek{ms}");
            compare(&actual, &wanted);
        }
    }
}

#[test]
fn adpcm_timing_fixtures_retain_their_exact_owned_video_companions() {
    use fvid::playback_native::{NativeReader, RawFrame};
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut video =
            NativeReader::software(Cursor::new(bytes(&format!("{name}.mp4"))), 32 << 20).unwrap();
        for i in 0..6 {
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

#[test]
fn constant_adpcm_exports_keep_pcm_with_a_non_native_mp4_duration_clock() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-adpcm-transitions");
    for case in document()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["name"].as_str().unwrap().starts_with("control-"))
    {
        let name = case["name"].as_str().unwrap();
        let destination = std::env::temp_dir().join(format!(
            "fvid-adpcm-clock-{}-{name}.f32le",
            std::process::id()
        ));
        let source = root.join(format!("{name}.mp4"));
        assert!(fvid::native_media::is_owned_audio_source(&source).unwrap());
        let info = fvid::native_media::audio_source_info_selected(&source, None).unwrap();
        assert_eq!(
            info.codec,
            match name {
                "control-0" => "adpcm_ima_qt",
                "control-1" => "adpcm_ima_wav",
                _ => "adpcm_ms",
            }
        );
        let full: Vec<u8> = expected(case, 0).into_iter().flat_map(|f| f.3).collect();
        let part = &case["parts"][0];
        let rate = part["rate"].as_u64().unwrap();
        let channels = part["channels"].as_u64().unwrap();
        for interval in [
            None,
            Some((
                std::time::Duration::from_millis(3),
                std::time::Duration::from_millis(14),
            )),
        ] {
            let stats = fvid::native_export::export_audio_pcm_selected(
                &source,
                &destination,
                interval,
                1.0,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let actual = std::fs::read(&destination).unwrap();
            std::fs::remove_file(&destination).unwrap();
            let stride = channels as usize * 4;
            let wanted = if interval.is_some() {
                let first = (3 * rate).div_ceil(1000) as usize;
                let last = (14 * rate).div_ceil(1000) as usize;
                &full[first * stride..last * stride]
            } else {
                &full
            };
            assert_eq!(
                actual, wanted,
                "{name}: exported PCM must retain the exact selected window"
            );
            assert_eq!(
                (stats.sample_rate, stats.channels),
                (rate as u32, channels as u16)
            );
            assert_eq!(stats.sample_frames, wanted.len() as u64 / (channels * 4));
        }
    }
}
