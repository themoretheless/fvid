use fvid_media::owned_webm::{Limits, WebmReader};
use std::{fs, io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/matroska-empty-pose")
}
#[test]
fn empty_projection_angles_use_zero_defaults_and_preserve_codec_packets() {
    let mut control = WebmReader::open(Cursor::new(fs::read(root().join("control.mkv")).unwrap()), Limits::default()).unwrap();
    control.scan_all().unwrap();
    for name in ["yaw.mkv", "pitch.mkv", "roll.mkv", "all.mkv"] {
        let mut reader = WebmReader::open(Cursor::new(fs::read(root().join(name)).unwrap()), Limits::default()).unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[0].rotation, 0);
        assert_eq!(reader.packets.len(),3);
        for index in 0..3 {
            assert_eq!(reader.packets[index].pts_ns,control.packets[index].pts_ns);
            assert_eq!(reader.read_packet(index).unwrap(),control.read_packet(index).unwrap());
        }
    }
}
#[test]
fn one_byte_projection_float_remains_invalid() {
    let error = match WebmReader::open(Cursor::new(fs::read(root().join("invalid-one-byte.mkv")).unwrap()), Limits::default()) { Ok(_) => panic!("invalid float accepted"), Err(e) => e };
    assert_eq!(error.to_string(),"invalid WebM float size");
}
