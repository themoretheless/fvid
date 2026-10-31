#![cfg(feature = "player")]
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/quicktime-mac-language")
        .join(n)
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(path(n)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn metadata(c: &serde_json::Value) {
    let n = c["file"].as_str().unwrap();
    let expected = c["expected"].as_str().unwrap();
    let r =
        fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    let o =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    assert_eq!(r.tracks()[0].language, expected, "root {n}");
    assert_eq!(o.tracks()[0].language, expected, "owned {n}");
    let info = fvid_media::probe(&path(n)).unwrap();
    assert_eq!(
        info.streams[0].metadata.get("language").map(String::as_str),
        if expected.is_empty() {
            None
        } else {
            Some(expected)
        },
        "probe {n}"
    );
}
#[test]
fn quicktime_numeric_languages_preserve_metadata_without_changing_iso() {
 for c in manifest()["cases"].as_array().unwrap() { metadata(c); }
}
#[test]
fn all_language_cases_preserve_actual_video_packets_pictures_and_seek() {
    for c in manifest()["cases"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        let mut r =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        for i in 0..3 {
            let mut p = Vec::new();
            r.read_packet(0, i, &mut p).unwrap();
            assert_eq!(p, bytes(&format!("frame-{i}.packet")));
            let s = r.tracks()[0].samples.get(i).unwrap();
            assert_eq!((s.pts, s.duration), (i as i64, 1));
        }
        let mut d = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let f = d.read_frame().unwrap().unwrap();
            let p: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|x| x.to_le_bytes())
                .collect();
            assert_eq!(p, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(d.read_frame().unwrap().is_none());
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let rgb: Vec<_> = (0..3)
            .map(|_| {
                v.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap()
            })
            .collect();
        assert!(v.read_frame_raw().unwrap().is_none());
        v.rewind().unwrap();
        for ms in [0, 19, 20, 39, 40, 59, 60] {
            let i = (ms / 20).min(2) as usize;
            assert_eq!(
                v.seek_raw(Duration::from_millis(ms))
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                rgb[i]
            );
        }
    }
}