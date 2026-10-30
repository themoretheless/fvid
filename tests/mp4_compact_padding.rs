#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-compact-padding")
            .join(n),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
#[test]
fn real_nibbles_and_zero_padding_preserve_packets_pixels_caption_seek_and_eof() {
    for c in manifest()["controls"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        for i in 0..c["texts"].as_array().unwrap().len() {
            let mut x = Vec::new();
            let mut y = Vec::new();
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(
                x,
                bytes(&format!(
                    "text-{}-{}-{i}.packet",
                    c["width"],
                    c["texts"].as_array().unwrap().len()
                ))
            );
            assert_eq!(y, x);
        }
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let f = video.read_frame().unwrap().unwrap();
            assert_eq!((f.presentation_time.ticks, f.duration.ticks), (i, 1));
            let p: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(p, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(video.read_frame().unwrap().is_none());
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        assert_eq!(cues.len(), c["texts"].as_array().unwrap().len());
        let scale = c["timescale"].as_u64().unwrap();
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for (i, cue) in cues.iter().enumerate() {
            let ns = i as u64 * 1_000_000_000 / scale;
            assert_eq!(cue.text, c["texts"][i].as_str().unwrap());
            assert_eq!(cue.start, Duration::from_nanos(ns));
            assert_eq!(
                cue.end,
                Duration::from_nanos((i as u64 + 1) * 1_000_000_000 / scale)
            );
            assert!(v.seek_raw(cue.start).unwrap().is_some());
            let frame = u128::from(ns / 20_000_000);
            assert_eq!(v.frame_interval(), Some((frame, frame + 1, 50)));
        }
    }
}
#[test]
fn unused_nonzero_padding_is_rejected_by_its_own_error_before_playback() {
    for c in manifest()["damaged"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .err()
            .expect("root accepted nonzero padding");
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .err()
            .expect("owned accepted nonzero padding");
        assert!(matches!(a, fvid::Error::Invalid(_)));
        assert!(!b.is_unsupported());
        assert_eq!(a.to_string(), "nonzero MP4 compact sample-size padding");
        assert_eq!(b.to_string(), a.to_string());
    }
}
