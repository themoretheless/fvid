//! Native texture depth refinement with owned mixed-role PCM/DI witnesses.
use fvid::codec::hevc_vps::LayeredVps;
#[test]
fn mixed_texture_depth_topology_selects_reference_view_depth_and_camera() {
    let bytes = include_bytes!("fixtures/playback-errors/hevc-wedge/texture-depth-refinement.vps");
    let vps = LayeredVps::parse(bytes, 1 << 20).unwrap();
    let graph = &vps.extension.as_ref().unwrap().graph;
    assert_eq!(graph.layer_ids, [0, 3, 5]);
    assert_eq!(graph.view_component_layer(0, true).unwrap(), Some(3));
    assert_eq!(graph.view_component_layer(3, false).unwrap(), Some(0));
    assert_eq!(graph.view_component_layer(5, true).unwrap(), None);
    let camera = vps.three_d_extension.as_ref().unwrap();
    assert_eq!(camera.views.len(), 2);
    assert_eq!(camera.views[1].pairs[0].reference_view, 0);
    assert_eq!(camera.views[1].pairs[0].parameters, Some([128, 0, 0, 0]));
}

#[test]
fn mixed_role_control_decodes_all_components_before_refinement_admission() {
    use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
    let bytes =
        include_bytes!("fixtures/playback-errors/hevc-wedge/texture-depth-refinement-control.mp4");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(bytes.as_slice()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    let mut ids = Vec::new();
    while let Some(frame) = reader.read_frame().unwrap() {
        let picture = &frame.decoded.picture;
        let depth = frame.decoded.layer_id == 3;
        assert_eq!(picture.planes[0].dimensions(), [32, 32]);
        for y in 0..32 {
            for x in 0..32 {
                let expected = if depth {
                    x * 7 + y * 11 + 37
                } else {
                    x * 3 + y * 5 + 17
                };
                assert_eq!(
                    picture.planes[0].samples()[y * 32 + x],
                    (expected % 256) as u16
                );
            }
        }
        for component in 1..3 {
            let plane = &picture.planes[component];
            assert_eq!(plane.dimensions(), if depth { [0, 0] } else { [16, 16] });
            if !depth {
                for y in 0..16 {
                    for x in 0..16 {
                        assert_eq!(
                            plane.samples()[y * 16 + x],
                            ((x * 13 + y * 7 + 29 + (component - 1) * 83) % 256) as u16
                        );
                    }
                }
            }
        }
        assert_eq!(frame.presentation_time.ticks, i64::from(frame.decoded.poc));
        ids.push((frame.decoded.poc, frame.decoded.layer_id));
    }
    assert_eq!(ids, [(0, 0), (0, 3), (0, 5), (1, 0), (1, 3), (1, 5)]);
}

#[test]
fn active_depth_refinement_changes_native_di_pixels_using_reference_depth() {
    use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
    let bytes =
        include_bytes!("fixtures/playback-errors/hevc-wedge/texture-depth-refinement-on.mp4");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(bytes.as_slice()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut count = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            count += 1;
            if frame.decoded.layer_id == 5 && frame.decoded.poc == 1 {
                // Corner depths 37,254,122,83: maximum 254. Camera scale 4/128
                // rounds to +8 quarter-pels, translating the source by two luma pixels.
                for y in 0..32 {
                    for x in 0..32 {
                        assert_eq!(
                            frame.decoded.picture.planes[0].samples()[y * 32 + x],
                            ((3 * (x + 2).min(31) + 5 * y + 17) % 256) as u16,
                            "x={x},y={y}"
                        );
                    }
                }
                for plane in &frame.decoded.picture.planes[1..] {
                    assert!(plane.samples().iter().all(|&v| v == 128));
                }
            }
        }
        assert_eq!(count, 6);
        reader.rewind();
    }
}

