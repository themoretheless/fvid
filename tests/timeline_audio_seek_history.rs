#![cfg(feature = "player")]
use fvid::audio::AudioStream;
#[path = "support/audio_capture.rs"]
mod capture;
use std::{io::Cursor, path::Path};
fn open(kind: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match kind {
        "smf" => Box::new(
            fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "xm" => Box::new(
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
        ),
        _ => unreachable!(),
    }
}
fn check(kind: &str, targets: &[u64]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/timeline-audio-seek-history");
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let bytes = std::fs::read(root.join(if kind == "smf" {
        "midi/two-notes.mid"
    } else {
        "xm/two-notes.xm"
    }))
    .unwrap();
    let mut source = open(kind, bytes.clone());
    let mut decoder = source.make_decoder().unwrap();
    let mut expected = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        assert_eq!(f.packet.presentation_time(), source.time_of(p.pts));
        expected.push((f.packet.presentation_time(), f.packet.data));
    }
    for target in targets {
        capture::worker_check_with_seek(
            open(kind, bytes.clone()),
            &expected,
            Some(std::time::Duration::from_millis(*target)),
        );
    }
}
#[test]
fn midi_seek_restores_held_notes_and_synth_phase() {
    check("smf", &[500, 1500, 2100]);
}
#[test]
fn xm_seek_restores_active_samples_and_tracker_state() {
    check("xm", &[120, 240, 360]);
}
