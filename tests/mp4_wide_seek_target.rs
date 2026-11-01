#![cfg(feature = "player")]
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
const B: usize = 32 << 20;
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-wide-seek-target")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn open(n: &str) -> fvid::playback_native::NativeReader<Cursor<Vec<u8>>> {
    fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), B).unwrap()
}
fn sequence(n: &str) -> Vec<((u128, u128, u32), Vec<u8>)> {
    let mut r = open(n);
    let mut seq = Vec::new();
    while let Some(f) = r.read_frame_raw().unwrap() {
        seq.push((r.frame_interval().unwrap(), f.into_rgb(B).unwrap()));
    }
    assert!(r.read_frame_raw().unwrap().is_none());
    seq
}
#[test]
fn authored_packets_pictures_and_ordinary_end_clamps_are_exact() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let scale = c["scale"].as_u64().unwrap();
        let step = c["step"].as_u64().unwrap();
        let frames = c["frames"].as_u64().unwrap();
        let mut raw = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
            Default::default(),
            B,
        )
        .unwrap();
        for i in 0..3 {
            let f = raw.read_frame().unwrap().unwrap();
            assert_eq!(f.presentation_time.ticks, i as i64 * step as i64);
            let yuv: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|x| x.to_le_bytes())
                .collect();
            assert_eq!(yuv, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(raw.read_frame().unwrap().is_none());
        let seq = sequence(n);
        assert_eq!(seq.len(), frames as usize);
        for (i, (span, _)) in seq.iter().enumerate() {
            assert_eq!(
                *span,
                (
                    i as u128 * step as u128,
                    (i + 1) as u128 * step as u128,
                    scale as u32
                )
            );
        }
        let mut r = open(n);
        assert_eq!(r.duration(), Some(Duration::from_millis(frames * 20)));
        for target in [
            Duration::from_millis(frames * 20),
            Duration::from_secs(1),
            Duration::from_secs(i64::MAX as u64 / scale),
        ] {
            let f = r.seek_raw(target).unwrap().unwrap().into_rgb(B).unwrap();
            assert_eq!(f, seq.last().unwrap().1);
            assert_eq!(r.frame_interval(), Some(seq.last().unwrap().0));
            assert!(r.read_frame_raw().unwrap().is_none());
        }
    }
}
#[test]
fn wide_targets_clamp_before_tick_conversion_for_both_native_seek_apis() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let scale = c["scale"].as_u64().unwrap();
        let seq = sequence(n);
        for rgb in [false, true] {
            let mut r = open(n);
            for target in [
                Duration::MAX,
                Duration::from_secs(u64::MAX),
                Duration::from_secs(i64::MAX as u64 / scale + 1),
            ] {
                r.seek_raw(Duration::ZERO).unwrap().unwrap();
                let result = if rgb {
                    r.seek(target).map(|_| r.rgb().to_vec())
                } else {
                    r.seek_raw(target).map(|f| f.unwrap().into_rgb(B).unwrap())
                };
                match result {
                    Ok(pixels) => {
                        assert_eq!(pixels, seq.last().unwrap().1, "{n}");
                        assert_eq!(r.frame_interval(), Some(seq.last().unwrap().0));
                        assert!(r.read_frame_raw().unwrap().is_none());
                    }
                    Err(e) => failures.push(format!("{n}:rgb={rgb}:{target:?}:{e}")),
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