#[test]
fn disabled_refinement_control_uses_native_di_without_depth() {
    use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
    for bytes in [
        include_bytes!("fixtures/playback-errors/hevc-wedge/texture-depth-refinement-off.mp4")
            .as_slice(),
        include_bytes!(
            "fixtures/playback-errors/hevc-wedge/texture-depth-refinement-no-camera.mp4"
        )
        .as_slice(),
    ] {
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(bytes),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut count = 0;
            while let Some(frame) = reader.read_frame().unwrap() {
                count += 1;
                if frame.decoded.layer_id == 5 && frame.decoded.poc == 1 {
                    for y in 0..32 {
                        for x in 0..32 {
                            assert_eq!(
                                frame.decoded.picture.planes[0].samples()[y * 32 + x],
                                ((3 * x + 5 * y + 17) % 256) as u16
                            );
                        }
                    }
                    for plane in &frame.decoded.picture.planes[1..] {
                        assert!(plane.samples().iter().all(|&v| v == 128));
                    }
                }
            }
            assert_eq!(count, 6);
            reader.rewind();
        }
    }
}

#[test]
fn software_player_selects_refined_texture_and_repeats_seeks() {
    use fvid::playback::video::{
        mp4::Mp4VideoReader,
        native::{NativeReader, RawFrame},
    };
    use std::{io::Cursor, time::Duration};
    let bytes =
        include_bytes!("fixtures/playback-errors/hevc-wedge/texture-depth-refinement-on.mp4");
    let check = |data: &[u8], poc: i64| {
        assert_eq!(data.len(), 1536);
        for y in 0..32 {
            for x in 0..32 {
                let source_x = if poc == 1 { (x + 2).min(31) } else { x };
                assert_eq!(data[y * 32 + x], ((3 * source_x + 5 * y + 17) % 256) as u8);
            }
        }
        assert!(data[1024..].iter().all(|&v| v == 128));
    };
    let mut reader =
        Mp4VideoReader::open_software(Cursor::new(bytes.as_slice()), Default::default(), 64 << 20)
            .unwrap();
    for _ in 0..2 {
        let mut seen = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            assert_eq!(reader.selected_layer_id(), Some(5));
            check(
                &frame.packed.unwrap().frame.data,
                frame.presentation_time.ticks,
            );
            seen.push(frame.presentation_time.ticks);
        }
        assert_eq!(seen, [0, 1]);
        reader.rewind();
    }
    let mut reader = NativeReader::software(Cursor::new(bytes.as_slice()), 64 << 20).unwrap();
    for target in [40, 0, 40, 40] {
        let frame = reader
            .seek_raw(Duration::from_millis(target))
            .unwrap()
            .unwrap();
        let RawFrame::Planar(frame) = frame else {
            panic!("expected owned packed frame")
        };
        check(&frame.frame.data, if target == 0 { 0 } else { 1 });
    }
}

#[test]
fn signed_camera_precision_rounding_and_edges_match_independent_filter_oracle() {
    use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
    use std::{io::Cursor, path::Path};
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("texture-depth-refinement-camera.json")).unwrap(),
    )
    .unwrap();
    let filters: [[i64; 8]; 4] = [
        [0, 0, 0, 64, 0, 0, 0, 0],
        [-1, 4, -10, 58, 17, -5, 1, 0],
        [-1, 4, -11, 40, 40, -11, 4, -1],
        [0, 1, -5, 17, 58, -10, 4, -1],
    ];
    let mut phases = [false; 4];
    let mut signs = [false; 3];
    for case in manifest["cases"].as_array().unwrap() {
        let scale = case["scale"].as_i64().unwrap();
        let offset = case["offset"].as_i64().unwrap();
        let shift = 7 + case["precision"].as_u64().unwrap();
        let disparity = (scale * 254 + offset * 256 + (1i64 << (shift - 1))) >> shift;
        assert_eq!(disparity, case["disparity"].as_i64().unwrap());
        let phase = disparity.rem_euclid(4) as usize;
        phases[phase] = true;
        signs[(disparity.signum() + 1) as usize] = true;
        let name = case["file"].as_str().unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(std::fs::read(root.join(name)).unwrap()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut target_seen = false;
            while let Some(frame) = reader.read_frame().unwrap() {
                if frame.decoded.layer_id != 5 || frame.decoded.poc != 1 {
                    continue;
                }
                target_seen = true;
                for y in 0..32 {
                    for x in 0..32 {
                        let base = x as i64 + disparity.div_euclid(4);
                        let sum: i64 = filters[phase]
                            .iter()
                            .enumerate()
                            .map(|(i, &weight)| {
                                let xx = (base + i as i64 - 3).clamp(0, 31);
                                weight * ((3 * xx + 5 * y as i64 + 17) % 256)
                            })
                            .sum();
                        let expected = ((sum + 32) >> 6).clamp(0, 255) as u16;
                        assert_eq!(
                            frame.decoded.picture.planes[0].samples()[y * 32 + x],
                            expected,
                            "{name} x{x} y{y}"
                        );
                    }
                }
                for plane in &frame.decoded.picture.planes[1..] {
                    assert!(plane.samples().iter().all(|&v| v == 128));
                }
            }
            assert!(target_seen, "{name}");
            reader.rewind();
        }
    }
    assert!(
        phases.into_iter().all(|v| v),
        "all quarter-luma phases must be exercised"
    );
    assert!(
        signs.into_iter().all(|v| v),
        "negative/zero/positive disparities must be exercised"
    );
}

