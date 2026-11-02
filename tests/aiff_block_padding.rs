#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"] mod transitions;
#[path = "support/rational_audio_seek.rs"] mod rational;
use fvid::{audio::AudioStream, playback_aiff::{AiffAudioReader, Limits}};
use std::{io::Cursor, time::Duration, sync::{Arc, Mutex}};
use rational::{Packet, collect, exact};
fn root() -> std::path::PathBuf { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aiff-block-padding") }
fn bytes(name: &str) -> Vec<u8> { std::fs::read(root().join(name)).unwrap() }
fn cases() -> Vec<serde_json::Value> { serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"].as_array().unwrap().clone() }
fn open(name: &str) -> AiffAudioReader { let data = bytes(name); AiffAudioReader::open(data.as_slice(), Limits::default()).unwrap() }
fn expected(c: &serde_json::Value, first: usize) -> Vec<Packet> {
    let data = bytes(c["oracle"].as_str().unwrap()); let channels = c["channels"].as_u64().unwrap() as u16;
    if first == 2051 { return vec![]; }
    vec![Packet { rate: 8000, channels, num: first as u128, den: 8000, data: data[first*usize::from(channels)*4..].to_vec() }]
}
#[test]
fn unaligned_audio_and_neutral_video_controls() {
    for c in cases().into_iter().filter(|c| c["offset"] == 0) {
        let kind = c["oracle"].as_str().unwrap().strip_suffix(".f32le").unwrap();
        exact(&collect(&mut open(&format!("{kind}-control.aiff"))), &expected(&c, 0), kind);
    }
    let mut video = fvid::playback_native::NativeReader::software(Cursor::new(bytes("neutral-video.mp4")), 32<<20).unwrap();
    for _ in 0..6 { assert_eq!(video.read_frame_raw().unwrap().unwrap().into_rgb(32<<20).unwrap(), vec![130; 16*16*3]); }
    assert!(video.read_frame_raw().unwrap().is_none());
}
#[test]
fn aligned_sound_data_padding_does_not_play_or_extend_seek_duration() {
    for c in cases() {
        let name = c["file"].as_str().unwrap(); let mut reader = open(name);
        assert_eq!(reader.aiff().frames, 2051, "{name}: padding is not audio");
        assert_eq!(reader.duration(), Some(Duration::from_micros(2051*125)), "{name}");
        exact(&collect(&mut reader), &expected(&c, 0), name);
        for target in [-1, 0, 1, 2047, 2048, 2050, 2051, i64::MAX] {
            let first = target.clamp(0,2051) as usize;
            assert_eq!(reader.seek_to(target), first as i64, "{name}:raw seek");
            exact(&collect(&mut reader), &expected(&c, first), name);
        }
        for target in [Duration::from_micros(125), Duration::from_micros(2048*125), Duration::from_micros(2051*125), Duration::MAX] {
            let first = (target.as_nanos().div_ceil(125000)).min(2051) as usize;
            let at = reader.seek_time(target); assert_eq!(reader.time_of(at), Duration::from_micros(first as u64*125));
            exact(&collect(&mut reader), &expected(&c, first), name);
        }
        reader.rewind(); exact(&collect(&mut reader), &expected(&c,0), "rewind");
    }
}
#[test]
fn actual_worker_stops_at_declared_audio_end_after_native_seek() {
    use fvid::audio_thread::AudioPlayback;
    use transitions::{Capture, Trace, ended};
    for c in cases() {
        let raw = Arc::new(Mutex::new(rational::RawTrace::default())); let captured = raw.clone();
        let trace = Arc::new(Mutex::new(Trace::default()));
        let mut worker = AudioPlayback::start(Box::new(open(c["file"].as_str().unwrap())), move || Box::new(rational::RawCapture { inner: Capture::new(trace), raw: captured }));
        worker.seek(Duration::from_micros(2048*125)); worker.play(); ended(&worker, worker.generation());
        let got = raw.lock().unwrap(); exact(&got.packets, &expected(&c,2048), c["file"].as_str().unwrap());
        assert_eq!(got.anchors, vec![Duration::from_micros(2048*125)]);
    }
}
#[test]
fn truncated_frames_and_full_sound_data_budget_remain_strict() {
    let mut reader = open("truncated-declared.aiff"); while reader.next_packet().unwrap().is_some() {}
    assert!(reader.validate_eof().unwrap_err().to_string().contains("payload is truncated"));
    let data = bytes("incomplete-frame.aiff");
    let error = AiffAudioReader::open(data.as_slice(), Limits::default()).err().expect("partial channel frame must refuse");
    assert!(error.to_string().contains("incomplete channel frame"), "{error}");
    for c in cases() {
        let data = bytes(c["file"].as_str().unwrap()); let mut limits = Limits::default();
        limits.data_bytes = c["coded_bytes"].as_u64().unwrap() as usize;
        let error = AiffAudioReader::open(data.as_slice(), limits).err().expect("all payload including padding remains charged");
        assert!(error.to_string().contains("sample run is over"), "{error}");
    }
}
