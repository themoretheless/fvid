#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-chunk-storage-order")
            .join(name),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn accepted(c: &serde_json::Value) {
    let name = c["file"].as_str().unwrap();
    let mut a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
        .unwrap();
    let mut b =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
            .unwrap();
    assert_eq!(a.tracks()[0].samples.len(), 3);
    assert_eq!(b.tracks()[0].samples.len(), 3);
    for i in [2, 0, 1] {
        let s = a.tracks()[0].samples.get(i).unwrap();
        assert_eq!((s.dts, s.pts, s.duration), (i as u64, i as i64, 1));
        assert_eq!(s.offset, c["offsets"][i].as_u64().unwrap());
        let mut x = Vec::new();
        let mut y = Vec::new();
        a.read_packet(0, i, &mut x).unwrap();
        b.read_packet(0, i, &mut y).unwrap();
        assert_eq!(x, bytes(&format!("frame-{i}.packet")), "{name}");
        assert_eq!(y, x);
    }
    let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(name)),
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
        assert_eq!(p, bytes(&format!("frame-{i}.yuv16")), "{name}");
    }
    assert!(video.read_frame().unwrap().is_none());
    let mut native =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20).unwrap();
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
    for i in [2, 0, 1, 2] {
        let frame = native
            .seek_raw(Duration::from_millis(i * 20))
            .unwrap()
            .unwrap();
        assert_eq!(frame.into_rgb(32 << 20).unwrap(), rgb[i as usize]);
        assert_eq!(
            native.frame_interval(),
            Some((i as u128, i as u128 + 1, 50)),
            "{name}"
        );
    }
    a.restrict_packet_bytes(1);
    assert!(
        a.read_packet(0, 0, &mut Vec::new())
            .unwrap_err()
            .to_string()
            .contains("packet exceeds budget")
    );
}
#[test]
fn ascending_controls_keep_exact_packets_pixels_seek_and_eof() {
    for c in cases().into_iter().filter(|c| {
        c.get("damage").is_none() && c["physical_order"] == serde_json::json!([0, 1, 2])
    }) {
        accepted(&c);
    }
}
#[test]
fn disjoint_chunks_keep_sample_order_independent_of_storage_order() {
    for c in cases().into_iter().filter(|c| {
        c.get("damage").is_none() && c["physical_order"] != serde_json::json!([0, 1, 2])
    }) {
        accepted(&c);
    }
}
#[test]
fn actual_overlap_and_outside_media_data_remain_explicit_refusals() {
    for c in cases().into_iter().filter(|c| c.get("damage").is_some()) {
        let name = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
            .err()
            .expect("damaged root input accepted");
        let b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
                .err()
                .expect("damaged owned input accepted");
        let message = if c["damage"] == "outside-mdat" {
            "sample chunk outside mdat"
        } else {
            "overlapping or non-increasing MP4 chunk offsets"
        };
        assert_eq!(a.to_string(), message, "{name}");
        assert_eq!(b.to_string(), message, "{name}");
    }
}
