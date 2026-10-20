use std::{
    io::{self, BufReader, Cursor},
    sync::{Arc, atomic::Ordering},
};
#[path = "support/io_fault.rs"]
mod io_fault;
use io_fault::FaultReader;

const VIDEO: &[u8] = include_bytes!("fixtures/playback-errors/y4m-interrupted-header.y4m");
const MANIFEST: &str = include_str!("fixtures/playback-errors/y4m-interrupted-header.json");

#[test]
fn interrupted_header_and_frame_marker_keep_every_frame_and_byte() {
    let manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
    for position in manifest["interrupt_at"].as_array().unwrap() {
        let at = position.as_u64().unwrap();
        for chunk in [1, 7, usize::MAX] {
            let reader = FaultReader::new(VIDEO, at, io::ErrorKind::Interrupted, chunk);
            let fired = Arc::clone(&reader.fired);
            let mut out = Vec::new();
            let stats = fvid::process(
                BufReader::new(reader),
                &mut out,
                fvid::Transform::default(),
                1024,
            )
            .unwrap();
            assert_eq!(stats.frames, 3);
            assert_eq!(out, VIDEO);
            assert!(
                fired.load(Ordering::Relaxed),
                "fault at {at} was not exercised"
            );

            let reader = FaultReader::new(VIDEO, at, io::ErrorKind::Interrupted, chunk);
            let fired = Arc::clone(&reader.fired);
            let stats =
                fvid_media::owned_y4m_decode::decode_reader(BufReader::new(reader)).unwrap();
            assert_eq!(stats.video_frames, 3);
            assert_eq!((stats.width, stats.height), (2, 2));
            assert!(fired.load(Ordering::Relaxed));
        }
    }
}

#[test]
fn valid_control_decodes_and_permanent_header_fault_is_not_retried() {
    let stats = fvid_media::owned_y4m_decode::decode_reader(Cursor::new(VIDEO)).unwrap();
    assert_eq!(stats.video_frames, 3);
    let mut out = Vec::new();
    fvid::process(
        Cursor::new(VIDEO),
        &mut out,
        fvid::Transform::default(),
        1024,
    )
    .unwrap();
    assert_eq!(out, VIDEO);
    let source = FaultReader::new(VIDEO, 8, io::ErrorKind::PermissionDenied, 7);
    let fired = Arc::clone(&source.fired);
    let error = fvid::process(
        BufReader::new(source),
        Vec::new(),
        fvid::Transform::default(),
        1024,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "synthetic container I/O fault");
    assert!(fired.load(Ordering::Relaxed));
}
