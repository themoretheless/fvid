#![cfg(feature = "player")]
use fvid::audio::{AudioStream, AudioTrack, EncodedPacket};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
struct Inject {
    source: fvid::playback_smf::SmfAudioReader,
    packet: Vec<u8>,
    done: bool,
}
impl AudioStream for Inject {
    fn codec(&self) -> &str {
        self.source.codec()
    }
    fn timescale(&self) -> u32 {
        self.source.timescale()
    }
    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.source.channels()
    }
    fn duration(&self) -> Option<Duration> {
        self.source.duration()
    }
    fn extra_data(&self) -> &[u8] {
        self.source.extra_data()
    }
    fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.source.audio_tracks()
    }
    fn next_packet(&mut self) -> fvid::Result<Option<EncodedPacket>> {
        let mut p = self.source.next_packet()?;
        if !self.done {
            if let Some(p) = &mut p {
                p.data = self.packet.clone();
                self.done = true;
            }
        }
        Ok(p)
    }
    fn rewind(&mut self) {
        self.source.rewind();
        self.done = false;
    }
    fn seek_to(&mut self, pts: i64) -> i64 {
        self.done = false;
        self.source.seek_to(pts)
    }
}
#[test]
fn native_midi_windows_refuse_out_of_window_or_descending_event_offsets() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/midi-window-timing");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let source = std::fs::read(base.join("control.mid")).unwrap();
    let mut native =
        fvid::playback_smf::SmfAudioReader::open(Cursor::new(source.clone()), Default::default())
            .unwrap();
    let mut native_decoder = native.make_decoder().unwrap();
    let mut expected = Vec::new();
    while let Some(p) = native.next_packet().unwrap() {
        let f = native_decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        expected.push((f.packet.presentation_time(), f.packet.data));
    }
    capture::worker_check_with_seek(
        Box::new(
            fvid::playback_smf::SmfAudioReader::open(
                Cursor::new(source.clone()),
                Default::default(),
            )
            .unwrap(),
        ),
        &expected,
        Some(Duration::from_millis(300)),
    );
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let packet = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let mut engine = fvid::codec::make_audio_decoder("midi", &[], 44100, 2, 16).unwrap();
        let result = engine.decode_packet(&packet, 0, 100);
        if case["bad"].as_bool().unwrap() {
            let needle = if name == "descending" {
                "event offsets are not ordered"
            } else {
                "event offset exceeds its window"
            };
            match result {
                Ok(_) => failures.push(format!("{name}: malformed window accepted as PCM")),
                Err(e) => {
                    assert!(e.to_string().contains(needle), "{name}: {e}");
                    let normal = std::fs::read(base.join("normal.packet")).unwrap();
                    let after = engine.decode_packet(&normal, 0, 100).unwrap().unwrap();
                    let mut fresh =
                        fvid::codec::make_audio_decoder("midi", &[], 44100, 2, 16).unwrap();
                    assert_eq!(
                        after.packet.data,
                        fresh
                            .decode_packet(&normal, 0, 100)
                            .unwrap()
                            .unwrap()
                            .packet
                            .data,
                        "{name}: rejection mutated synth state"
                    );
                    capture::worker_refusal(
                        Box::new(Inject {
                            source: fvid::playback_smf::SmfAudioReader::open(
                                Cursor::new(source.clone()),
                                Default::default(),
                            )
                            .unwrap(),
                            packet,
                            done: false,
                        }),
                        needle,
                    );
                }
            }
        } else {
            let f = result.unwrap().unwrap();
            assert_eq!(f.packet.data.len(), 4410 * 2 * 4);
            assert_eq!(f.packet.presentation_time(), Duration::ZERO);
            assert!(f
                .packet
                .data
                .chunks_exact(4)
                .all(|p| f32::from_le_bytes(p.try_into().unwrap()).is_finite()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