#[test]
fn refined_iv_uses_depth_shifted_source_motion_grid_and_preserves_base_disparity() {
    use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
    use std::{io::Cursor, path::Path};
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("texture-depth-refinement-iv.json")).unwrap(),
    )
    .unwrap();
    let mut changed = 0;
    for case in manifest["cases"].as_array().unwrap() {
        let cut = case["cut"].as_i64().unwrap();
        let disparity = case["disparity"].as_i64().unwrap();
        let name = case["file"].as_str().unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(std::fs::read(root.join(name)).unwrap()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for pass in 0..2 {
            let mut target_seen = false;
            while let Some(frame) = reader.read_frame().unwrap() {
                if frame.decoded.poc != 1 {
                    continue;
                }
                let id = frame.decoded.layer_id;
                if id != 0 && id != 5 {
                    continue;
                }
                target_seen |= id == 5;
                for y in 0..32 {
                    for x in 0..32 {
                        // IV: 8x8 block centre + rounded disparity, align to 8, then clip.
                        let source_x = if id == 0 {
                            x as i64
                        } else {
                            (((x / 8 * 8 + 4) as i64 + ((disparity + 2) >> 2)) >> 3 << 3)
                                .clamp(0, 31)
                        };
                        let dy = if source_x < cut { 1 } else { -1 };
                        let yy = (y as i64 + dy).clamp(0, 31);
                        let expected = ((3 * x as i64 + 5 * yy + 17) % 256) as u16;
                        assert_eq!(
                            frame.decoded.picture.planes[0].samples()[y * 32 + x],
                            expected,
                            "{name} layer{id} x{x} y{y}"
                        );
                        if id == 5
                            && pass == 0
                            && dy
                                != if ((x / 8 * 8 + 4) as i64) < cut {
                                    1
                                } else {
                                    -1
                                }
                        {
                            changed += 1;
                        }
                    }
                }
                for plane in &frame.decoded.picture.planes[1..] {
                    assert!(plane.samples().iter().all(|&v| v == 128));
                }
                if id == 5 {
                    let state = frame
                        .decoded
                        .picture
                        .inter_view_disparity_at([0, 0])
                        .unwrap()
                        .unwrap();
                    assert_eq!(state.vector.reference_view, 0);
                    assert_eq!(state.vector.mv, [0, 0]);
                    assert_eq!(state.refined_mv, [disparity, 0]);
                }
            }
            assert!(target_seen, "{name}");
            reader.rewind();
        }
        use fvid::playback::video::{
            mp4::Mp4VideoReader,
            native::{NativeReader, RawFrame},
        };
        let check = |data: &[u8], poc: i64| {
            assert_eq!(data.len(), 1536);
            for y in 0..32 {
                for x in 0..32 {
                    let yy = if poc == 0 {
                        y as i64
                    } else {
                        let source_x = ((((x / 8 * 8 + 4) as i64 + ((disparity + 2) >> 2)) >> 3)
                            << 3)
                            .clamp(0, 31);
                        (y as i64 + if source_x < cut { 1 } else { -1 }).clamp(0, 31)
                    };
                    assert_eq!(
                        data[y * 32 + x],
                        ((3 * x as i64 + 5 * yy + 17) % 256) as u8,
                        "{name} packed POC{poc} x{x} y{y}"
                    );
                }
            }
            assert!(data[1024..].iter().all(|&v| v == 128));
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
                panic!("{name}: planar frame required")
            };
            check(&frame.frame.data, if target == 0 { 0 } else { 1 });
        }
    }
    assert!(
        changed > 0,
        "refinement must change source motion selection, not just metadata"
    );
}

