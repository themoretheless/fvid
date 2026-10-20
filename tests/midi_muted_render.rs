#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(bytes: Vec<u8>) -> fvid::playback_smf::SmfAudioReader {
    fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default()).unwrap()
}
fn render(bytes: Vec<u8>, channels: u16) -> Vec<(Duration, Vec<u8>)> {
    let mut source = open(bytes);
    let mut engine = fvid::codec::midi_decoder::MidiDecoder::new(44100, channels).unwrap();
    let mut packets = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = engine
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        assert_eq!(f.packet.data.len(), 4410 * channels as usize * 4);
        assert_eq!(
            f.packet.presentation_time(),
            Duration::from_millis(packets.len() as u64 * 100)
        );
        assert!(f.packet.data.chunks_exact(4).all(|s| {
            let v = f32::from_le_bytes(s.try_into().unwrap());
            v.is_finite() && v.abs() <= 1.0
        }));
        packets.push((f.packet.presentation_time(), f.packet.data));
    }
    assert_eq!(packets.len(), 5);
    packets
}
fn zero(p: &(Duration, Vec<u8>)) -> bool {
    p.1.chunks_exact(4)
        .all(|s| f32::from_le_bytes(s.try_into().unwrap()) == 0.0)
}
#[test]
fn muting_keeps_native_midi_phase_envelopes_and_channel_isolation() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/midi-muted-render");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let bytes = std::fs::read(base.join(name)).unwrap();
        let control = std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();
        let mut stereo = Vec::new();
        for channels in [1, 2] {
            let actual = render(bytes.clone(), channels);
            let expected = render(control.clone(), channels);
            assert!(
                expected
                    .iter()
                    .flat_map(|(_, p)| p.chunks_exact(4))
                    .any(|s| f32::from_le_bytes(s.try_into().unwrap()).abs() > 0.001),
                "{name}: audible reference"
            );
            match case["mode"].as_str().unwrap() {
                "silent" => assert!(actual.iter().all(zero), "{name}: silence/release"),
                "resume" => {
                    assert!(actual[..2].iter().all(zero), "{name}: muted interval");
                    assert!(
                        actual[2..] == expected[2..],
                        "{name}: phase/envelope changed under mute"
                    );
                }
                "fractional" => {
                    let edge =
                        case["edge_frame"].as_u64().unwrap() as usize * channels as usize * 4;
                    assert!(
                        actual[0].1[..edge] == expected[0].1[..edge],
                        "{name}: before mute"
                    );
                    assert!(
                        actual[0].1[edge..]
                            .chunks_exact(4)
                            .all(|s| f32::from_le_bytes(s.try_into().unwrap()) == 0.0)
                    );
                    assert!(zero(&actual[1]));
                    assert!(
                        actual[2].1[..edge]
                            .chunks_exact(4)
                            .all(|s| f32::from_le_bytes(s.try_into().unwrap()) == 0.0)
                    );
                    assert!(
                        actual[2].1[edge..] == expected[2].1[edge..],
                        "{name}: fractional unmute phase"
                    );
                    assert!(actual[3..] == expected[3..]);
                }
                "reference" => assert!(
                    actual == expected,
                    "{name}: muted channels changed other voices"
                ),
                _ => unreachable!(),
            }
            if channels == 2 {
                stereo = actual;
            }
        }
        for target in [Duration::from_millis(100), Duration::from_millis(500)] {
            capture::worker_check_with_seek(Box::new(open(bytes.clone())), &stereo, Some(target));
        }
    }
}
