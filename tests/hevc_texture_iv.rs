use fvid::{
    codec::hevc_motion::{
        self, Reference, Vector, inter_view_candidate, texture::Identity, view_synthesis,
    },
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::{io::Cursor, path::Path};

fn fixtures() -> (std::path::PathBuf, serde_json::Value, Vec<u16>) {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest =
        serde_json::from_slice(&std::fs::read(root.join("texture-iv-manifest.json")).unwrap())
            .unwrap();
    let bytes = std::fs::read(root.join("texture-iv-oracles.y16")).unwrap();
    let pixels = bytes
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
        .collect();
    (root, manifest, pixels)
}

fn verify_playback(enabled: bool) {
    let (root, manifest, golden) = fixtures();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 30);
    for case in cases
        .iter()
        .filter(|v| v["enabled"].as_bool() == Some(enabled))
    {
        let name = case["file"].as_str().unwrap();
        let intra = case["intra"].as_bool().unwrap();
        let index = case["index"].as_u64().unwrap();
        let top = case["top"].as_u64().unwrap() as u32;
        let offset = case["offset"].as_u64().unwrap() as usize;
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(std::fs::read(root.join(name)).unwrap()),
            Default::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        for _ in 0..2 {
            let mut seen = Vec::new();
            let mut past = None;
            let mut source = None;
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let id = frame.decoded.layer_id;
                let poc = frame.decoded.poc;
                let picture = frame.decoded.picture;
                seen.push((id, poc));
                if id == 5 && poc == 0 {
                    past = Some(picture.clone());
                }
                if id == 3 && poc == 1 {
                    for y in 0..32usize {
                        for x in 0..32usize {
                            let shift = if intra {
                                0
                            } else if y < top as usize {
                                1
                            } else {
                                -1
                            };
                            let row = (y as i32 + shift).clamp(0, 31) as usize;
                            assert_eq!(
                                picture.planes[0].samples()[y * 32 + x],
                                ((3 * x + 5 * row + 17) % 256) as u16,
                                "{name}: source{x},{y}"
                            );
                            let motion =
                                picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                            if intra {
                                assert_eq!(motion, [None; 2]);
                            } else {
                                assert_eq!(motion[0].unwrap().mv, [0, i64::from(shift) * 4]);
                            }
                        }
                    }
                    source = Some(picture.clone());
                }
                if id != 5 || poc != 1 {
                    continue;
                }
                let is_iv = enabled && !intra && (index == 0 || top == 24 && index == 2);
                for y in (0..32u32).step_by(8) {
                    for x in (0..32u32).step_by(8) {
                        let state = picture.inter_view_disparity_at([x, y]).unwrap();
                        assert_eq!(state.is_some(), is_iv, "{name}: published IV flag");
                        if let Some(state) = state {
                            assert_eq!(state.vector.reference_view, 1);
                            assert_eq!(state.vector.mv, [0, 0]);
                            assert_eq!(state.refined_mv, [0, 0]);
                        }
                        let motion = picture.prediction_motion_at([x, y]).unwrap();
                        let vector = motion[0].unwrap();
                        let reference = if enabled {
                            if top == 24 {
                                if index < 3 || index == 4 { 1 } else { 0 }
                            } else {
                                if index < 2 || index == 3 { 1 } else { 0 }
                            }
                        } else {
                            u8::from(index == 1)
                        };
                        assert_eq!(vector.reference, reference, "{name}: saved reference");
                        let mv = if is_iv {
                            [
                                0,
                                if index == 2 {
                                    -4
                                } else if y < top {
                                    4
                                } else {
                                    -4
                                },
                            ]
                        } else if enabled && intra && index == 1 {
                            [4, 0]
                        } else {
                            [0, 0]
                        };
                        assert_eq!(vector.mv, mv, "{name}: saved motion at{x},{y}");
                        assert!(motion[1].is_none());
                    }
                }
                assert_eq!(
                    picture.planes[0].samples(),
                    &golden[offset..offset + 1024],
                    "{name}: native playback"
                );
                if !enabled && index == 0 {
                    let source = source.as_ref().unwrap();
                    let references = [
                        vec![
                            Reference {
                                layer_id: 5,
                                poc: 0,
                                long_term: false,
                                picture: past.clone(),
                            },
                            Reference {
                                layer_id: 3,
                                poc: 1,
                                long_term: true,
                                picture: Some(source.clone()),
                            },
                        ],
                        vec![],
                    ];
                    let active = cases
                        .iter()
                        .find(|v| {
                            v["enabled"].as_bool() == Some(true)
                                && v["intra"].as_bool() == Some(intra)
                                && v["index"].as_u64() == Some(0)
                                && v["top"].as_u64() == Some(u64::from(top))
                        })
                        .unwrap();
                    let expected = active["offset"].as_u64().unwrap() as usize;
                    let blocks = inter_view_candidate::subblocks(
                        source,
                        [0, 0, 32, 32],
                        [32, 32],
                        3,
                        |_| Ok([0, 0]),
                        [&[0, 1], &[]],
                        false,
                        1 << 20,
                    )
                    .unwrap();
                    assert_eq!(blocks.default == [None; 2], intra);
                    let identities = [Identity { poc: 0, view: 2 }, Identity { poc: 1, view: 1 }];
                    let (di, shifted) =
                        view_synthesis::disparity_information([&identities, &[]], false, 1, 1, 0)
                            .unwrap();
                    assert_eq!(di[0].unwrap().reference, 1);
                    assert_eq!(di[0].unwrap().mv, [0, 0]);
                    assert_eq!(shifted[0].unwrap().mv, [4, 0]);
                    let mut actual = vec![0u16; 1024];
                    if intra {
                        actual = hevc_motion::predict(
                            &references,
                            [
                                Some(Vector {
                                    reference: 1,
                                    mv: [0, 0],
                                }),
                                None,
                            ],
                            [0, 0, 32, 32],
                            0,
                            8,
                            None,
                            &mut Vec::new(),
                        )
                        .unwrap();
                        let shifted_pixels = hevc_motion::predict(
                            &references,
                            [
                                Some(Vector {
                                    reference: 1,
                                    mv: [4, 0],
                                }),
                                None,
                            ],
                            [0, 0, 32, 32],
                            0,
                            8,
                            None,
                            &mut Vec::new(),
                        )
                        .unwrap();
                        let shifted_case = cases
                            .iter()
                            .find(|v| {
                                v["enabled"].as_bool() == Some(true)
                                    && v["intra"].as_bool() == Some(true)
                                    && v["index"].as_u64() == Some(1)
                            })
                            .unwrap();
                        let start = shifted_case["offset"].as_u64().unwrap() as usize;
                        assert_eq!(
                            shifted_pixels,
                            &golden[start..start + 1024],
                            "{name}: DIShift pixels"
                        );
                    } else {
                        assert_eq!(blocks.default[0].unwrap().reference, 1);
                        assert_eq!(
                            blocks.default[0].unwrap().mv,
                            [0, if top == 24 { 4 } else { -4 }]
                        );
                        let flat = hevc_motion::predict(
                            &references,
                            blocks.default,
                            [0, 0, 32, 32],
                            0,
                            8,
                            None,
                            &mut Vec::new(),
                        )
                        .unwrap();
                        assert_ne!(
                            flat,
                            &golden[expected..expected + 1024],
                            "{name}: fixture must distinguish flattened IV motion"
                        );
                        let shifted_iv = inter_view_candidate::from_picture(
                            source,
                            [0, 0],
                            [32, 32],
                            [0, 0],
                            [64, 64],
                            [&[0, 1], &[]],
                            false,
                        )
                        .unwrap();
                        if top == 24 {
                            assert_ne!(shifted_iv, blocks.default);
                        } else {
                            assert_eq!(
                                shifted_iv, blocks.default,
                                "IVShift available/equal still suppresses DIShift"
                            );
                        }
                        for (i, &motion) in blocks.motion.iter().enumerate() {
                            let x = (i % 4) * 8;
                            let y = (i / 4) * 8;
                            assert_eq!(
                                motion[0].unwrap().mv,
                                [0, if y < top as usize { 4 } else { -4 }]
                            );
                            let pixels = hevc_motion::predict(
                                &references,
                                motion,
                                [x as u32, y as u32, 8, 8],
                                0,
                                8,
                                None,
                                &mut Vec::new(),
                            )
                            .unwrap();
                            for row in 0..8 {
                                actual[(y + row) * 32 + x..(y + row) * 32 + x + 8]
                                    .copy_from_slice(&pixels[row * 8..row * 8 + 8]);
                            }
                        }
                    }
                    assert_eq!(
                        actual,
                        &golden[expected..expected + 1024],
                        "{name}: component oracle differs"
                    );
                }
            }
            assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
            reader.rewind();
        }
        use fvid::playback::video::{
            mp4::Mp4VideoReader,
            native::{NativeReader, RawFrame},
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
                seen.push(poc);
                let packed = frame.packed.unwrap();
                let expected: Vec<u8> = if poc == 1 {
                    golden[offset..offset + 1024]
                        .iter()
                        .map(|&v| v as u8)
                        .collect()
                } else {
                    (0..32)
                        .flat_map(|y| (0..32).map(move |x| ((x * 3 + y * 5 + 17) % 256) as u8))
                        .collect()
                };
                assert_eq!(
                    &packed.frame.data[..1024],
                    expected,
                    "{name}: common player POC{poc}"
                );
                assert!(packed.frame.data[1024..].iter().all(|&v| v == 128));
            }
            assert_eq!(seen, [0, 1]);
            common.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 40, 40] {
            let frame = native
                .seek_raw(std::time::Duration::from_millis(target))
                .unwrap()
                .unwrap();
            if target != 40 {
                continue;
            }
            let RawFrame::Planar(packed) = frame else {
                panic!("{name}: expected planar frame")
            };
            assert_eq!(
                &packed.frame.data[..1024],
                golden[offset..offset + 1024]
                    .iter()
                    .map(|&v| v as u8)
                    .collect::<Vec<_>>(),
                "{name}: repeated native seek"
            );
        }
    }
}

