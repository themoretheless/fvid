#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{
    audio::AudioStream,
    audio_thread::{AudioEvent, AudioPlayback},
    playback_mp4_audio::Mp4AudioReader,
};
use std::{
    collections::HashMap,
    io::Cursor,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use transitions::{Capture, Frames, Trace, collect, compare, ended};
fn read(dir: &str, name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(dir)
            .join(name),
    )
    .unwrap()
}
fn bytes(name: &str) -> Vec<u8> {
    read("mp4-variable-rate-edits", name)
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
fn expected(case: &serde_json::Value, target_us: u64) -> Frames {
    let mut controls = HashMap::<String, Frames>::new();
    let mut out = Vec::new();
    for part in case["parts"].as_array().unwrap() {
        let rate = part["rate"].as_u64().unwrap();
        let channels = part["channels"].as_u64().unwrap();
        let stride = channels as usize * 4;
        let first = part["first"].as_u64().unwrap() as usize;
        let last = part["last"].as_u64().unwrap() as usize;
        let data = match part["oracle"].as_str() {
            None => vec![0; (last - first) * stride],
            Some(name) => {
                let full = if let Some((name, index)) = name.split_once('#') {
                    if !controls.contains_key(name) {
                        let mut control = Mp4AudioReader::open(
                            Cursor::new(read("mp4-audio-reconfiguration", name)),
                            Default::default(),
                        )
                        .unwrap();
                        controls.insert(name.to_string(), collect(&mut control));
                    }
                    controls[name][index.parse::<usize>().unwrap()].3.clone()
                } else {
                    read("mp4-audio-reconfiguration", name)
                };
                full[first * stride..last * stride].to_vec()
            }
        };
        let n = u128::from(part["pts"][0].as_u64().unwrap());
        let d = u128::from(part["pts"][1].as_u64().unwrap());
        // Independent oracle: explicit authored cuts, then retain sample starts
        // at/after the public microsecond seek target. No scheduler code is reused.
        let difference = u128::from(target_us) * d;
        let skip = if difference <= n * 1_000_000 {
            0
        } else {
            ((difference - n * 1_000_000) * u128::from(rate)).div_ceil(d * 1_000_000) as usize
        };
        if skip >= last - first {
            continue;
        }
        let ns = (n * u128::from(rate) + skip as u128 * d) * 1_000_000_000 / (d * u128::from(rate));
        out.push((
            rate as u32,
            channels as u16,
            Duration::from_nanos(ns as u64),
            data[skip * stride..].to_vec(),
        ));
    }
    out
}
#[test]
fn explicit_ranges_negative_pts_and_fractional_clocks_keep_exact_source_samples() {
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut stream = open(name);
        let n = case["duration"][0].as_u64().unwrap();
        let d = case["duration"][1].as_u64().unwrap();
        assert_eq!(
            stream.duration(),
            Some(Duration::from_nanos(n * 1_000_000_000 / d)),
            "{name}"
        );
        compare(&collect(&mut stream), &expected(case, 0));
        stream.rewind();
        compare(&collect(&mut stream), &expected(case, 0));
    }
}
#[test]
fn presentation_seek_matches_continuous_suffix_even_at_fractional_edit_edges() {
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut stream = open(name);
        for us in [
            0, 5_000, 9_999, 10_000, 20_000, 26_999, 27_000, 27_300, 27_667, 40_000, 49_999,
            60_000, 81_999, 95_000, 99_999, 154_000, 400_000,
        ] {
            stream.seek_to(us);
            let mut decoder = stream.make_decoder().unwrap();
            stream.reset_decoder(&mut decoder).unwrap();
            let got = collect(&mut stream);
            let wanted = expected(case, us as u64);
            assert_eq!(got.len(), wanted.len(), "{name}/seek{us}: packet count");
            compare(&got, &wanted);
        }
    }
}
#[test]
fn actual_workers_keep_edit_repeats_device_geometry_rewind_and_seek() {
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let mut audio =
            AudioPlayback::start(Box::new(open(name)), move || Box::new(Capture::new(shared)));
        audio.play();
        ended(&audio, 0);
        compare(&trace.lock().unwrap().packets, &expected(case, 0));
        audio.rewind();
        ended(&audio, audio.generation());
        compare(&trace.lock().unwrap().packets, &expected(case, 0));
        for us in [27_000, 27_300, 49_999, 99_999] {
            audio.seek(Duration::from_micros(us));
            ended(&audio, audio.generation());
            compare(&trace.lock().unwrap().packets, &expected(case, us));
        }
    }
}
#[test]
fn missing_source_extent_fails_after_the_available_pcm_instead_of_false_eof() {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let shared = trace.clone();
    let stream =
        Mp4AudioReader::open(Cursor::new(bytes("past-end.mp4")), Default::default()).unwrap();
    let audio = AudioPlayback::start(Box::new(stream), move || Box::new(Capture::new(shared)));
    audio.play();
    let until = Instant::now() + Duration::from_secs(3);
    'error: loop {
        assert!(Instant::now() < until, "missing source extent did not fail");
        while let Some(event) = audio.poll() {
            match event {
                AudioEvent::Error(e) => {
                    assert!(e.contains("edit extends beyond source packets"), "{e}");
                    break 'error;
                }
                AudioEvent::Ended(_) => panic!("incomplete edit signaled EOF"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut source = Mp4AudioReader::open(
        Cursor::new(read("mp4-audio-reconfiguration", "pcm.mp4")),
        Default::default(),
    )
    .unwrap();
    compare(&trace.lock().unwrap().packets, &collect(&mut source));
}

#[test]
fn edited_audio_keeps_its_owned_video_companion_decodable() {
    use fvid::playback_native::{NativeReader, RawFrame};
    for case in document()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let count = if name.starts_with("eac3") { 17 } else { 6 };
        let mut video =
            NativeReader::software(Cursor::new(bytes(&format!("{name}.mp4"))), 32 << 20).unwrap();
        for i in 0..count {
            match video.read_frame_raw().unwrap().unwrap() {
                RawFrame::Planar(p) => {
                    assert_eq!((p.depth, p.frame.width, p.frame.height), (8, 16, 16));
                    assert!(p.frame.data == vec![128; 384], "{name}/{i}");
                }
                _ => panic!("owned VP9 must retain exact planes"),
            }
            assert_eq!(video.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(video.read_frame_raw().unwrap().is_none());
    }
}
