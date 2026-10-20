#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn midi_render_workloads_keep_pcm_timestamps_and_navigation() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/midi-render-bench");
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
        let mut source = fvid::playback_smf::SmfAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(source.duration(), Some(Duration::from_millis(500)));
        let mut decoder = source.make_decoder().unwrap();
        let mut expected = Vec::new();
        let mut peak = 0.0f32;
        while let Some(p) = source.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            assert_eq!(f.packet.data.len(), 4410 * 2 * 4);
            assert_eq!(
                f.packet.presentation_time(),
                Duration::from_millis(expected.len() as u64 * 100)
            );
            for sample in f.packet.data.chunks_exact(4) {
                let value = f32::from_le_bytes(sample.try_into().unwrap());
                assert!(value.is_finite() && value.abs() <= 1.0);
                peak = peak.max(value.abs());
            }
            expected.push((f.packet.presentation_time(), f.packet.data));
        }
        assert_eq!(expected.len(), 5);
        assert!(peak > 0.001, "{name}: audible workloadcontrol");
        for target in [Duration::from_millis(100), Duration::from_millis(500)] {
            capture::worker_check_with_seek(
                Box::new(
                    fvid::playback_smf::SmfAudioReader::open(
                        Cursor::new(bytes.clone()),
                        Default::default(),
                    )
                    .unwrap(),
                ),
                &expected,
                Some(target),
            );
        }
    }
}
