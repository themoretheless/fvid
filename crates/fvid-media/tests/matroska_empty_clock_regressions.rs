use fvid_media::owned_webm::{Limits, WebmReader};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/matroska-empty-clock")
}
#[test]
fn clock_defaults_preserve_exact_index_and_stored_packet_bytes() {
    let mut control = WebmReader::open(
        Cursor::new(fs::read(root().join("control.mkv")).unwrap()),
        Limits::default(),
    )
    .unwrap();
    control.scan_all().unwrap();
    let packets: Vec<_> = (0..3).map(|i| control.read_packet(i).unwrap()).collect();
    for name in ["segment", "track", "both"] {
        let mut reader = WebmReader::open(
            Cursor::new(fs::read(root().join(format!("{name}.mkv"))).unwrap()),
            Limits::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.duration_ns, Some(55_000_000));
        assert_eq!(reader.timestamp_scale_ns(), 1_000_000);
        assert_eq!(reader.packets.len(), 3);
        for (i, pts) in [0, 10_000_000, 50_000_000].into_iter().enumerate() {
            assert_eq!(reader.packets[i].pts_ns, pts);
            assert_eq!(reader.packets[i].duration_ns, Some(5_000_000));
            assert_eq!(reader.read_packet(i).unwrap(), packets[i]);
        }
    }
}
#[test]
fn defaulted_clocks_do_not_bypass_index_resource_limits() {
    let data = fs::read(root().join("both.mkv")).unwrap();
    for limits in [
        Limits {
            packet_bytes: 1,
            ..Default::default()
        },
        Limits {
            packets: 1,
            ..Default::default()
        },
        Limits {
            elements: 1,
            ..Default::default()
        },
    ] {
        assert!(WebmReader::open(Cursor::new(&data), limits).is_err());
    }
}