#[test]
fn colored_refinement_covers_all_chroma_phases_and_distinct_reference_planes() {
    use fvid::{
        container::mp4::Limits,
        playback::video::{
            mp4::{LayeredMp4VideoReader, Mp4VideoReader},
            native::{NativeReader, RawFrame},
        },
    };
    use std::{io::Cursor, path::Path, time::Duration};
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("texture-depth-refinement-colored.json")).unwrap(),
    )
    .unwrap();
    let chroma: [[i64; 4]; 8] = [
        [0, 64, 0, 0],
        [-2, 58, 10, -2],
        [-4, 54, 16, -2],
        [-6, 46, 28, -4],
        [-4, 36, 36, -4],
        [-4, 28, 46, -6],
        [-2, 16, 54, -4],
        [-2, 10, 58, -2],
    ];
    let luma: [[i64; 8]; 4] = [
        [0, 0, 0, 64, 0, 0, 0, 0],
        [-1, 4, -10, 58, 17, -5, 1, 0],
        [-1, 4, -11, 40, 40, -11, 4, -1],
        [0, 1, -5, 17, 58, -10, 4, -1],
    ];
    let mut phases = [false; 8];
    let mut distinct = 0;
    for case in manifest["cases"].as_array().unwrap() {
        let shift = 7 + case["precision"].as_u64().unwrap();
        let disparity = (case["scale"].as_i64().unwrap() * 254
            + case["offset"].as_i64().unwrap() * 256
            + (1 << (shift - 1)))
            >> shift;
        assert_eq!(disparity, case["disparity"].as_i64().unwrap());
        phases[disparity.rem_euclid(8) as usize] = true;
        let expected = |poc: i64, component: usize| -> Vec<u16> {
            let side = if component == 0 { 32 } else { 16 };
            let mut pixels = Vec::new();
            for y in 0..side {
                for x in 0..side {
                    let raw = |xx: i64| {
                        if component == 0 {
                            (3 * xx + 5 * y + 17) % 256
                        } else {
                            (13 * xx
                                + 7 * y
                                + 29
                                + 83 * (component as i64 - 1)
                                + if poc == 0 {
                                    [71, 113][component - 1]
                                } else {
                                    0
                                })
                                % 256
                        }
                    };
                    let value = if poc == 0 {
                        raw(x)
                    } else {
                        let (denom, left, weights): (i64, i64, &[i64]) = if component == 0 {
                            (4, 3, &luma[disparity.rem_euclid(4) as usize])
                        } else {
                            (8, 1, &chroma[disparity.rem_euclid(8) as usize])
                        };
                        let base = x + disparity.div_euclid(denom);
                        let sum: i64 = weights
                            .iter()
                            .enumerate()
                            .map(|(i, &w)| w * raw((base + i as i64 - left).clamp(0, side - 1)))
                            .sum();
                        ((sum + 32) >> 6).clamp(0, 255)
                    };
                    pixels.push(value as u16);
                }
            }
            pixels
        };
        let name = case["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(data.clone()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader.read_frame().unwrap() {
                if frame.decoded.layer_id != 5 {
                    continue;
                }
                let poc = i64::from(frame.decoded.poc);
                seen.push(poc);
                for component in 0..3 {
                    let wanted = expected(poc, component);
                    assert_eq!(
                        frame.decoded.picture.planes[component].samples(),
                        wanted,
                        "{name} component{component} POC{poc}"
                    );
                    if component > 0 && poc == 1 && wanted != expected(0, component) {
                        distinct += 1;
                    }
                }
            }
            assert_eq!(seen, [0, 1]);
            reader.rewind();
        }
        let check = |data: &[u8], poc| {
            let wanted: Vec<u8> = (0..3)
                .flat_map(|c| expected(poc, c))
                .map(|v| v as u8)
                .collect();
            assert_eq!(data, wanted, "{name} packed POC{poc}");
        };
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
                .seek_raw(Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar frame")
            };
            check(&frame.frame.data, if target == 0 { 0 } else { 1 });
        }
    }
    assert!(
        phases.into_iter().all(|p| p),
        "every eighth-chroma phase required"
    );
    assert!(
        distinct > 0,
        "copying own target chroma must differ from correct DI"
    );
}

