//! Multi-CTU DI acceptance across exact-motion-map and reconstruction boundaries.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
use std::{io::Cursor, path::Path};
#[test]
fn multicu_di_retains_signed_wide_vectors_and_exact_pixels_in_both_reconstruction_modes() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("multicu-wide.json")).unwrap()).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 20);
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let disparity = case["offset"].as_i64().unwrap() * 2;
        assert_eq!(disparity, case["disparity"].as_i64().unwrap());
        let direct = case["direct"].as_bool().unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(std::fs::read(root.join(name)).unwrap()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let id = frame.decoded.layer_id;
                let poc = frame.decoded.poc;
                seen.push((id, poc));
                assert_eq!(frame.decoded.binding.pps.pps.constrained_intra, direct);
                if id != 5 {
                    continue;
                }
                let picture = &frame.decoded.picture;
                for y in 0..size {
                    for x in 0..size {
                        let xx = if poc == 0 {
                            x as i64
                        } else {
                            (x as i64 + disparity / 4).clamp(0, size as i64 - 1)
                        };
                        let expected = ((3 * xx + 5 * y as i64 + 17) % 256) as u16;
                        assert_eq!(
                            picture.planes[0].samples()[y * size + x],
                            expected,
                            "{name} POC{poc} x{x} y{y}"
                        );
                    }
                }
                for plane in &picture.planes[1..] {
                    assert!(plane.samples().iter().all(|&v| v == 128));
                }
                if poc == 1 {
                    for y in (0..size).step_by(8) {
                        for x in (0..size).step_by(8) {
                            let motion =
                                picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                            assert_eq!(motion[0].unwrap().reference, 1);
                            assert_eq!(
                                motion[0].unwrap().mv,
                                [disparity, 0],
                                "{name}: vector x{x} y{y}"
                            );
                            assert!(motion[1].is_none());
                        }
                    }
                }
            }
            assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
            reader.rewind();
        }
        use fvid::playback::video::{
            mp4::Mp4VideoReader,
            native::{NativeReader, RawFrame},
        };
        let check = |data: &[u8], poc: i64| {
            assert_eq!(data.len(), size * size * 3 / 2);
            for y in 0..size {
                for x in 0..size {
                    let xx = if poc == 0 {
                        x as i64
                    } else {
                        (x as i64 + disparity / 4).clamp(0, size as i64 - 1)
                    };
                    assert_eq!(
                        data[y * size + x],
                        ((3 * xx + 5 * y as i64 + 17) % 256) as u8,
                        "{name} packed x{x} y{y}"
                    );
                }
            }
            assert!(data[size * size..].iter().all(|&v| v == 128));
        };
        let data = std::fs::read(root.join(name)).unwrap();
        let mut common =
            Mp4VideoReader::open_software(Cursor::new(data.clone()), Default::default(), 64 << 20)
                .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = common.read_frame().unwrap() {
                assert_eq!(common.selected_layer_id(), Some(5));
                let poc = frame.presentation_time.ticks;
                check(&frame.packed.unwrap().frame.data, poc);
                seen.push(poc);
            }
            assert_eq!(seen, [0, 1]);
            common.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(std::time::Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar frame")
            };
            check(&frame.frame.data, if target == 0 { 0 } else { 1 });
        }
    }
}
