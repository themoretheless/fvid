#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(folder: &str, name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(folder)
            .join(name),
    )
    .unwrap()
}
fn open(folder: &str, name: &str) -> fvid::playback_mp4::Mp4VideoReader<Cursor<Vec<u8>>> {
    fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(folder, name)),
        Default::default(),
        32 << 20,
    )
    .unwrap()
}
type Frames = Vec<(i64, i64, usize, Vec<u8>)>;
fn frames(r: &mut fvid::playback_mp4::Mp4VideoReader<Cursor<Vec<u8>>>) -> Frames {
    let mut out = Vec::new();
    while let Some(f) = r.read_frame().unwrap() {
        let picture = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|p| p.to_le_bytes())
            .collect();
        out.push((
            f.presentation_time.ticks,
            f.duration.ticks,
            f.sample_index,
            picture,
        ));
    }
    assert!(r.read_frame().unwrap().is_none());
    out
}
#[test]
fn ordered_sparse_sync_landings_keep_exact_sequential_suffix() {
    let mut r = open("mp4-seek-reordered-coverage", "ordered.mp4");
    let expected = frames(&mut r);
    assert_eq!(expected.len(), 5);
    for (target, landing) in [
        (i64::MIN, 0),
        (-1, 0),
        (0, 0),
        (1, 0),
        (2, 2),
        (3, 2),
        (4, 4),
        (5, 4),
        (i64::MAX, 4),
    ] {
        assert_eq!(r.seek_to_sync(target), landing);
        let want: Frames = expected
            .iter()
            .filter(|f| f.0 >= landing)
            .cloned()
            .collect();
        assert_eq!(frames(&mut r), want, "target{target}");
    }
}
#[test]
fn reordered_and_equal_pts_keep_coverage_and_duplicate_duration() {
    for (folder, name) in [
        ("mp4-seek-reordered-coverage", "reordered.mp4"),
        (
            "mp4-presentation-endpoint",
            "duplicate-tail-shift-0-unknown.mp4",
        ),
    ] {
        let mut r = open(folder, name);
        let expected = frames(&mut r);
        assert!(!expected.is_empty());
        for target in [i64::MIN, 0, 1, 7, 8, 9, 10, 12, i64::MAX] {
            let landing = r.seek_to_sync(target);
            let want: Frames = expected
                .iter()
                .filter(|f| f.0 >= landing)
                .cloned()
                .collect();
            assert_eq!(frames(&mut r), want, "{name} target{target}");
        }
    }
}
#[test]
fn native_seek_keeps_rgb_intervals_eof_and_index_budget_refusal() {
    let data = bytes("mp4-seek-reordered-coverage", "ordered.mp4");
    let mut r =
        fvid::playback_native::NativeReader::software(Cursor::new(data.clone()), 32 << 20).unwrap();
    let expected: Vec<_> = (0..5)
        .map(|_| {
            r.read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap()
        })
        .collect();
    assert!(r.read_frame_raw().unwrap().is_none());
    for ms in [0, 19, 20, 39, 40, 59, 60, 79, 80, 99, 100, 200] {
        let i = (ms / 20).min(4) as usize;
        let f = r.seek_raw(Duration::from_millis(ms)).unwrap().unwrap();
        assert_eq!(f.into_rgb(32 << 20).unwrap(), expected[i]);
        assert_eq!(r.frame_interval(), Some((i as u128, i as u128 + 1, 50)));
        if ms >= 100 {
            assert!(r.read_frame_raw().unwrap().is_none());
        }
    }
    let error =
        fvid::playback_mp4::Mp4VideoReader::open_software(Cursor::new(data), Default::default(), 1)
            .err()
            .expect("budget must refuse");
    assert!(error.to_string().contains("budget"), "{error}");
}
