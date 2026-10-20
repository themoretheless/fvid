//! Matroska HEVC preserves format/depth just as the independently qualified MP4 path.
use fvid::playback_native::{NativeReader, RawFrame};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("hevc-matroska-formats.json")).unwrap()
}
fn samples(raw: &RawFrame) -> Vec<u8> {
    match raw {
        RawFrame::Planar(p) => p.frame.data.clone(),
        RawFrame::Planar8(p) => [p.y.as_slice(), p.cb.as_slice(), p.cr.as_slice()].concat(),
        RawFrame::Avc { picture, .. } => {
            let mut v = Vec::new();
            picture.write_planar(&mut v).unwrap();
            v
        }
        _ => panic!("software sample payload"),
    }
}
#[test]
fn matroska_hevc_preserves_chroma_depth_pixels_display_geometry_and_seek() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 112);
    for case in m["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let mut control = NativeReader::software(
            Cursor::new(bytes(case["source"].as_str().unwrap())),
            32 << 20,
        )
        .unwrap();
        let mut expected_rgb = Vec::new();
        for frame in case["frames"].as_array().unwrap() {
            let raw = control.read_frame_raw().unwrap().unwrap();
            assert!(
                samples(&raw) == bytes(frame["packed"].as_str().unwrap()),
                "{file}: MP4 control must match saved synthetic samples"
            );
            expected_rgb.push(raw.into_rgb(32 << 20).unwrap());
        }
        assert!(control.read_frame_raw().unwrap().is_none());
        let mut reader = NativeReader::software(Cursor::new(bytes(file)), 32 << 20).unwrap();
        let rotation = reader.rotation();
        let insets = reader.insets();
        assert_eq!(rotation, case["rotation"].as_u64().unwrap() as u16);
        let depth = case["depth"].as_u64().unwrap() as u8;
        let sub = [
            case["subsampling"][0].as_u64().unwrap() as usize,
            case["subsampling"][1].as_u64().unwrap() as usize,
        ];
        let mut intervals = Vec::new();
        for pass in 0..2 {
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                // Compare actual presentation first, so the old 4:2:0 assumption
                // reproduces a specific chroma-sampling failure, not just a variant change.
                let rgb = match &raw {
                    RawFrame::Planar(p) => {
                        let mut rgb = Vec::new();
                        p.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                        rgb
                    }
                    RawFrame::Avc { picture, colour } => {
                        let mut rgb = Vec::new();
                        fvid::playback_native::avc_to_rgb(picture, *colour, &mut rgb, 32 << 20)
                            .unwrap();
                        rgb
                    }
                    _ => panic!("software presentation payload"),
                };
                assert!(
                    rgb == expected_rgb[i],
                    "{file}: Matroska RGB differs from qualified MP4 chroma sampling at byte {:?}",
                    rgb.iter().zip(&expected_rgb[i]).position(|(a, b)| a != b)
                );
                let RawFrame::Planar(p) = raw else {
                    panic!("{file}: HEVC must retain its actual packed sample geometry")
                };
                assert_eq!(p.depth, depth);
                assert_eq!(p.frame.subsampling, Some(sub));
                assert!(
                    p.frame.data == bytes(frame["packed"].as_str().unwrap()),
                    "{file}: raw source samples"
                );
                let display = fvid::native_geometry::VideoGeometry::default()
                    .apply_packed_display_media(&p, rotation, insets)
                    .unwrap();
                assert!(
                    display.frame.data == bytes(frame["display"].as_str().unwrap()),
                    "{file}: independent crop/rotation samples"
                );
                let out_sub = if rotation == 90 || rotation == 270 {
                    [sub[1], sub[0]]
                } else {
                    sub
                };
                assert_eq!(display.frame.subsampling, Some(out_sub));
                let mut visible = [
                    case["size"][0].as_u64().unwrap() as usize
                        - case["crop"][0].as_u64().unwrap() as usize
                        - case["crop"][2].as_u64().unwrap() as usize,
                    case["size"][1].as_u64().unwrap() as usize
                        - case["crop"][1].as_u64().unwrap() as usize
                        - case["crop"][3].as_u64().unwrap() as usize,
                ];
                if rotation == 90 || rotation == 270 {
                    visible.swap(0, 1);
                }
                assert_eq!([display.frame.width, display.frame.height], visible);
                let interval = reader.frame_interval().unwrap();
                if pass == 0 {
                    intervals.push(interval);
                } else {
                    assert_eq!(interval, intervals[i]);
                }
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for index in [intervals.len() - 1, 0] {
            let raw = reader
                .seek_raw(std::time::Duration::from_nanos(
                    index as u64 * 1_000_000_000 / case["rate"].as_u64().unwrap(),
                ))
                .unwrap()
                .unwrap();
            assert!(
                samples(&raw) == bytes(case["frames"][index]["packed"].as_str().unwrap()),
                "{file}: seek source precision"
            );
            assert_eq!(reader.frame_interval().unwrap(), intervals[index]);
        }
        reader.rewind().unwrap();
        let w = case["size"][0].as_u64().unwrap() as usize;
        let h = case["size"][1].as_u64().unwrap() as usize;
        for source_rgb in &expected_rgb {
            assert!(reader.read_frame().unwrap());
            let (ow, oh) = if rotation == 90 || rotation == 270 {
                (h, w)
            } else {
                (w, h)
            };
            let mut expected = Vec::new();
            for y in 0..oh {
                for x in 0..ow {
                    let (sx, sy) = match rotation {
                        0 => (x, y),
                        90 => (y, h - 1 - x),
                        180 => (w - 1 - x, h - 1 - y),
                        270 => (w - 1 - y, x),
                        _ => unreachable!(),
                    };
                    expected
                        .extend_from_slice(&source_rgb[(sy * w + sx) * 3..(sy * w + sx) * 3 + 3]);
                }
            }
            assert!(
                reader.rgb() == expected,
                "{file}: legacy RGB reader must retain correctly rotated coded pixels"
            );
        }
        assert!(!reader.read_frame().unwrap());
    }
}
