#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-preview-prefix")
            .join(n),
    )
    .unwrap()
}
fn verify(n: &str) {
    // Direct parser and software decoder prove this is a dispatch failure.
    let mut raw =
        fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    let mut owned =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    for i in 0..3 {
        let mut packet = Vec::new();
        raw.read_packet(0, i, &mut packet).unwrap();
        assert_eq!(packet, bytes(&format!("frame-{i}.packet")));
        owned.read_packet(0, i, &mut packet).unwrap();
        assert_eq!(packet, bytes(&format!("frame-{i}.packet")));
        let s = raw.tracks()[0].samples.get(i).unwrap();
        assert_eq!((s.pts, s.duration), (i as i64, 1));
    }
    let mut decoder = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(n)),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let frame = decoder.read_frame().unwrap().unwrap();
        let pixels: Vec<u8> = frame
            .picture
            .y
            .iter()
            .chain(&frame.picture.cb)
            .chain(&frame.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(pixels, bytes(&format!("frame-{i}.yuv16")));
        assert_eq!(
            (frame.presentation_time.ticks, frame.duration.ticks),
            (i, 1)
        );
    }
    assert!(decoder.read_frame().unwrap().is_none());
    let mut native = fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20)
        .unwrap_or_else(|e| panic!("dispatch {n}: {e}"));
    let rgb: Vec<_> = (0..3)
        .map(|_| {
            native
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap()
        })
        .collect();
    assert!(native.read_frame_raw().unwrap().is_none());
    native.rewind().unwrap();
    for ns in [
        0, 19_999_999, 20_000_000, 39_999_999, 40_000_000, 59_999_999, 60_000_000,
    ] {
        let index = (ns / 20_000_000).min(2) as usize;
        assert_eq!(
            native
                .seek_raw(Duration::from_nanos(ns))
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            rgb[index]
        );
    }
}
#[test]
fn recognized_movie_controls() {
    for n in ["ftyp-control.mov", "wide-control.mov", "mdat-control.mov"] {
        verify(n);
    }
}
#[test]
fn quicktime_preview_prefixes_play_and_seek() {
    for n in ["pnot-first.mov", "pict-first.mov"] {
        verify(n);
    }
}
#[test]
fn malformed_preview_movies_keep_parser_refusal() {
    for n in [
        "bad-pnot-extent.mov",
        "bad-PICT-extent.mov",
        "empty-pnot.mov",
        "empty-PICT.mov",
    ] {
        let direct = match fvid::container::mp4::Mp4Reader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        ) {
            Ok(_) => panic!("accepted {n}"),
            Err(e) => e.to_string(),
        };
        let routed =
            match fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20) {
                Ok(_) => panic!("accepted {n}"),
                Err(e) => e.to_string(),
            };
        assert!(
            routed.contains(&direct),
            "{n}: direct={direct}, routed={routed}"
        );
    }
}
#[test]
fn owned_probe_detects_preview_movies() {
    for n in [
        "ftyp-control.mov",
        "wide-control.mov",
        "mdat-control.mov",
        "pnot-first.mov",
        "pict-first.mov",
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-preview-prefix")
            .join(n);
        let hinted = fvid_media::probe_as(&path, Some("mov")).unwrap();
        let detected = fvid_media::probe(&path).unwrap_or_else(|e| panic!("probe {n}: {e}"));
        assert_eq!(format!("{detected:?}"), format!("{hinted:?}"));
    }
}

#[test]
fn still_images_keep_specific_refusal() {
    for data in [
        b"\xff\xd8\xff\xe0\0\0\0\0\0".as_slice(),
        b"\x89PNG\r\n\x1a\n\0",
        b"GIF89a\0\0\0",
    ] {
        let e = match fvid::playback_native::NativeReader::software(Cursor::new(data), 32 << 20) {
            Ok(_) => panic!("accepted image"),
            Err(e) => e.to_string(),
        };
        assert!(e.contains("still-image playback is not supported"), "{e}");
    }
}
