#![cfg(feature = "player")]
use std::{io::Cursor, path::Path};
fn data(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-seek-reordered-coverage")
            .join(n),
    )
    .unwrap()
}
fn reader(n: &str) -> fvid::playback_mp4::Mp4VideoReader<Cursor<Vec<u8>>> {
    fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(data(n)),
        Default::default(),
        32 << 20,
    )
    .unwrap()
}
fn frames(
    r: &mut fvid::playback_mp4::Mp4VideoReader<Cursor<Vec<u8>>>,
) -> Vec<(i64, i64, usize, Vec<u8>)> {
    let mut out = Vec::new();
    while let Some(f) = r.read_frame().unwrap() {
        let p: Vec<u8> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|x| x.to_le_bytes())
            .collect();
        assert_eq!(p, data(&format!("frame-{}.yuv16", f.sample_index % 3)));
        out.push((
            f.presentation_time.ticks,
            f.duration.ticks,
            f.sample_index,
            p,
        ));
    }
    assert!(r.read_frame().unwrap().is_none());
    out
}
fn check(n: &str, target: i64) {
    let mut r = reader(n);
    let sequential = frames(&mut r);
    assert_eq!(sequential.len(), 5);
    let landing = r.seek_to_sync(target);
    let actual = frames(&mut r);
    let expected: Vec<_> = sequential.into_iter().filter(|f| f.0 >= landing).collect();
    assert_eq!(
        actual, expected,
        "lost sequential coverage after landing {landing} for target {target}"
    );
}
#[test]
fn ordered_sync_seek_keeps_sequential_suffix() {
    check("ordered.mp4", 3);
}
#[test]
fn reordered_sync_seek_keeps_every_frame_after_returned_landing() {
    check("reordered.mp4", 10);
}
