use fvid::{
    codec::hevc_motion::disparity::derivation::{self, Vector},
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::{io::Cursor, path::Path};

#[test]
fn spatial_disparity_binds_coded_skip_and_canonical_reference_identity() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    for merge in [false, true] {
        let name = format!(
            "inter-view-temporal-scaling-on{}.mp4",
            if merge { "-merge" } else { "" }
        );
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(std::fs::read(root.join(&name)).unwrap()),
            Default::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut checked = 0;
            while let Some(frame) = reader.read_frame().unwrap() {
                let graph = &frame.decoded.binding.vps.extension.as_ref().unwrap().graph;
                let picture = &frame.decoded.picture;
                if frame.decoded.poc == 0 || frame.decoded.layer_id == 0 {
                    assert!(
                        derivation::neighbour_from_picture(picture, [0, 0], false, None, |id| {
                            graph.view_index(id)
                        })
                        .unwrap()
                        .is_none()
                    );
                    assert!(!picture.prediction_is_skipped_at([0, 0]).unwrap());
                    continue;
                }
                let is_target = frame.decoded.layer_id == 5;
                let skip = is_target && merge;
                let saved = Vector {
                    reference_view: 0,
                    mv: [37, -4],
                };
                for y in 0..32 {
                    for x in 0..32 {
                        let neighbour = derivation::neighbour_from_picture(
                            picture,
                            [x, y],
                            skip,
                            skip.then_some(saved),
                            |id| graph.view_index(id),
                        )
                        .unwrap()
                        .unwrap();
                        assert_eq!(picture.prediction_is_skipped_at([x, y]).unwrap(), skip);
                        assert_eq!(neighbour.skip, skip);
                        let motion = neighbour.motion[0].unwrap();
                        assert_eq!(motion.reference_view, 0);
                        assert_eq!(motion.poc, 1);
                        assert_eq!(motion.mv, [0, if skip { 16 } else { 4 }]);
                        assert!(neighbour.motion[1].is_none());
                        let chosen = derivation::texture(
                            [32, 0],
                            8,
                            2,
                            true,
                            2,
                            5,
                            None,
                            Some(2),
                            |point| {
                                Ok(if point[0] == 31 {
                                    Some(neighbour)
                                } else {
                                    None
                                })
                            },
                        )
                        .unwrap();
                        assert_eq!(
                            chosen,
                            Some(if skip {
                                saved
                            } else {
                                Vector {
                                    reference_view: 2,
                                    mv: [0, 0],
                                }
                            })
                        );
                    }
                }
                if skip {
                    assert!(
                        derivation::neighbour_from_picture(picture, [0, 0], true, None, |id| graph
                            .view_index(id))
                        .is_err()
                    );
                }
                assert!(
                    derivation::neighbour_from_picture(picture, [32, 0], false, None, |id| graph
                        .view_index(id))
                    .is_err()
                );
                assert!(
                    derivation::neighbour_from_picture(picture, [0, 0], false, None, |_| Err(
                        fvid::Error::Invalid("view mapping unavailable".into())
                    ))
                    .is_err()
                );
                checked += 1;
            }
            assert_eq!(checked, 2);
            reader.rewind();
        }
    }
}
