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
            .join("tests/fixtures/playback-errors/avi-seek-index")
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
    let n = c["frames"].as_u64().unwrap() as usize;
    if first == n {
        return vec![];
    }
    let channels = c["channels"].as_u64().unwrap() as u16;
    vec![Packet {
        rate: 48000,
        channels,
        num: u128::from(c["start"].as_u64().unwrap()) + first as u128,
        den: 48000,
        data: bytes(c["oracle"].as_str().unwrap())[first * usize::from(channels) * 4..].to_vec(),
    }]
}
#[test]
fn all_indexed_records_preserve_independent_pcm_and_native_clocks() {
    for c in cases() {
        let b = bytes(c["file"].as_str().unwrap());
        let parsed = fvid::container::avi::Avi::parse(&b, &Default::default()).unwrap();
        assert_eq!(
            parsed.audio()[0].records.len(),
            c["records"].as_u64().unwrap() as usize
        );
        let mut r = open(&c);
        exact(
            &collect(&mut r),
            &oracle(&c, 0),
            c["file"].as_str().unwrap(),
        );
        assert!(r.next_packet().unwrap().is_none());
        r.rewind();
        let p = r.next_packet().unwrap().unwrap();
        assert_eq!(p.pts, c["start"].as_i64().unwrap());
    }
    let mut v =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes("video.mkv")), 32 << 20)
            .unwrap();
    assert!(v.read_frame_raw().unwrap().is_some());
    assert!(v.read_frame_raw().unwrap().is_none());
}
#[test]
fn dense_and_irregular_seek_landings_match_known_record_intervals() {
    for c in cases() {
        let start = c["start"].as_i64().unwrap();
        let n = c["frames"].as_i64().unwrap();
        let end = start + n;
        let channels = c["channels"].as_u64().unwrap() as usize;
        let gold = bytes(c["oracle"].as_str().unwrap());
        let mut r = open(&c);
        let mut targets = vec![
            i64::MIN,
            -1,
            0,
            start,
            start + 1,
            start + 2,
            start + 15,
            start + 16,
            start + 17,
            start + 1023,
            start + 1024,
            start + 1025,
            start + 8191,
            start + 32767,
            end - 1,
            end,
            i64::MAX,
        ];
        targets.extend((0..128).map(|i| start + (i * 104729) % (n + 1)));
        for target in targets {
            let at = target.clamp(start, end);
            assert_eq!(r.seek_to(target), at);
            let packet = r.next_packet().unwrap();
            if at == end {
                assert!(packet.is_none());
                continue;
            }
            let p = packet.unwrap();
            assert_eq!(p.pts, at);
            let mut frontier = start;
            let expected = c["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| {
                    frontier += n.as_i64().unwrap();
                    frontier
                })
                .find(|&end| end > at)
                .unwrap();
            assert_eq!(p.duration, (expected - at).min(2048));
            let mut decoder = r.make_decoder().unwrap();
            let pcm = decoder
                .decode_encoded(&p.data, p.pts as u64, p.duration as u64)
                .unwrap()
                .unwrap();
            assert_eq!(
                (pcm.pts, pcm.timebase_num, pcm.timebase_den),
                (at as u64, 1, 48000)
            );
            let from = (at - start) as usize * channels * 4;
            assert_eq!(
                pcm.data,
                gold[from..from + p.duration as usize * channels * 4]
            );
        }
        for at in [start, start + 1, start + 1024, end, end + 1] {
            let time = rational::duration(at as u128, 48000);
            assert_eq!(r.seek_time(time), at.min(end));
        }
        assert_eq!(r.seek_time(Duration::MAX), end);
        assert!(r.next_packet().unwrap().is_none());
    }
}
#[test]
fn real_workers_seek_to_exact_native_tails_without_changing_packet_admission() {
    use fvid::audio_thread::AudioPlayback;
    use transitions::{Capture, Trace, ended};
    for c in cases()
        .into_iter()
        .filter(|c| c["records"].as_u64().unwrap() > 1024)
    {
        let first = c["frames"].as_u64().unwrap() as usize - 7;
        let time = rational::duration(
            u128::from(c["start"].as_u64().unwrap()) + first as u128,
            48000,
        );
        let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
        let captured = raw.clone();
        let trace = Arc::new(Mutex::new(Trace::default()));
        let mut worker = AudioPlayback::start(Box::new(open(&c)), move || {
            Box::new(rational::RawCapture {
                inner: Capture::new(trace),
                raw: captured,
            })
        });
        worker.seek(time);
        worker.play();
        ended(&worker, worker.generation());
        let got = raw.lock().unwrap();
        exact(
            &got.packets,
            &oracle(&c, first),
            c["file"].as_str().unwrap(),
        );
        assert_eq!(got.anchors, vec![time]);
        let b = bytes(c["file"].as_str().unwrap());
        let limits = fvid::container::avi::Limits {
            records: 16,
            ..Default::default()
        };
        assert!(AviAudioReader::open(b.as_slice(), limits).is_err());
    }
}