#[test]
fn colored_refined_iv_filters_own_reference_chroma_after_depth_motion_lookup() {
    use fvid::{
        container::mp4::Limits,
        playback::video::{
            mp4::{LayeredMp4VideoReader, Mp4VideoReader},
            native::{NativeReader, RawFrame},
        },
    };
    use std::{io::Cursor, path::Path, time::Duration};
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("texture-depth-refinement-colored-iv.json")).unwrap(),
    )
    .unwrap();
    let mut differences = 0;
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let cut = case["cut"].as_i64().unwrap();
        let disparity = case["disparity"].as_i64().unwrap();
        let expected = |poc: i64, component: usize| -> Vec<u16> {
            let side = if component == 0 { 32 } else { 16 };
            let mut out = Vec::new();
            for y in 0..side {
                for x in 0..side {
                    let raw = |yy: i64| {
                        if component == 0 {
                            (3 * x + 5 * yy + 17) % 256
                        } else {
                            (13 * x
                                + 7 * yy
                                + 29
                                + 83 * (component as i64 - 1)
                                + [71, 113][component - 1])
                                % 256
                        }
                    };
                    let value = if poc == 0 {
                        raw(y)
                    } else {
                        let luma_x = if component == 0 { x } else { x * 2 };
                        let source_x = ((((luma_x / 8 * 8 + 4) + ((disparity + 2) >> 2)) >> 3)
                            << 3)
                            .clamp(0, 31);
                        let dy = if source_x < cut { 1 } else { -1 };
                        if component == 0 {
                            raw((y + dy).clamp(0, 31))
                        } else {
                            // +/-4 eighth-chroma samples: same phase 4, different floor.
                            let base = y + if dy < 0 { -1 } else { 0 };
                            let sum: i64 = [-4, 36, 36, -4]
                                .iter()
                                .enumerate()
                                .map(|(i, &w)| w * raw((base + i as i64 - 1).clamp(0, 15)))
                                .sum();
                            ((sum + 32) >> 6).clamp(0, 255)
                        }
                    };
                    out.push(value as u16);
                }
            }
            out
        };
        let check = |data: &[u8], poc| {
            let wanted: Vec<u8> = (0..3)
                .flat_map(|c| expected(poc, c))
                .map(|v| v as u8)
                .collect();
            assert_eq!(data, wanted, "{name} packed POC{poc}");
        };
        let data = std::fs::read(root.join(name)).unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(data.clone()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader.read_frame().unwrap() {
                if frame.decoded.layer_id != 5 {
                    continue;
                }
                let poc = i64::from(frame.decoded.poc);
                seen.push(poc);
                for component in 0..3 {
                    let wanted = expected(poc, component);
                    assert_eq!(
                        frame.decoded.picture.planes[component].samples(),
                        wanted,
                        "{name} c{component} POC{poc}"
                    );
                    if poc == 1 && component > 0 && wanted != expected(0, component) {
                        differences += 1;
                    }
                }
            }
            assert_eq!(seen, [0, 1]);
            reader.rewind();
        }
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
                .seek_raw(Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar frame")
            };
            check(&frame.frame.data, if target == 0 { 0 } else { 1 });
        }
    }
    assert!(
        differences > 0,
        "fractional chroma prediction must change target reference pixels"
    );
}
