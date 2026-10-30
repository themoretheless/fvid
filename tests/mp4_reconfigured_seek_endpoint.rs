#![cfg(feature = "player")]
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_transitions.rs"]
mod transitions;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-reconfigured-seek-endpoint")
            .join(name),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn presentation_tail_seeks_preserve_the_exact_pcm_suffix() {
    let mut wrong = Vec::new();
    for case in cases() {
        let name = case["file"].as_str().unwrap();
        let shift = case["shift_ticks"].as_i64().unwrap();
        let end = case["end_ticks"].as_u64().unwrap();
        let golden = bytes(case["oracle"].as_str().unwrap());
        let mut stream =
            Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        assert_eq!(
            stream.duration(),
            Some(Duration::from_micros(end)),
            "{name}"
        );
        for source_sample in [0, 160, 800, 920, 959, 960] {
            let target = shift + source_sample * 125;
            if target < 0 {
                continue;
            }
            stream.seek_time(Duration::from_micros(target as u64));
            let got = transitions::collect(&mut stream);
            let pcm: Vec<_> = got.iter().flat_map(|f| f.3.iter().copied()).collect();
            let expected = &golden[source_sample as usize * 4..];
            if pcm != expected {
                wrong.push(format!(
                    "{name} seek {target}us: {} bytes instead of {}",
                    pcm.len(),
                    expected.len()
                ));
            }
            if let Some(first) = got.first() {
                if first.2 != Duration::from_micros(target as u64) {
                    wrong.push(format!("{name}: wrong first PTS {:?} at {target}", first.2));
                }
            }
            assert!(
                stream.next_step().unwrap().is_none(),
                "{name}: EOF must persist"
            );
            stream.rewind();
        }
    }
    assert!(
        wrong.is_empty(),
        "presentation tail seek failures: {wrong:?}"
    );
}
#[test]
fn ordinary_playback_rewind_and_video_are_valid_controls() {
    for case in cases() {
        let name = case["file"].as_str().unwrap();
        let shift = case["shift_ticks"].as_i64().unwrap();
        let mut stream =
            Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        let golden = bytes(case["oracle"].as_str().unwrap());
        let drop = (-shift).max(0) as usize / 125 * 4;
        for _ in 0..2 {
            let got = transitions::collect(&mut stream);
            assert_eq!(
                got.iter()
                    .flat_map(|f| f.3.iter().copied())
                    .collect::<Vec<_>>(),
                golden[drop..],
                "{name}"
            );
            stream.rewind();
        }
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        let mut count = 0;
        while video.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 6, "{name}");
    }
}
#[test]
fn aac_preroll_seeks_keep_exact_history_and_the_presentation_tail() {
    let mut control =
        Mp4AudioReader::open(Cursor::new(bytes("aac-control.mp4")), Default::default()).unwrap();
    let golden = transitions::collect(&mut control)
        .iter()
        .flat_map(|f| f.3.iter().copied())
        .collect::<Vec<_>>();
    let reference = bytes("aac-reference.f32le");
    assert_eq!(golden.len(), 3072 * 4);
    assert_eq!(golden.len(), reference.len());
    for (a, b) in golden.chunks_exact(4).zip(reference.chunks_exact(4)) {
        assert!(
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                <= 1e-7
        );
    }
    let m: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    let mut wrong = Vec::new();
    for c in m["aac_cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let shift = c["shift_ticks"].as_i64().unwrap();
        let mut stream =
            Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        let full = transitions::collect(&mut stream)
            .iter()
            .flat_map(|f| f.3.iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(
            full,
            golden[(-shift).max(0) as usize * 4..],
            "{name}: full AAC presentation must keep its tail"
        );
        for source in [480, 1024, 2048, 3000, 3071, 3072] {
            let target = source + shift;
            assert!(target >= 0);
            stream.seek_to(target);
            let got = transitions::collect(&mut stream);
            let pcm = got
                .iter()
                .flat_map(|f| f.3.iter().copied())
                .collect::<Vec<_>>();
            if pcm != golden[source as usize * 4..] {
                wrong.push(format!(
                    "{name} source{source}: {} bytes instead of {}",
                    pcm.len(),
                    golden.len() - source as usize * 4
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "AAC presentation seek failures: {wrong:?}"
    );
}
#[test]
fn the_audio_worker_seeks_the_shifted_tail_and_ends_without_replaying_it() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    for (name, target) in [
        ("changing-regular-10000.mp4", 125000),
        ("changing-fragmented-10000.mp4", 125000),
        ("changing-regular-200000.mp4", 315000),
    ] {
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let captured = trace.clone();
        let stream = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        let mut audio = AudioPlayback::start(Box::new(stream), move || {
            Box::new(transitions::Capture::new(captured))
        });
        audio.play();
        transitions::ended(&audio, 0);
        trace.lock().unwrap().packets.clear();
        audio.seek(Duration::from_micros(target));
        audio.play();
        transitions::ended(&audio, audio.generation());
        let frames = trace.lock().unwrap().packets.clone();
        assert_eq!(
            frames
                .iter()
                .flat_map(|f| f.3.iter().copied())
                .collect::<Vec<_>>(),
            bytes("changing.f32le")[920 * 4..],
            "{name}: worker seek suffix"
        );
        assert_eq!(frames[0].2, Duration::from_micros(target), "{name}");
        trace.lock().unwrap().packets.clear();
        audio.seek(Duration::from_micros(target + 5000));
        audio.play();
        transitions::ended(&audio, audio.generation());
        assert!(
            trace.lock().unwrap().packets.is_empty(),
            "{name}: worker replayed PCM at EOF"
        );
    }
}

#[test]
fn the_audio_worker_keeps_aac_overlap_when_seeking_the_shifted_tail() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    let mut control =
        Mp4AudioReader::open(Cursor::new(bytes("aac-control.mp4")), Default::default()).unwrap();
    let golden = transitions::collect(&mut control)
        .iter()
        .flat_map(|f| f.3.iter().copied())
        .collect::<Vec<_>>();
    for (name, shift) in [("aac-480.mp4", 480u64), ("aac-4800.mp4", 4800)] {
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let captured = trace.clone();
        let stream = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        let mut audio = AudioPlayback::start(Box::new(stream), move || {
            Box::new(transitions::Capture::new(captured))
        });
        audio.play();
        transitions::ended(&audio, 0);
        assert_eq!(
            trace
                .lock()
                .unwrap()
                .packets
                .iter()
                .flat_map(|f| f.3.iter().copied())
                .collect::<Vec<_>>(),
            golden,
            "{name}: full worker PCM"
        );
        trace.lock().unwrap().packets.clear();
        let target = Duration::from_nanos((shift + 3000) * 1_000_000_000 / 48000);
        audio.seek(target);
        audio.play();
        transitions::ended(&audio, audio.generation());
        let frames = trace.lock().unwrap().packets.clone();
        assert_eq!(
            frames
                .iter()
                .flat_map(|f| f.3.iter().copied())
                .collect::<Vec<_>>(),
            golden[3000 * 4..],
            "{name}: worker AAC seek suffix"
        );
        assert_eq!(frames[0].2, target);
    }
}
