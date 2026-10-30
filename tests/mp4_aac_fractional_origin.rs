#![cfg(feature = "player")]
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_transitions.rs"]
mod transitions;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-aac-fractional-origin")
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
fn control() -> Vec<u8> {
    let mut s =
        Mp4AudioReader::open(Cursor::new(bytes("control.mp4")), Default::default()).unwrap();
    let pcm = transitions::collect(&mut s)
        .iter()
        .flat_map(|f| f.3.iter().copied())
        .collect::<Vec<_>>();
    let reference = bytes("reference.f32le");
    assert_eq!(pcm.len(), 3072 * 4);
    assert_eq!(pcm.len(), reference.len());
    for (a, b) in pcm.chunks_exact(4).zip(reference.chunks_exact(4)) {
        assert!(
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                <= 1e-7
        );
    }
    pcm
}
#[test]
fn fractional_composition_points_keep_exact_pcm_and_do_not_round_into_negative_time() {
    let golden = control();
    let mut wrong = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let shift = c["shift"].as_i64().unwrap();
        let factor = c["factor"].as_i64().unwrap();
        let scale = c["scale"].as_u64().unwrap();
        let drop = c["drop"].as_u64().unwrap() as usize;
        let mut s = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        assert_eq!(
            s.duration(),
            Some(Duration::from_nanos(
                ((3072 * factor + shift) as u64) * 1_000_000_000 / scale
            )),
            "{name}"
        );
        for _ in 0..2 {
            let got = transitions::collect(&mut s);
            let actual = got
                .iter()
                .flat_map(|f| f.3.iter().copied())
                .collect::<Vec<_>>();
            if actual != golden[drop * 4..] {
                wrong.push(format!(
                    "{name}: {} samples instead of {}",
                    actual.len() / 4,
                    3072 - drop
                ));
            }
            for (i, f) in got.iter().enumerate() {
                let first = if i == 0 { drop } else { i * 1024 };
                let expected = Duration::from_nanos(
                    ((first as i64 * factor + shift) as u64) * 1_000_000_000 / scale,
                );
                if f.2 != expected {
                    wrong.push(format!(
                        "{name} packet{i}: {:?} instead of {expected:?}",
                        f.2
                    ));
                }
            }
            s.rewind();
        }
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        for _ in 0..3 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
    }
    assert!(wrong.is_empty(), "fractional AAC origins: {wrong:?}");
}
#[test]
fn seeks_to_fractional_native_points_preserve_exact_suffixes() {
    let golden = control();
    let mut wrong = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let shift = c["shift"].as_i64().unwrap();
        let factor = c["factor"].as_i64().unwrap();
        let scale = c["scale"].as_u64().unwrap();
        let mut s = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        for source in [3, 1024, 3000, 3071, 3072] {
            for time in [false, true] {
                let target = source * factor + shift;
                let at = Duration::from_nanos(target as u64 * 1_000_000_000 / scale);
                if time {
                    s.seek_time(at);
                } else {
                    s.seek_to(target);
                }
                let got = transitions::collect(&mut s);
                let actual = got
                    .iter()
                    .flat_map(|f| f.3.iter().copied())
                    .collect::<Vec<_>>();
                if actual != golden[source as usize * 4..] {
                    wrong.push(format!(
                        "{name} source{source} time{time}: {} samples instead of {}",
                        actual.len() / 4,
                        3072 - source
                    ));
                }
                if let Some(f) = got.first() {
                    if f.2 != at {
                        wrong.push(format!(
                            "{name} source{source}: {:?} instead of {at:?}",
                            f.2
                        ));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "fractional AAC seeks: {wrong:?}");
}

#[test]
fn the_audio_worker_keeps_fractional_origins_and_seek_pcm() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    let golden = control();
    for (name, factor, shift, drop) in [
        ("factor4-shift-5.mp4", 4, -5, 2),
        ("factor4-shift-1.mp4", 4, -1, 1),
        ("factor4-shift1.mp4", 4, 1, 0),
        ("factor65537-shift1.mp4", 65537, 1, 0),
    ] {
        let scale = 48000u64 * factor as u64;
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let captured = trace.clone();
        let stream = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        let mut audio = AudioPlayback::start(Box::new(stream), move || {
            Box::new(transitions::Capture::new(captured))
        });
        audio.play();
        transitions::ended(&audio, 0);
        let frames = trace.lock().unwrap().packets.clone();
        let actual = frames
            .iter()
            .flat_map(|f| f.3.iter().copied())
            .collect::<Vec<_>>();
        assert!(
            actual == golden[drop * 4..],
            "{name}: worker kept {} rather than {} full samples",
            actual.len() / 4,
            3072 - drop
        );
        let first =
            Duration::from_nanos((drop as i64 * factor + shift) as u64 * 1_000_000_000 / scale);
        assert_eq!(frames[0].2, first, "{name}");
        trace.lock().unwrap().packets.clear();
        let target = Duration::from_nanos((3000 * factor + shift) as u64 * 1_000_000_000 / scale);
        audio.seek(target);
        audio.play();
        transitions::ended(&audio, audio.generation());
        let frames = trace.lock().unwrap().packets.clone();
        let actual = frames
            .iter()
            .flat_map(|f| f.3.iter().copied())
            .collect::<Vec<_>>();
        assert!(
            actual == golden[3000 * 4..],
            "{name}: worker seek returned {} rather than72 samples",
            actual.len() / 4
        );
        assert_eq!(frames[0].2, target);
        trace.lock().unwrap().packets.clear();
        audio.seek(Duration::from_millis(65));
        audio.play();
        transitions::ended(&audio, audio.generation());
        assert!(
            trace.lock().unwrap().packets.is_empty(),
            "{name}: seek beyond EOF replayed audio"
        );
    }
}
