//! Wide camera-vector pixel prediction; native CU motion storage remains separate.
use fvid::{
    codec::hevc_motion::{self, Reference, disparity::CameraTransform, view_synthesis::WideVector},
    container::mp4::Limits,
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::io::Cursor;
#[test]
fn wide_camera_and_i64_extremes_extend_picture_edges_without_narrowing_or_overflow() {
    let data =
        include_bytes!("fixtures/playback-errors/hevc-wedge/texture-depth-refinement-control.mp4");
    let mut reader = LayeredMp4VideoReader::open(
        Cursor::new(data.as_slice()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    let mut source = None;
    while let Some(frame) = reader.read_frame().unwrap() {
        if frame.decoded.layer_id == 0 && frame.decoded.poc == 1 {
            source = Some(frame.decoded.picture.clone());
        }
    }
    let source = source.unwrap();
    let reference = Reference {
        layer_id: 0,
        poc: 1,
        long_term: false,
        picture: Some(source.clone()),
    };
    let lists = [vec![reference.clone()], vec![reference]];
    let camera = CameraTransform::new([i32::MAX, i32::MAX, 0, 0], 0, 8).unwrap();
    let camera_mv = camera.sample(254).unwrap().backward;
    assert!(camera_mv > i64::from(i32::MAX));
    let mut vectors = vec![
        [camera_mv, 0],
        [-camera_mv, 0],
        [i64::MAX, 0],
        [i64::MIN, 0],
        [i64::MAX, i64::MAX],
        [i64::MIN, i64::MIN],
        [i64::MIN, i64::MAX],
        [i64::MAX, i64::MIN],
    ];
    for phase in 0..8 {
        vectors.push([i64::from(i32::MAX) + 1 + phase, 0]);
        vectors.push([i64::from(i32::MIN) - 1 - phase, 0]);
    }
    let mut scratch = Vec::new();
    for mv in vectors {
        let motion = [Some(WideVector { reference: 0, mv }), None];
        for component in 0..3 {
            let side = if component == 0 { 32 } else { 16 };
            let expected: Vec<_> = (0..side)
                .flat_map(|y| (0..side).map(move |_| y))
                .map(|y| {
                    let x = if mv[0] > 0 { side - 1 } else { 0 };
                    let y = if mv[1] > 0 {
                        side - 1
                    } else if mv[1] < 0 {
                        0
                    } else {
                        y
                    };
                    source.planes[component].samples()[y * side + x]
                })
                .collect();
            let actual = hevc_motion::predict_wide_with_chroma_format(
                &lists,
                motion,
                [0, 0, 32, 32],
                component,
                8,
                None,
                &mut scratch,
                1,
            )
            .unwrap();
            assert_eq!(actual, expected, "component{component} mv{mv:?}");
        }
    }
    for phase in 0..8 {
        let motion = [
            Some(WideVector {
                reference: 0,
                mv: [i64::from(i32::MAX) + 1 + phase, 0],
            }),
            Some(WideVector {
                reference: 0,
                mv: [i64::from(i32::MIN) - 1 - phase, 0],
            }),
        ];
        for component in 0..3 {
            let side = if component == 0 { 32 } else { 16 };
            let plane = source.planes[component].samples();
            let expected: Vec<_> = (0..side)
                .flat_map(|y| {
                    let average =
                        (u32::from(plane[y * side]) + u32::from(plane[y * side + side - 1]) + 1)
                            / 2;
                    std::iter::repeat_n(average as u16, side)
                })
                .collect();
            let actual = hevc_motion::predict_wide_with_chroma_format(
                &lists,
                motion,
                [0, 0, 32, 32],
                component,
                8,
                None,
                &mut scratch,
                1,
            )
            .unwrap();
            assert_eq!(actual, expected, "bidirectional c{component} phase{phase}");
        }
    }
}

#[test]
fn native_wide_di_decodes_and_retains_camera_vector_without_narrowing() {
    use fvid::playback::video::{
        mp4::Mp4VideoReader,
        native::{NativeReader, RawFrame},
    };
    for (scale, bytes) in [
        (
            2147483647i64,
            include_bytes!(
                "fixtures/playback-errors/hevc-wedge/texture-depth-refinement-wide-positive.mp4"
            )
            .as_slice(),
        ),
        (
            -2147483647i64,
            include_bytes!(
                "fixtures/playback-errors/hevc-wedge/texture-depth-refinement-wide-negative.mp4"
            )
            .as_slice(),
        ),
    ] {
        let disparity = (scale * 254 + 64) >> 7;
        assert!(disparity > i64::from(i32::MAX) || disparity < i64::from(i32::MIN));
        let check = |data: &[u8], poc: i64| {
            assert_eq!(data.len(), 1536);
            for y in 0..32 {
                for x in 0..32 {
                    let xx = if poc == 0 {
                        x
                    } else if disparity > 0 {
                        31
                    } else {
                        0
                    };
                    assert_eq!(data[y * 32 + x], ((3 * xx + 5 * y + 17) % 256) as u8);
                }
            }
            assert!(data[1024..].iter().all(|&v| v == 128));
        };
        let mut reader =
            LayeredMp4VideoReader::open(Cursor::new(bytes), Limits::default(), 1, 0, 64 << 20)
                .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader.read_frame().unwrap() {
                seen.push((frame.decoded.layer_id, frame.decoded.poc));
                if frame.decoded.layer_id == 5 && frame.decoded.poc == 1 {
                    let picture = &frame.decoded.picture;
                    for y in 0..32 {
                        for x in 0..32 {
                            let xx = if disparity > 0 { 31 } else { 0 };
                            assert_eq!(
                                picture.planes[0].samples()[y * 32 + x],
                                ((3 * xx + 5 * y + 17) % 256) as u16
                            );
                        }
                    }
                    for y in (0..32).step_by(8) {
                        for x in (0..32).step_by(8) {
                            let motion = picture.prediction_motion_at([x, y]).unwrap();
                            assert_eq!(motion[0].unwrap().reference, 1);
                            assert_eq!(motion[0].unwrap().mv, [disparity, 0]);
                            assert!(motion[1].is_none());
                        }
                    }
                }
            }
            assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
            reader.rewind();
        }
        let mut common =
            Mp4VideoReader::open_software(Cursor::new(bytes), Default::default(), 64 << 20)
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
        let mut native = NativeReader::software(Cursor::new(bytes), 64 << 20).unwrap();
        for target in [40, 0, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(std::time::Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("planar frame")
            };
            check(&frame.frame.data, if target == 0 { 0 } else { 1 });
        }
    }
}
