//! Spatial SVC display selection and timeline use owned committed media only.
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn goldens() -> Vec<Vec<u8>> {
    let bytes = std::fs::read(root().join("av1-spatial-operating-points-op0.yuv")).unwrap();
    let mut at = 0;
    (0..8)
        .map(|n| {
            let len = if n % 2 == 0 {
                32 * 24 * 3 / 2
            } else {
                64 * 48 * 3 / 2
            };
            let frame = bytes[at..at + len].to_vec();
            at += len;
            frame
        })
        .collect()
}
#[test]
fn spatial_webm_highest_present_layer_pixels_timestamps_rewind_seek() {
    let golden = goldens();
    for (name, indices) in [
        ("complete", [1, 3, 5, 7]),
        ("missing-last-upper", [1, 3, 5, 6]),
    ] {
        let data =
            std::fs::read(root().join(format!("av1-spatial-container-{name}.webm"))).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(data), 32 << 20).unwrap();
        for replay in 0..3 {
            for (n, i) in indices.iter().enumerate() {
                let f = reader.read_frame_raw().unwrap().unwrap();
                let pixels = match f {
                    fvid::playback_native::RawFrame::Planar8(p) => {
                        p.y.iter()
                            .chain(&p.cb)
                            .chain(&p.cr)
                            .copied()
                            .collect::<Vec<_>>()
                    }
                    _ => panic!("expected 8-bit planes"),
                };
                assert_eq!(pixels, golden[*i], "{name}, unit {n}");
                assert_eq!(
                    reader.frame_interval(),
                    Some((
                        n as u128 * 20_000_000,
                        (n as u128 + 1) * 20_000_000,
                        1_000_000_000
                    ))
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            if replay == 0 {
                reader.rewind();
            } else if replay == 1 {
                assert_eq!(reader.seek_to_sync(40_000_000).unwrap(), 0);
            }
        }
        assert_eq!(reader.seek_to_sync(40_000_000).unwrap(), 0);
        assert!(reader.read_frame_raw().unwrap().is_some());
    }
}
#[test]
fn spatial_mp4_highest_present_layer_pixels_timestamps_rewind_seek() {
    let golden = goldens();
    for (name, indices) in [
        ("complete", [1, 3, 5, 7]),
        ("missing-last-upper", [1, 3, 5, 6]),
    ] {
        let data = std::fs::read(root().join(format!("av1-spatial-container-{name}.mp4"))).unwrap();
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(data),
            fvid::container::mp4::Limits::default(),
            32 << 20,
        )
        .unwrap();
        for replay in 0..3 {
            for (n, i) in indices.iter().enumerate() {
                let f = reader.read_frame().unwrap().unwrap();
                let p = &f.picture;
                let pixels =
                    p.y.iter()
                        .chain(&p.cb)
                        .chain(&p.cr)
                        .map(|v| *v as u8)
                        .collect::<Vec<_>>();
                assert_eq!(pixels, golden[*i], "{name}, unit {n}");
                assert_eq!(
                    f.presentation_time.nanoseconds().unwrap(),
                    n as i128 * 20_000_000
                );
                assert_eq!(f.duration.nanoseconds().unwrap(), 20_000_000);
            }
            assert!(reader.read_frame().unwrap().is_none());
            if replay == 0 {
                reader.rewind();
            } else if replay == 1 {
                assert_eq!(reader.seek_to_sync(2), 0);
            }
        }
        assert_eq!(reader.seek_to_sync(2), 0);
        assert!(reader.read_frame().unwrap().is_some());
    }
}
#[test]
fn multiple_temporal_units_in_one_timestamped_packet_remain_refused() {
    for name in ["invalid-two-units", "invalid-repeated-layers"] {
        let data =
            std::fs::read(root().join(format!("av1-spatial-container-{name}.webm"))).unwrap();
        let mut r =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(data), 32 << 20).unwrap();
        assert!(
            r.read_frame_raw()
                .err()
                .unwrap()
                .to_string()
                .contains("multiple AV1 temporal units")
        );
        let data = std::fs::read(root().join(format!("av1-spatial-container-{name}.mp4"))).unwrap();
        let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(data),
            fvid::container::mp4::Limits::default(),
            32 << 20,
        )
        .unwrap();
        assert!(
            r.read_frame()
                .err()
                .unwrap()
                .to_string()
                .contains("multiple AV1 temporal units")
        );
    }
}
#[test]
fn temporal_unit_errors_require_reset_and_raw_api_keeps_all_layers() {
    use fvid::codec::{av1::Obus, av1_decoder::Decoder};
    let data = std::fs::read(root().join("av1-spatial-operating-points.obu")).unwrap();
    let next = Obus::new(&data)
        .map(Result::unwrap)
        .filter(|o| o.kind == 2)
        .nth(1)
        .unwrap();
    let end = next.payload.as_ptr() as usize - data.as_ptr() as usize - 2;
    let mut decoder = Decoder::new(32 << 20);
    assert!(
        decoder
            .decode_temporal_unit(&data)
            .err()
            .unwrap()
            .to_string()
            .contains("multiple AV1 temporal units")
    );
    assert!(
        decoder
            .finish()
            .err()
            .unwrap()
            .to_string()
            .contains("requires reset")
    );
    decoder.reset();
    let frame = decoder.decode_temporal_unit(&data[..end]).unwrap().unwrap();
    assert_eq!(frame.spatial_id, 1);
    assert_eq!(frame.picture.size, [64, 48]);
    decoder.finish().unwrap();
    decoder.reset();
    assert_eq!(decoder.decode_packet(&data).unwrap().len(), 8);
    decoder.finish().unwrap();
}
