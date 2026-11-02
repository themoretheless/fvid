#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, audio_thread::AudioPlayback, playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor, sync::{Arc, Mutex}, time::Duration};
use transitions::{Capture, Frames, Trace, collect, ended};
fn open(name: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    let bytes = std::fs::read(format!("{}/tests/fixtures/playback-errors/mp4-constant-edit-subtick-seek/{name}.mp4", env!("CARGO_MANIFEST_DIR"))).unwrap();
    Mp4AudioReader::open(Cursor::new(bytes), Default::default()).unwrap()
}
fn check(name: &str, target: Duration, frames: &Frames, rate: u32, channels: u16) {
    let end = u128::from(rate) * 21 / 1000;
    let mut cursor = (target.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000).min(end);
    for packet in frames {
        assert_eq!((packet.0, packet.1), (rate, channels), "{name}:{target:?}");
        // Compare the actual packet Duration directly. Never normalize it to
        // a sample index before comparison: that could hide a nanosecond error.
        assert_eq!(packet.2, Duration::from_nanos((cursor * 1_000_000_000 / u128::from(rate)) as u64), "{name}:{target:?}:packet stamp");
        let stride = usize::from(channels) * 4;
        assert!(!packet.3.is_empty() && packet.3.len().is_multiple_of(stride));
        cursor += (packet.3.len() / stride) as u128;
        assert!(cursor <= end, "{name}:{target:?}:extra samples");
    }
    assert_eq!(cursor, end, "{name}:{target:?}:missing samples");
}
#[test]
fn direct_and_actual_worker_packet_stamps_match_the_integer_native_cursor() {
    let cases = [("ima4",8000,1),("ima-wav",16000,2),("ms-adpcm",8000,1),("ac3",48000,6),("ac3-grouped",48000,6),("ac3-one-packet",48000,6),("eac3-48000",48000,2),("eac3-32000",32000,1)];
    let mut targets: Vec<_> = [0,1,125001,2999999,3000001,10999999,11000001,12999999,13000001,20999999,21000000,21000001].into_iter().map(Duration::from_nanos).collect();
    targets.push(Duration::MAX);
    for (name,rate,channels) in cases {
        let mut reader = open(name);
        check(name, Duration::ZERO, &collect(&mut reader), rate, channels);
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let mut worker = AudioPlayback::start(Box::new(open(name)), move || Box::new(Capture::new(shared)));
        worker.play(); ended(&worker, 0);
        check(name, Duration::ZERO, &trace.lock().unwrap().packets, rate, channels);
        for &target in &targets {
            reader.seek_time(target);
            check(name, target, &collect(&mut reader), rate, channels);
            worker.seek(target); ended(&worker, worker.generation());
            check(name, target, &trace.lock().unwrap().packets, rate, channels);
        }
    }
}
