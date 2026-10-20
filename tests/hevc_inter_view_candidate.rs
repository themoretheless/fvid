//! Component pipeline on owned coded views; SPS inter-view tool admission is
//! separate and not asserted by this test's manually supplied disparity.
use fvid::{
    codec::hevc_motion::{self, Reference, Vector, inter_view_candidate as candidate},
    container::mp4::Limits,
    playback::video::mp4::LayeredMp4VideoReader,
};
#[test]
fn coded_inter_view_motion_maps_to_reference_and_predicts_exact_pixels() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge/inter-view-temporal-scaling-on.mp4");
    let data = std::fs::read(path).unwrap();
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(data),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut base = None;
        let mut checked = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            if frame.decoded.poc == 0 {
                assert_eq!(
                    candidate::from_picture(
                        &frame.decoded.picture,
                        [0, 0],
                        [8, 8],
                        [0, 0],
                        [0, 0],
                        [&[0], &[]],
                        false
                    )
                    .unwrap(),
                    [None; 2]
                );
                continue;
            }
            if frame.decoded.layer_id == 0 {
                base = Some(frame.decoded.picture.clone());
                continue;
            }
            // Both coded predictive views use explicit zero-MVD AMVP.
            let vector = 4;
            let picture = &frame.decoded.picture;
            // All sampled grid positions retain the exact saved source vector.
            for disparity in [[0, 0], [-3, 2], [14, -18], [i64::MIN, i64::MAX]] {
                let motion = candidate::from_picture(
                    picture,
                    [8, 8],
                    [8, 8],
                    disparity,
                    [0, 0],
                    [&[-99, 1, 1], &[1]],
                    true,
                )
                .unwrap();
                assert_eq!(
                    motion,
                    [
                        Some(Vector {
                            reference: 1,
                            mv: [0, vector]
                        }),
                        Some(Vector {
                            reference: 0,
                            mv: [0, vector]
                        })
                    ]
                );
            }
            let motion = candidate::from_picture(
                picture,
                [0, 0],
                [32, 32],
                [0, 0],
                [0, 0],
                [&[1], &[]],
                false,
            )
            .unwrap();
            assert_eq!(
                motion,
                [
                    Some(Vector {
                        reference: 0,
                        mv: [0, vector]
                    }),
                    None
                ]
            );
            assert_eq!(
                candidate::from_picture(
                    picture,
                    [0, 0],
                    [32, 32],
                    [0, 0],
                    [0, 0],
                    [&[123], &[]],
                    false
                )
                .unwrap(),
                [None; 2]
            );
            let references = [
                vec![Reference {
                    layer_id: 0,
                    long_term: true,
                    poc: 1,
                    picture: base.clone(),
                }],
                vec![],
            ];
            let output = hevc_motion::predict(
                &references,
                motion,
                [0, 0, 32, 32],
                0,
                8,
                None,
                &mut Vec::new(),
            )
            .unwrap();
            for y in 0..32 {
                for x in 0..32 {
                    let row = (y + vector as usize / 4).min(31);
                    assert_eq!(output[y * 32 + x], ((x * 3 + row * 5 + 17) % 256) as u16);
                }
            }
            checked += 1;
        }
        assert_eq!(checked, 2);
        reader.rewind();
    }
}

#[test]
fn inter_view_lookup_preserves_eight_row_amp_motion_boundary() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge/cqt-exact-texture-motion.mp4");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(path).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    let mut checked = false;
    while let Some(frame) = reader.read_frame().unwrap() {
        if frame.decoded.layer_id != 0 || frame.decoded.poc != 1 {
            continue;
        }
        for (origin, disparity, expected) in [
            ([0, 0], [0, 0], 1),
            ([0, 4], [0, 0], -1),
            ([0, 0], [0, 13], 1),
            ([0, 0], [0, 14], -1),
        ] {
            let result = candidate::from_picture(
                &frame.decoded.picture,
                origin,
                [8, 8],
                disparity,
                [0, 0],
                [&[0], &[]],
                false,
            )
            .unwrap();
            assert_eq!(
                result,
                [
                    Some(Vector {
                        reference: 0,
                        mv: [0, expected]
                    }),
                    None
                ]
            );
        }
        checked = true;
    }
    assert!(checked);
}
