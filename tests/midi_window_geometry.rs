#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream, AudioTrack, EncodedPacket};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
struct Inject {
    source: fvid::playback_smf::SmfAudioReader,
    duration: i64,
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
                p.duration = self.duration;
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
fn midi_window_geometry_refuses_before_allocation_or_synth_state_changes() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/midi-window-geometry");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let bytes = std::fs::read(base.join("control.mid")).unwrap();
    let mut source =
        fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes.clone()), Default::default())
            .unwrap();
    let mut decoder = source.make_decoder().unwrap();
    let mut pcm = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        pcm.push((f.packet.presentation_time(), f.packet.data));
    }
    capture::worker_check_with_seek(
        Box::new(
            fvid::playback_smf::SmfAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap(),
        ),
        &pcm,
        Some(Duration::from_millis(300)),
    );
    let packet = std::fs::read(base.join("note.packet")).unwrap();
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap() as u32;
        let channels = case["channels"].as_u64().unwrap() as u16;
        let duration = case["duration"].as_u64().unwrap();
        let mut engine = fvid::codec::midi_decoder::MidiDecoder::new(rate, channels).unwrap();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.decode_packet(&packet, 0, duration)
        })) {
            Err(_) => failures.push(format!("{name}: publicdecoderpanicked")),
            Ok(Ok(_)) => failures.push(format!("{name}: unaddressablePCMaccepted")),
            Ok(Err(e)) => assert!(
                e.to_string().contains("MIDI PCM window"),
                "{name}: unrelatedrefusal{e}"
            ),
        }
        if rate <= 44100 {
            let after = engine.decode_packet(&packet, 0, 100).unwrap().unwrap();
            let mut fresh = fvid::codec::midi_decoder::MidiDecoder::new(rate, channels).unwrap();
            let expected = fresh.decode_packet(&packet, 0, 100).unwrap().unwrap();
            assert_eq!(
                after.packet.data, expected.packet.data,
                "{name}: rejectionmutatedsynth"
            );
            assert_eq!(
                after.packet.data.len(),
                rate as usize / 10 * channels as usize * 4
            );
        }
    }
    if failures.is_empty() {
        capture::worker_refusal(
            Box::new(Inject {
                source: fvid::playback_smf::SmfAudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
                duration: i64::MAX,
                done: false,
            }),
            "MIDI PCM window",
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

#[test]
fn ordinary_long_and_empty_midi_windows_keep_the_existing_pcm_contract() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/midi-window-geometry");
    let packet = std::fs::read(base.join("note.packet")).unwrap();
    let mut long = fvid::codec::midi_decoder::MidiDecoder::new(44100, 2).unwrap();
    let output = long.decode_packet(&packet, 0, 1000).unwrap().unwrap();
    assert_eq!(output.packet.data.len(), 44100 * 2 * 4);
    let mut sliced = fvid::codec::midi_decoder::MidiDecoder::new(44100, 2).unwrap();
    let mut expected = Vec::new();
    for index in 0..10 {
        let data = if index == 0 { packet.as_slice() } else { &[] };
        expected.extend_from_slice(
            &sliced
                .decode_packet(data, index * 100, 100)
                .unwrap()
                .unwrap()
                .packet
                .data,
        );
    }
    assert!(
        output.packet.data == expected,
        "one-second PCM differs from ten continuous native windows"
    );
    let empty = long.decode_packet(&[], 123, 0).unwrap().unwrap();
    assert!(empty.packet.data.is_empty());
    assert_eq!(empty.packet.presentation_time(), Duration::from_millis(123));
}
