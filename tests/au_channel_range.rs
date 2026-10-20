#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(bytes: Vec<u8>, channels: usize) -> fvid::Result<fvid::playback_au::AuAudioReader> {
    fvid::playback_au::AuAudioReader::open(
        Cursor::new(bytes),
        fvid::playback_au::Limits {
            channels,
            ..Default::default()
        },
    )
}
#[test]
fn au_channels_are_checked_before_narrowing_to_the_audio_interface() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/au-channel-range");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let video = std::fs::read(base.join("video.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for (name, channels) in [("mono", 1), ("stereo", 2)] {
        let bytes = std::fs::read(base.join(format!("{name}.au"))).unwrap();
        let expected = std::fs::read(base.join(format!("{name}.f32le"))).unwrap();
        let mut source = open(bytes.clone(), usize::MAX).unwrap();
        assert_eq!(
            (source.channels(), source.duration()),
            (channels, Some(Duration::from_millis(20)))
        );
        let mut decoder = source.make_decoder().unwrap();
        let mut actual = Vec::new();
        let mut intervals = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            actual.extend_from_slice(&frame.packet.data);
            intervals.push((frame.packet.presentation_time(), frame.packet.data));
        }
        assert_eq!(actual, expected);
        capture::worker_check_with_seek(
            Box::new(open(bytes, usize::MAX).unwrap()),
            &intervals,
            Some(Duration::from_millis(20)),
        );
    }
    let mut boundary = open(
        std::fs::read(base.join("representable-max.au")).unwrap(),
        usize::MAX,
    )
    .unwrap();
    assert_eq!(
        (boundary.channels(), boundary.duration()),
        (u16::MAX, Some(Duration::from_nanos(125000)))
    );
    let p = boundary.next_packet().unwrap().unwrap();
    assert_eq!((p.pts, p.duration, p.data.len()), (0, 1, 65535));
    let mut decoder = boundary.make_decoder().unwrap();
    let frame = decoder
        .decode_packet(&p.data, p.pts, p.duration as u64)
        .unwrap()
        .unwrap();
    assert_eq!(frame.packet.data.len(), 65535 * 4);
    assert!(frame.packet.data.iter().all(|&byte| byte == 0));
    assert!(boundary.next_packet().unwrap().is_none());
    let mut wrong = Vec::new();
    for case in manifest["bad"].as_array().unwrap() {
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let channels = case["channels"].as_u64().unwrap();
        let default = match open(bytes.clone(), 32) {
            Err(error) => error,
            Ok(_) => panic!("ordinary channel budget bypassed"),
        };
        assert!(
            default
                .to_string()
                .contains("channel count is out of range")
        );
        match std::panic::catch_unwind(|| open(bytes, usize::MAX)) {
            Ok(Err(error))
                if error
                    .to_string()
                    .contains("AU channel count exceeds the audio interface range") => {}
            Ok(Err(error)) => wrong.push(format!("{channels}: wrong-field refusal {error}")),
            Ok(Ok(source)) => wrong.push(format!(
                "{channels}: aliased to{}channels with duration{:?} for one declared frame",
                source.channels(),
                source.duration()
            )),
            Err(_) => wrong.push(format!("{channels}: narrowed geometry panicked")),
        }
    }
    assert!(
        wrong.is_empty(),
        "u32 AU channel counts were narrowed without admission: {wrong:?}"
    );
}
