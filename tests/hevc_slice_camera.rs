use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn independent_i_slices_with_camera_values_preserve_owned_pixels_and_rewind() {
    for name in ["control", "deferred", "limits"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/hevc-wedge/slice-camera-{name}.mp4"
        ));
        let data = std::fs::read(path).unwrap();
        let mut input = fvid::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(data.clone()),
            Limits::default(),
        )
        .unwrap();
        let params = fvid::codec::hevc_decoder::LayeredParameters::from_configuration(
            &input.tracks()[0].configuration,
            64 << 20,
        )
        .unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let plan = fvid::codec::hevc_decoder::LayeredAccessUnit::parse(
            &params,
            &fvid::codec::hevc_poc::LayeredPocState::default(),
            &packet,
            4,
            1,
            64 << 20,
        )
        .unwrap();
        for picture in plan.pictures() {
            let syntax = picture.slices[0].layered.as_ref().unwrap();
            assert!(syntax.illumination_compensation.is_none());
            if name == "control" || picture.binding.sps.layer_id == 0 {
                assert!(syntax.camera.is_none());
            } else {
                let camera = syntax.camera.as_ref().unwrap();
                assert_eq!(camera.precision, if name == "limits" { 5 } else { 0 });
                let index = if picture.binding.sps.layer_id == 3 {
                    1
                } else {
                    2
                };
                assert_eq!(camera.view_index, index);
                let expected = if name == "limits" {
                    if index == 1 {
                        vec![(0, [i32::MAX, -i32::MAX, 0, 1])]
                    } else {
                        vec![(0, [i32::MAX, 0, -i32::MAX, 1]), (1, [0, i32::MAX, -1, 0])]
                    }
                } else if index == 1 {
                    vec![(0, [1, 0, 0, 0])]
                } else {
                    vec![(0, [2, 1, -1, 3]), (1, [-3, -1, 2, 4])]
                };
                assert_eq!(
                    camera
                        .pairs
                        .iter()
                        .map(|p| (p.reference_view, p.values))
                        .collect::<Vec<_>>(),
                    expected
                );
                assert!(camera.disparity_transform(65535, 8).is_err());
                // Bind the default-view search to parameters from coded headers.
                // Active-list metadata is authored here; these I-only movies do
                // not claim acceptance of inter-view disparity reconstruction.
                use fvid::codec::hevc_motion::disparity::CandidatePicture;
                let reference = |view_index, poc| CandidatePicture {
                    view_index,
                    poc,
                    temporal_id: 0,
                    irap: true,
                };
                let l0 = [reference(index, 0), reference(0, -1)];
                let l1 = [reference(0, 0)];
                assert!(
                    camera
                        .default_disparity_transform([&l0, &l1], false, 0, 8)
                        .unwrap()
                        .is_none()
                );
                let (view, transform) = camera
                    .default_disparity_transform([&l0, &l1], true, 0, 8)
                    .unwrap()
                    .unwrap();
                assert_eq!(view, 0);
                for depth in [0, 64, 127, 255] {
                    assert_eq!(
                        transform.sample(depth).unwrap(),
                        camera
                            .disparity_transform(0, 8)
                            .unwrap()
                            .sample(depth)
                            .unwrap()
                    );
                }
                let mut missing = camera.clone();
                missing.pairs.retain(|p| p.reference_view != 0);
                assert!(
                    missing
                        .default_disparity_transform([&l0, &l1], true, 0, 8)
                        .is_err()
                );
                assert!(
                    missing
                        .default_disparity_transform([&l0, &l1], false, 0, 8)
                        .unwrap()
                        .is_none()
                );

                if name == "limits" {
                    use fvid::codec::hevc_motion::disparity::Disparity;
                    let transform = camera.disparity_transform(0, 8).unwrap();
                    let (sample, expected) = if index == 1 {
                        (
                            0,
                            Disparity {
                                backward: -134217728,
                                forward: 134217728,
                            },
                        )
                    } else {
                        (
                            128,
                            Disparity {
                                backward: 67108864,
                                forward: -134217728,
                            },
                        )
                    };
                    assert_eq!(transform.sample(sample).unwrap(), expected);
                }
                if name == "deferred" {
                    let transform = camera.disparity_transform(0, 8).unwrap();
                    use fvid::codec::hevc_motion::disparity::Disparity;
                    let values = if index == 1 {
                        [(0, 0), (1, 0), (1, -1), (2, -2)]
                    } else {
                        [(2, 4), (3, 3), (4, 1), (6, -2)]
                    };
                    for (d, (backward, forward)) in [0, 64, 127, 255].into_iter().zip(values) {
                        assert_eq!(
                            transform.sample(d).unwrap(),
                            Disparity { backward, forward }
                        );
                    }
                }
            }
        }
        let mut r = LayeredMp4VideoReader::open(
            std::io::Cursor::new(data),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut count = 0;
            while let Some(frame) = r.read_frame().unwrap() {
                assert_eq!(frame.presentation_time.ticks, i64::from(frame.decoded.poc));
                assert_eq!(frame.duration.ticks, 1);
                for y in 0..32 {
                    for x in 0..32 {
                        assert_eq!(
                            frame.decoded.picture.planes[0].samples()[y * 32 + x],
                            ((3 * x + 5 * y + 17) % 256) as u16
                        );
                    }
                }
                count += 1;
            }
            assert_eq!(count, 6);
            r.rewind();
        }
    }
}
