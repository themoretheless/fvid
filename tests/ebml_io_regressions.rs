use fvid::container::webm::{Limits, WebmReader};
use fvid::playback_webm::WebmVideoReader;
use std::{
    io::{self, Cursor},
    sync::{Arc, atomic::Ordering},
};
#[path = "support/io_fault.rs"]
mod io_fault;
use io_fault::FaultReader;

const VIDEO: &[u8] = include_bytes!("fixtures/playback-errors/ebml-interrupted-skip.webm");
const MANIFEST: &str = include_str!("fixtures/playback-errors/ebml-interrupted-skip.json");

fn source(kind: io::ErrorKind, max_read: usize) -> FaultReader {
    let manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
    FaultReader::new(
        VIDEO,
        manifest["interrupt_at"].as_u64().unwrap(),
        kind,
        max_read,
    )
}

#[test]
fn control_is_a_decodable_three_frame_video_with_exact_timestamps() {
    let mut demux = WebmReader::open(Cursor::new(VIDEO), Limits::default()).unwrap();
    demux.scan_all().unwrap();
    assert_eq!(demux.tracks[0].codec, "V_VP9");
    assert_eq!(demux.tracks[0].visible(), (64, 64));
    assert_eq!(
        demux.packets.iter().map(|p| p.pts_ns).collect::<Vec<_>>(),
        [0, 20_000_000, 40_000_000]
    );
    let expected = demux.read_packet(0).unwrap();
    for i in 1..3 {
        assert_eq!(demux.read_packet(i).unwrap(), expected);
    }
    let mut playback = WebmVideoReader::open_software(Cursor::new(VIDEO), 32 << 20).unwrap();
    let mut count = 0;
    while playback.read_frame().unwrap() {
        assert_eq!(playback.dimensions(), [64, 64]);
        assert!(playback.rgb().iter().all(|&sample| sample == 130));
        count += 1;
    }
    assert_eq!(count, 3);
}

#[test]
fn interrupted_skip_retries_without_losing_packets_or_pixels() {
    for max_read in [1, 7, usize::MAX] {
        let source = source(io::ErrorKind::Interrupted, max_read);
        let fired = Arc::clone(&source.fired);
        let mut playback = WebmVideoReader::open_software(source, 32 << 20).unwrap();
        let mut control = WebmVideoReader::open_software(Cursor::new(VIDEO), 32 << 20).unwrap();
        for _ in 0..3 {
            assert!(playback.read_frame().unwrap());
            assert!(control.read_frame().unwrap());
            assert_eq!(playback.rgb(), control.rgb());
            assert_eq!(playback.frame_interval(), control.frame_interval());
        }
        assert!(!playback.read_frame().unwrap());
        playback.rewind();
        assert!(playback.read_frame().unwrap());
        playback.seek_to_frame(40_000_000).unwrap();
        control.seek_to_frame(40_000_000).unwrap();
        assert_eq!(playback.rgb(), control.rgb());
        assert_eq!(playback.frame_interval(), control.frame_interval());
        assert!(!playback.read_frame().unwrap());
        assert!(fired.load(Ordering::Relaxed), "fault was not exercised");
    }
}

#[test]
fn non_retryable_io_failure_is_preserved() {
    let source = source(io::ErrorKind::PermissionDenied, 7);
    let fired = Arc::clone(&source.fired);
    let error = match WebmReader::open(source, Limits::default()) {
        Ok(_) => panic!("permission failure must stop traversal"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "synthetic container I/O fault");
    assert!(fired.load(Ordering::Relaxed));
}
