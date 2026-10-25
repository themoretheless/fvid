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
        .join("tests/fixtures/playback-errors/mp4-eac3-programme-clock")
}
fn open(name: &str, p: u8) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open_programme(
        Cursor::new(std::fs::read(root().join(name)).unwrap()),
        Default::default(),
        0,
        p,
    )
    .unwrap()
}
#[test]
fn selected_eac3_programme_keeps_native_pcm_and_time_under_coarse_ticks() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let mut controls = std::collections::HashMap::new();
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let p = c["programme"].as_u64().unwrap() as u8;
        eprintln!("{name} programme {p}");
        let control = c["control"].as_str().unwrap();
        let expected = controls
            .entry(control.to_owned())
            .or_insert_with(|| transitions::collect(&mut open(control, 0)));
        let mut source = open(name, p);
        assert_eq!(
            (source.sample_rate(), source.channels()),
            (
                c["rate"].as_u64().unwrap() as u32,
                c["channels"].as_u64().unwrap() as u16
            )
        );
        transitions::compare(&transitions::collect(&mut source), expected);
        assert_eq!(
            source.duration(),
            Some(Duration::from_nanos(
                4608 * 1_000_000_000 / c["rate"].as_u64().unwrap()
            ))
        );
    }
}

fn suffix(
    frames: &transitions::Frames,
    counts: &[serde_json::Value],
    target: Duration,
) -> transitions::Frames {
    let rate = frames[0].0;
    let wanted = (target.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as u64;
    let mut native = 0u64;
    let mut result = Vec::new();
    for ((_, ch, _, pcm), count) in frames.iter().zip(counts) {
        let count = count.as_u64().unwrap();
        let skip = wanted.saturating_sub(native).min(count);
        assert_eq!(pcm.len(), count as usize * usize::from(*ch) * 4);
        if skip < count {
            result.push((
                rate,
                *ch,
                Duration::from_nanos((native + skip) * 1_000_000_000 / u64::from(rate)),
                pcm[skip as usize * usize::from(*ch) * 4..].to_vec(),
            ));
        }
        native += count;
    }
    result
}
#[test]
fn selected_eac3_seek_and_rewind_keep_delayed_native_windows_and_history() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let mut controls = std::collections::HashMap::new();
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let p = c["programme"].as_u64().unwrap() as u8;
        eprintln!("seek {name} p{p}");
        let control = c["control"].as_str().unwrap();
        let expected = controls
            .entry(control.to_owned())
            .or_insert_with(|| transitions::collect(&mut open(control, 0)));
        let counts = c["samples_per_packet"].as_array().unwrap();
        assert_eq!(expected.len(), counts.len());
        let mut source = open(name, p);
        for ns in [1, 22_001_001, 63_000_000, 100_000_000, 150_000_000] {
            let target = Duration::from_nanos(ns);
            let landed = source.seek_time(target);
            let actual = transitions::collect(&mut source);
            transitions::compare(&actual, &suffix(expected, counts, target));
            if let Some(first) = actual.first() {
                assert_eq!(source.time_of(landed), first.2);
            }
        }
        source.rewind();
        transitions::compare(&transitions::collect(&mut source), expected);
    }
}

fn worker(name: &str, p: u8) -> transitions::Frames {
    use std::sync::{Arc, Mutex};
    let trace = Arc::new(Mutex::new(transitions::Trace::default()));
    let shared = trace.clone();
    let audio = fvid::audio_thread::AudioPlayback::start(Box::new(open(name, p)), move || {
        Box::new(transitions::Capture::new(shared))
    });
    audio.play();
    transitions::ended(&audio, 0);
    let result = trace.lock().unwrap().packets.clone();
    result
}
#[test]
fn real_selected_programme_worker_preserves_eof_seek_rewind_and_pcm_geometry() {
    use std::sync::{Arc, Mutex};
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for (name, p) in [
        ("dual-48000-1000-floor.mp4", 1),
        ("heterogeneous-44100-900-nearest.mp4", 7),
        ("expanded-48000-b3-1001-ceil.mp4", 3),
        ("expanded-44100-b6-900-floor.mp4", 7),
        ("51-32000-1001-nearest.mp4", 1),
        ("placeholder-rate-900-ceil.mp4", 7),
        ("heterogeneous-44100-variable-900-nearest.mp4", 7),
    ] {
        eprintln!("worker {name} p{p}");
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["file"] == name && c["programme"] == p)
            .unwrap();
        let expected = worker(c["control"].as_str().unwrap(), 0);
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let shared = trace.clone();
        let mut audio =
            fvid::audio_thread::AudioPlayback::start(Box::new(open(name, p)), move || {
                Box::new(transitions::Capture::new(shared))
            });
        audio.play();
        transitions::ended(&audio, 0);
        transitions::compare(&trace.lock().unwrap().packets, &expected);
        for (i, ns) in [22_001_001, 63_000_000].into_iter().enumerate() {
            let target = Duration::from_nanos(ns);
            audio.seek(target);
            transitions::ended(&audio, i as u64 + 1);
            transitions::compare(
                &trace.lock().unwrap().packets,
                &suffix(
                    &expected,
                    c["samples_per_packet"].as_array().unwrap(),
                    target,
                ),
            );
        }
        audio.rewind();
        transitions::ended(&audio, 3);
        transitions::compare(&trace.lock().unwrap().packets, &expected);
    }
}

#[test]
fn missing_or_corrupt_selected_programme_refuses_without_default_fallback() {
    for (name, p) in [
        ("without-programme7-900.mp4", 7),
        ("corrupt-unselected-p1-900-floor.mp4", 7),
    ] {
        let error = Mp4AudioReader::open_programme(
            Cursor::new(std::fs::read(root().join(name)).unwrap()),
            Default::default(),
            0,
            p,
        )
        .err()
        .expect("invalid selection must refuse");
        assert!(
            error
                .to_string()
                .contains("selected E-AC-3 programme is missing or corrupt"),
            "{name}: {error}"
        );
    }
}
