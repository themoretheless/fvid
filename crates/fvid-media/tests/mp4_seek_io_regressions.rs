use fvid_media::owned_mp4::{Limits, Mp4Reader};
use std::{
    io::{self, Cursor},
    sync::atomic::Ordering,
};
#[path = "../../../tests/support/mp4_seek_fault.rs"]
mod seek_fault;
const VIDEO: &[u8] =
    include_bytes!("../../../tests/fixtures/playback-errors/mp4-seek-io/three-colors.mp4");
const PACKETS: [&[u8]; 3] = [
    include_bytes!("../../../tests/fixtures/playback-errors/mp4-seek-io/frame-0.packet"),
    include_bytes!("../../../tests/fixtures/playback-errors/mp4-seek-io/frame-1.packet"),
    include_bytes!("../../../tests/fixtures/playback-errors/mp4-seek-io/frame-2.packet"),
];
#[test]
fn control_samples_are_contiguous_distinct_and_exact() {
    let mut reader = Mp4Reader::open(Cursor::new(VIDEO), Limits::default()).unwrap();
    assert_eq!(reader.tracks()[0].samples.len(), 3);
    let mut out = Vec::new();
    for (i, expected) in PACKETS.iter().enumerate() {
        reader.read_packet(0, i, &mut out).unwrap();
        assert_eq!(&out, expected);
        if i > 0 {
            let previous = reader.tracks()[0].samples.get(i - 1).unwrap();
            let current = reader.tracks()[0].samples.get(i).unwrap();
            assert_eq!(previous.offset + u64::from(previous.size), current.offset);
        }
    }
    assert_ne!(PACKETS[1], PACKETS[2]);
}
#[test]
fn a_failed_seek_invalidates_the_cached_cursor_before_recovery() {
    for kind in [io::ErrorKind::Other, io::ErrorKind::PermissionDenied] {
        let source = seek_fault::SeekFault::new(VIDEO, kind);
        let armed = source.armed.clone();
        let fired = source.fired.clone();
        let mut reader = Mp4Reader::open(source, Limits::default()).unwrap();
        let mut out = Vec::new();
        reader.read_packet(0, 0, &mut out).unwrap();
        assert_eq!(out, PACKETS[0]);
        armed.store(true, Ordering::Relaxed);
        let error = reader.read_packet(0, 2, &mut out).unwrap_err();
        assert_eq!(error.to_string(), "synthetic seek moved before failing");
        assert!(fired.load(Ordering::Relaxed));
        assert!(
            out.is_empty(),
            "failed seeks must not expose a packet-shaped output"
        );
        reader.read_packet(0, 1, &mut out).unwrap();
        assert_eq!(
            out, PACKETS[1],
            "a seek error can move the source; cached position must become unknown"
        );
        reader.read_packet(0, 2, &mut out).unwrap();
        assert_eq!(out, PACKETS[2]);
        reader.read_packet(0, 0, &mut out).unwrap();
        assert_eq!(out, PACKETS[0]);
    }
}
