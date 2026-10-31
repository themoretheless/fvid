#![cfg(feature = "player")]
use std::{io::Cursor, path::Path};
const TIGHT: usize = 2 << 20;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-ordered-index-budget")
            .join(n),
    )
    .unwrap()
}
fn open(n: &str, budget: usize) -> fvid::playback_mp4::Mp4VideoReader<Cursor<Vec<u8>>> {
    fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(n)),
        Default::default(),
        budget,
    )
    .unwrap_or_else(|e| panic!("{n} budget{budget}: {e}"))
}
fn picture(f: fvid::playback_mp4::VideoFrame, index: usize, pts: i64) {
    assert_eq!(
        (f.sample_index, f.presentation_time.ticks, f.duration.ticks),
        (index, pts, 1)
    );
    let p: Vec<u8> = f
        .picture
        .y
        .iter()
        .chain(&f.picture.cb)
        .chain(&f.picture.cr)
        .flat_map(|x| x.to_le_bytes())
        .collect();
    static ORACLES: std::sync::OnceLock<[Vec<u8>; 3]> = std::sync::OnceLock::new();
    let oracles =
        ORACLES.get_or_init(|| std::array::from_fn(|i| bytes(&format!("frame-{i}.yuv16"))));
    assert_eq!(p, oracles[index % 3]);
}
#[test]
fn small_control_uses_the_same_codec_under_the_tight_budget() {
    let mut r = open("small-control.mp4", TIGHT);
    for i in 0..3 {
        picture(r.read_frame().unwrap().unwrap(), i, i as i64);
    }
    assert!(r.read_frame().unwrap().is_none());
}
#[test]
fn dense_inputs_are_valid_and_decodable_with_roomy_budget() {
    for n in ["ordered.mp4", "reordered.mp4"] {
        let mut r = open(n, 32 << 20);
        assert_eq!(r.track().samples.len(), 150000);
        assert_eq!(r.track().timescale, 1000000);
        for rank in 0..3 {
            let i = if n == "reordered.mp4" {
                [0, 2, 1][rank]
            } else {
                rank
            };
            picture(r.read_frame().unwrap().unwrap(), i, rank as i64);
        }
        assert_eq!(r.seek_to_sync(149999), 149999);
        picture(r.read_frame().unwrap().unwrap(), 149999, 149999);
        assert!(r.read_frame().unwrap().is_none());
    }
}
#[test]
fn ordered_dense_input_preserves_all_pictures_under_tight_budget() {
    for repeat in 0..7 {
        let mut r = open("ordered.mp4", TIGHT);
        if repeat == 0 {
            for i in 0..150000 {
                picture(r.read_frame().unwrap().unwrap(), i, i as i64);
            }
            assert!(r.read_frame().unwrap().is_none());
            r.rewind();
        }
        for i in 0..3 {
            picture(r.read_frame().unwrap().unwrap(), i, i as i64);
        }
        assert_eq!(r.seek_to_sync(149999), 149999);
        picture(r.read_frame().unwrap().unwrap(), 149999, 149999);
        assert!(r.read_frame().unwrap().is_none());
    }
}
#[test]
fn reordered_dense_input_still_refuses_insufficient_reorder_budget() {
    let e = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("reordered.mp4")),
        Default::default(),
        64 << 10,
    )
    .err()
    .expect("reordered index must retain its budget charge");
    assert!(
        e.to_string().contains("MP4 reorder index exceeds budget"),
        "{e}"
    );
}

#[test]
fn intrinsic_vp9_picture_budget_is_not_relaxed() {
    let mut r = open("small-control.mp4", 512 << 10);
    let e = r
        .read_frame()
        .err()
        .expect("intrinsic picture work must still refuse");
    assert!(
        e.to_string().contains("VP9 picture exceeds memory budget"),
        "{e}"
    );
}
