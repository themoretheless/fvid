use fvid::container::mp4::{Limits, Mp4Reader};
use std::{
    io::{self, Cursor},
    sync::atomic::Ordering,
};
#[path = "support/mp4_seek_fault.rs"]
mod seek_fault;
const VIDEO: &[u8] = include_bytes!("fixtures/playback-errors/mp4-seek-io/three-colors.mp4");
const PACKETS: [&[u8]; 3] = [
    include_bytes!("fixtures/playback-errors/mp4-seek-io/frame-0.packet"),
    include_bytes!("fixtures/playback-errors/mp4-seek-io/frame-1.packet"),
    include_bytes!("fixtures/playback-errors/mp4-seek-io/frame-2.packet"),
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

#[test]
fn authored_colors_play_with_exact_samples_timestamps_seek_rewind_and_eof() {
    use fvid::playback_mp4::Mp4VideoReader;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-seek-io");
    let mut reader =
        Mp4VideoReader::open_software(Cursor::new(VIDEO), Limits::default(), 32 << 20).unwrap();
    let mut previous = None;
    for _ in 0..2 {
        for index in 0..3 {
            let frame = reader.read_frame().unwrap().unwrap();
            let picture = &frame.picture;
            let bytes: Vec<u8> = picture
                .y
                .iter()
                .chain(&picture.cb)
                .chain(&picture.cr)
                .flat_map(|value| value.to_le_bytes())
                .collect();
            assert_eq!(
                bytes,
                std::fs::read(root.join(format!("frame-{index}.yuv16"))).unwrap()
            );
            if let Some(previous) = previous.as_ref() {
                if index > 0 {
                    assert_ne!(&bytes, previous);
                }
            }
            previous = Some(bytes);
            assert_eq!(frame.presentation_time.ticks, index as i64);
            assert_eq!(frame.presentation_time.timescale, 50);
            assert_eq!(frame.duration.ticks, 1);
        }
        assert!(reader.read_frame().unwrap().is_none());
        reader.rewind();
    }
    assert_eq!(reader.seek_to_sync(2), 2);
    assert_eq!(reader.read_frame().unwrap().unwrap().sample_index, 2);
    assert!(reader.read_frame().unwrap().is_none());
}
