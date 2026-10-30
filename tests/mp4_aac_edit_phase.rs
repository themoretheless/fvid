#![cfg(feature = "player")]
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_transitions.rs"]
mod transitions;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-aac-edit-phase")
            .join(n),
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
    assert_eq!(pcm.len(), reference.len());
    assert_eq!(pcm.len(), 3072 * 4);
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
fn finite_aac_edits_keep_exact_source_windows_and_their_presentation_phase() {
    let golden = control();
    let mut wrong = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let factor = c["factor"].as_i64().unwrap();
        let start = c["start"].as_i64().unwrap();
        let shift = c["shift"].as_i64().unwrap();
        let first = c["first"].as_u64().unwrap() as usize;
        let last = c["last"].as_u64().unwrap() as usize;
        let mut s = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        assert_eq!(
            s.duration(),
            Some(Duration::from_nanos(2048 * 1_000_000_000 / 48000))
        );
        for _ in 0..2 {
            let got = transitions::collect(&mut s);
            let actual = got
                .iter()
                .flat_map(|f| f.3.iter().copied())
                .collect::<Vec<_>>();
            if actual != golden[first * 4..last * 4] {
                wrong.push(format!(
                    "{name}: wrong source window, {} samples instead of{}",
                    actual.len() / 4,
                    last - first
                ));
            }
            let mut source = first;
            for f in &got {
                let expected = Duration::from_nanos(
                    (source as i64 * factor + shift - start) as u64 * 1_000_000_000
                        / (48000 * factor as u64),
                );
                if f.2 != expected {
                    wrong.push(format!(
                        "{name} at source{source}: {:?} instead of{expected:?}",
                        f.2
                    ));
                }
                source += f.3.len() / 4;
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
    assert!(wrong.is_empty(), "finite AAC edit phase: {wrong:?}");
}
#[test]
fn finite_edit_seek_matches_the_continuous_source_suffix() {
    let golden = control();
    let mut wrong = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let factor = c["factor"].as_i64().unwrap();
        let shift = c["shift"].as_i64().unwrap();
        let start = c["start"].as_i64().unwrap();
        let last = c["last"].as_u64().unwrap() as usize;
        let mut s = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        for sample in [0, 123, 1023, 2047, 2048] {
            s.seek_to(sample * factor);
            let got = transitions::collect(&mut s);
            let first = ((start + sample * factor - shift + factor - 1) / factor).max(0) as usize;
            let actual = got
                .iter()
                .flat_map(|f| f.3.iter().copied())
                .collect::<Vec<_>>();
            let expected = if first >= last {
                &[][..]
            } else {
                &golden[first * 4..last * 4]
            };
            if actual != expected {
                wrong.push(format!("{name} seek{sample}: wrong PCM suffix"));
            }
            if let Some(f) = got.first() {
                let at = Duration::from_nanos(
                    (first as i64 * factor + shift - start) as u64 * 1_000_000_000
                        / (48000 * factor as u64),
                );
                if f.2 != at {
                    wrong.push(format!("{name} seek{sample}: {:?} instead of{at:?}", f.2));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "finite AAC edit seeks: {wrong:?}");
}
#[test]
fn the_audio_worker_keeps_finite_edit_pcm_phase_and_seek() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    let golden = control();
    for c in cases()
        .into_iter()
        .filter(|c| c["factor"] == 4 && (c["start"] == 1 || c["shift"] == 0))
    {
        let name = c["file"].as_str().unwrap();
        let first = c["first"].as_u64().unwrap() as usize;
        let last = c["last"].as_u64().unwrap() as usize;
        let phase = c["phase"].as_u64().unwrap();
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let captured = trace.clone();
        let s = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        let mut audio = AudioPlayback::start(Box::new(s), move || {
            Box::new(transitions::Capture::new(captured))
        });
        audio.play();
        transitions::ended(&audio, 0);
        let packets = trace.lock().unwrap().packets.clone();
        let actual = packets
            .iter()
            .flat_map(|f| f.3.iter().copied())
            .collect::<Vec<_>>();
        assert!(
            actual == golden[first * 4..last * 4],
            "{name}: worker finite-edit PCM"
        );
        assert_eq!(
            packets[0].2,
            Duration::from_nanos(phase * 1_000_000_000 / 192000),
            "{name}"
        );
        trace.lock().unwrap().packets.clear();
        audio.seek(Duration::from_millis(20));
        audio.play();
        transitions::ended(&audio, audio.generation());
        let actual = trace
            .lock()
            .unwrap()
            .packets
            .iter()
            .flat_map(|f| f.3.iter().copied())
            .collect::<Vec<_>>();
        assert!(
            actual == golden[(first + 960) * 4..last * 4],
            "{name}: worker seek source suffix"
        );
    }
}
