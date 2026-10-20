//! Current-AU CQT/partition inference on independently authored texture/depth pictures.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn cqt_full_symmetric_and_amp_texture_partition_predictions_play_exactly() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for (size, log, split) in [
        (64usize, 5, false),
        (64, 6, true),
        (128, 6, true),
        (64, 6, false),
        (128, 6, false),
    ] {
        for layout in ["full", "hv", "top-bottom", "left-right"] {
            for enabled in [false, true] {
                for mode in ["match", "dc", "skip"] {
                    let stem = format!(
                        "cqt-{size}-ctu{}{}-{layout}-{}-{mode}",
                        1 << log,
                        if log == 6 && !split { "-keep" } else { "" },
                        if enabled { "on" } else { "off" }
                    );
                    let expected: Vec<u16> = std::fs::read(root.join(format!("{stem}.y16")))
                        .unwrap()
                        .chunks_exact(2)
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .collect();
                    for dlt in if enabled && mode == "dc" {
                        &[false, true][..]
                    } else {
                        &[false][..]
                    } {
                        for direct in if size == 128 {
                            &[false, true][..]
                        } else {
                            &[false][..]
                        } {
                            let movie = format!(
                                "{stem}{}{}.mp4",
                                if *dlt { "-dlt" } else { "" },
                                if *direct { "-direct" } else { "" }
                            );
                            let mut reader = LayeredMp4VideoReader::open(
                                std::io::Cursor::new(std::fs::read(root.join(&movie)).unwrap()),
                                Limits::default(),
                                1,
                                0,
                                64 << 20,
                            )
                            .unwrap();
                            for _ in 0..2 {
                                let mut depths = 0;
                                let mut layers = Vec::new();
                                while let Some(frame) = reader
                                    .read_frame()
                                    .unwrap_or_else(|e| panic!("{movie}: {e}"))
                                {
                                    layers.push(frame.decoded.layer_id);
                                    let tex_parts = match layout {
                                        "full" => [0, 0],
                                        "hv" => [1, 2],
                                        "top-bottom" => [4, 5],
                                        _ => [6, 7],
                                    };
                                    let poc = frame.decoded.poc as usize;
                                    let texture_part =
                                        if poc == 0 { 0 } else { tex_parts[poc - 1] };
                                    let expected_part = if frame.decoded.layer_id == 0 {
                                        texture_part
                                    } else if poc == 0 || mode != "match" {
                                        0
                                    } else {
                                        texture_part
                                    };
                                    let expected_log = if frame.decoded.layer_id == 0
                                        || poc == 0
                                        || split
                                        || log == 5
                                    {
                                        5
                                    } else {
                                        6
                                    };
                                    for y in (0..size).step_by(8) {
                                        for x in (0..size).step_by(8) {
                                            let (actual_log, actual_part) = frame
                                                .decoded
                                                .picture
                                                .coding_unit_at([x as u32, y as u32])
                                                .unwrap();
                                            assert_eq!(
                                                (actual_log, actual_part as u8),
                                                (expected_log, expected_part),
                                                "{movie}: CU geometry at {x},{y}"
                                            );
                                        }
                                    }
                                    assert!(
                                        frame
                                            .decoded
                                            .picture
                                            .coding_unit_at([size as u32, 0])
                                            .is_err()
                                    );
                                    let actual = frame.decoded.picture.planes[0].samples();
                                    if frame.decoded.layer_id == 3 {
                                        assert_eq!(frame.decoded.poc, depths as i32, "{movie}");
                                        let wanted = &expected
                                            [depths * size * size..(depths + 1) * size * size];
                                        assert_eq!(actual.len(), wanted.len());
                                        assert_eq!(
                                            actual
                                                .iter()
                                                .zip(wanted)
                                                .enumerate()
                                                .find(|(_, (a, b))| a != b),
                                            None,
                                            "{movie}: POC{depths}"
                                        );
                                        depths += 1;
                                    } else {
                                        for y in 0..size {
                                            for x in 0..size {
                                                assert_eq!(
                                                    actual[y * size + x],
                                                    ((x * 3 + y * 5 + 17) % 256) as u16,
                                                    "{movie}"
                                                );
                                            }
                                        }
                                    }
                                }
                                assert_eq!(depths, 3);
                                assert_eq!(layers, [0, 3, 0, 3, 0, 3]);
                                reader.rewind();
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn mixed_first_intra_then_predictive_slice_still_binds_current_texture_map() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let expected: Vec<u16> = std::fs::read(root.join("cqt-mixed-I-P.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    for label in ["off", "on"] {
        let movie = format!("cqt-mixed-I-P-{label}.mp4");
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(std::fs::read(root.join(&movie)).unwrap()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut count = 0;
            let mut layers = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{movie}: {e}"))
            {
                layers.push(frame.decoded.layer_id);
                if frame.decoded.layer_id == 3 {
                    assert_eq!(frame.decoded.poc, count as i32);
                    assert_eq!(
                        frame.decoded.picture.planes[0].samples(),
                        &expected[count * 4096..(count + 1) * 4096],
                        "{movie}"
                    );
                    count += 1;
                    assert_eq!(frame.decoded.picture.coding_unit_at([32, 0]).unwrap().0, 5);
                }
            }
            assert_eq!(count, 3);
            assert_eq!(layers, [0, 3, 0, 3, 0, 3]);
            reader.rewind();
        }
    }
}

/// This is exact motion-metadata acceptance, not texture_mc playback admission.
#[test]
fn adjacent_amp_prediction_blocks_preserve_exact_texture_motion_and_reference_identity() {
    use fvid::codec::hevc_motion::{
        Vector,
        texture::{Identity, Vector as DepthVector},
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(root.join("cqt-exact-texture-motion.mp4")).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut layers = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            layers.push(frame.decoded.layer_id);
            let picture = &frame.decoded.picture;
            if frame.decoded.layer_id == 0 {
                assert!(picture.planes[0].samples().iter().all(|&v| v == 128));
                for y in 0..32 {
                    let wanted = if frame.decoded.poc == 0 {
                        [None; 2]
                    } else {
                        [
                            Some(Vector {
                                reference: 0,
                                mv: [0, if y < 8 { 1 } else { -1 }],
                            }),
                            None,
                        ]
                    };
                    assert_eq!(
                        picture.prediction_motion_at([0, y]).unwrap(),
                        wanted,
                        "POC{} row{y}",
                        frame.decoded.poc
                    );
                    let target = [Identity { poc: 0, view: 9 }];
                    let candidate = picture
                        .texture_merge_candidate_at([0, y], [&target, &[]], false, |layer| {
                            assert_eq!(layer, 0);
                            Ok(9)
                        })
                        .unwrap();
                    assert_eq!(
                        candidate,
                        if frame.decoded.poc == 0 {
                            [None; 2]
                        } else {
                            [
                                Some(DepthVector {
                                    reference: 0,
                                    samples: [0; 2],
                                }),
                                None,
                            ]
                        }
                    );
                    let mismatch = [Identity { poc: 0, view: 8 }];
                    assert_eq!(
                        picture
                            .texture_merge_candidate_at([0, y], [&mismatch, &[]], false, |_| Ok(9))
                            .unwrap(),
                        [None; 2]
                    );
                }
                assert!(picture.prediction_motion_at([32, 0]).is_err());
            } else {
                for y in 0..32 {
                    for x in 0..32 {
                        assert_eq!(
                            picture.planes[0].samples()[y * 32 + x],
                            ((x * 7 + y * 11 + 37) % 256) as u16
                        );
                    }
                }
            }
        }
        assert_eq!(layers, [0, 3, 0, 3]);
        reader.rewind();
    }
}

#[test]
fn tiny_b_prediction_blocks_disable_l1_after_parallel_shared_cu_derivation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for layout in ["vertical", "horizontal"] {
        for parallel in [2, 3] {
            let movie = format!("tiny-merge-{layout}-parallel{parallel}.mp4");
            let mut reader = LayeredMp4VideoReader::open(
                std::io::Cursor::new(std::fs::read(root.join(&movie)).unwrap()),
                Limits::default(),
                1,
                0,
                64 << 20,
            )
            .unwrap();
            for _ in 0..2 {
                let mut layers = Vec::new();
                while let Some(frame) = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{movie}: {e}"))
                {
                    layers.push(frame.decoded.layer_id);
                    let picture = &frame.decoded.picture;
                    if frame.decoded.layer_id == 0 {
                        assert!(picture.planes[0].samples().iter().all(|&v| v == 128));
                    } else {
                        for y in 0..32 {
                            for x in 0..32 {
                                assert_eq!(
                                    picture.planes[0].samples()[y * 32 + x],
                                    ((x * 7 + y * 11 + 37) % 256) as u16,
                                    "{movie}"
                                );
                            }
                        }
                        if frame.decoded.poc == 2 {
                            for y in (0..32).step_by(8) {
                                for x in (0..32).step_by(8) {
                                    let motion = picture.prediction_motion_at([x, y]).unwrap();
                                    assert!(motion[0].is_some(), "{movie}: missing L0 at{x},{y}");
                                    assert!(
                                        motion[1].is_none(),
                                        "{movie}: tiny PB incorrectly keeps L1 at{x},{y}"
                                    );
                                }
                            }
                        }
                    }
                }
                assert_eq!(layers, [0, 3, 0, 3, 0, 3]);
                reader.rewind();
            }
        }
    }
}

/// Native sub-block sample prediction from coded texture motion. The MP4 does
/// not enable tex_mc; full active-tool playback admission remains a separate gate.
#[test]
fn component_integer_subblock_prediction_follows_coded_texture_pb_boundaries() {
    use fvid::codec::hevc_motion::{
        Reference,
        texture::{self, Identity},
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(root.join("cqt-integer-texture-motion.mp4")).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut reference = None;
        let mut checked = false;
        while let Some(frame) = reader.read_frame().unwrap() {
            if frame.decoded.layer_id == 3 && frame.decoded.poc == 0 {
                reference = Some(frame.decoded.picture.clone());
            }
            if frame.decoded.layer_id != 0 || frame.decoded.poc != 1 {
                continue;
            }
            let picture = &frame.decoded.picture;
            assert!(picture.planes[0].samples().iter().all(|&v| v == 128));
            for y in 0..32 {
                let motion = picture.prediction_motion_at([0, y]).unwrap()[0].unwrap();
                assert_eq!(motion.mv, [0, if y < 8 { 4 } else { -4 }], "texture row{y}");
            }
            let target = [Identity { poc: 0, view: 9 }];
            let blocks = texture::subblocks([0, 0, 32, 32], [32, 32], 3, |point| {
                picture.texture_merge_candidate_at(point, [&target, &[]], false, |_| Ok(9))
            })
            .unwrap();
            assert_eq!(blocks.size, [8, 8]);
            assert_eq!(blocks.default[0].unwrap().samples, [0, -1]);
            assert_eq!(blocks.motion.len(), 16);
            let reference = reference.as_ref().unwrap();
            let lists = [
                vec![Reference {
                    layer_id: 3,
                    poc: 0,
                    long_term: false,
                    picture: Some(reference.clone()),
                }],
                vec![],
            ];
            let mut actual = vec![0u16; 1024];
            for (index, motion) in blocks.motion.into_iter().enumerate() {
                let x = (index % 4) * 8;
                let y = (index / 4) * 8;
                let pixels =
                    texture::predict(&lists, motion, [x as u32, y as u32, 8, 8], 8, None).unwrap();
                for row in 0..8 {
                    actual[(y + row) * 32 + x..(y + row) * 32 + x + 8]
                        .copy_from_slice(&pixels[row * 8..row * 8 + 8]);
                }
            }
            for y in 0..32 {
                for x in 0..32 {
                    let yy = (y as i32 + if y < 8 { 1 } else { -1 }).clamp(0, 31) as usize;
                    assert_eq!(
                        actual[y * 32 + x],
                        ((x * 7 + yy * 11 + 37) % 256) as u16,
                        "depth sample{x},{y}"
                    );
                }
            }
            assert_ne!(actual, reference.planes[0].samples());
            checked = true;
        }
        assert!(checked);
        reader.rewind();
    }
}

#[test]
fn texture_subblocks_bind_real_vps_views_and_active_depth_references() {
    use fvid::codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredDpb, LayeredParameters},
        hevc_motion::{Reference, texture},
        hevc_picture::derive_texture_subblocks,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let data = std::fs::read(root.join("cqt-integer-texture-motion.mp4")).unwrap();
    for _ in 0..2 {
        let mut movie = fvid::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(data.clone()),
            Limits::default(),
        )
        .unwrap();
        let mut params =
            LayeredParameters::from_configuration(&movie.tracks()[0].configuration, 64 << 20)
                .unwrap();
        let mut dpb = LayeredDpb::default();
        let mut packet = Vec::new();
        movie.read_packet(0, 0, &mut packet).unwrap();
        let plan = LayeredAccessUnit::parse(&params, dpb.poc(), &packet, 4, 1, 64 << 20).unwrap();
        let (updated, frames, _) = dpb.decode(plan, 64 << 20).unwrap();
        params = updated;
        let reference = frames
            .iter()
            .find(|f| f.layer_id == 3)
            .unwrap()
            .picture
            .clone();
        movie.read_packet(0, 1, &mut packet).unwrap();
        let plan = LayeredAccessUnit::parse(&params, dpb.poc(), &packet, 4, 1, 64 << 20).unwrap();
        let depth = plan
            .pictures()
            .iter()
            .find(|p| p.binding.sps.layer_id == 3)
            .unwrap();
        let binding = depth.binding.clone();
        let header = depth.slices[0].clone();
        let (_, frames, _) = dpb.decode(plan, 64 << 20).unwrap();
        let picture = &frames.iter().find(|f| f.layer_id == 0).unwrap().picture;
        let lists = [
            vec![Reference {
                layer_id: 3,
                poc: 0,
                long_term: false,
                picture: Some(reference),
            }],
            vec![],
        ];
        let blocks =
            derive_texture_subblocks(&binding.context(), &header, picture, &lists, [0, 0, 32, 32])
                .unwrap();
        assert_eq!(blocks.size, [8, 8]);
        assert_eq!(blocks.motion.len(), 16);
        for (index, motion) in blocks.motion.iter().enumerate() {
            assert_eq!(motion[0].unwrap().reference, 0);
            assert_eq!(
                motion[0].unwrap().samples,
                [0, if index < 4 { 1 } else { -1 }]
            );
            let x = (index % 4) * 8;
            let y = (index / 4) * 8;
            let pixels =
                texture::predict(&lists, *motion, [x as u32, y as u32, 8, 8], 8, None).unwrap();
            for row in 0..8 {
                for col in 0..8 {
                    let yy = (y as i32 + row as i32 + if index < 4 { 1 } else { -1 }).clamp(0, 31)
                        as usize;
                    assert_eq!(
                        pixels[row * 8 + col],
                        (((x + col) * 7 + yy * 11 + 37) % 256) as u16
                    );
                }
            }
        }
        let wrong_count = [vec![], vec![]];
        assert!(
            derive_texture_subblocks(
                &binding.context(),
                &header,
                picture,
                &wrong_count,
                [0, 0, 32, 32]
            )
            .is_err()
        );
        let mut unknown = lists.clone();
        unknown[0][0].layer_id = 62;
        assert!(
            derive_texture_subblocks(
                &binding.context(),
                &header,
                picture,
                &unknown,
                [0, 0, 32, 32]
            )
            .is_err()
        );
        let mut opposite_role = lists.clone();
        opposite_role[0][0].layer_id = 0;
        assert!(
            derive_texture_subblocks(
                &binding.context(),
                &header,
                picture,
                &opposite_role,
                [0, 0, 32, 32]
            )
            .is_err()
        );
        let mut different_poc = lists.clone();
        different_poc[0][0].poc = 99;
        let missing = derive_texture_subblocks(
            &binding.context(),
            &header,
            picture,
            &different_poc,
            [0, 0, 32, 32],
        )
        .unwrap();
        assert!(missing.motion.is_empty());
        assert_eq!(missing.default, [None; 2]);
    }
}

#[test]
fn depth_nonzero_mvd_is_integer_sample_motion_with_full_signed_range() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for size in [32usize, 64, 128] {
        for (label, mv) in [
            ("zero", [0, 0]),
            ("right", [1, 0]),
            ("left", [-1, 0]),
            ("down", [0, 1]),
            ("up", [0, -1]),
            ("positive-limit", [0, 32767]),
            ("negative-limit", [0, -32768]),
        ] {
            let stem = format!(
                "depth-mvd-{}{label}",
                if size == 32 {
                    String::new()
                } else {
                    format!("{size}-")
                }
            );
            let expected: Vec<_> = std::fs::read(root.join(format!("{stem}.y16")))
                .unwrap()
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect();
            let mut reader = LayeredMp4VideoReader::open(
                std::io::Cursor::new(std::fs::read(root.join(format!("{stem}.mp4"))).unwrap()),
                Limits::default(),
                1,
                0,
                64 << 20,
            )
            .unwrap();
            for _ in 0..2 {
                let mut count = 0;
                while let Some(frame) = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{stem}: {e}"))
                {
                    if frame.decoded.layer_id != 3 {
                        continue;
                    }
                    assert_eq!(frame.decoded.poc, count);
                    let picture = &frame.decoded.picture;
                    let wanted =
                        &expected[count as usize * size * size..(count as usize + 1) * size * size];
                    assert_eq!(
                        picture.planes[0]
                            .samples()
                            .iter()
                            .zip(wanted)
                            .enumerate()
                            .find(|(_, (a, b))| a != b),
                        None,
                        "{stem}: POC{count} integer-pixel prediction"
                    );
                    if count > 0 {
                        for y in (0..size).step_by(32) {
                            for x in (0..size).step_by(32) {
                                let motion =
                                    picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                                assert_eq!(
                                    motion[0].unwrap().mv,
                                    if count == 1 {
                                        mv.map(|v| v * 4)
                                    } else {
                                        [0; 2]
                                    },
                                    "{stem}: published motion at{x},{y}"
                                );
                                assert!(motion[1].is_none());
                            }
                        }
                    }
                    count += 1;
                }
                assert_eq!(count, 3);
                reader.rewind();
            }
        }
    }
}

#[test]
fn active_texture_motion_reconstructs_and_publishes_each_depth_subblock() {
    use fvid::codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredParameters},
        hevc_poc::LayeredPocState,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for size in [32usize, 128] {
        for limit in [false, true] {
            for enabled in [false, true] {
                for intra_source in [false, true] {
                    for dc in [false, true] {
                        for cqt in [false, true] {
                            for direct in if size == 128 {
                                &[false, true][..]
                            } else {
                                &[false][..]
                            } {
                                let movie = format!(
                                    "texture-motion{}{}-{}{}{}{}{}.mp4",
                                    if size == 32 {
                                        String::new()
                                    } else {
                                        format!("-{size}")
                                    },
                                    if limit { "-limit" } else { "" },
                                    if enabled { "on" } else { "off" },
                                    if dc { "-dc" } else { "" },
                                    if intra_source { "-intra-source" } else { "" },
                                    if cqt { "" } else { "-no-cqt" },
                                    if *direct { "-direct" } else { "" }
                                );
                                let data = std::fs::read(root.join(&movie)).unwrap();
                                let mut input = fvid::container::mp4::Mp4Reader::open(
                                    std::io::Cursor::new(data.clone()),
                                    Limits::default(),
                                )
                                .unwrap();
                                let params = LayeredParameters::from_configuration(
                                    &input.tracks()[0].configuration,
                                    64 << 20,
                                )
                                .unwrap();
                                let mut packet = Vec::new();
                                input.read_packet(0, 0, &mut packet).unwrap();
                                let plan = LayeredAccessUnit::parse(
                                    &params,
                                    &LayeredPocState::default(),
                                    &packet,
                                    4,
                                    1,
                                    64 << 20,
                                )
                                .unwrap();
                                let base = plan
                                    .pictures()
                                    .iter()
                                    .find(|p| p.binding.sps.layer_id == 0)
                                    .unwrap();
                                let depth = plan
                                    .pictures()
                                    .iter()
                                    .find(|p| p.binding.sps.layer_id == 3)
                                    .unwrap();
                                assert_eq!(
                                    base.binding.sps.effective_profile.profile.unwrap().idc,
                                    1
                                );
                                let profile = depth.binding.sps.effective_profile.profile.unwrap();
                                assert_eq!(profile.idc, 8, "{movie}: selected enhancement PTL");
                                assert_eq!(
                                    profile.constraints,
                                    [43, 42, 41, 40, 39, 35]
                                        .iter()
                                        .map(|bit| 1u64 << bit)
                                        .sum::<u64>()
                                );
                                assert_eq!(
                                    base.binding.sps.sps.dimensions,
                                    depth.binding.sps.sps.dimensions
                                );
                                assert_eq!(base.binding.sps.sps.chroma_format, 1);
                                assert_eq!(depth.binding.sps.sps.chroma_format, 0);
                                assert_eq!(depth.binding.sps.sps.depth, [8, 8]);
                                assert!(depth.binding.pps.reference_locations.is_empty());
                                assert_eq!(base.binding.sps.sps.depth, [8, 8]);
                                for binding in [&base.binding, &depth.binding] {
                                    let s = &binding.sps.sps;
                                    assert!(!s.separate_colour_plane && !s.scc_extension);
                                    assert!(
                                        ![
                                            s.transform_skip_rotation,
                                            s.transform_skip_context,
                                            s.implicit_rdpcm,
                                            s.explicit_rdpcm,
                                            s.extended_precision,
                                            s.intra_smoothing_disabled,
                                            s.high_precision_offsets,
                                            s.persistent_rice,
                                            s.cabac_bypass_alignment
                                        ]
                                        .into_iter()
                                        .any(|v| v)
                                    );
                                    assert!((4..=6).contains(&s.coding_block_log2[1]));
                                    assert_eq!(binding.pps.pps.transform_skip_max_log2, 2);
                                    assert!(binding.pps.pps.chroma_qp_offset_list.is_none());
                                    assert!(binding.pps.colour_mapping.is_none());
                                }

                                let graph = &depth
                                    .binding
                                    .context()
                                    .vps
                                    .extension
                                    .as_ref()
                                    .unwrap()
                                    .graph;
                                assert_eq!(graph.layer_ids, [0, 3]);
                                assert_eq!(graph.scalability_ids[0][0], 0);
                                assert_eq!(graph.scalability_ids[1][0], 1);
                                assert_eq!(
                                    graph.scalability_ids[0][1],
                                    graph.scalability_ids[1][1]
                                );
                                assert!(
                                    graph
                                        .scalability_ids
                                        .iter()
                                        .all(|d| d[2..].iter().all(|v| *v == 0))
                                );
                                let mut reader = LayeredMp4VideoReader::open(
                                    std::io::Cursor::new(data),
                                    Limits::default(),
                                    1,
                                    0,
                                    64 << 20,
                                )
                                .unwrap_or_else(|e| panic!("{movie}: {e}"));
                                for _ in 0..2 {
                                    let mut layers = Vec::new();
                                    let mut previous: Vec<u16> = (0..size)
                                        .flat_map(|y| {
                                            (0..size)
                                                .map(move |x| ((x * 7 + y * 11 + 37) % 256) as u16)
                                        })
                                        .collect();
                                    while let Some(frame) = reader
                                        .read_frame()
                                        .unwrap_or_else(|e| panic!("{movie}: {e}"))
                                    {
                                        layers.push(frame.decoded.layer_id);
                                        if frame.decoded.layer_id == 0 {
                                            assert!(
                                                frame.decoded.picture.planes[0]
                                                    .samples()
                                                    .iter()
                                                    .all(|&v| v == 128)
                                            );
                                            continue;
                                        }
                                        let active =
                                            enabled && !intra_source && frame.decoded.poc > 0;
                                        let motion_at = |y: usize| {
                                            if active {
                                                let texture = if y % 32 < 8 {
                                                    if limit { 32767 } else { 4 }
                                                } else {
                                                    if limit { -32767 } else { -4 }
                                                };
                                                (texture + 2) >> 2
                                            } else {
                                                0
                                            }
                                        };
                                        let mut expected = Vec::with_capacity(size * size);
                                        for y in 0..size {
                                            for x in 0..size {
                                                let yy = (y as i32 + motion_at(y))
                                                    .clamp(0, size as i32 - 1)
                                                    as usize;
                                                let offset = if dc && frame.decoded.poc > 0 {
                                                    if frame.decoded.poc == 1 { 10 } else { -10 }
                                                } else {
                                                    0
                                                };
                                                let value = (i32::from(previous[yy * size + x])
                                                    + offset)
                                                    .clamp(0, 255)
                                                    as u16;
                                                expected.push(value);
                                                assert_eq!(
                                                    frame.decoded.picture.planes[0].samples()
                                                        [y * size + x],
                                                    value,
                                                    "{movie}: sample{x},{y}"
                                                );
                                            }
                                        }
                                        previous = expected;
                                        if frame.decoded.poc > 0 {
                                            for y in (0..size).step_by(8) {
                                                for x in (0..size).step_by(8) {
                                                    let motion = frame
                                                        .decoded
                                                        .picture
                                                        .prediction_motion_at([x as u32, y as u32])
                                                        .unwrap();
                                                    assert_eq!(
                                                        motion[0].unwrap().mv,
                                                        [0, i64::from(motion_at(y)) * 4],
                                                        "{movie}: motion{x},{y}"
                                                    );
                                                    if active || frame.decoded.poc == 1 {
                                                        assert!(motion[1].is_none());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    assert_eq!(layers, [0, 3, 0, 3, 0, 3]);
                                    reader.rewind();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn common_player_entry_renders_and_seeks_selected_3d_main_depth_layer() {
    use fvid::playback::video::{
        mp4::Mp4VideoReader,
        native::{NativeReader, RawFrame},
    };
    use std::time::Duration;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for limit in [false, true] {
        for dc in [false, true] {
            let movie = format!(
                "texture-motion{}-on{}.mp4",
                if limit { "-limit" } else { "" },
                if dc { "-dc" } else { "" }
            );
            let data = std::fs::read(root.join(&movie)).unwrap();
            let mut expected: Vec<Vec<u8>> = vec![
                (0..32)
                    .flat_map(|y| (0..32).map(move |x| ((x * 7 + y * 11 + 37) % 256) as u8))
                    .collect(),
            ];
            for poc in 1..=2 {
                let previous = expected.last().unwrap();
                let mut next = Vec::with_capacity(1024);
                for y in 0..32 {
                    for x in 0..32 {
                        let texture = if y < 8 {
                            if limit { 32767 } else { 4 }
                        } else {
                            if limit { -32767 } else { -4 }
                        };
                        let yy = (y as i32 + ((texture + 2) >> 2)).clamp(0, 31) as usize;
                        let offset = if dc {
                            if poc == 1 { 10 } else { -10 }
                        } else {
                            0
                        };
                        next.push((i32::from(previous[yy * 32 + x]) + offset).clamp(0, 255) as u8);
                    }
                }
                expected.push(next);
            }
            let mut reader = Mp4VideoReader::open_software(
                std::io::Cursor::new(data.clone()),
                Limits::default(),
                64 << 20,
            )
            .unwrap();
            for _ in 0..2 {
                let mut count = 0;
                while let Some(frame) = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{movie}: common reader: {e}"))
                {
                    assert_eq!(reader.selected_layer_id(), Some(3));
                    let packed = frame.packed.unwrap();
                    assert_eq!(packed.depth, 8);
                    assert_eq!((packed.frame.width, packed.frame.height), (32, 32));
                    assert_eq!(
                        &packed.frame.data[..1024],
                        expected[frame.sample_index],
                        "{movie}: common packed Y"
                    );
                    assert!(packed.frame.data[1024..].iter().all(|v| *v == 128));
                    count += 1;
                }
                assert_eq!(count, 3);
                reader.rewind();
            }
            let mut reader = NativeReader::software(std::io::Cursor::new(data), 64 << 20).unwrap();
            assert_eq!(reader.video_codec(), "H.265");
            let frame = reader.seek_raw(Duration::from_millis(40)).unwrap().unwrap();
            let RawFrame::Planar(packed) = &frame else {
                panic!("{movie}: expected native packed picture")
            };
            assert_eq!(
                &packed.frame.data[..1024],
                expected[1],
                "{movie}: exact seek Y"
            );
            let rgba = frame.into_rgba(64 << 20).unwrap();
            assert_eq!(rgba.len(), 32 * 32 * 4);
            assert!(rgba.chunks_exact(4).all(|v| v[3] == 255));
        }
    }
}

/// Distinct POCs and distinct authored PCM patterns prevent L0-only or wrong
/// reference-identity implementations from passing this coded T witness.
#[test]
fn texture_bi_prediction_uses_distinct_past_and_future_depth_references() {
    check_texture_bi_prediction(None, None, 32, false);
}

#[test]
fn weighted_texture_bi_prediction_applies_signed_weights_offsets_and_clipping() {
    for weights in [
        (0, [(-3, -7), (5, 11)]),
        (1, [(3, 9), (-1, -5)]),
        (7, [(1, -127), (255, 127)]),
    ] {
        check_texture_bi_prediction(Some(weights), None, 32, false);
    }
}

fn check_texture_bi_prediction(
    weights: Option<(u8, [(i32, i32); 2])>,
    sao: Option<i32>,
    size: usize,
    direct: bool,
) {
    let reference_bias = if sao.is_some() { 37 } else { 97 };
    let prefix = format!(
        "texture-motion-bi-distinct{}{}",
        if size == 32 {
            String::new()
        } else {
            format!("-{size}")
        },
        if direct { "-direct" } else { "" }
    );
    let filter_suffix = sao.map_or(String::new(), |mode| {
        if mode < 32 {
            format!("-sao-{mode}")
        } else {
            format!("-sao-edge-{}", mode - 32)
        }
    });
    let filtered = sao.map(|mode| {
        let before: Vec<u16> = (0..size)
            .flat_map(|y| {
                (0..size).map(move |x| {
                    let (m0, m1) = if y % 32 < 8 { (1, -2) } else { (-1, 2) };
                    let a = ((7 * x + 11 * (y as i32 + m0).clamp(0, size as i32 - 1) as usize + 37)
                        % 256) as i32;
                    let b = ((7 * x
                        + 11 * (y as i32 + m1).clamp(0, size as i32 - 1) as usize
                        + 37
                        + reference_bias)
                        % 256) as i32;
                    if let Some((d, [(w0, o0), (w1, o1)])) = weights {
                        ((w0 * a + w1 * b + ((o0 + o1 + 1) << d)) >> (d + 1)).clamp(0, 255) as u16
                    } else {
                        ((a + b + 1) / 2) as u16
                    }
                })
            })
            .collect();
        let after = weighted_sao_oracle(&before, size, mode);
        assert!(
            after != before,
            "{prefix}: SAO must change pixels, mode{mode}"
        );
        after
    });
    let suffix = weights.map_or(String::new(), |(d, _)| format!("-weighted-{d}"));
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/fixtures/playback-errors/hevc-wedge/{prefix}{suffix}{filter_suffix}.mp4"
    ));
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(path).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut seen = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            let poc = frame.decoded.poc;
            let layer = frame.decoded.layer_id;
            seen.push((layer, poc));
            let picture = &frame.decoded.picture;
            for y in 0..size {
                for x in 0..size {
                    let pattern =
                        |row: usize, bias: usize| ((7 * x + 11 * row + 37 + bias) % 256) as u16;
                    let expected = if layer == 0 {
                        128
                    } else if poc != 1 {
                        pattern(y, if poc == 2 { reference_bias } else { 0 })
                    } else {
                        let (m0, m1) = if y % 32 < 8 { (1, -2) } else { (-1, 2) };
                        let a = pattern((y as i32 + m0).clamp(0, size as i32 - 1) as usize, 0);
                        let b = pattern(
                            (y as i32 + m1).clamp(0, size as i32 - 1) as usize,
                            reference_bias,
                        );
                        if let Some((d, [(w0, o0), (w1, o1)])) = weights {
                            ((w0 * i32::from(a) + w1 * i32::from(b) + ((o0 + o1 + 1) << d))
                                >> (d + 1))
                                .clamp(0, 255) as u16
                        } else {
                            (a + b + 1) / 2
                        }
                    };
                    let expected = if layer == 3 && poc == 1 {
                        filtered.as_ref().map_or(expected, |p| p[y * size + x])
                    } else {
                        expected
                    };
                    assert_eq!(
                        picture.planes[0].samples()[y * size + x],
                        expected,
                        "layer{layer} poc{poc} at{x},{y}"
                    );
                }
            }
            if poc == 1 {
                for y in (0..size as u32).step_by(8) {
                    let motion = picture.prediction_motion_at([0, y]).unwrap();
                    let vectors = if y % 32 < 8 { [4, -8] } else { [-4, 8] };
                    use fvid::codec::hevc_motion::texture::Identity;
                    let past = [Identity { poc: 0, view: 0 }];
                    let future = [Identity { poc: 2, view: 0 }];
                    let candidate = picture
                        .texture_merge_candidate_at(
                            [0, y],
                            [&past, &future],
                            true,
                            |source_layer| {
                                assert_eq!(source_layer, layer);
                                Ok(0)
                            },
                        )
                        .unwrap();
                    for list in 0..2 {
                        assert_eq!(candidate[list].unwrap().reference, 0);
                        assert_eq!(
                            candidate[list].unwrap().samples,
                            [0, (vectors[list] / 4) as i16]
                        );
                        assert_eq!(
                            motion[list].unwrap().mv,
                            [0, vectors[list]],
                            "layer{layer} list{list} row{y}"
                        );
                    }
                }
            }
        }
        assert_eq!(seen.len(), 6);
        assert!(seen.contains(&(3, 1)) && seen.contains(&(3, 2)));
        reader.rewind();
    }
}

#[test]
fn weighted_texture_p_prediction_matches_rounding_clipping_and_dc_in_direct_and_queued_paths() {
    for (size, direct) in [(32usize, false), (128, false), (128, true)] {
        for dc in [false, true] {
            for (denominator, weight, offset) in
                [(0u32, -3i32, 127i32), (0, 5, -7), (1, 3, 9), (7, 255, -127)]
            {
                let name = format!(
                    "texture-motion{}-on{}{}-weighted-{denominator}-{weight}-{offset}.mp4",
                    if size == 32 {
                        String::new()
                    } else {
                        format!("-{size}")
                    },
                    if dc { "-dc" } else { "" },
                    if direct { "-direct" } else { "" }
                );
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/playback-errors/hevc-wedge")
                    .join(&name);
                let mut reader = LayeredMp4VideoReader::open(
                    std::io::Cursor::new(std::fs::read(path).unwrap()),
                    Limits::default(),
                    1,
                    0,
                    64 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut seen = Vec::new();
                    while let Some(frame) = reader.read_frame().unwrap() {
                        let layer = frame.decoded.layer_id;
                        let poc = frame.decoded.poc;
                        seen.push((layer, poc));
                        let picture = &frame.decoded.picture;
                        for y in 0..size {
                            let motion = if y % 32 < 8 { 1 } else { -1 };
                            for x in 0..size {
                                let row = if poc == 1 {
                                    (y as i32 + motion).clamp(0, size as i32 - 1) as usize
                                } else {
                                    y
                                };
                                let source = ((7 * x + 11 * row + 37) % 256) as i32;
                                let expected = if layer == 0 {
                                    128
                                } else if poc == 0 {
                                    source
                                } else {
                                    let rounding = if denominator == 0 {
                                        0
                                    } else {
                                        1 << (denominator - 1)
                                    };
                                    // Weighting clips prediction before adding CU DC-only residual.
                                    let prediction =
                                        (((weight * source + rounding) >> denominator) + offset)
                                            .clamp(0, 255);
                                    (prediction + if dc { 10 } else { 0 }).clamp(0, 255)
                                } as u16;
                                assert_eq!(
                                    picture.planes[0].samples()[y * size + x],
                                    expected,
                                    "{name}: layer{layer} poc{poc} at{x},{y}"
                                );
                            }
                            if poc == 1 && y % 8 == 0 {
                                let mv = picture.prediction_motion_at([0, y as u32]).unwrap();
                                assert_eq!(mv[0].unwrap().mv, [0, 4 * i64::from(motion)], "{name}: row{y}");
                                assert!(mv[1].is_none());
                            }
                        }
                    }
                    assert_eq!(seen, [(0, 0), (3, 0), (0, 1), (3, 1)], "{name}");
                    reader.rewind();
                }
            }
        }
    }
}

#[test]
fn texture_subblock_motion_does_not_create_spurious_deblocking_edges() {
    for direct in [false, true] {
        for enabled in [false, true] {
            let name = format!(
                "texture-motion-128-on{}-deblock-{}.mp4",
                if direct { "-direct" } else { "" },
                if enabled { "on" } else { "off" }
            );
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/hevc-wedge")
                .join(&name);
            let mut reader = LayeredMp4VideoReader::open(
                std::io::Cursor::new(std::fs::read(path).unwrap()),
                Limits::default(),
                1,
                0,
                64 << 20,
            )
            .unwrap();
            for _ in 0..2 {
                let mut seen = Vec::new();
                while let Some(frame) = reader.read_frame().unwrap() {
                    let layer = frame.decoded.layer_id;
                    let poc = frame.decoded.poc;
                    seen.push((layer, poc));
                    for y in 0..128usize {
                        let row = if poc == 1 {
                            (y as i32 + if y % 32 < 8 { 1 } else { -1 }).clamp(0, 127)
                        } else {
                            y as i32
                        };
                        // QP26, beta16, tC1, bS1: weak delta=1 at true
                        // 32-row CU/PB boundaries. The per-8-row T boundaries
                        // are not independent prediction or transform edges.
                        let adjustment = if enabled && poc == 1 {
                            if y % 32 == 31 && y < 127 {
                                1
                            } else if y % 32 == 0 && y > 0 {
                                -1
                            } else {
                                0
                            }
                        } else {
                            0
                        };
                        let expected = if layer == 0 {
                            128
                        } else {
                            (row + 37 + adjustment) as u16
                        };
                        for x in 0..128usize {
                            assert_eq!(
                                frame.decoded.picture.planes[0].samples()[y * 128 + x],
                                expected,
                                "{name}: layer{layer} poc{poc} at{x},{y}"
                            );
                        }
                    }
                }
                assert_eq!(seen, [(0, 0), (3, 0), (0, 1), (3, 1)]);
                reader.rewind();
            }
        }
    }
}

#[test]
fn texture_motion_sao_band_offsets_merge_and_filtered_references_match_pixels() {
    for (size, direct) in [(32usize, false), (128, false), (128, true)] {
        for dc in [false, true] {
            for position in [0i32, 8, 31] {
                let name = format!(
                    "texture-motion{}-on{}{}-sao-{position}.mp4",
                    if size == 32 {
                        String::new()
                    } else {
                        format!("-{size}")
                    },
                    if dc { "-dc" } else { "" },
                    if direct { "-direct" } else { "" }
                );
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/playback-errors/hevc-wedge")
                    .join(&name);
                let mut reader = LayeredMp4VideoReader::open(
                    std::io::Cursor::new(std::fs::read(path).unwrap()),
                    Limits::default(),
                    1,
                    0,
                    64 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut previous: Vec<u16> = (0..size)
                        .flat_map(|y| (0..size).map(move |x| ((7 * x + 11 * y + 37) % 256) as u16))
                        .collect();
                    let mut seen = Vec::new();
                    while let Some(frame) = reader.read_frame().unwrap() {
                        let layer = frame.decoded.layer_id;
                        let poc = frame.decoded.poc;
                        seen.push((layer, poc));
                        let mut expected = Vec::new();
                        for y in 0..size {
                            let row = if poc > 0 {
                                (y as i32 + if y % 32 < 8 { 1 } else { -1 })
                                    .clamp(0, size as i32 - 1)
                                    as usize
                            } else {
                                y
                            };
                            for x in 0..size {
                                let value = if layer == 0 {
                                    128
                                } else if poc == 0 {
                                    previous[y * size + x]
                                } else {
                                    let residual = if dc {
                                        if poc == 1 { 10 } else { -10 }
                                    } else {
                                        0
                                    };
                                    let predicted = (i32::from(previous[row * size + x])
                                        + residual)
                                        .clamp(0, 255);
                                    let band =
                                        ((predicted >> 3) - position).rem_euclid(32) as usize;
                                    let offset = [7, -3, 2, -7].get(band).copied().unwrap_or(0);
                                    (predicted + offset).clamp(0, 255) as u16
                                };
                                assert_eq!(
                                    frame.decoded.picture.planes[0].samples()[y * size + x],
                                    value,
                                    "{name}: layer{layer} poc{poc} at{x},{y}"
                                );
                                expected.push(value);
                            }
                        }
                        if layer == 3 {
                            previous = expected;
                        }
                    }
                    assert_eq!(seen, [(0, 0), (3, 0), (0, 1), (3, 1), (0, 2), (3, 2)]);
                    reader.rewind();
                }
            }
        }
    }
}

#[test]
fn texture_motion_sao_edges_use_unfiltered_neighbours_and_picture_boundaries() {
    let mut categories = 0u8;
    for (size, direct) in [(32usize, false), (128, false), (128, true)] {
        for dc in [false, true] {
            for class in 0..4usize {
                let name = format!(
                    "texture-motion{}-on{}{}-sao-edge-{class}.mp4",
                    if size == 32 {
                        String::new()
                    } else {
                        format!("-{size}")
                    },
                    if dc { "-dc" } else { "" },
                    if direct { "-direct" } else { "" }
                );
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/playback-errors/hevc-wedge")
                    .join(&name);
                let mut reader = LayeredMp4VideoReader::open(
                    std::io::Cursor::new(std::fs::read(path).unwrap()),
                    Limits::default(),
                    1,
                    0,
                    64 << 20,
                )
                .unwrap();
                let (dx, dy) = [(1i32, 0i32), (0, 1), (1, 1), (-1, 1)][class];
                for _ in 0..2 {
                    let mut previous: Vec<u16> = (0..size)
                        .flat_map(|y| (0..size).map(move |x| ((7 * x + 11 * y + 37) % 256) as u16))
                        .collect();
                    let mut seen = Vec::new();
                    while let Some(frame) = reader.read_frame().unwrap() {
                        let layer = frame.decoded.layer_id;
                        let poc = frame.decoded.poc;
                        seen.push((layer, poc));
                        let mut reconstructed = vec![0i32; size * size];
                        for y in 0..size {
                            let row = if poc > 0 {
                                (y as i32 + if y % 32 < 8 { 1 } else { -1 })
                                    .clamp(0, size as i32 - 1)
                                    as usize
                            } else {
                                y
                            };
                            for x in 0..size {
                                let residual = if dc && poc > 0 {
                                    if poc == 1 { 10 } else { -10 }
                                } else {
                                    0
                                };
                                reconstructed[y * size + x] =
                                    (i32::from(previous[row * size + x]) + residual).clamp(0, 255);
                            }
                        }
                        let mut expected = Vec::new();
                        let mut changed = 0;
                        for y in 0..size {
                            for x in 0..size {
                                let sample = reconstructed[y * size + x];
                                let inside = |x: i32, y: i32| {
                                    x >= 0 && y >= 0 && x < size as i32 && y < size as i32
                                };
                                let (ax, ay) = (x as i32 - dx, y as i32 - dy);
                                let (bx, by) = (x as i32 + dx, y as i32 + dy);
                                let offset =
                                    if layer == 3 && poc > 0 && inside(ax, ay) && inside(bx, by) {
                                        let a = reconstructed[ay as usize * size + ax as usize];
                                        let b = reconstructed[by as usize * size + bx as usize];
                                        let category =
                                            (sample - a).signum() + (sample - b).signum();
                                        categories |= 1 << ((category + 2) as u32);
                                        match category {
                                            -2 => 7,
                                            -1 => 3,
                                            1 => -2,
                                            2 => -7,
                                            _ => 0,
                                        }
                                    } else {
                                        0
                                    };
                                let value = if layer == 0 {
                                    128
                                } else {
                                    (sample + offset).clamp(0, 255) as u16
                                };
                                if layer == 3 && i32::from(value) != sample {
                                    changed += 1;
                                }
                                assert_eq!(
                                    frame.decoded.picture.planes[0].samples()[y * size + x],
                                    value,
                                    "{name}: layer{layer} poc{poc} at{x},{y}"
                                );
                                expected.push(value);
                            }
                        }
                        if layer == 3 {
                            if poc > 0 {
                                assert!(changed > 0, "{name}: filter must change pixels");
                            }
                            previous = expected;
                        }
                    }
                    assert_eq!(seen, [(0, 0), (3, 0), (0, 1), (3, 1), (0, 2), (3, 2)]);
                    reader.rewind();
                }
            }
        }
    }
    assert_eq!(categories, 0b11111, "all five edge categories must occur");
}

/// Independent sample oracle: classification always reads the complete
/// reconstructed plane, never this function's progressively written output.
fn weighted_sao_oracle(before: &[u16], size: usize, mode: i32) -> Vec<u16> {
    let mut out = before.to_vec();
    for y in 0..size {
        for x in 0..size {
            let sample = i32::from(before[y * size + x]);
            let offset = if mode < 32 {
                let band = ((sample >> 3) - mode).rem_euclid(32) as usize;
                [7, -3, 2, -7].get(band).copied().unwrap_or(0)
            } else {
                let (dx, dy) = [(1i32, 0i32), (0, 1), (1, 1), (-1, 1)][(mode - 32) as usize];
                let inside =
                    |x: i32, y: i32| x >= 0 && y >= 0 && x < size as i32 && y < size as i32;
                let (ax, ay) = (x as i32 - dx, y as i32 - dy);
                let (bx, by) = (x as i32 + dx, y as i32 + dy);
                if inside(ax, ay) && inside(bx, by) {
                    let a = i32::from(before[ay as usize * size + ax as usize]);
                    let b = i32::from(before[by as usize * size + bx as usize]);
                    match (sample - a).signum() + (sample - b).signum() {
                        -2 => 7,
                        -1 => 3,
                        1 => -2,
                        2 => -7,
                        _ => 0,
                    }
                } else {
                    0
                }
            };
            out[y * size + x] = (sample + offset).clamp(0, 255) as u16;
        }
    }
    out
}

#[test]
fn weighted_texture_prediction_and_sao_preserve_reconstruction_order() {
    for (size, direct) in [(32usize, false), (128, false), (128, true)] {
        for dc in [false, true] {
            for (d, w, o) in [(0u32, -3i32, 127i32), (0, 5, -7), (1, 3, 9), (7, 255, -127)] {
                for sao in [0i32, 8, 31, 32, 33, 34, 35] {
                    let label = if sao < 32 {
                        sao.to_string()
                    } else {
                        format!("edge-{}", sao - 32)
                    };
                    let name = format!(
                        "texture-motion{}-on{}{}-sao-{label}-weighted-{d}-{w}-{o}.mp4",
                        if size == 32 {
                            String::new()
                        } else {
                            format!("-{size}")
                        },
                        if dc { "-dc" } else { "" },
                        if direct { "-direct" } else { "" }
                    );
                    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/playback-errors/hevc-wedge")
                        .join(&name);
                    let mut reader = LayeredMp4VideoReader::open(
                        std::io::Cursor::new(std::fs::read(path).unwrap()),
                        Limits::default(),
                        1,
                        0,
                        64 << 20,
                    )
                    .unwrap();
                    let initial: Vec<u16> = (0..size)
                        .flat_map(|y| (0..size).map(move |x| ((7 * x + 11 * y + 37) % 256) as u16))
                        .collect();
                    let before: Vec<u16> = (0..size)
                        .flat_map(|y| {
                            let initial = &initial;
                            (0..size).map(move |x| {
                                let row = (y as i32 + if y % 32 < 8 { 1 } else { -1 })
                                    .clamp(0, size as i32 - 1)
                                    as usize;
                                let rounding = if d == 0 { 0 } else { 1 << (d - 1) };
                                let prediction =
                                    (((w * i32::from(initial[row * size + x]) + rounding) >> d)
                                        + o)
                                        .clamp(0, 255);
                                (prediction + if dc { 10 } else { 0 }).clamp(0, 255) as u16
                            })
                        })
                        .collect();
                    let filtered = weighted_sao_oracle(&before, size, sao);
                    assert_ne!(
                        filtered, before,
                        "{name}: SAO must have an observable effect"
                    );
                    for _ in 0..2 {
                        let mut seen = Vec::new();
                        while let Some(frame) = reader.read_frame().unwrap() {
                            let layer = frame.decoded.layer_id;
                            let poc = frame.decoded.poc;
                            seen.push((layer, poc));
                            let pixels = frame.decoded.picture.planes[0].samples();
                            if layer == 0 {
                                assert!(pixels.iter().all(|&v| v == 128));
                            } else {
                                assert_eq!(
                                    pixels,
                                    if poc == 0 { &initial } else { &filtered },
                                    "{name}: poc{poc}"
                                );
                            }
                        }
                        assert_eq!(seen, [(0, 0), (3, 0), (0, 1), (3, 1)]);
                        reader.rewind();
                    }
                }
            }
        }
    }
}

#[test]
fn weighted_b_texture_sao_combinations_match_distinct_reference_oracles() {
    for (size, direct) in [(32usize, false), (128, false), (128, true)] {
        if size != 32 {
            check_texture_bi_prediction(None, None, size, direct);
        }
        for weights in [
            (0, [(-3, -7), (5, 11)]),
            (1, [(3, 9), (-1, -5)]),
            (7, [(1, -127), (255, 127)]),
        ] {
            if size != 32 {
                check_texture_bi_prediction(Some(weights), None, size, direct);
            }
            for mode in [0, 8, 31, 32, 33, 34, 35] {
                check_texture_bi_prediction(Some(weights), Some(mode), size, direct);
            }
        }
    }
}
