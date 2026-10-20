#![cfg(feature = "player")]
use fvid::audio::{AudioStream, AudioTrack, EncodedPacket};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
struct Script {
    packets: Vec<Vec<u8>>,
    index: usize,
}
impl AudioStream for Script {
    fn codec(&self) -> &str {
        "midi"
    }
    fn timescale(&self) -> u32 {
        1000
    }
    fn sample_rate(&self) -> u32 {
        8000
    }
    fn channels(&self) -> u16 {
        1
    }
    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_millis(200))
    }
    fn extra_data(&self) -> &[u8] {
        &[]
    }
    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![]
    }
    fn next_packet(&mut self) -> fvid::Result<Option<EncodedPacket>> {
        let Some(data) = self.packets.get(self.index) else {
            return Ok(None);
        };
        let p = EncodedPacket {
            data: data.clone(),
            pts: self.index as i64 * 100,
            duration: 100,
        };
        self.index += 1;
        Ok(Some(p))
    }
    fn rewind(&mut self) {
        self.index = 0;
    }
    fn seek_to(&mut self, pts: i64) -> i64 {
        self.index = (pts.max(0) as usize / 100).min(2);
        self.index as i64 * 100
    }
}
fn decode(packets: &[Vec<u8>]) -> Vec<(Duration, Vec<u8>)> {
    let mut engine = fvid::codec::make_audio_decoder("midi", &[], 8000, 1, 16).unwrap();
    packets
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let f = engine
                .decode_packet(p, i as i64 * 100, 100)
                .unwrap()
                .unwrap();
            (f.packet.presentation_time(), f.packet.data)
        })
        .collect()
}
#[test]
fn midi_events_never_advance_a_sample_before_their_time_or_disappear_at_window_tail() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/midi-event-sample");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let load = |mode: &str| {
            (0..2)
                .map(|i| std::fs::read(base.join(format!("{name}-{mode}-{i}.packet"))).unwrap())
                .collect::<Vec<_>>()
        };
        let packets = load("actual");
        let expected = decode(&load("control"));
        let actual = decode(&packets);
        if actual != expected {
            failures.push(format!(
                "{name}: PCM differs from first-eligible-sample boundary control"
            ));
        } else {
            capture::worker_check_with_seek(
                Box::new(Script { packets, index: 0 }),
                &expected,
                Some(Duration::from_millis(200)),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