#[test]
fn texture_iv_disabled_controls_and_component_pixels_are_valid() {
    verify_playback(false);
}

#[test]
fn texture_iv_active_sps_and_reference_lists_are_admitted() {
    use fvid::codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredParameters},
        hevc_poc::LayeredPocState,
    };
    let (root, manifest, _) = fixtures();
    for case in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["enabled"].as_bool() == Some(true))
    {
        let name = case["file"].as_str().unwrap();
        let mut input = fvid::container::mp4::Mp4Reader::open(
            Cursor::new(std::fs::read(root.join(name)).unwrap()),
            Default::default(),
        )
        .unwrap();
        let params =
            LayeredParameters::from_configuration(&input.tracks()[0].configuration, 64 << 20)
                .unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let first = LayeredAccessUnit::parse(
            &params,
            &LayeredPocState::default(),
            &packet,
            4,
            1,
            64 << 20,
        )
        .unwrap();
        let (params, transition, _, _) = first.into_parts();
        let state = transition.unwrap().into_state();
        input.read_packet(0, 1, &mut packet).unwrap();
        let second = LayeredAccessUnit::parse(&params, &state, &packet, 4, 1, 64 << 20).unwrap();
        let current = second
            .pictures()
            .iter()
            .find(|p| p.binding.sps.layer_id == 5)
            .unwrap();
        let tools = current.binding.sps.sps.three_d.as_ref().unwrap();
        assert!(tools.inter_view_disparity[0]);
        assert_eq!(tools.disparity_sub_block_log2, 3);
        assert_eq!(current.slices[0].references, [2, 0]);
        assert_eq!(
            current.slices[0].max_merge_candidates, 5,
            "{name}: I-35/I-36"
        );
        assert_eq!(
            current.slices[0].layered.as_ref().unwrap().reference_layers,
            [3]
        );
    }
}

#[test]
fn texture_iv_native_acceptance_matches_owned_pixels() {
    verify_playback(true);
}
