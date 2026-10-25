#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-ac3-sample-clock")
}
fn open(name: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(
        Cursor::new(std::fs::read(root().join(name)).unwrap()),
        Default::default(),
    )
    .unwrap()
}
#[test]
fn coarse_ac3_packets_keep_native_pcm_timestamps_and_duration() {
    let mut exact = open("exact.mp4");
    let expected = transitions::collect(&mut exact);
    assert_eq!(expected.len(), 3);
    for (i, p) in expected.iter().enumerate() {
        assert_eq!(
            (p.0, p.1, p.2, p.3.len()),
            (48000, 6, Duration::from_millis(i as u64 * 32), 1536 * 6 * 4)
        );
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        eprintln!("case {name}");
        let expected = transitions::collect(&mut open(c["exact"].as_str().unwrap()));
        let counts = c["samples_per_packet"].as_array().unwrap();
        assert_eq!(expected.len(), counts.len());
        let mut native = 0u64;
        for (packet, count) in expected.iter().zip(counts) {
            let count = count.as_u64().unwrap();
            assert_eq!(packet.3.len(), count as usize * 6 * 4);
            assert_eq!(
                packet.2,
                Duration::from_nanos(native * 1_000_000_000 / 48000)
            );
            native += count;
        }
        assert_eq!(native, 4608);
        let mut source = open(name);
        transitions::compare(&transitions::collect(&mut source), &expected);
        assert_eq!(source.duration(), Some(Duration::from_millis(96)), "{name}");
    }
}

fn suffix(frames: &transitions::Frames, target: Duration) -> transitions::Frames {
    frames
        .iter()
        .filter_map(|(rate, ch, at, pcm)| {
            let stride = usize::from(*ch) * 4;
            let skip = target
                .saturating_sub(*at)
                .as_nanos()
                .checked_mul(u128::from(*rate))
                .unwrap()
                .div_ceil(1_000_000_000) as usize;
            if skip >= pcm.len() / stride {
                return None;
            }
            Some((
                *rate,
                *ch,
                *at + Duration::from_nanos((skip as u64 * 1_000_000_000) / u64::from(*rate)),
                pcm[skip * stride..].to_vec(),
            ))
        })
        .collect()
}
#[test]
fn native_ac3_seek_and_rewind_preserve_history_below_a_container_tick() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        eprintln!("seek {name}");
        let expected = transitions::collect(&mut open(c["exact"].as_str().unwrap()));
        let mut source = open(name);
        for ns in [
            1,
            22_001_001,
            32_000_001,
            63_000_000,
            95_999_999,
            100_000_000,
        ] {
            let target = Duration::from_nanos(ns);
            let landed = source.seek_time(target);
            let actual = transitions::collect(&mut source);
            let wanted = suffix(&expected, target);
            transitions::compare(&actual, &wanted);
            if let Some(first) = actual.first() {
                assert_eq!(source.time_of(landed), first.2);
            }
        }
        source.rewind();
        transitions::compare(&transitions::collect(&mut source), &expected);
    }
}

fn worker(name: &str) -> transitions::Frames {
    use std::sync::{Arc, Mutex};
    let trace = Arc::new(Mutex::new(transitions::Trace::default()));
    let shared = trace.clone();
    let audio = fvid::audio_thread::AudioPlayback::start(Box::new(open(name)), move || {
        Box::new(transitions::Capture::new(shared))
    });
    audio.play();
    transitions::ended(&audio, 0);
    let result = trace.lock().unwrap().packets.clone();
    result
}
#[test]
fn real_ac3_worker_seek_after_eof_and_rewind_match_exact_control_pcm() {
    use std::sync::{Arc, Mutex};
    for (name, exact) in [
        ("ac3-1000-floor.mp4", "exact.mp4"),
        ("grouped-900-nearest.mp4", "grouped-exact.mp4"),
        ("bad-first-1001-ceil.mp4", "bad-first-exact.mp4"),
        ("one-packet-900-floor.mp4", "one-packet-exact.mp4"),
        (
            "one-packet-bad-first-1001-nearest.mp4",
            "one-packet-bad-first-exact.mp4",
        ),
    ] {
        eprintln!("worker {name}");
        let expected = worker(exact);
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let shared = trace.clone();
        let mut audio = fvid::audio_thread::AudioPlayback::start(Box::new(open(name)), move || {
            Box::new(transitions::Capture::new(shared))
        });
        audio.play();
        transitions::ended(&audio, 0);
        transitions::compare(&trace.lock().unwrap().packets, &expected);
        for (i, ns) in [22_001_001, 63_000_000].into_iter().enumerate() {
            let target = Duration::from_nanos(ns);
            audio.seek(target);
            transitions::ended(&audio, i as u64 + 1);
            transitions::compare(&trace.lock().unwrap().packets, &suffix(&expected, target));
        }
        audio.rewind();
        transitions::ended(&audio, 3);
        transitions::compare(&trace.lock().unwrap().packets, &expected);
    }
}
