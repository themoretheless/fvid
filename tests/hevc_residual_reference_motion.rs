use fvid::{
    codec::{
        hevc_motion::{
            inter_view_residual::{reference_block_motion, reference_targets},
            texture::Identity,
        },
        hevc_picture::Picture,
    },
    container::mp4::Limits,
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::{io::Cursor, sync::Arc};
fn source(name: &str) -> Arc<Picture> {
    source_layer(name, 0)
}
fn source_layer(name: &str, layer: u8) -> Arc<Picture> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let mut reader = LayeredMp4VideoReader::open(
        Cursor::new(std::fs::read(root.join(name)).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    while let Some(frame) = reader.read_frame().unwrap() {
        if frame.decoded.layer_id == layer && frame.decoded.poc == 1 {
            return frame.decoded.picture;
        }
    }
    panic!("missing source frame: {name}")
}
#[test]
fn coded_amp_source_uses_displaced_eight_aligned_motion_and_exact_reference() {
    let picture = source("multicu-iv-64-0-8-128-0.mp4");
    let list = [Identity { poc: 0, view: 1 }];
    let targets = reference_targets([&list, &[]], false, 1, true, &[0], |_, _| Ok(false)).unwrap();
    for (horizontal, point, vertical) in [
        (13, [0, 0], 4),
        (14, [8, 0], -4),
        (i64::MAX, [63, 0], -4),
        (i64::MIN, [0, 0], 4),
    ] {
        let motion =
            reference_block_motion(&picture, 1, 1, [0, 0, 8, 8], [horizontal, 0], &targets)
                .unwrap()
                .unwrap();
        assert_eq!(motion.point, point);
        assert_eq!(motion.list, 0);
        assert_eq!(motion.mv, [0, vertical]);
        assert_eq!(motion.source_reference.layer_id, 0);
        assert_eq!(motion.source_reference.poc, 0);
    }
    let target = [Identity { poc: -1, view: 1 }];
    let scaled =
        reference_targets([&target, &[]], false, 1, true, &[], |_, _| unreachable!()).unwrap();
    assert_eq!(
        reference_block_motion(&picture, 1, 1, [0, 0, 8, 8], [0, 0], &scaled)
            .unwrap()
            .unwrap()
            .mv,
        [0, 8]
    );
}
#[test]
fn coded_b_source_prioritizes_l0_and_uses_l1_when_l0_has_no_temporal_target() {
    let picture = source("texture-refinement-b-1-0-bi-4.mp4");
    let past = [Identity { poc: 0, view: 1 }];
    let future = [Identity { poc: 2, view: 1 }];
    let targets =
        reference_targets([&past, &future], true, 1, true, &[], |_, _| unreachable!()).unwrap();
    let motion = reference_block_motion(&picture, 1, 1, [0, 0, 8, 8], [0, 0], &targets)
        .unwrap()
        .unwrap();
    assert_eq!(motion.list, 0);
    assert_eq!(motion.mv, [0, 4]);
    assert_eq!(motion.source_reference.poc, 0);
    let l1 = reference_targets([&[], &future], true, 1, true, &[], |_, _| unreachable!()).unwrap();
    let motion = reference_block_motion(&picture, 1, 1, [0, 0, 8, 8], [0, 0], &l1)
        .unwrap()
        .unwrap();
    assert_eq!(motion.list, 1);
    assert_eq!(motion.mv, [0, -8]);
    assert_eq!(motion.source_reference.poc, 2);
}
#[test]
fn coded_intra_source_has_no_reference_block_motion() {
    let picture = source("illumination-b-64-0-0-2-1--40-both.mp4");
    let past = [Identity { poc: 0, view: 1 }];
    let targets =
        reference_targets([&past, &[]], false, 1, true, &[], |_, _| unreachable!()).unwrap();
    assert!(
        reference_block_motion(&picture, 1, 1, [0, 0, 8, 8], [0, 0], &targets)
            .unwrap()
            .is_none()
    );
}

#[test]
fn current_poc_inter_view_motion_is_skipped_before_temporal_l1() {
    let picture = source_layer("illumination-b-64-0-0-2-1--40-source-future.mp4", 5);
    let past = [Identity { poc: 0, view: 1 }];
    let future = [Identity { poc: 2, view: 1 }];
    let targets =
        reference_targets([&past, &future], true, 1, true, &[], |_, _| unreachable!()).unwrap();
    let motion = reference_block_motion(&picture, 1, 1, [32, 0, 8, 8], [0, 0], &targets)
        .unwrap()
        .unwrap();
    assert_eq!(motion.list, 1);
    assert_eq!(motion.mv, [0, 0]);
    assert_eq!(motion.source_reference.layer_id, 5);
    assert_eq!(motion.source_reference.poc, 2);
}
