#![cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, playback_avi_audio::AviAudioReader};
use rational::{Packet, collect, exact};
use std::{
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avi-g711-windows")
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
fn open(c: &serde_json::Value) -> AviAudioReader {
    let b = bytes(c["file"].as_str().unwrap());
    AviAudioReader::open(b.as_slice(), Default::default()).unwrap()
}
fn oracle(c: &serde_json::Value, first: usize) -> Vec<Packet> {
    let channels = c["channels"].as_u64().unwrap() as u16;
    let n = c["frames"].as_u64().unwrap() as usize;
    if first == n {
        return vec![];
    }
    vec![Packet {
        rate: c["rate"].as_u64().unwrap() as u32,
        channels,
        num: u128::from(c["start"].as_u64().unwrap()) + first as u128,
        den: u128::from(c["rate"].as_u64().unwrap()),
        data: bytes(c["oracle"].as_str().unwrap())[first * usize::from(channels) * 4..].to_vec(),
    }]
}
#[test]
fn whole_chunk_pcm_native_clock_seek_and_eof_controls() {
    for c in cases() {
        let mut r = open(&c);
        let name = c["file"].as_str().unwrap();
        let start = c["start"].as_i64().unwrap();
        let n = c["frames"].as_i64().unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        exact(&collect(&mut r), &oracle(&c, 0), name);
        assert_eq!(
            r.duration(),
            Some(rational::duration((start + n) as u128, u128::from(rate)))
        );
        for target in [
            -1,
            0,
            start,
            start + 1,
            start + 2047,
            start + 2048,
            start + 2049,
            start + n - 1,
            start + n,
            i64::MAX,
        ] {
            let at = target.clamp(start, start + n);
            assert_eq!(r.seek_to(target), at);
            exact(&collect(&mut r), &oracle(&c, (at - start) as usize), name);
        }
        for first in [0, 1, 2047, 2048, 2049, n as usize] {
            let target = rational::duration((start + first as i64) as u128, u128::from(rate));
            let landed = r.seek_time(target);
            assert_eq!(landed, start + first as i64);
            exact(&collect(&mut r), &oracle(&c, first), name);
        }
        r.rewind();
        exact(&collect(&mut r), &oracle(&c, 0), name);
        assert!(r.next_packet().unwrap().is_none());
    }
    let mut v =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes("video.mkv")), 32 << 20)
            .unwrap();
    assert!(v.read_frame_raw().unwrap().is_some());
    assert!(v.read_frame_raw().unwrap().is_none());
}
#[test]
fn encoded_and_decoded_live_packet_buffers_are_bounded() {
    let mut failures = Vec::new();
    for c in cases() {
        let mut r = open(&c);
        let mut d = r.make_decoder().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let start = c["start"].as_i64().unwrap();
        let mut cursor = start;
        let (mut peak_encoded, mut peak_live, mut packets) = (0, 0, 0);
        while let Some(p) = r.next_packet().unwrap() {
            assert_eq!(p.pts, cursor);
            assert_eq!(p.data.len(), p.duration as usize * channels);
            let pcm = d
                .decode_encoded(&p.data, p.pts as u64, p.duration as u64)
                .unwrap()
                .unwrap();
            assert_eq!(pcm.data.len(), p.data.len() * 4);
            peak_encoded = peak_encoded.max(p.data.capacity());
            peak_live = peak_live.max(p.data.capacity() + pcm.data.capacity());
            packets += 1;
            cursor += p.duration;
            if p.duration > 2048 {
                failures.push(format!("{}: {} frames", c["file"], p.duration));
            }
        }
        assert_eq!(cursor, start + c["frames"].as_i64().unwrap());
        println!(
            "MEMORY {}",
            serde_json::json!({"file":c["file"],"channels":channels,"packets":packets,"peak_encoded_capacity":peak_encoded,"peak_live_packet_capacity":peak_live})
        );
    }
    assert!(
        failures.is_empty(),
        "unbounded G711 record copies: {failures:?}"
    );
}
#[test]
fn actual_workers_preserve_pcm_and_native_seek_anchors() {
    use fvid::audio_thread::AudioPlayback;
    use transitions::{Capture, Trace, ended};
    for c in cases() {
        let first = 2048usize;
        let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
        let captured = raw.clone();
        let trace = Arc::new(Mutex::new(Trace::default()));
        let mut worker = AudioPlayback::start(Box::new(open(&c)), move || {
            Box::new(rational::RawCapture {
                inner: Capture::new(trace),
                raw: captured,
            })
        });
        let at = rational::duration(
            u128::from(c["start"].as_u64().unwrap()) + first as u128,
            u128::from(c["rate"].as_u64().unwrap()),
        );
        worker.seek(at);
        worker.play();
        ended(&worker, worker.generation());
        let got = raw.lock().unwrap();
        exact(
            &got.packets,
            &oracle(&c, first),
            c["file"].as_str().unwrap(),
        );
        assert_eq!(got.anchors, vec![at]);
    }
}
#[test]
fn full_file_resource_admission_remains_strict() {
    for c in cases() {
        let b = bytes(c["file"].as_str().unwrap());
        let limits = fvid::container::avi::Limits {
            file_bytes: b.len() - 1,
            ..Default::default()
        };
        let e = AviAudioReader::open(b.as_slice(), limits)
            .err()
            .expect("the complete AVI must remain charged");
        assert!(e.to_string().contains("file is over"), "{e}");
    }
}
